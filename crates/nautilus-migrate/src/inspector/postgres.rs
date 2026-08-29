use std::collections::{HashMap, HashSet};

use sqlx::postgres::PgRow;
use sqlx::PgPool;

use super::{
    group_pg_foreign_keys, group_pg_indexes, normalize_pg_check_expr,
    normalize_pg_composite_field_type, normalize_pg_default, normalize_pg_type, SchemaInspector,
};
use crate::error::{MigrationError, Result};
use crate::live::{
    ComputedKind, LiveColumn, LiveCompositeField, LiveCompositeType, LiveSchema, LiveTable,
};

const TABLES_SQL: &str = "SELECT c.relname AS table_name \
     FROM pg_class c \
     JOIN pg_namespace n ON n.oid = c.relnamespace \
     WHERE n.nspname = $1 \
       AND c.relkind IN ('r', 'p') \
       AND c.relname !~ '^_nautilus_' \
     ORDER BY c.relname";

const COLUMNS_SQL: &str = "SELECT c.table_name, \
            column_name, \
            udt_name, \
            pg_catalog.format_type(a.atttypid, a.atttypmod) AS formatted_type, \
            is_nullable, \
            column_default, \
            character_maximum_length, \
            numeric_precision, \
            numeric_scale, \
            generation_expression \
     FROM information_schema.columns c \
     JOIN pg_namespace n ON n.nspname = c.table_schema \
     JOIN pg_class cls ON cls.relnamespace = n.oid AND cls.relname = c.table_name \
     JOIN pg_attribute a ON a.attrelid = cls.oid AND a.attname = c.column_name \
     WHERE c.table_schema = $1 \
       AND c.table_name !~ '^_nautilus_' \
       AND a.attnum > 0 \
       AND NOT a.attisdropped \
     ORDER BY c.table_name, ordinal_position";

const PRIMARY_KEYS_SQL: &str = "SELECT tc.table_name, kcu.column_name \
     FROM information_schema.table_constraints tc \
     JOIN information_schema.key_column_usage kcu \
       ON tc.constraint_name = kcu.constraint_name \
      AND tc.table_schema    = kcu.table_schema \
     WHERE tc.constraint_type = 'PRIMARY KEY' \
       AND tc.table_schema    = $1 \
       AND tc.table_name !~ '^_nautilus_' \
     ORDER BY tc.table_name, kcu.ordinal_position";

const INDEXES_SQL: &str = "SELECT \
         tbl.relname                                           AS table_name, \
         idx.relname                                           AS index_name, \
         attr.attname                                          AS column_name, \
         ix.indisunique                                        AS is_unique, \
         am.amname                                             AS index_method, \
         CASE WHEN op.opcname LIKE 'vector_%' THEN op.opcname END AS opclass, \
         idx.reloptions                                         AS index_options, \
         pg_get_expr(ix.indpred, ix.indrelid)                   AS index_predicate, \
         k.ord                                                  AS column_position \
     FROM pg_class       tbl \
     JOIN pg_namespace   ns   ON ns.oid            = tbl.relnamespace \
     JOIN pg_index       ix   ON tbl.oid           = ix.indrelid \
     JOIN pg_class       idx  ON idx.oid           = ix.indexrelid \
     JOIN pg_am          am   ON am.oid            = idx.relam \
     JOIN unnest(ix.indkey::int[], ix.indclass::oid[]) \
          WITH ORDINALITY AS k(attnum, opclass_oid, ord) ON true \
     JOIN pg_attribute   attr ON attr.attrelid      = tbl.oid \
                             AND attr.attnum        = k.attnum \
     LEFT JOIN pg_opclass op  ON op.oid             = k.opclass_oid \
     WHERE ns.nspname = $1 \
       AND tbl.relname !~ '^_nautilus_' \
       AND tbl.relkind = 'r' \
       AND ix.indisprimary = false \
     ORDER BY tbl.relname, idx.relname, k.ord";

const CHECKS_SQL: &str = "SELECT t.relname AS table_name, \
            c.conname AS constraint_name, \
            pg_get_constraintdef(c.oid) AS constraint_def \
     FROM pg_constraint c \
     JOIN pg_class t ON t.oid = c.conrelid \
     JOIN pg_namespace n ON n.oid = t.relnamespace \
     WHERE c.contype = 'c' \
       AND n.nspname = $1 \
       AND t.relname !~ '^_nautilus_' \
     ORDER BY t.relname, c.conname";

const FOREIGN_KEYS_SQL: &str = "SELECT \
         t.relname                                    AS table_name, \
         c.conname                                    AS constraint_name, \
         a.attname                                    AS column_name, \
         rf.relname                                   AS referenced_table, \
         ra.attname                                   AS referenced_column, \
         c.confdeltype::text                          AS delete_type, \
         c.confupdtype::text                          AS update_type \
     FROM pg_constraint c \
     JOIN pg_class t   ON t.oid  = c.conrelid \
     JOIN pg_class rf  ON rf.oid = c.confrelid \
     JOIN pg_namespace n ON n.oid = t.relnamespace \
     JOIN LATERAL unnest(c.conkey, c.confkey) \
          WITH ORDINALITY AS u(local_att, ref_att, pos) ON true \
     JOIN pg_attribute a  \
       ON a.attrelid = c.conrelid  AND a.attnum = u.local_att \
     JOIN pg_attribute ra \
       ON ra.attrelid = c.confrelid AND ra.attnum = u.ref_att \
     WHERE c.contype = 'f' \
       AND n.nspname = $1 \
       AND t.relname !~ '^_nautilus_' \
     ORDER BY t.relname, c.conname, u.pos";

const ENUMS_SQL: &str = "SELECT t.typname AS enum_name, e.enumlabel AS variant \
     FROM pg_type t \
     JOIN pg_enum e ON t.oid = e.enumtypid \
     JOIN pg_namespace n ON n.oid = t.typnamespace \
     WHERE n.nspname = $1 \
     ORDER BY t.typname, e.enumsortorder";

const COMPOSITE_TYPES_SQL: &str = "SELECT t.typname AS composite_name, \
            a.attname AS field_name, \
            pg_catalog.format_type(a.atttypid, a.atttypmod) AS field_type \
     FROM pg_type t \
     JOIN pg_namespace n ON n.oid = t.typnamespace \
     JOIN pg_attribute a ON a.attrelid = t.typrelid \
     WHERE t.typtype = 'c' \
       AND n.nspname = $1 \
       AND a.attnum > 0 \
       AND NOT a.attisdropped \
       AND NOT EXISTS ( \
           SELECT 1 FROM pg_class c \
           WHERE c.reltype = t.oid \
             AND c.relkind IN ('r', 'v', 'm', 'p') \
       ) \
     ORDER BY t.typname, a.attnum";

const EXTENSIONS_SQL: &str = "SELECT e.extname, e.extversion, n.nspname AS extschema \
     FROM pg_extension e \
     JOIN pg_namespace n ON n.oid = e.extnamespace \
     WHERE e.extname <> 'plpgsql'";

impl SchemaInspector {
    pub(super) async fn inspect_postgres(&self) -> Result<LiveSchema> {
        let pool = PgPool::connect_with(postgres_connect_options(&self.url)?)
            .await
            .map_err(|e| MigrationError::Database(format!("PostgreSQL connection failed: {e}")))?;

        let schema_name = fetch_current_schema(&pool).await?;
        let table_names = fetch_table_names(&pool, &schema_name).await?;
        let mut metadata = TableMetadata::fetch(&pool, &schema_name).await?;

        let mut live = LiveSchema::default();
        for table_name in table_names {
            let table = metadata.build_table(table_name)?;
            live.tables.insert(table.name.clone(), table);
        }

        load_enums(&pool, &schema_name, &mut live).await?;
        load_composite_types(&pool, &schema_name, &mut live).await?;
        load_extensions(&pool, &mut live).await?;

        Ok(live)
    }
}

/// Catalog rows for every table of the inspected schema, grouped by table name.
///
/// Metadata is pulled with one query per kind and grouped in memory: querying
/// per table would cost five extra round-trips for every table in the schema on
/// each `db pull`/`db push`.
struct TableMetadata {
    columns: HashMap<String, Vec<PgRow>>,
    primary_keys: HashMap<String, Vec<PgRow>>,
    indexes: HashMap<String, Vec<PgRow>>,
    checks: HashMap<String, Vec<PgRow>>,
    foreign_keys: HashMap<String, Vec<PgRow>>,
}

impl TableMetadata {
    async fn fetch(pool: &PgPool, schema_name: &str) -> Result<Self> {
        Ok(Self {
            columns: fetch_grouped(pool, schema_name, COLUMNS_SQL, "column metadata").await?,
            primary_keys: fetch_grouped(
                pool,
                schema_name,
                PRIMARY_KEYS_SQL,
                "primary key metadata",
            )
            .await?,
            indexes: fetch_grouped(pool, schema_name, INDEXES_SQL, "index metadata").await?,
            checks: fetch_grouped(pool, schema_name, CHECKS_SQL, "CHECK constraints").await?,
            foreign_keys: fetch_grouped(
                pool,
                schema_name,
                FOREIGN_KEYS_SQL,
                "foreign key metadata",
            )
            .await?,
        })
    }

    fn build_table(&mut self, table_name: String) -> Result<LiveTable> {
        let mut columns = build_columns(&take_rows(&mut self.columns, &table_name), &table_name)?;
        let primary_key =
            build_primary_key(&take_rows(&mut self.primary_keys, &table_name), &table_name)?;
        let indexes = group_pg_indexes(take_rows(&mut self.indexes, &table_name));
        let check_constraints = apply_check_constraints(
            &mut columns,
            &take_rows(&mut self.checks, &table_name),
            &table_name,
        )?;
        let foreign_keys = group_pg_foreign_keys(take_rows(&mut self.foreign_keys, &table_name));

        Ok(LiveTable {
            name: table_name,
            columns,
            primary_key,
            indexes,
            check_constraints,
            foreign_keys,
        })
    }
}

fn take_rows(grouped: &mut HashMap<String, Vec<PgRow>>, table_name: &str) -> Vec<PgRow> {
    grouped.remove(table_name).unwrap_or_default()
}

async fn fetch_current_schema(pool: &PgPool) -> Result<String> {
    let row = pg_query("SELECT current_schema() AS schema_name")
        .fetch_one(pool)
        .await
        .map_err(|e| {
            MigrationError::Database(format!("failed to resolve current PostgreSQL schema: {e}"))
        })?;
    let schema_name: Option<String> = read_column(&row, "schema_name", || {
        "current PostgreSQL schema name".to_string()
    })?;
    Ok(schema_name.unwrap_or_else(|| "public".to_string()))
}

async fn fetch_table_names(pool: &PgPool, schema_name: &str) -> Result<Vec<String>> {
    let rows = pg_query(TABLES_SQL)
        .bind(schema_name)
        .fetch_all(pool)
        .await
        .map_err(|e| {
            MigrationError::Database(format!(
                "failed to list tables in PostgreSQL schema \"{schema_name}\": {e}"
            ))
        })?;

    rows.iter()
        .map(|row| {
            read_column(row, "table_name", || {
                format!("table metadata in PostgreSQL schema \"{schema_name}\"")
            })
        })
        .collect()
}

async fn fetch_grouped(
    pool: &PgPool,
    schema_name: &str,
    sql: &str,
    metadata_label: &str,
) -> Result<HashMap<String, Vec<PgRow>>> {
    let rows = pg_query(sql)
        .bind(schema_name)
        .fetch_all(pool)
        .await
        .map_err(|e| {
            MigrationError::Database(format!(
                "failed to fetch {metadata_label} in PostgreSQL schema \"{schema_name}\": {e}"
            ))
        })?;
    split_pg_rows_by_table(rows, metadata_label, schema_name)
}

fn build_columns(rows: &[PgRow], table_name: &str) -> Result<Vec<LiveColumn>> {
    let mut columns = Vec::with_capacity(rows.len());
    for row in rows {
        let col_name: String = read_column(row, "column_name", || {
            format!("column_name while inspecting table \"{table_name}\"")
        })?;
        let describe =
            |what: &str| format!("{what} for column \"{col_name}\" in table \"{table_name}\"");

        let udt_name: String = read_column(row, "udt_name", || describe("udt_name"))?;
        let is_nullable: String = read_column(row, "is_nullable", || describe("nullability"))?;
        let formatted_type: String =
            read_column(row, "formatted_type", || describe("formatted_type"))?;
        let column_default: Option<String> =
            read_column(row, "column_default", || describe("default value"))?;
        let character_maximum_length: Option<i32> =
            read_column(row, "character_maximum_length", || {
                describe("character_maximum_length")
            })?;
        let numeric_precision: Option<i32> =
            read_column(row, "numeric_precision", || describe("numeric_precision"))?;
        let numeric_scale: Option<i32> =
            read_column(row, "numeric_scale", || describe("numeric_scale"))?;
        let generation_expression: Option<String> =
            read_column(row, "generation_expression", || {
                describe("generation_expression")
            })?;

        let generated_expr = generation_expression
            .filter(|expr| !expr.is_empty())
            .map(|expr| normalize_pg_default(&expr));

        columns.push(LiveColumn {
            name: col_name,
            col_type: normalize_pg_type(
                &udt_name,
                numeric_precision,
                numeric_scale,
                character_maximum_length,
                Some(&formatted_type),
            ),
            nullable: is_nullable.eq_ignore_ascii_case("YES"),
            default_value: column_default.map(|default| normalize_pg_default(&default)),
            computed_kind: generated_expr.as_ref().map(|_| ComputedKind::Stored),
            generated_expr,
            check_expr: None,
            auto_increment: false,
        });
    }
    Ok(columns)
}

fn build_primary_key(rows: &[PgRow], table_name: &str) -> Result<Vec<String>> {
    rows.iter()
        .map(|row| {
            read_column(row, "column_name", || {
                format!("primary key metadata for table \"{table_name}\"")
            })
        })
        .collect()
}

/// Split CHECK constraints into per-column and table-level expressions, moving
/// the per-column ones onto their [`LiveColumn`].
///
/// A constraint is attributed to a column when its name follows the
/// `chk_{table}_{column}` convention emitted by `db push` and the suffix names
/// an existing column; everything else stays a table-level constraint.
fn apply_check_constraints(
    columns: &mut [LiveColumn],
    rows: &[PgRow],
    table_name: &str,
) -> Result<Vec<String>> {
    let column_prefix = format!("chk_{}_", table_name);
    let column_names: HashSet<&str> = columns.iter().map(|c| c.name.as_str()).collect();

    let mut table_checks = Vec::new();
    let mut column_checks: HashMap<String, String> = HashMap::new();
    for row in rows {
        let con_name: String = read_column(row, "constraint_name", || {
            format!("CHECK constraint name for table \"{table_name}\"")
        })?;
        let constraint_def: String = read_column(row, "constraint_def", || {
            format!("CHECK constraint definition \"{con_name}\" on table \"{table_name}\"")
        })?;

        let expr = normalize_pg_check_expr(&constraint_def);
        match con_name
            .strip_prefix(&column_prefix)
            .filter(|candidate| column_names.contains(candidate))
        {
            Some(column) => {
                column_checks.insert(column.to_string(), expr);
            }
            None => table_checks.push(expr),
        }
    }

    for column in columns {
        if let Some(expr) = column_checks.get(&column.name) {
            column.check_expr = Some(expr.clone());
        }
    }

    Ok(table_checks)
}

async fn load_enums(pool: &PgPool, schema_name: &str, live: &mut LiveSchema) -> Result<()> {
    let rows = pg_query(ENUMS_SQL)
        .bind(schema_name)
        .fetch_all(pool)
        .await
        .map_err(|e| {
            MigrationError::Database(format!(
                "failed to fetch enum types in PostgreSQL schema \"{schema_name}\": {e}"
            ))
        })?;

    for row in &rows {
        let enum_name: String = read_column(row, "enum_name", || {
            format!("enum type name in schema \"{schema_name}\"")
        })?;
        let variant: String = read_column(row, "variant", || {
            format!("enum variant for type \"{enum_name}\" in schema \"{schema_name}\"")
        })?;
        live.enums.entry(enum_name).or_default().push(variant);
    }
    Ok(())
}

async fn load_composite_types(
    pool: &PgPool,
    schema_name: &str,
    live: &mut LiveSchema,
) -> Result<()> {
    let rows = pg_query(COMPOSITE_TYPES_SQL)
        .bind(schema_name)
        .fetch_all(pool)
        .await
        .map_err(|e| {
            MigrationError::Database(format!(
                "failed to fetch composite types in PostgreSQL schema \"{schema_name}\": {e}"
            ))
        })?;

    for row in &rows {
        let composite_name: String = read_column(row, "composite_name", || {
            format!("composite type name in schema \"{schema_name}\"")
        })?;
        let field_name: String = read_column(row, "field_name", || {
            format!(
                "field name for composite type \"{composite_name}\" in schema \"{schema_name}\""
            )
        })?;
        let field_type: String = read_column(row, "field_type", || {
            format!("field type for \"{composite_name}.{field_name}\" in schema \"{schema_name}\"")
        })?;

        live.composite_types
            .entry(composite_name.clone())
            .or_insert_with(|| LiveCompositeType {
                name: composite_name,
                fields: Vec::new(),
            })
            .fields
            .push(LiveCompositeField {
                name: field_name,
                col_type: normalize_pg_composite_field_type(&field_type),
            });
    }
    Ok(())
}

async fn load_extensions(pool: &PgPool, live: &mut LiveSchema) -> Result<()> {
    let rows = pg_query(EXTENSIONS_SQL)
        .fetch_all(pool)
        .await
        .map_err(|e| {
            MigrationError::Database(format!("failed to fetch installed extensions: {e}"))
        })?;

    for row in &rows {
        let name: String = read_column(row, "extname", || "extension name".to_string())?;
        let version: String = read_column(row, "extversion", || {
            format!("version for extension \"{name}\"")
        })?;
        let schema: String = read_column(row, "extschema", || {
            format!("schema for extension \"{name}\"")
        })?;
        live.extensions.insert(
            name.to_lowercase(),
            crate::live::LiveExtension { version, schema },
        );
    }
    Ok(())
}

fn read_column<'r, T>(row: &'r PgRow, column: &str, describe: impl FnOnce() -> String) -> Result<T>
where
    T: sqlx::Decode<'r, sqlx::Postgres> + sqlx::Type<sqlx::Postgres>,
{
    use sqlx::Row as _;

    row.try_get::<T, _>(column)
        .map_err(|e| MigrationError::Database(format!("failed to read {}: {e}", describe())))
}

fn split_pg_rows_by_table(
    rows: Vec<sqlx::postgres::PgRow>,
    metadata_label: &str,
    schema_name: &str,
) -> Result<HashMap<String, Vec<sqlx::postgres::PgRow>>> {
    use sqlx::Row as _;

    let mut grouped: HashMap<String, Vec<sqlx::postgres::PgRow>> = HashMap::new();
    for row in rows {
        let table_name: String = row.try_get("table_name").map_err(|e| {
            MigrationError::Database(format!(
                "failed to read table_name while grouping PostgreSQL {metadata_label} in schema \"{schema_name}\": {e}"
            ))
        })?;
        grouped.entry(table_name).or_default().push(row);
    }
    Ok(grouped)
}

fn pg_query(sql: &str) -> sqlx::query::Query<'_, sqlx::Postgres, sqlx::postgres::PgArguments> {
    // PgBouncer transaction pooling and similar proxies can reject named
    // prepared statements. `persistent(false)` keeps these metadata queries on
    // unnamed statements while still letting us bind parameters safely.
    sqlx::query::<sqlx::Postgres>(sql).persistent(false)
}

fn postgres_connect_options(url: &str) -> Result<sqlx::postgres::PgConnectOptions> {
    use std::str::FromStr;

    // `db pull`/`db push` introspection is often run through PgBouncer or other
    // transaction-pooling proxies where persistent named prepared statements are
    // not safe. Disabling the statement cache plus non-persistent queries keeps
    // introspection portable.
    sqlx::postgres::PgConnectOptions::from_str(url)
        .map(|options| options.statement_cache_capacity(0))
        .map_err(|e| MigrationError::Database(format!("Invalid PostgreSQL URL: {e}")))
}

#[cfg(test)]
mod tests {
    use super::pg_query;

    #[test]
    fn pg_introspection_queries_are_non_persistent() {
        assert!(!sqlx::Execute::persistent(&pg_query("SELECT 1")));
    }
}
