//! Serializer: converts a [`LiveSchema`] snapshot into canonical `.nautilus` source text.
//!
//! Used by `nautilus db pull` to introspect an existing database and emit a
//! schema file that can be fed back into `db push`.

use std::collections::HashMap;

use crate::{
    ddl::DatabaseProvider,
    live::{ComputedKind, LiveCompositeType, LiveForeignKey, LiveSchema, LiveTable},
};
use nautilus_schema::ir::IndexType;

/// Convert a [`LiveSchema`] to a `.nautilus` schema source string.
///
/// * `live` - the introspected live database schema
/// * `provider` - which SQL dialect was used during introspection
/// * `url` - the raw database URL written into the datasource block verbatim
pub fn serialize_live_schema(live: &LiveSchema, provider: DatabaseProvider, url: &str) -> String {
    let mut parts: Vec<String> = Vec::new();

    parts.push(format!(
        "datasource db {{\n  provider = \"{}\"\n  url      = \"{}\"\n}}",
        provider.schema_provider_name(),
        url
    ));

    let mut ct_names: Vec<&String> = live.composite_types.keys().collect();
    ct_names.sort();
    for ct_db_name in ct_names {
        let ct = &live.composite_types[ct_db_name];
        let type_name = to_pascal_case(ct_db_name);
        let max_name = ct.fields.iter().map(|f| f.name.len()).max().unwrap_or(0);
        let mut lines = vec![format!("type {} {{", type_name)];
        for field in &ct.fields {
            let nautilus_type =
                infer_nautilus_type(&field.col_type, &live.enums, &live.composite_types);
            lines.push(format!(
                "  {:<name_w$}  {}",
                field.name,
                nautilus_type,
                name_w = max_name,
            ));
        }
        lines.push("}".to_string());
        parts.push(lines.join("\n"));
    }

    let mut enum_names: Vec<&String> = live.enums.keys().collect();
    enum_names.sort();
    for enum_db_name in enum_names {
        let variants = &live.enums[enum_db_name];
        let type_name = to_pascal_case(enum_db_name);
        let mut lines = vec![format!("enum {} {{", type_name)];
        for variant in variants {
            lines.push(format!("  {}", variant));
        }
        lines.push("}".to_string());
        parts.push(lines.join("\n"));
    }

    let mut table_names: Vec<&String> = live.tables.keys().collect();
    table_names.sort();

    let mut back_refs: HashMap<String, Vec<(String, &LiveForeignKey)>> = HashMap::new();
    for tname in &table_names {
        for fk in &live.tables[*tname].foreign_keys {
            back_refs
                .entry(fk.referenced_table.clone())
                .or_default()
                .push(((*tname).clone(), fk));
        }
    }

    for table_name in &table_names {
        let table = &live.tables[*table_name];
        let model_name = to_pascal_case(table_name);
        let is_composite_pk = table.primary_key.len() > 1;

        let max_name = table
            .columns
            .iter()
            .map(|c| c.name.len())
            .max()
            .unwrap_or(0);
        let max_type = table
            .columns
            .iter()
            .map(|c| {
                let t = infer_nautilus_type(&c.col_type, &live.enums, &live.composite_types);
                let nullable_suffix = if c.nullable && type_supports_optional_modifier(&t) {
                    1
                } else {
                    0
                };
                t.len() + nullable_suffix
            })
            .max()
            .unwrap_or(0);

        let mut lines = vec![format!("model {} {{", model_name)];

        for col in &table.columns {
            let type_str = infer_nautilus_type(&col.col_type, &live.enums, &live.composite_types);
            let type_with_mod = if col.nullable && type_supports_optional_modifier(&type_str) {
                format!("{}?", type_str)
            } else {
                type_str
            };

            let is_pk_col = table.primary_key.contains(&col.name);
            let mut attrs: Vec<String> = Vec::new();

            if is_pk_col && !is_composite_pk {
                attrs.push("@id".to_string());
            }
            if let (Some(expr), Some(kind)) = (&col.generated_expr, &col.computed_kind) {
                let kind_str = match kind {
                    ComputedKind::Stored => "Stored",
                    ComputedKind::Virtual => "Virtual",
                };
                attrs.push(format!("@computed({}, {})", expr, kind_str));
            } else if let Some(def) = &col.default_value {
                if let Some(attr) = infer_default_attr(def, &col.col_type, &live.enums) {
                    attrs.push(attr);
                }
            }
            if let Some(check) = &col.check_expr {
                attrs.push(format!("@check({})", check));
            }

            let line = if attrs.is_empty() {
                format!("  {}  {}", col.name, type_with_mod)
            } else {
                format!(
                    "  {:<name_w$}  {:<type_w$}  {}",
                    col.name,
                    type_with_mod,
                    attrs.join("  "),
                    name_w = max_name,
                    type_w = max_type,
                )
            };
            lines.push(line.trim_end().to_string());
        }

        for fk in &table.foreign_keys {
            let ref_model = to_pascal_case(&fk.referenced_table);
            let field_name = infer_relation_field_name(&fk.columns, &fk.referenced_table);

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
                ref_model
            };

            let fields_list = fk.columns.join(", ");
            let references_list = fk.referenced_columns.join(", ");
            let mut rel_args = format!(
                "fields: [{}], references: [{}]",
                fields_list, references_list
            );
            if let Some(action) = &fk.on_delete {
                rel_args.push_str(&format!(
                    ", onDelete: {}",
                    render_referential_action(action)
                ));
            }
            if let Some(action) = &fk.on_update {
                rel_args.push_str(&format!(
                    ", onUpdate: {}",
                    render_referential_action(action)
                ));
            }
            lines.push(format!(
                "  {}  {}  @relation({})",
                field_name, type_str, rel_args
            ));
        }

        if let Some(refs) = back_refs.get(*table_name) {
            for (owning_table, fk) in refs {
                let owning_model = to_pascal_case(owning_table);
                if is_one_to_one_back_relation(live, owning_table, fk) {
                    lines.push(format!(
                        "  {}  {}?",
                        singular_name(owning_table),
                        owning_model
                    ));
                } else {
                    lines.push(format!("  {}  {}[]", owning_table, owning_model));
                }
            }
        }

        if is_composite_pk {
            lines.push(format!("  @@id([{}])", table.primary_key.join(", ")));
        }

        // Keep @@map explicit so the model/table mapping survives round-trips.
        lines.push(format!("  @@map(\"{}\")", table_name));

        for idx in &table.indexes {
            if idx.unique {
                lines.push(format!("  @@unique([{}])", idx.columns.join(", ")));
            } else {
                let mut args = Vec::new();
                if let Some(index_type) = render_index_type(idx.method.as_deref()) {
                    args.push(format!("type: {}", index_type));
                }
                let default_name = default_index_name(table_name, &idx.columns);
                if idx.name != default_name {
                    args.push(format!("map: \"{}\"", idx.name));
                }

                if args.is_empty() {
                    lines.push(format!("  @@index([{}])", idx.columns.join(", ")));
                } else {
                    lines.push(format!(
                        "  @@index([{}], {})",
                        idx.columns.join(", "),
                        args.join(", ")
                    ));
                }
            }
        }

        for check in &table.check_constraints {
            lines.push(format!("  @@check({})", check));
        }

        lines.push("}".to_string());
        parts.push(lines.join("\n"));
    }

    let mut out = parts.join("\n\n");
    if !out.ends_with('\n') {
        out.push('\n');
    }
    out
}

/// Infer the `.nautilus` scalar type name from a normalised SQL type string.
///
/// `enums` is the map of live enum type names (lower-cased) to their variants.
/// `composite_types` is the map of live composite type names (lower-cased) to their definitions.
/// When `sql_type` matches a known enum or composite type the corresponding PascalCase name
/// is returned.  Array types (ending with `[]`) are handled recursively.
/// Unrecognised types fall back to `String`.
fn infer_nautilus_type(
    sql_type: &str,
    enums: &HashMap<String, Vec<String>>,
    composite_types: &HashMap<String, LiveCompositeType>,
) -> String {
    let t = sql_type.trim().to_lowercase();

    if let Some(inner) = t.strip_suffix("[]") {
        let inner_type = infer_nautilus_type(inner, enums, composite_types);
        return format!("{}[]", inner_type);
    }

    if enums.contains_key(t.as_str()) {
        return to_pascal_case(&t);
    }

    if composite_types.contains_key(t.as_str()) {
        return to_pascal_case(&t);
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
        "json" | "jsonb" => "Json".to_string(),
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

    if t.parse::<f64>().is_ok() {
        return Some(format!("@default({})", t));
    }

    if t.starts_with('\'') && t.ends_with('\'') && t.len() >= 2 {
        let inner = &raw.trim()[1..raw.trim().len() - 1];
        let base_type = col_type.trim().to_lowercase();
        let base_type = base_type.strip_suffix("[]").unwrap_or(&base_type);
        if enums.contains_key(base_type) {
            return Some(format!("@default({})", inner));
        }
        return Some(format!("@default(\"{}\")", inner));
    }

    if t == "now()" || t == "current_timestamp" || t.starts_with("current_timestamp") {
        return Some("@default(now())".to_string());
    }

    if t.contains("uuid") || t.contains("newid") {
        return Some("@default(uuid())".to_string());
    }

    None
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

fn render_index_type(method: Option<&str>) -> Option<&'static str> {
    let index_type = method?.parse::<IndexType>().ok()?;
    (index_type != IndexType::BTree).then(|| index_type.as_str())
}

fn default_index_name(table_name: &str, columns: &[String]) -> String {
    let mut sorted_columns = columns.to_vec();
    sorted_columns.sort();
    format!("idx_{}_{}", table_name, sorted_columns.join("_"))
}

fn is_one_to_one_back_relation(live: &LiveSchema, owning_table: &str, fk: &LiveForeignKey) -> bool {
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
            infer_nautilus_type("jsonb", &no_enums, &no_composites),
            "Json"
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
