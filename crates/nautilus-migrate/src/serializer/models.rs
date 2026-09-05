//! Rendering of the blocks that describe a relation: models, views, enums and
//! composite types, with their columns, indexes and relation fields.

use std::collections::{HashMap, HashSet};

use super::escape_schema_string;
use super::expressions::{
    infer_default_attr, remap_bool_expr_identifiers, remap_sql_expr_identifiers,
};
use super::naming::{
    apply_scalar_field_case, choose_unique_field_name, default_index_name, join_logical_fields,
    to_pascal_case, TableNamingContext,
};
use super::relations::{BackRelation, ForwardRelation, ManyToManyEnd};
use super::types::{
    can_infer_autoincrement, infer_nautilus_type, is_representable_type, render_column_type,
};
use super::PullNamingOptions;
use crate::live::{ComputedKind, LiveIndexKind, LiveSchema, LiveTable};
use nautilus_core::TableName;
use nautilus_schema::ir::{BasicIndexType, PgvectorIndexOptions};

pub(super) fn render_composite_type_block(
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

pub(super) fn render_enum_block(db_name: &str, variants: &[String]) -> String {
    let mut lines = vec![format!("enum {} {{", to_pascal_case(db_name))];
    for variant in variants {
        lines.push(format!("  {}", variant));
    }
    lines.push("}".to_string());
    lines.join("\n")
}

pub(super) fn render_model_block(
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
    lines.push(format!(
        "  @@map(\"{}\")",
        escape_schema_string(&table_name.name)
    ));
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
pub(super) fn render_view_block(
    live: &LiveSchema,
    view_name: &TableName,
    table_naming: &HashMap<TableName, TableNamingContext>,
) -> String {
    let view = &live.views[view_name];
    let naming = &table_naming[view_name];

    let mut lines = vec![format!("view {} {{", naming.model_name)];
    lines.extend(render_column_lines(live, view, naming));
    lines.push(format!(
        "  @@map(\"{}\")",
        escape_schema_string(&view_name.name)
    ));
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
        } else if col.self_updating {
            attrs.push("@updatedAt".to_string());
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
                args.push(format!("map: \"{}\"", escape_schema_string(&idx.name)));
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

/// Whether a raw SQL expression names any of `columns` as a whole identifier.
fn mentions_unmodellable_column(expr: &str, columns: &HashSet<&str>) -> bool {
    if columns.is_empty() {
        return false;
    }
    expr.split(|c: char| !c.is_alphanumeric() && c != '_')
        .any(|word| columns.contains(word))
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
