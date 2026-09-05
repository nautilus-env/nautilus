//! Scalar type inference: what a live column's SQL type is called in a schema,
//! and whether it has a faithful spelling there at all.

use std::collections::HashMap;

use super::naming::to_pascal_case;
use crate::live::{LiveCompositeType, LiveSchema};
use nautilus_core::TableName;

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
pub(super) fn is_representable_type(
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

pub(super) fn infer_nautilus_type(
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
        t if t.starts_with("datetime(") || t.starts_with("timestamp(") => "DateTime".to_string(),
        "bytea" | "blob" | "binary" | "varbinary" => "Bytes".to_string(),
        "json" => "Json".to_string(),
        "jsonb" => "Jsonb".to_string(),
        _ => "String".to_string(),
    }
}

pub(super) fn matching_named_type<'a, T>(
    candidate: &str,
    named_types: &'a HashMap<String, T>,
) -> Option<&'a str> {
    named_types
        .keys()
        .find(|type_name| type_name.eq_ignore_ascii_case(candidate))
        .map(String::as_str)
}

pub(super) fn render_column_type(live: &LiveSchema, column: &crate::live::LiveColumn) -> String {
    let type_str = infer_nautilus_type(&column.col_type, &live.enums, &live.composite_types);
    if column.nullable && type_supports_optional_modifier(&type_str) {
        format!("{}?", type_str)
    } else {
        type_str
    }
}

fn type_supports_optional_modifier(nautilus_type: &str) -> bool {
    !nautilus_type.ends_with("[]")
}

fn parse_sized_type_length(sql_type: &str, prefix: &str) -> Option<usize> {
    let inner = sql_type.strip_prefix(prefix)?.strip_suffix(')')?;
    inner.trim().parse().ok()
}

pub(super) fn can_infer_autoincrement(col_type: &str) -> bool {
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

/// Parse the variants out of a MySQL inline column enum, `enum('A','B')`.
///
/// Returns `None` for any other column type. A quote inside a variant is
/// doubled by MySQL, matching the escaping the DDL generator emits.
pub(super) fn parse_inline_enum_variants(col_type: &str) -> Option<Vec<String>> {
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
pub(super) fn lift_inline_enums(live: &LiveSchema) -> Option<LiveSchema> {
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

#[cfg(test)]
mod tests {
    use super::*;

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
}
