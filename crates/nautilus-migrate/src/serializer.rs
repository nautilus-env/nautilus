//! Serializer: converts a [`LiveSchema`] snapshot into canonical `.nautilus` source text.
//!
//! Used by `nautilus db pull` to introspect an existing database and emit a
//! schema file that can be fed back into `db push`.

use std::collections::{HashMap, HashSet};

use crate::live::LiveIndexKind;
use crate::{
    ddl::DatabaseProvider,
    live::{ComputedKind, LiveCompositeType, LiveForeignKey, LiveSchema, LiveTable},
};
use nautilus_core::TableName;
use nautilus_schema::ir::{BasicIndexType, PgvectorIndexOptions};
use nautilus_schema::{
    bool_expr::{parse_bool_expr, BoolExpr, Operand},
    sql_expr::{parse_sql_expr, SqlExpr},
    Lexer, Span, Token, TokenKind,
};

/// Naming mode for identifiers emitted by `db pull`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PullNameCase {
    /// Preserve the current serializer behaviour.
    #[default]
    Auto,
    /// Render identifiers in `snake_case`.
    Snake,
    /// Render identifiers in `PascalCase`.
    Pascal,
}

/// Naming options used when serialising an introspected schema.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PullNamingOptions {
    /// Logical model name rendering mode.
    pub model_case: PullNameCase,
    /// Logical field name rendering mode.
    pub field_case: PullNameCase,
}

#[derive(Debug, Clone)]
struct ForwardRelation {
    fk_index: usize,
    field_name: String,
    relation_name: Option<String>,
}

#[derive(Debug, Clone)]
struct BackRelation {
    owning_table: TableName,
    field_name: String,
    relation_name: Option<String>,
    is_one_to_one: bool,
}

/// One end of an implicit many-to-many recovered from the live database.
struct ManyToManyEnd {
    /// The live table on the other side of the join.
    target_table: TableName,
    field_name: String,
    /// Set when the join table's name does not spell the default
    /// `_<A model>To<B model>` for this pair, so the schema has to say which
    /// relation it is.
    relation_name: Option<String>,
}

#[derive(Debug, Clone)]
struct TableNamingContext {
    model_name: String,
    db_to_logical_field: HashMap<String, String>,
    logical_field_order: Vec<String>,
}

/// Convert a [`LiveSchema`] to a `.nautilus` schema source string.
///
/// * `live` - the introspected live database schema
/// * `provider` - which SQL dialect was used during introspection
/// * `url` - what to write as the datasource `url`: an `env("NAME")` reference
///   is emitted as-is, anything else is quoted as a literal connection string
pub fn serialize_live_schema(live: &LiveSchema, provider: DatabaseProvider, url: &str) -> String {
    serialize_live_schema_with_options(live, provider, url, PullNamingOptions::default())
}

/// Convert a [`LiveSchema`] to a `.nautilus` schema source string using custom
/// logical naming options for models and fields.
pub fn serialize_live_schema_with_options(
    live: &LiveSchema,
    provider: DatabaseProvider,
    url: &str,
    options: PullNamingOptions,
) -> String {
    // MySQL has no CREATE TYPE: an enum lives inline in the column type. Lift
    // those into real enum declarations so a pulled schema round-trips instead
    // of degrading every enum column to a plain string.
    let lifted = lift_inline_enums(live);
    let live = lifted.as_ref().unwrap_or(live);

    let mut parts: Vec<String> = vec![render_datasource_block(live, provider, url)];

    let mut composite_names: Vec<&String> = live.composite_types.keys().collect();
    composite_names.sort();
    for db_name in composite_names {
        parts.push(render_composite_type_block(live, db_name, options));
    }

    let mut enum_names: Vec<&String> = live.enums.keys().collect();
    enum_names.sort();
    for db_name in enum_names {
        parts.push(render_enum_block(db_name, &live.enums[db_name]));
    }

    let mut table_names: Vec<&TableName> = live.tables.keys().collect();
    table_names.sort();
    let mut view_names: Vec<&TableName> = live.views.keys().collect();
    view_names.sort();

    // A join table is the storage of a relation, not a model: it comes back as
    // an array field on each of the two models it links.
    let joins = find_join_tables(live, &table_names);
    table_names.retain(|name| !joins.iter().any(|join| join.name == **name));

    let table_naming = build_table_naming_contexts(live, &table_names, &view_names, options);
    let relation_pair_counts = build_relation_pair_counts(live, &table_names);
    let directional_relation_counts = build_directional_relation_counts(live, &table_names);
    let forward_relations = build_forward_relations(
        live,
        &table_names,
        &table_naming,
        &relation_pair_counts,
        options,
    );
    let back_relations = build_back_relations(
        live,
        &table_names,
        &table_naming,
        &forward_relations,
        &directional_relation_counts,
        options,
    );

    let many_to_many = build_many_to_many_ends(
        &joins,
        &table_naming,
        &forward_relations,
        &back_relations,
        options,
    );

    for table_name in &table_names {
        parts.push(render_model_block(
            live,
            table_name,
            &table_naming,
            slice_for(&forward_relations, table_name),
            slice_for(&back_relations, table_name),
            slice_for(&many_to_many, table_name),
        ));
    }

    for view_name in &view_names {
        parts.push(render_view_block(live, view_name, &table_naming));
    }

    let mut out = parts.join("\n\n");
    if !out.ends_with('\n') {
        out.push('\n');
    }
    out
}

/// A live table that is the join table of an implicit many-to-many.
struct JoinTable {
    name: TableName,
    /// The table the `A` column points at.
    a_table: TableName,
    /// The table the `B` column points at.
    b_table: TableName,
}

/// Recognise the join tables among the live ones.
///
/// The shape is the one Nautilus creates and nothing else plausibly is: a name
/// starting with `_`, exactly the two required columns `A` and `B`, a primary
/// key over the pair, and one foreign key per column. Recovering them is what
/// makes `db pull` return the schema that produced the database rather than a
/// second, explicit spelling of it.
fn find_join_tables(live: &LiveSchema, table_names: &[&TableName]) -> Vec<JoinTable> {
    let mut joins: Vec<JoinTable> = table_names
        .iter()
        .filter_map(|name| {
            let table = &live.tables[*name];
            if !name.name.starts_with('_') || table.primary_key != ["A", "B"] {
                return None;
            }
            let columns: Vec<&str> = table.columns.iter().map(|c| c.name.as_str()).collect();
            if columns != ["A", "B"] || table.columns.iter().any(|column| column.nullable) {
                return None;
            }

            let referenced = |column: &str| {
                table
                    .foreign_keys
                    .iter()
                    .find(|fk| fk.columns == [column])
                    .filter(|fk| live.tables.contains_key(&fk.referenced_table))
                    .map(|fk| fk.referenced_table.clone())
            };
            if table.foreign_keys.len() != 2 {
                return None;
            }

            Some(JoinTable {
                name: (*name).clone(),
                a_table: referenced("A")?,
                b_table: referenced("B")?,
            })
        })
        .collect();
    joins.sort_by(|a, b| a.name.cmp(&b.name));
    joins
}

/// Build the array relation field each side of a recovered many-to-many needs.
///
/// Field names are chosen against the names the model already carries, so a
/// recovered relation never collides with a column or another relation.
fn build_many_to_many_ends(
    joins: &[JoinTable],
    table_naming: &HashMap<TableName, TableNamingContext>,
    forward_relations: &HashMap<TableName, Vec<ForwardRelation>>,
    back_relations: &HashMap<TableName, Vec<BackRelation>>,
    options: PullNamingOptions,
) -> HashMap<TableName, Vec<ManyToManyEnd>> {
    let mut used_fields: HashMap<&TableName, HashSet<String>> = HashMap::new();
    let mut result: HashMap<TableName, Vec<ManyToManyEnd>> = HashMap::new();

    for (table_name, naming) in table_naming {
        let mut used: HashSet<String> = naming.logical_field_order.iter().cloned().collect();
        used.extend(
            slice_for(forward_relations, table_name)
                .iter()
                .map(|relation| relation.field_name.clone()),
        );
        used.extend(
            slice_for(back_relations, table_name)
                .iter()
                .map(|relation| relation.field_name.clone()),
        );
        used_fields.insert(table_name, used);
    }

    for join in joins {
        let Some(a_naming) = table_naming.get(&join.a_table) else {
            continue;
        };
        let Some(b_naming) = table_naming.get(&join.b_table) else {
            continue;
        };

        let default_name = format!("_{}To{}", a_naming.model_name, b_naming.model_name);
        let relation_name =
            (join.name != default_name).then(|| join.name.name.trim_start_matches('_').to_string());

        for (owner, target, target_model) in [
            (&join.a_table, &join.b_table, &b_naming.model_name),
            (&join.b_table, &join.a_table, &a_naming.model_name),
        ] {
            let used = used_fields
                .get_mut(owner)
                .expect("every live table has a naming context");
            let base = apply_derived_field_case(
                &pluralize_name(&to_snake_case_identifier(&singular_name(target_model))),
                options.field_case,
            );
            let field_name = choose_unique_field_name(vec![base], used);
            result
                .entry(owner.clone())
                .or_default()
                .push(ManyToManyEnd {
                    target_table: target.clone(),
                    field_name,
                    relation_name: relation_name.clone(),
                });
        }
    }

    result
}

fn render_many_to_many_lines(
    table_naming: &HashMap<TableName, TableNamingContext>,
    ends: &[ManyToManyEnd],
) -> Vec<String> {
    ends.iter()
        .map(|end| {
            let target_model = &table_naming[&end.target_table].model_name;
            match &end.relation_name {
                Some(relation_name) => format!(
                    "  {}  {}[]  @relation(name: \"{}\")",
                    end.field_name,
                    target_model,
                    escape_schema_string(relation_name)
                ),
                None => format!("  {}  {}[]", end.field_name, target_model),
            }
        })
        .collect()
}

fn slice_for<'a, T>(map: &'a HashMap<TableName, Vec<T>>, table_name: &TableName) -> &'a [T] {
    map.get(table_name).map(Vec::as_slice).unwrap_or(&[])
}

fn render_composite_type_block(
    live: &LiveSchema,
    db_name: &str,
    options: PullNamingOptions,
) -> String {
    let composite = &live.composite_types[db_name];
    let mut used_field_names = HashSet::new();
    let fields: Vec<(String, &crate::live::LiveCompositeField)> = composite
        .fields
        .iter()
        .map(|field| {
            let logical_name = choose_unique_field_name(
                vec![apply_scalar_field_case(&field.name, options.field_case)],
                &mut used_field_names,
            );
            (logical_name, field)
        })
        .collect();
    let max_name = fields
        .iter()
        .map(|(logical_name, _)| logical_name.len())
        .max()
        .unwrap_or(0);

    let mut lines = vec![format!("type {} {{", to_pascal_case(db_name))];
    for (logical_name, field) in &fields {
        let nautilus_type =
            infer_nautilus_type(&field.col_type, &live.enums, &live.composite_types);
        if logical_name == &field.name {
            lines.push(format!(
                "  {:<name_w$}  {}",
                logical_name,
                nautilus_type,
                name_w = max_name,
            ));
        } else {
            lines.push(format!(
                "  {:<name_w$}  {}  @map(\"{}\")",
                logical_name,
                nautilus_type,
                escape_schema_string(&field.name),
                name_w = max_name,
            ));
        }
    }
    // Keep @@map explicit so the type/SQL-type mapping survives round-trips.
    lines.push(format!("  @@map(\"{}\")", escape_schema_string(db_name)));
    lines.push("}".to_string());
    lines.join("\n")
}

/// Parse the variants out of a MySQL inline column enum, `enum('A','B')`.
///
/// Returns `None` for any other column type. A quote inside a variant is
/// doubled by MySQL, matching the escaping the DDL generator emits.
fn parse_inline_enum_variants(col_type: &str) -> Option<Vec<String>> {
    let inner = col_type
        .trim()
        .strip_prefix("enum(")
        .or_else(|| col_type.trim().strip_prefix("ENUM("))?
        .strip_suffix(')')?;

    let mut variants = Vec::new();
    let mut current = String::new();
    let mut chars = inner.chars().peekable();
    let mut in_quotes = false;

    while let Some(ch) = chars.next() {
        match ch {
            '\'' if in_quotes && chars.peek() == Some(&'\'') => {
                current.push('\'');
                chars.next();
            }
            '\'' => {
                if in_quotes {
                    variants.push(std::mem::take(&mut current));
                }
                in_quotes = !in_quotes;
            }
            _ if in_quotes => current.push(ch),
            _ => {}
        }
    }

    (!variants.is_empty() && !in_quotes).then_some(variants)
}

/// Promote every inline column enum to a named entry in [`LiveSchema::enums`].
///
/// Columns sharing a variant list share one declaration, named after the first
/// table and column that introduce it, so the pulled schema does not repeat the
/// same enum once per column. Returns `None` when there is nothing to lift, so
/// the common case does not pay for a clone.
fn lift_inline_enums(live: &LiveSchema) -> Option<LiveSchema> {
    let mut lifted: Vec<(String, Vec<String>)> = Vec::new();

    let mut table_names: Vec<&TableName> = live.tables.keys().collect();
    table_names.sort();
    for table_name in table_names {
        let table = &live.tables[table_name];
        for column in &table.columns {
            let Some(variants) = parse_inline_enum_variants(&column.col_type) else {
                continue;
            };
            if lifted.iter().any(|(_, known)| *known == variants) {
                continue;
            }
            lifted.push((format!("{}_{}", table_name, column.name), variants));
        }
    }

    if lifted.is_empty() {
        return None;
    }

    let mut augmented = live.clone();
    augmented.enums.extend(lifted);
    Some(augmented)
}

fn render_enum_block(db_name: &str, variants: &[String]) -> String {
    let mut lines = vec![format!("enum {} {{", to_pascal_case(db_name))];
    for variant in variants {
        lines.push(format!("  {}", variant));
    }
    lines.push("}".to_string());
    lines.join("\n")
}

fn render_model_block(
    live: &LiveSchema,
    table_name: &TableName,
    table_naming: &HashMap<TableName, TableNamingContext>,
    forward_relations: &[ForwardRelation],
    back_relations: &[BackRelation],
    many_to_many: &[ManyToManyEnd],
) -> String {
    let table = &live.tables[table_name];
    let naming = &table_naming[table_name];

    let mut lines = vec![format!("model {} {{", naming.model_name)];
    lines.extend(render_column_lines(live, table, naming));
    lines.extend(render_forward_relation_lines(
        table,
        naming,
        table_naming,
        forward_relations,
    ));
    lines.extend(render_back_relation_lines(table_naming, back_relations));
    lines.extend(render_many_to_many_lines(table_naming, many_to_many));

    if table.primary_key.len() > 1 {
        lines.push(format!(
            "  @@id([{}])",
            join_logical_fields(naming, &table.primary_key)
        ));
    }

    // Keep @@map explicit so the model/table mapping survives round-trips.
    lines.push(format!("  @@map(\"{}\")", table_name.name));
    if let Some(schema) = table_name.schema() {
        lines.push(format!("  @@schema(\"{}\")", escape_schema_string(schema)));
    }
    if table_is_unmodellable(live, table) {
        lines.push("  @@ignore".to_string());
    }
    let unmodellable = unmodellable_columns(live, table);
    lines.extend(render_index_lines(table_name, table, naming, &unmodellable));

    for check in &table.check_constraints {
        if mentions_unmodellable_column(check, &unmodellable) {
            continue;
        }
        lines.push(format!(
            "  @@check({})",
            remap_bool_expr_identifiers(check, &naming.db_to_logical_field)
        ));
    }

    lines.push("}".to_string());
    lines.join("\n")
}

/// Render a `view` block for one introspected view.
///
/// A view has no key, index, constraint or foreign key of its own, so the block
/// carries its columns and the `@@map` that ties it back to the database name.
fn render_view_block(
    live: &LiveSchema,
    view_name: &TableName,
    table_naming: &HashMap<TableName, TableNamingContext>,
) -> String {
    let view = &live.views[view_name];
    let naming = &table_naming[view_name];

    let mut lines = vec![format!("view {} {{", naming.model_name)];
    lines.extend(render_column_lines(live, view, naming));
    lines.push(format!("  @@map(\"{}\")", view_name.name));
    if let Some(schema) = view_name.schema() {
        lines.push(format!("  @@schema(\"{}\")", escape_schema_string(schema)));
    }
    if !unmodellable_columns(live, view).is_empty() {
        lines.push("  @@ignore".to_string());
    }
    lines.push("}".to_string());
    lines.join(
        "
",
    )
}

/// The DB names of the columns of `table` that Nautilus cannot model, and so
/// marks `@ignore`.
fn unmodellable_columns<'a>(live: &LiveSchema, table: &'a LiveTable) -> HashSet<&'a str> {
    table
        .columns
        .iter()
        .filter(|column| {
            !is_representable_type(&column.col_type, &live.enums, &live.composite_types)
        })
        .map(|column| column.name.as_str())
        .collect()
}

/// Whether a pulled table is unusable rather than merely incomplete, and so has
/// to carry `@@ignore`.
///
/// Two cases qualify. A `NOT NULL` column with no default that Nautilus cannot
/// model makes every generated `create` fail at the database, and a primary key
/// it cannot model leaves the model with no way to address a row. Both are also
/// what the validator demands: an `@ignore`d required field without a default,
/// or an `@ignore`d key field, is an error unless the model itself is ignored.
fn table_is_unmodellable(live: &LiveSchema, table: &LiveTable) -> bool {
    let unmodellable = unmodellable_columns(live, table);
    if unmodellable.is_empty() {
        return false;
    }

    if table
        .primary_key
        .iter()
        .any(|column| unmodellable.contains(column.as_str()))
    {
        return true;
    }

    table.columns.iter().any(|column| {
        unmodellable.contains(column.name.as_str())
            && !column.nullable
            && column.default_value.is_none()
            && column.generated_expr.is_none()
            && !column.auto_increment
    })
}

fn render_column_lines(
    live: &LiveSchema,
    table: &LiveTable,
    naming: &TableNamingContext,
) -> Vec<String> {
    let is_composite_pk = table.primary_key.len() > 1;
    let max_name = naming
        .logical_field_order
        .iter()
        .map(|name| name.len())
        .max()
        .unwrap_or(0);
    let max_type = table
        .columns
        .iter()
        .map(|column| {
            let rendered = render_column_type(live, column);
            rendered.len()
        })
        .max()
        .unwrap_or(0);

    let mut lines = Vec::with_capacity(table.columns.len());
    for (index, col) in table.columns.iter().enumerate() {
        let logical_field_name = &naming.logical_field_order[index];
        let type_with_mod = render_column_type(live, col);

        let mut attrs: Vec<String> = Vec::new();
        if table.primary_key.contains(&col.name) && !is_composite_pk {
            attrs.push("@id".to_string());
        }
        if let (Some(expr), Some(kind)) = (&col.generated_expr, &col.computed_kind) {
            let kind_str = match kind {
                ComputedKind::Stored => "Stored",
                ComputedKind::Virtual => "Virtual",
            };
            attrs.push(format!(
                "@computed({}, {})",
                remap_sql_expr_identifiers(expr, &naming.db_to_logical_field),
                kind_str
            ));
        } else if col.auto_increment && can_infer_autoincrement(&col.col_type) {
            // MySQL keeps AUTO_INCREMENT on the column and leaves COLUMN_DEFAULT
            // empty, so there is no default expression to infer it from the way
            // PostgreSQL's `nextval(...)` allows.
            attrs.push("@default(autoincrement())".to_string());
        } else if let Some(def) = &col.default_value {
            if let Some(attr) = infer_default_attr(def, &col.col_type, &live.enums) {
                attrs.push(attr);
            }
        }
        let is_unmodellable =
            !is_representable_type(&col.col_type, &live.enums, &live.composite_types);
        if let Some(check) = &col.check_expr {
            // A constraint on an `@ignore`d column is not Nautilus's to model:
            // the schema would name a column the client does not have, and the
            // diff already leaves such live constraints alone.
            if !is_unmodellable {
                attrs.push(format!(
                    "@check({})",
                    remap_bool_expr_identifiers(check, &naming.db_to_logical_field)
                ));
            }
        }
        if logical_field_name != &col.name {
            attrs.push(format!("@map(\"{}\")", escape_schema_string(&col.name)));
        }
        if is_unmodellable {
            attrs.push("@ignore".to_string());
        }

        let line = if attrs.is_empty() {
            format!("  {}  {}", logical_field_name, type_with_mod)
        } else {
            format!(
                "  {:<name_w$}  {:<type_w$}  {}",
                logical_field_name,
                type_with_mod,
                attrs.join("  "),
                name_w = max_name,
                type_w = max_type,
            )
        };
        lines.push(line.trim_end().to_string());
    }
    lines
}

fn render_column_type(live: &LiveSchema, column: &crate::live::LiveColumn) -> String {
    let type_str = infer_nautilus_type(&column.col_type, &live.enums, &live.composite_types);
    if column.nullable && type_supports_optional_modifier(&type_str) {
        format!("{}?", type_str)
    } else {
        type_str
    }
}

fn render_forward_relation_lines(
    table: &LiveTable,
    naming: &TableNamingContext,
    table_naming: &HashMap<TableName, TableNamingContext>,
    forward_relations: &[ForwardRelation],
) -> Vec<String> {
    let mut lines = Vec::with_capacity(forward_relations.len());
    for relation in forward_relations {
        let fk = &table.foreign_keys[relation.fk_index];
        let ref_model = &table_naming[&fk.referenced_table].model_name;

        let is_nullable = fk.columns.iter().any(|col_name| {
            table
                .columns
                .iter()
                .find(|c| &c.name == col_name)
                .map(|c| c.nullable)
                .unwrap_or(true)
        });
        let type_str = if is_nullable {
            format!("{}?", ref_model)
        } else {
            ref_model.clone()
        };

        let mut rel_args: Vec<String> = Vec::new();
        if let Some(relation_name) = &relation.relation_name {
            rel_args.push(format!("name: \"{}\"", escape_schema_string(relation_name)));
        }
        rel_args.push(format!(
            "fields: [{}], references: [{}]",
            join_logical_fields(naming, &fk.columns),
            join_logical_fields(&table_naming[&fk.referenced_table], &fk.referenced_columns)
        ));
        if let Some(action) = &fk.on_delete {
            rel_args.push(format!("onDelete: {}", render_referential_action(action)));
        }
        if let Some(action) = &fk.on_update {
            rel_args.push(format!("onUpdate: {}", render_referential_action(action)));
        }
        lines.push(format!(
            "  {}  {}  @relation({})",
            relation.field_name,
            type_str,
            rel_args.join(", ")
        ));
    }
    lines
}

fn render_back_relation_lines(
    table_naming: &HashMap<TableName, TableNamingContext>,
    back_relations: &[BackRelation],
) -> Vec<String> {
    back_relations
        .iter()
        .map(|relation| {
            let owning_model = &table_naming[&relation.owning_table].model_name;
            let relation_type = if relation.is_one_to_one {
                format!("{}?", owning_model)
            } else {
                format!("{}[]", owning_model)
            };
            match &relation.relation_name {
                Some(relation_name) => format!(
                    "  {}  {}  @relation(name: \"{}\")",
                    relation.field_name,
                    relation_type,
                    escape_schema_string(relation_name)
                ),
                None => format!("  {}  {}", relation.field_name, relation_type),
            }
        })
        .collect()
}

/// Whether a raw SQL expression names any of `columns` as a whole identifier.
fn mentions_unmodellable_column(expr: &str, columns: &HashSet<&str>) -> bool {
    if columns.is_empty() {
        return false;
    }
    expr.split(|c: char| !c.is_alphanumeric() && c != '_')
        .any(|word| columns.contains(word))
}

fn render_index_lines(
    table_name: &TableName,
    table: &LiveTable,
    naming: &TableNamingContext,
    unmodellable_columns: &HashSet<&str>,
) -> Vec<String> {
    table
        .indexes
        .iter()
        .filter(|idx| {
            // An index over a column the schema marks `@ignore` cannot be
            // declared: `@@index` would name a field the model does not have.
            !idx.columns
                .iter()
                .any(|column| unmodellable_columns.contains(column.as_str()))
        })
        .map(|idx| {
            let columns = join_logical_fields(naming, &idx.columns);
            if idx.unique {
                return format!("  @@unique([{}])", columns);
            }

            let mut args = Vec::new();
            match &idx.kind {
                LiveIndexKind::Unknown(_) => {}
                LiveIndexKind::Basic(b) => {
                    if !matches!(b, BasicIndexType::BTree) {
                        args.push(format!("type: {}", b.as_str()));
                    }
                }
                LiveIndexKind::Pgvector(p) => {
                    args.push(format!("type: {}", p.method.as_str()));
                    if let Some(opclass) = p.opclass {
                        args.push(format!("opclass: {}", opclass.as_str()));
                    }
                    push_pgvector_option_args(&mut args, &p.options);
                }
            }
            if idx.name != default_index_name(&table_name.name, &idx.columns) {
                args.push(format!("map: \"{}\"", idx.name));
            }
            if let Some(predicate) = &idx.predicate {
                args.push(format!(
                    "where: {}",
                    remap_bool_expr_identifiers(predicate, &naming.db_to_logical_field)
                ));
            }

            if args.is_empty() {
                format!("  @@index([{}])", columns)
            } else {
                format!("  @@index([{}], {})", columns, args.join(", "))
            }
        })
        .collect()
}

fn join_logical_fields(naming: &TableNamingContext, columns: &[String]) -> String {
    columns
        .iter()
        .map(|column| logical_field_name(naming, column))
        .collect::<Vec<_>>()
        .join(", ")
}

/// Render the `url` field of a datasource block.
///
/// An `env("NAME")` reference is emitted verbatim so a pulled schema points at
/// the variable instead of embedding a connection string — which carries the
/// password — in a file that usually ends up committed.
fn render_datasource_url(url: &str) -> String {
    let trimmed = url.trim();
    if trimmed.starts_with("env(") && trimmed.ends_with(')') {
        return trimmed.to_string();
    }
    format!("\"{}\"", escape_schema_string(url))
}

/// The schemas the introspected relations live in, in sorted order.
///
/// Non-empty only when the inspector was given a schema list: a single-schema
/// pull leaves every table unqualified, and emitting `schemas = [...]` for it
/// would force `@@schema` onto a schema that never asked for it.
fn declared_schemas(live: &LiveSchema) -> Vec<&str> {
    let mut schemas: Vec<&str> = live
        .tables
        .keys()
        .chain(live.views.keys())
        .filter_map(TableName::schema)
        .collect();
    schemas.sort_unstable();
    schemas.dedup();
    schemas
}

fn render_datasource_block(live: &LiveSchema, provider: DatabaseProvider, url: &str) -> String {
    let mut fields = vec![
        (
            "provider".to_string(),
            format!("\"{}\"", provider.schema_provider_name()),
        ),
        ("url".to_string(), render_datasource_url(url)),
    ];

    let schemas = declared_schemas(live);
    if !schemas.is_empty() {
        fields.push((
            "schemas".to_string(),
            format!(
                "[{}]",
                schemas
                    .iter()
                    .map(|schema| format!("\"{}\"", escape_schema_string(schema)))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        ));
    }

    if provider == DatabaseProvider::Postgres && !live.extensions.is_empty() {
        let mut extensions: Vec<(&str, &str)> = live
            .extensions
            .iter()
            .map(|(name, state)| (name.as_str(), state.schema.as_str()))
            .collect();
        extensions.sort_unstable_by(|a, b| a.0.cmp(b.0));
        fields.push((
            "extensions".to_string(),
            format!(
                "[{}]",
                extensions
                    .iter()
                    .map(|(name, schema)| render_extension_entry(name, schema))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        ));
    }

    let max_key = fields.iter().map(|(key, _)| key.len()).max().unwrap_or(0);
    let mut lines = vec!["datasource db {".to_string()];
    for (key, value) in fields {
        let padding = max_key - key.len() + 1;
        lines.push(format!("  {}{}= {}", key, " ".repeat(padding), value));
    }
    lines.push("}".to_string());
    lines.join("\n")
}

/// Infer the `.nautilus` scalar type name from a normalised SQL type string.
///
/// `enums` is the map of live enum type names (lower-cased) to their variants.
/// `composite_types` is the map of live composite type names (lower-cased) to their definitions.
/// When `sql_type` matches a known enum or composite type the corresponding PascalCase name
/// is returned.  Array types (ending with `[]`) are handled recursively.
/// Unrecognised types fall back to `String`.
/// Whether `db pull` has a faithful Nautilus spelling for a live column type.
///
/// `infer_nautilus_type` falls back to `String` for anything it does not know,
/// which is fine for display but disastrous for a round-trip: the very next
/// `db push` would see `interval` on one side and `text` on the other and
/// propose rewriting the column. Columns that hit the fallback are marked
/// `@ignore` instead, so Nautilus leaves them alone.
fn is_representable_type(
    sql_type: &str,
    enums: &HashMap<String, Vec<String>>,
    composite_types: &HashMap<String, LiveCompositeType>,
) -> bool {
    let t = sql_type.trim().to_lowercase();
    if let Some(inner) = t.strip_suffix("[]") {
        return is_representable_type(inner, enums, composite_types);
    }
    if t.is_empty() {
        return false;
    }
    infer_nautilus_type(sql_type, enums, composite_types) != "String" || is_known_string_type(&t)
}

/// The live types that legitimately map onto `String`, as opposed to reaching
/// the `infer_nautilus_type` fallback.
fn is_known_string_type(lowercased: &str) -> bool {
    matches!(lowercased, "text" | "clob")
        || lowercased.starts_with("varchar")
        || lowercased.starts_with("character varying")
        || (lowercased.starts_with("char(") && !lowercased.starts_with("char(36"))
}

fn infer_nautilus_type(
    sql_type: &str,
    enums: &HashMap<String, Vec<String>>,
    composite_types: &HashMap<String, LiveCompositeType>,
) -> String {
    // A MySQL column enum names no type, so it is resolved by its variants
    // against the declarations `lift_inline_enums` produced for them.
    if let Some(variants) = parse_inline_enum_variants(sql_type) {
        if let Some(name) = enums
            .iter()
            .find(|(_, known)| **known == variants)
            .map(|(name, _)| name)
        {
            return to_pascal_case(name);
        }
    }

    let t = sql_type.trim().to_lowercase();

    if let Some(inner) = t.strip_suffix("[]") {
        let inner_type = infer_nautilus_type(inner, enums, composite_types);
        return format!("{}[]", inner_type);
    }

    if let Some(enum_name) = matching_named_type(t.as_str(), enums) {
        return to_pascal_case(enum_name);
    }

    if let Some(composite_name) = matching_named_type(t.as_str(), composite_types) {
        return to_pascal_case(composite_name);
    }

    if let Some(inner) = t
        .strip_prefix("decimal(")
        .or_else(|| t.strip_prefix("numeric("))
    {
        if let Some(inner) = inner.strip_suffix(')') {
            let parts: Vec<&str> = inner.splitn(2, ',').collect();
            if parts.len() == 2 {
                let p = parts[0].trim();
                let s = parts[1].trim();
                return format!("Decimal({}, {})", p, s);
            }
        }
    }

    if let Some(length) = parse_sized_type_length(&t, "varchar(")
        .or_else(|| parse_sized_type_length(&t, "character varying("))
    {
        return format!("VarChar({})", length);
    }

    if let Some(dimension) = parse_sized_type_length(&t, "vector(") {
        return format!("Vector({})", dimension);
    }

    if t == "geometry" || t.starts_with("geometry(") {
        return "Geometry".to_string();
    }

    if t == "geography" || t.starts_with("geography(") {
        return "Geography".to_string();
    }

    if let Some(length) =
        parse_sized_type_length(&t, "char(").or_else(|| parse_sized_type_length(&t, "character("))
    {
        if length == 36 {
            return "Uuid".to_string();
        }
        return format!("Char({})", length);
    }

    match t.as_str() {
        "text" | "clob" => "String".to_string(),
        "citext" => "Citext".to_string(),
        "hstore" => "Hstore".to_string(),
        "ltree" => "Ltree".to_string(),
        "geometry" => "Geometry".to_string(),
        "geography" => "Geography".to_string(),
        t if t.starts_with("varchar") || t.starts_with("character varying") => "String".to_string(),
        "uuid" | "char(36)" => "Uuid".to_string(),
        t if t.starts_with("char(") && !t.starts_with("char(36") => "String".to_string(),
        "integer" | "int" | "int4" | "int2" | "smallint" | "tinyint" | "mediumint" => {
            "Int".to_string()
        }
        "bigint" | "int8" | "bigserial" | "unsigned bigint" => "BigInt".to_string(),
        "boolean" | "bool" => "Boolean".to_string(),
        "real" | "float4" | "double precision" | "float8" | "double" | "float" => {
            "Float".to_string()
        }
        "decimal" | "numeric" => "Float".to_string(),
        "timestamp"
        | "timestamp without time zone"
        | "timestamp with time zone"
        | "timestamptz"
        | "datetime" => "DateTime".to_string(),
        "bytea" | "blob" | "binary" | "varbinary" => "Bytes".to_string(),
        "json" => "Json".to_string(),
        "jsonb" => "Jsonb".to_string(),
        _ => "String".to_string(),
    }
}

/// Try to produce a `@default(...)` attribute from a raw DEFAULT expression
/// string as returned by the database. Returns `None` when the default is too
/// complex to round-trip safely.
fn infer_default_attr(
    raw: &str,
    col_type: &str,
    enums: &HashMap<String, Vec<String>>,
) -> Option<String> {
    let t = raw.trim().to_lowercase();

    if t.contains("nextval") || t.contains("autoincrement") {
        if can_infer_autoincrement(col_type) {
            return Some("@default(autoincrement())".to_string());
        }
        return None;
    }

    if t == "true" || t == "false" {
        return Some(format!("@default({})", t));
    }

    // MySQL stores booleans as `tinyint(1)` and reports their default as `1` or
    // `0`. Rendering that verbatim produces `Boolean @default(1)`, which the
    // schema validator rejects as a type mismatch.
    if col_type.trim().eq_ignore_ascii_case("boolean") {
        match t.as_str() {
            "1" => return Some("@default(true)".to_string()),
            "0" => return Some("@default(false)".to_string()),
            _ => {}
        }
    }

    if t.parse::<f64>().is_ok() {
        return Some(format!("@default({})", t));
    }

    if t.starts_with('\'') && t.ends_with('\'') && t.len() >= 2 {
        let inner = &raw.trim()[1..raw.trim().len() - 1];
        let base_type = col_type.trim().to_lowercase();
        let base_type = base_type.strip_suffix("[]").unwrap_or(&base_type);
        // An enum default is a bare variant, not a string literal — including
        // for a MySQL column enum, which names no type to match on.
        let is_enum = matching_named_type(base_type, enums).is_some()
            || parse_inline_enum_variants(col_type)
                .is_some_and(|variants| variants.iter().any(|variant| variant == inner));
        if is_enum {
            return Some(format!("@default({})", inner));
        }
        return Some(format!("@default(\"{}\")", inner));
    }

    if t == "now()" || t == "current_timestamp" || t.starts_with("current_timestamp") {
        return Some("@default(now())".to_string());
    }

    if t.contains("uuidv7") {
        return Some("@default(uuidv7())".to_string());
    }

    if t.contains("uuid") || t.contains("newid") {
        return Some("@default(uuid())".to_string());
    }

    None
}

fn matching_named_type<'a, T>(
    candidate: &str,
    named_types: &'a HashMap<String, T>,
) -> Option<&'a str> {
    named_types
        .keys()
        .find(|type_name| type_name.eq_ignore_ascii_case(candidate))
        .map(String::as_str)
}

/// Infer a logical relation field name from FK columns and the referenced table.
///
/// Examples:
/// - columns = `["user_id"]`  -> `"user"`   (strip `_id` suffix)
/// - columns = `["author_id"]` -> `"author"`
/// - columns = `["a_id", "b_id"]` -> singular form of `referenced_table`
fn infer_relation_field_name(fk_cols: &[String], ref_table: &str) -> String {
    if fk_cols.len() == 1 {
        let col = &fk_cols[0];
        if let Some(name) = col.strip_suffix("_id") {
            if !name.is_empty() {
                return name.to_string();
            }
        }
    }
    singular_name(ref_table)
}

fn build_table_naming_contexts(
    live: &LiveSchema,
    table_names: &[&TableName],
    view_names: &[&TableName],
    options: PullNamingOptions,
) -> HashMap<TableName, TableNamingContext> {
    let mut contexts = HashMap::new();
    let mut used_model_names = HashSet::new();

    for &table_name in table_names.iter().chain(view_names.iter()) {
        let table = live
            .tables
            .get(table_name)
            .unwrap_or_else(|| &live.views[table_name]);
        let model_name = choose_unique_field_name(
            vec![apply_model_case(&table_name.name, options.model_case)],
            &mut used_model_names,
        );

        let mut used_field_names = HashSet::new();
        let mut db_to_logical_field = HashMap::new();
        let mut logical_field_order = Vec::new();

        for column in &table.columns {
            let logical_name = choose_unique_field_name(
                vec![apply_scalar_field_case(&column.name, options.field_case)],
                &mut used_field_names,
            );
            db_to_logical_field.insert(column.name.clone(), logical_name.clone());
            logical_field_order.push(logical_name);
        }

        contexts.insert(
            table_name.clone(),
            TableNamingContext {
                model_name,
                db_to_logical_field,
                logical_field_order,
            },
        );
    }

    contexts
}

fn build_relation_pair_counts(
    live: &LiveSchema,
    table_names: &[&TableName],
) -> HashMap<(TableName, TableName), usize> {
    let mut counts = HashMap::new();
    for &table_name in table_names {
        for fk in &live.tables[table_name].foreign_keys {
            *counts
                .entry(relation_pair_key(table_name, &fk.referenced_table))
                .or_insert(0) += 1;
        }
    }
    counts
}

fn build_directional_relation_counts(
    live: &LiveSchema,
    table_names: &[&TableName],
) -> HashMap<(TableName, TableName), usize> {
    let mut counts = HashMap::new();
    for &table_name in table_names {
        for fk in &live.tables[table_name].foreign_keys {
            *counts
                .entry((table_name.clone(), fk.referenced_table.clone()))
                .or_insert(0) += 1;
        }
    }
    counts
}

fn build_forward_relations(
    live: &LiveSchema,
    table_names: &[&TableName],
    table_naming: &HashMap<TableName, TableNamingContext>,
    relation_pair_counts: &HashMap<(TableName, TableName), usize>,
    options: PullNamingOptions,
) -> HashMap<TableName, Vec<ForwardRelation>> {
    let mut result = HashMap::new();

    for &table_name in table_names {
        let table = &live.tables[table_name];
        let mut used_fields: HashSet<String> = table_naming[table_name]
            .logical_field_order
            .iter()
            .cloned()
            .collect();
        let mut relations = Vec::new();

        for (fk_index, fk) in table.foreign_keys.iter().enumerate() {
            let base_name = relation_field_name_base(&fk.columns, &fk.referenced_table.name);
            let fallback_name = apply_derived_field_case(
                &to_snake_case_identifier(&singular_name(&fk.referenced_table.name)),
                options.field_case,
            );
            let mut candidates = vec![apply_derived_field_case(&base_name, options.field_case)];
            if fallback_name != candidates[0] {
                candidates.push(fallback_name);
            }
            if let Some(first_col) = fk.columns.first() {
                let qualified = apply_derived_field_case(
                    &format!("{}_{}", base_name, to_snake_case_identifier(first_col)),
                    options.field_case,
                );
                if qualified != candidates[0] {
                    candidates.push(qualified);
                }
            }

            let field_name = choose_unique_field_name(candidates, &mut used_fields);
            let relation_name = needs_explicit_relation_name(
                table_name,
                &fk.referenced_table,
                relation_pair_counts,
            )
            .then(|| format!("{}_{}", table_naming[table_name].model_name, field_name));

            relations.push(ForwardRelation {
                fk_index,
                field_name,
                relation_name,
            });
        }

        result.insert(table_name.clone(), relations);
    }

    result
}

fn build_back_relations(
    live: &LiveSchema,
    table_names: &[&TableName],
    table_naming: &HashMap<TableName, TableNamingContext>,
    forward_relations: &HashMap<TableName, Vec<ForwardRelation>>,
    directional_relation_counts: &HashMap<(TableName, TableName), usize>,
    options: PullNamingOptions,
) -> HashMap<TableName, Vec<BackRelation>> {
    type IncomingEntry = (TableName, String, Option<String>, bool);
    let mut incoming: HashMap<TableName, Vec<IncomingEntry>> = HashMap::new();

    for &table_name in table_names {
        let table = &live.tables[table_name];
        for relation in forward_relations
            .get(table_name)
            .into_iter()
            .flat_map(|relations| relations.iter())
        {
            let fk = &table.foreign_keys[relation.fk_index];
            incoming
                .entry(fk.referenced_table.clone())
                .or_default()
                .push((
                    table_name.clone(),
                    relation.field_name.clone(),
                    relation.relation_name.clone(),
                    is_one_to_one_back_relation(live, table_name, fk),
                ));
        }
    }

    let mut result = HashMap::new();

    for &table_name in table_names {
        let mut used_fields: HashSet<String> = table_naming[table_name]
            .logical_field_order
            .iter()
            .cloned()
            .collect();
        if let Some(relations) = forward_relations.get(table_name) {
            used_fields.extend(relations.iter().map(|relation| relation.field_name.clone()));
        }

        let mut back_refs = Vec::new();
        if let Some(entries) = incoming.remove(table_name) {
            for (owning_table, forward_field_name, relation_name, is_one_to_one) in entries {
                let is_self_relation = owning_table == *table_name;
                let default_name =
                    default_back_relation_field_name(&owning_table.name, is_one_to_one, options);
                let qualified_name =
                    qualify_back_relation_field_name(&default_name, &forward_field_name, options);
                let direction_count = directional_relation_counts
                    .get(&(owning_table.clone(), table_name.clone()))
                    .copied()
                    .unwrap_or(0);

                let mut candidates = Vec::new();
                if direction_count <= 1 {
                    candidates.push(default_name.clone());
                }
                if qualified_name != default_name {
                    candidates.push(qualified_name);
                }
                candidates.push(default_name);

                let field_name = choose_unique_field_name(candidates, &mut used_fields);
                back_refs.push(BackRelation {
                    owning_table,
                    field_name,
                    relation_name: if is_self_relation {
                        None
                    } else {
                        relation_name
                    },
                    is_one_to_one,
                });
            }
        }

        result.insert(table_name.clone(), back_refs);
    }

    result
}

fn relation_pair_key(left: &TableName, right: &TableName) -> (TableName, TableName) {
    if left <= right {
        (left.clone(), right.clone())
    } else {
        (right.clone(), left.clone())
    }
}

fn needs_explicit_relation_name(
    owning_table: &TableName,
    referenced_table: &TableName,
    relation_pair_counts: &HashMap<(TableName, TableName), usize>,
) -> bool {
    owning_table == referenced_table
        || relation_pair_counts
            .get(&relation_pair_key(owning_table, referenced_table))
            .copied()
            .unwrap_or(0)
            > 1
}

fn relation_field_name_base(fk_cols: &[String], ref_table: &str) -> String {
    let raw = infer_relation_field_name(fk_cols, ref_table);
    let normalized = to_snake_case_identifier(&raw);
    if normalized.is_empty() {
        "relation".to_string()
    } else {
        normalized
    }
}

fn default_back_relation_field_name(
    owning_table: &str,
    is_one_to_one: bool,
    options: PullNamingOptions,
) -> String {
    let singular = to_snake_case_identifier(&singular_name(owning_table));
    if is_one_to_one {
        apply_derived_field_case(&singular, options.field_case)
    } else {
        apply_derived_field_case(&pluralize_name(&singular), options.field_case)
    }
}

fn qualify_back_relation_field_name(
    default_name: &str,
    forward_field_name: &str,
    options: PullNamingOptions,
) -> String {
    apply_derived_field_case(
        &format!(
            "{}_{}",
            to_snake_case_identifier(default_name),
            to_snake_case_identifier(forward_field_name)
        ),
        options.field_case,
    )
}

fn choose_unique_field_name(candidates: Vec<String>, used_fields: &mut HashSet<String>) -> String {
    let mut first_candidate = None;
    for candidate in candidates {
        let candidate = sanitize_logical_identifier(&candidate);
        if candidate.is_empty() {
            continue;
        }
        if first_candidate.is_none() {
            first_candidate = Some(candidate.clone());
        }
        if used_fields.insert(candidate.clone()) {
            return candidate;
        }
    }

    let base = first_candidate.unwrap_or_else(|| "relation".to_string());
    let mut suffix = 2usize;
    loop {
        let candidate = sanitize_logical_identifier(&format!("{}_{}", base, suffix));
        if used_fields.insert(candidate.clone()) {
            return candidate;
        }
        suffix += 1;
    }
}

fn escape_schema_string(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
        .replace('\t', "\\t")
}

fn render_extension_schema_name(name: &str) -> String {
    if is_bare_schema_identifier(name) {
        name.to_string()
    } else {
        format!("\"{}\"", escape_schema_string(name))
    }
}

/// Render a single array entry for the `extensions = [...]` field in a
/// serialized datasource block.
///
/// Round-trips live state back into source: when the extension lives in the
/// default `public` namespace we emit the compact form (`pg_trgm` or
/// `"uuid-ossp"`); any other schema is captured explicitly via the structured
/// `extension(name = ..., schema = "...")` syntax so `db pull` does not
/// silently drop the namespace information.
fn render_extension_entry(name: &str, schema: &str) -> String {
    if schema == "public" {
        render_extension_schema_name(name)
    } else {
        format!(
            "extension(name = {}, schema = \"{}\")",
            render_extension_schema_name(name),
            escape_schema_string(schema)
        )
    }
}

fn is_bare_schema_identifier(name: &str) -> bool {
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !first.is_ascii_alphabetic() && first != '_' {
        return false;
    }
    if !chars.all(|ch| ch.is_ascii_alphanumeric() || ch == '_') {
        return false;
    }
    matches!(TokenKind::from_ident(name), TokenKind::Ident(_))
}

fn logical_field_name(naming: &TableNamingContext, db_column_name: &str) -> String {
    naming
        .db_to_logical_field
        .get(db_column_name)
        .cloned()
        .unwrap_or_else(|| db_column_name.to_string())
}

fn sanitize_logical_identifier(name: &str) -> String {
    let mut candidate = name
        .chars()
        .map(|ch| {
            if ch.is_alphanumeric() || ch == '_' {
                ch
            } else {
                '_'
            }
        })
        .collect::<String>();

    if candidate.is_empty() {
        candidate.push('_');
    }

    if candidate
        .chars()
        .next()
        .is_some_and(|ch| !ch.is_alphabetic() && ch != '_')
    {
        candidate.insert(0, '_');
    }

    if TokenKind::from_ident(&candidate).is_keyword() {
        candidate.push('_');
    }

    candidate
}

fn apply_model_case(name: &str, case: PullNameCase) -> String {
    match case {
        PullNameCase::Auto => to_pascal_case(name),
        PullNameCase::Snake => to_snake_case_identifier(name),
        PullNameCase::Pascal => normalized_pascal_case(name),
    }
}

fn apply_scalar_field_case(name: &str, case: PullNameCase) -> String {
    match case {
        PullNameCase::Auto => name.to_string(),
        PullNameCase::Snake => to_snake_case_identifier(name),
        PullNameCase::Pascal => normalized_pascal_case(name),
    }
}

fn apply_derived_field_case(name: &str, case: PullNameCase) -> String {
    match case {
        PullNameCase::Auto | PullNameCase::Snake => to_snake_case_identifier(name),
        PullNameCase::Pascal => normalized_pascal_case(name),
    }
}

fn normalized_pascal_case(name: &str) -> String {
    let snake = to_snake_case_identifier(name);
    if snake.is_empty() {
        String::new()
    } else {
        to_pascal_case(&snake)
    }
}

fn remap_sql_expr_identifiers(expr: &str, field_map: &HashMap<String, String>) -> String {
    parse_schema_sql_expr(expr)
        .map(|parsed| render_sql_expr_with_field_map(&parsed, field_map))
        .unwrap_or_else(|| expr.to_string())
}

/// Render a live CHECK expression (or index predicate) as schema text.
///
/// Predicates the expression language covers are re-rendered with logical field
/// names. Anything else — `IS NULL`, `LIKE`, function calls, typed literals —
/// is emitted as a raw quoted predicate rather than as bare database text: bare
/// text does not reparse, so the pulled schema would be unusable, while the raw
/// form round-trips and still pushes back verbatim.
fn remap_bool_expr_identifiers(expr: &str, field_map: &HashMap<String, String>) -> String {
    parse_schema_bool_expr(expr)
        .map(|parsed| render_bool_expr_with_field_map(&parsed, field_map))
        .unwrap_or_else(|| format!("\"{}\"", escape_schema_string(&restore_sql_in_lists(expr))))
}

/// Undo the inspector's `IN (...)` -> `IN [...]` rewrite.
///
/// A raw predicate is pushed back to the database verbatim, so it has to be
/// SQL. The inspector normalises live expressions into the schema dialect
/// before the serializer sees them, and the bracket form is the one piece of
/// that dialect no database accepts.
fn restore_sql_in_lists(expr: &str) -> String {
    let lower = expr.to_lowercase();
    let marker = " in [";
    if !lower.contains(marker) {
        return expr.to_string();
    }

    let mut result = String::with_capacity(expr.len());
    let mut pos = 0usize;

    while let Some(rel) = lower[pos..].find(marker) {
        let abs = pos + rel;
        result.push_str(&expr[pos..abs]);
        result.push_str(" IN (");

        let after_open = abs + marker.len();
        let rest = &expr[after_open..];

        let mut depth = 1i32;
        let mut close = rest.len();
        for (i, c) in rest.char_indices() {
            match c {
                '[' => depth += 1,
                ']' => {
                    depth -= 1;
                    if depth == 0 {
                        close = i;
                        break;
                    }
                }
                _ => {}
            }
        }

        result.push_str(&rest[..close]);
        result.push(')');
        pos = (after_open + close + 1).min(expr.len());
    }

    result.push_str(&expr[pos..]);
    result
}

fn parse_schema_sql_expr(expr: &str) -> Option<SqlExpr> {
    let tokens = lex_expression_tokens(expr).ok()?;
    parse_sql_expr(&tokens, Span::new(0, expr.len())).ok()
}

fn parse_schema_bool_expr(expr: &str) -> Option<BoolExpr> {
    let tokens = lex_expression_tokens(expr).ok()?;
    parse_bool_expr(&tokens, Span::new(0, expr.len())).ok()
}

fn lex_expression_tokens(expr: &str) -> nautilus_schema::Result<Vec<Token>> {
    let mut lexer = Lexer::new(expr);
    let mut tokens = Vec::new();
    loop {
        let token = lexer.next_token()?;
        if matches!(token.kind, TokenKind::Eof) {
            break;
        }
        tokens.push(token);
    }
    Ok(tokens)
}

fn render_sql_expr_with_field_map(expr: &SqlExpr, field_map: &HashMap<String, String>) -> String {
    match expr {
        SqlExpr::Ident(name) => field_map.get(name).cloned().unwrap_or_else(|| name.clone()),
        SqlExpr::Number(n) => n.clone(),
        SqlExpr::StringLit(s) => format!("\"{}\"", s),
        SqlExpr::Bool(b) => b.to_string(),
        SqlExpr::BinaryOp { left, op, right } => format!(
            "{} {} {}",
            render_sql_expr_with_field_map(left, field_map),
            op,
            render_sql_expr_with_field_map(right, field_map)
        ),
        SqlExpr::UnaryOp { op, operand } => {
            format!(
                "{}{}",
                op,
                render_sql_expr_with_field_map(operand, field_map)
            )
        }
        SqlExpr::FnCall { name, args } => format!(
            "{}({})",
            name,
            args.iter()
                .map(|arg| render_sql_expr_with_field_map(arg, field_map))
                .collect::<Vec<_>>()
                .join(", ")
        ),
        SqlExpr::Paren(inner) => format!("({})", render_sql_expr_with_field_map(inner, field_map)),
    }
}

fn render_bool_expr_with_field_map(expr: &BoolExpr, field_map: &HashMap<String, String>) -> String {
    match expr {
        BoolExpr::Comparison { left, op, right } => format!(
            "{} {} {}",
            render_bool_operand_with_field_map(left, field_map, false),
            op,
            render_bool_operand_with_field_map(right, field_map, false)
        ),
        BoolExpr::And(left, right) => format!(
            "{} AND {}",
            render_bool_expr_with_field_map(left, field_map),
            render_bool_expr_with_field_map(right, field_map)
        ),
        BoolExpr::Or(left, right) => format!(
            "{} OR {}",
            render_bool_expr_with_field_map(left, field_map),
            render_bool_expr_with_field_map(right, field_map)
        ),
        BoolExpr::Not(inner) => {
            format!("NOT {}", render_bool_expr_with_field_map(inner, field_map))
        }
        BoolExpr::In { field, values } => format!(
            "{} IN [{}]",
            field_map
                .get(field)
                .cloned()
                .unwrap_or_else(|| field.clone()),
            values
                .iter()
                .map(|value| render_bool_operand_with_field_map(value, field_map, true))
                .collect::<Vec<_>>()
                .join(", ")
        ),
        BoolExpr::Paren(inner) => {
            format!("({})", render_bool_expr_with_field_map(inner, field_map))
        }
        BoolExpr::Raw(_) => expr.to_string(),
    }
}

fn render_bool_operand_with_field_map(
    operand: &Operand,
    field_map: &HashMap<String, String>,
    enum_variant_in_list: bool,
) -> String {
    match operand {
        Operand::Field(name) => field_map.get(name).cloned().unwrap_or_else(|| name.clone()),
        Operand::EnumVariant(variant) if enum_variant_in_list => variant.clone(),
        other => other.to_string(),
    }
}

fn type_supports_optional_modifier(nautilus_type: &str) -> bool {
    !nautilus_type.ends_with("[]")
}

fn render_referential_action(action: &str) -> String {
    let normalized: String = action
        .chars()
        .filter(|ch| !ch.is_whitespace())
        .flat_map(|ch| ch.to_lowercase())
        .collect();
    match normalized.as_str() {
        "cascade" => "Cascade".to_string(),
        "restrict" => "Restrict".to_string(),
        "noaction" => "NoAction".to_string(),
        "setnull" => "SetNull".to_string(),
        "setdefault" => "SetDefault".to_string(),
        _ => action.to_string(),
    }
}

fn push_pgvector_option_args(args: &mut Vec<String>, options: &PgvectorIndexOptions) {
    if let Some(value) = options.m {
        args.push(format!("m: {}", value));
    }
    if let Some(value) = options.ef_construction {
        args.push(format!("ef_construction: {}", value));
    }
    if let Some(value) = options.lists {
        args.push(format!("lists: {}", value));
    }
}

fn default_index_name(table_name: &str, columns: &[String]) -> String {
    let mut sorted_columns = columns.to_vec();
    sorted_columns.sort();
    format!("idx_{}_{}", table_name, sorted_columns.join("_"))
}

fn is_one_to_one_back_relation(
    live: &LiveSchema,
    owning_table: &TableName,
    fk: &LiveForeignKey,
) -> bool {
    live.tables
        .get(owning_table)
        .is_some_and(|table| columns_form_unique_key(table, &fk.columns))
}

fn columns_form_unique_key(table: &LiveTable, columns: &[String]) -> bool {
    let mut normalized_columns = columns.to_vec();
    normalized_columns.sort();

    let mut primary_key = table.primary_key.clone();
    primary_key.sort();
    if normalized_columns == primary_key {
        return true;
    }

    table.indexes.iter().any(|idx| {
        if !idx.unique {
            return false;
        }
        let mut index_columns = idx.columns.clone();
        index_columns.sort();
        index_columns == normalized_columns
    })
}

fn parse_sized_type_length(sql_type: &str, prefix: &str) -> Option<usize> {
    let inner = sql_type.strip_prefix(prefix)?.strip_suffix(')')?;
    inner.trim().parse().ok()
}

fn can_infer_autoincrement(col_type: &str) -> bool {
    let normalized = col_type.trim().to_lowercase();
    let base = normalized.strip_suffix("[]").unwrap_or(&normalized);
    matches!(
        base,
        "integer"
            | "int"
            | "int2"
            | "int4"
            | "smallint"
            | "tinyint"
            | "mediumint"
            | "bigint"
            | "int8"
            | "unsigned bigint"
    )
}

fn pluralize_name(name: &str) -> String {
    if name.ends_with('y')
        && !matches!(name.chars().rev().nth(1), Some('a' | 'e' | 'i' | 'o' | 'u'))
    {
        format!("{}ies", &name[..name.len() - 1])
    } else if matches!(name.chars().last(), Some('s' | 'x' | 'z'))
        || name.ends_with("ch")
        || name.ends_with("sh")
    {
        format!("{name}es")
    } else {
        format!("{name}s")
    }
}

/// Very simple singularisation: strip a trailing `s` (handles the common
/// plural pattern; no full inflection library is needed here).
fn singular_name(name: &str) -> String {
    if name.ends_with("ies") && name.len() > 3 {
        format!("{}y", &name[..name.len() - 3])
    } else if name.ends_with('s') && name.len() > 1 {
        name[..name.len() - 1].to_string()
    } else {
        name.to_string()
    }
}

fn to_snake_case_identifier(s: &str) -> String {
    let chars: Vec<char> = s
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric() || *ch == '_')
        .collect();
    let mut out = String::new();

    for (idx, ch) in chars.iter().copied().enumerate() {
        if ch == '_' {
            if !out.is_empty() && !out.ends_with('_') {
                out.push('_');
            }
            continue;
        }

        let prev = idx.checked_sub(1).and_then(|i| chars.get(i)).copied();
        let next = chars.get(idx + 1).copied();
        let is_upper = ch.is_ascii_uppercase();
        let prev_is_lower_or_digit =
            prev.is_some_and(|prev| prev.is_ascii_lowercase() || prev.is_ascii_digit());
        let prev_is_upper = prev.is_some_and(|prev| prev.is_ascii_uppercase());
        let next_is_lower = next.is_some_and(|next| next.is_ascii_lowercase());

        if is_upper
            && !out.is_empty()
            && (prev_is_lower_or_digit || (prev_is_upper && next_is_lower))
            && !out.ends_with('_')
        {
            out.push('_');
        }

        out.push(ch.to_ascii_lowercase());
    }

    out.trim_matches('_').to_string()
}

/// Convert a snake_case table name to PascalCase (for example `blog_posts` -> `BlogPosts`).
fn to_pascal_case(s: &str) -> String {
    s.split('_')
        .filter(|p| !p.is_empty())
        .map(|p| {
            let mut chars = p.chars();
            match chars.next() {
                None => String::new(),
                Some(c) => c.to_uppercase().to_string() + chars.as_str(),
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pascal_case_snake() {
        assert_eq!(to_pascal_case("blog_posts"), "BlogPosts");
    }

    #[test]
    fn pascal_case_single() {
        assert_eq!(to_pascal_case("users"), "Users");
    }

    #[test]
    fn pascal_case_already() {
        assert_eq!(to_pascal_case("User"), "User");
    }

    #[test]
    fn infers_types_correctly() {
        let no_enums = HashMap::new();
        let no_composites = HashMap::new();
        assert_eq!(
            infer_nautilus_type("text", &no_enums, &no_composites),
            "String"
        );
        assert_eq!(
            infer_nautilus_type("integer", &no_enums, &no_composites),
            "Int"
        );
        assert_eq!(
            infer_nautilus_type("bigint", &no_enums, &no_composites),
            "BigInt"
        );
        assert_eq!(
            infer_nautilus_type("boolean", &no_enums, &no_composites),
            "Boolean"
        );
        assert_eq!(
            infer_nautilus_type("double precision", &no_enums, &no_composites),
            "Float"
        );
        assert_eq!(
            infer_nautilus_type("timestamp", &no_enums, &no_composites),
            "DateTime"
        );
        assert_eq!(
            infer_nautilus_type("uuid", &no_enums, &no_composites),
            "Uuid"
        );
        assert_eq!(
            infer_nautilus_type("citext", &no_enums, &no_composites),
            "Citext"
        );
        assert_eq!(
            infer_nautilus_type("hstore", &no_enums, &no_composites),
            "Hstore"
        );
        assert_eq!(
            infer_nautilus_type("ltree", &no_enums, &no_composites),
            "Ltree"
        );
        assert_eq!(
            infer_nautilus_type("vector(1536)", &no_enums, &no_composites),
            "Vector(1536)"
        );
        assert_eq!(
            infer_nautilus_type("jsonb", &no_enums, &no_composites),
            "Jsonb"
        );
        assert_eq!(
            infer_nautilus_type("bytea", &no_enums, &no_composites),
            "Bytes"
        );
        assert_eq!(
            infer_nautilus_type("decimal(10, 2)", &no_enums, &no_composites),
            "Decimal(10, 2)"
        );
        assert_eq!(
            infer_nautilus_type("varchar(255)", &no_enums, &no_composites),
            "VarChar(255)"
        );
        assert_eq!(
            infer_nautilus_type("char(36)", &no_enums, &no_composites),
            "Uuid"
        );
        assert_eq!(
            infer_nautilus_type("char(10)", &no_enums, &no_composites),
            "Char(10)"
        );

        let mut with_enums = HashMap::new();
        with_enums.insert(
            "role".to_string(),
            vec!["ADMIN".to_string(), "USER".to_string()],
        );
        assert_eq!(
            infer_nautilus_type("role", &with_enums, &no_composites),
            "Role"
        );
    }

    #[test]
    fn infers_scalar_arrays() {
        let no_enums = HashMap::new();
        let no_composites = HashMap::new();
        assert_eq!(
            infer_nautilus_type("integer[]", &no_enums, &no_composites),
            "Int[]"
        );
        assert_eq!(
            infer_nautilus_type("text[]", &no_enums, &no_composites),
            "String[]"
        );
        assert_eq!(
            infer_nautilus_type("boolean[]", &no_enums, &no_composites),
            "Boolean[]"
        );
        assert_eq!(
            infer_nautilus_type("uuid[]", &no_enums, &no_composites),
            "Uuid[]"
        );
        assert_eq!(
            infer_nautilus_type("citext[]", &no_enums, &no_composites),
            "Citext[]"
        );
        assert_eq!(
            infer_nautilus_type("jsonb[]", &no_enums, &no_composites),
            "Jsonb[]"
        );
    }

    #[test]
    fn infers_enum_array() {
        let no_composites = HashMap::new();
        let mut enums = HashMap::new();
        enums.insert(
            "status".to_string(),
            vec!["ACTIVE".to_string(), "INACTIVE".to_string()],
        );
        assert_eq!(
            infer_nautilus_type("status[]", &enums, &no_composites),
            "Status[]"
        );
    }

    #[test]
    fn infers_composite_type() {
        use crate::live::LiveCompositeType;
        let no_enums = HashMap::new();
        let mut composites = HashMap::new();
        composites.insert(
            "address".to_string(),
            LiveCompositeType {
                name: "address".to_string(),
                fields: vec![],
            },
        );
        assert_eq!(
            infer_nautilus_type("address", &no_enums, &composites),
            "Address"
        );
        assert_eq!(
            infer_nautilus_type("address[]", &no_enums, &composites),
            "Address[]"
        );
    }

    #[test]
    fn default_boolean() {
        let no_enums: HashMap<String, Vec<String>> = HashMap::new();
        assert_eq!(
            infer_default_attr("true", "boolean", &no_enums),
            Some("@default(true)".into())
        );
    }

    #[test]
    fn default_boolean_from_mysql_tinyint() {
        let no_enums = HashMap::new();
        // MySQL reports a boolean default as 1/0; rendering it verbatim would
        // produce `Boolean @default(1)`, which fails schema validation.
        assert_eq!(
            infer_default_attr("1", "boolean", &no_enums),
            Some("@default(true)".into())
        );
        assert_eq!(
            infer_default_attr("0", "boolean", &no_enums),
            Some("@default(false)".into())
        );
    }

    #[test]
    fn numeric_defaults_on_non_boolean_columns_are_untouched() {
        let no_enums = HashMap::new();
        assert_eq!(
            infer_default_attr("1", "integer", &no_enums),
            Some("@default(1)".into())
        );
        assert_eq!(
            infer_default_attr("0", "integer", &no_enums),
            Some("@default(0)".into())
        );
    }

    #[test]
    fn default_number() {
        let no_enums: HashMap<String, Vec<String>> = HashMap::new();
        assert_eq!(
            infer_default_attr("42", "integer", &no_enums),
            Some("@default(42)".into())
        );
    }

    #[test]
    fn default_string() {
        let no_enums: HashMap<String, Vec<String>> = HashMap::new();
        assert_eq!(
            infer_default_attr("'hello'", "text", &no_enums),
            Some("@default(\"hello\")".into())
        );
    }

    #[test]
    fn default_now() {
        let no_enums: HashMap<String, Vec<String>> = HashMap::new();
        assert_eq!(
            infer_default_attr("current_timestamp", "timestamp", &no_enums),
            Some("@default(now())".into())
        );
    }

    #[test]
    fn default_uuid() {
        let no_enums: HashMap<String, Vec<String>> = HashMap::new();
        assert_eq!(
            infer_default_attr("gen_random_uuid()", "uuid", &no_enums),
            Some("@default(uuid())".into())
        );
    }

    #[test]
    fn default_uuidv7() {
        let no_enums: HashMap<String, Vec<String>> = HashMap::new();
        assert_eq!(
            infer_default_attr("uuidv7()", "uuid", &no_enums),
            Some("@default(uuidv7())".into())
        );
    }

    #[test]
    fn default_nextval_skipped() {
        let no_enums: HashMap<String, Vec<String>> = HashMap::new();
        assert_eq!(
            infer_default_attr("nextval('seq')", "integer", &no_enums),
            Some("@default(autoincrement())".into())
        );
    }

    #[test]
    fn default_enum_literal() {
        let mut enums: HashMap<String, Vec<String>> = HashMap::new();
        enums.insert(
            "status".to_string(),
            vec!["DRAFT".to_string(), "PUBLISHED".to_string()],
        );
        assert_eq!(
            infer_default_attr("'DRAFT'", "status", &enums),
            Some("@default(DRAFT)".into())
        );
    }

    #[test]
    fn default_string_not_confused_with_enum() {
        let no_enums: HashMap<String, Vec<String>> = HashMap::new();
        assert_eq!(
            infer_default_attr("'hello'", "text", &no_enums),
            Some("@default(\"hello\")".into())
        );
    }
}
