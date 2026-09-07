use super::DiffApplier;
use crate::error::{MigrationError, Result};
use nautilus_core::TableName;

impl DiffApplier<'_> {
    pub(super) fn sql_create_composite_type(&self, name: &str) -> Result<Vec<String>> {
        let ct = self
            .schema
            .composite_types
            .values()
            .find(|ct| ct.db_name == *name)
            .ok_or_else(|| {
                MigrationError::Other(format!(
                    "Composite type definition not found for '{}'",
                    name
                ))
            })?;
        Ok(vec![self.ddl.generate_composite_type(ct)?])
    }

    pub(super) fn sql_drop_type(&self, name: &str) -> String {
        self.strategy().drop_type_sql(name)
    }

    pub(super) fn sql_alter_composite_type(
        &self,
        name: &str,
        added_fields: &[(String, String)],
        dropped_fields: &[String],
        type_changed_fields: &[(String, String, String)],
    ) -> Vec<String> {
        let mut stmts: Vec<String> = Vec::new();
        for (field_name, sql_type) in added_fields {
            stmts.push(format!(
                "ALTER TYPE {} ADD ATTRIBUTE {} {}",
                self.type_q(name),
                self.q(field_name),
                sql_type,
            ));
        }
        for (field_name, _from, to) in type_changed_fields {
            stmts.push(format!(
                "ALTER TYPE {} ALTER ATTRIBUTE {} TYPE {} CASCADE",
                self.type_q(name),
                self.q(field_name),
                to,
            ));
        }
        for field_name in dropped_fields {
            stmts.push(format!(
                "ALTER TYPE {} DROP ATTRIBUTE {} CASCADE",
                self.type_q(name),
                self.q(field_name),
            ));
        }
        stmts
    }

    pub(super) fn sql_create_enum(&self, name: &str, variants: &[String]) -> String {
        if let Some(def) = self
            .schema
            .enums
            .values()
            .find(|e| e.logical_name.eq_ignore_ascii_case(name))
        {
            return self.ddl.generate_enum_type(def);
        }

        self.strategy().create_enum_sql(name, variants)
    }

    /// Alter an enum type.
    ///
    /// Adding variants is a cheap `ADD VALUE`.  Removing them is not supported
    /// by PostgreSQL, so the type is renamed, recreated, every dependent column
    /// is cast across via `text`, and the old type is dropped.
    pub(super) fn sql_alter_enum(
        &self,
        name: &str,
        added_variants: &[String],
        removed_variants: &[String],
    ) -> Result<Vec<String>> {
        if removed_variants.is_empty() {
            return Ok(added_variants
                .iter()
                .map(|v| {
                    format!(
                        "ALTER TYPE {} ADD VALUE IF NOT EXISTS '{}'",
                        self.type_q(name),
                        v
                    )
                })
                .collect());
        }

        let enum_def = self
            .schema
            .enums
            .values()
            .find(|e| e.logical_name.eq_ignore_ascii_case(name))
            .ok_or_else(|| {
                MigrationError::Other(format!("Enum definition not found for '{}'", name))
            })?;

        let old_name = format!("{}_old", name);
        let variants_sql = enum_def
            .variants
            .iter()
            .map(|v| format!("'{}'", v))
            .collect::<Vec<_>>()
            .join(", ");

        let mut stmts = vec![
            format!(
                "ALTER TYPE {} RENAME TO {}",
                self.type_q(name),
                self.q(&old_name)
            ),
            format!(
                "CREATE TYPE {} AS ENUM ({})",
                self.type_q(name),
                variants_sql
            ),
        ];

        for (table_name, table) in &self.live.tables {
            for col in &table.columns {
                if col.col_type != *name {
                    continue;
                }
                stmts.extend(self.recast_enum_column(table_name, col, name, &old_name));
            }
        }

        stmts.push(format!("DROP TYPE {}", self.type_q(&old_name)));
        Ok(stmts)
    }

    /// Statements that move one column from the renamed old enum type onto the
    /// freshly created one, preserving its DEFAULT across the cast.
    fn recast_enum_column(
        &self,
        table_name: &TableName,
        col: &crate::live::LiveColumn,
        enum_name: &str,
        old_name: &str,
    ) -> Vec<String> {
        let mut stmts = Vec::new();

        if col.default_value.is_some() {
            stmts.push(format!(
                "ALTER TABLE {} ALTER COLUMN {} DROP DEFAULT",
                self.q_table(table_name),
                self.q(&col.name),
            ));
        }

        stmts.push(format!(
            "ALTER TABLE {} ALTER COLUMN {} TYPE {} \
             USING {}::text::{}",
            self.q_table(table_name),
            self.q(&col.name),
            self.type_q(enum_name),
            self.q(&col.name),
            self.type_q(enum_name),
        ));

        if let Some(default) = &col.default_value {
            let new_default = if let Some(val) = default.strip_suffix(&format!("::{}", old_name)) {
                format!("{}::{}", val, enum_name)
            } else if let Some(val) = default.strip_suffix(&format!("::{}", self.type_q(old_name)))
            {
                format!("{}::{}", val, enum_name)
            } else {
                default.clone()
            };
            stmts.push(format!(
                "ALTER TABLE {} ALTER COLUMN {} SET DEFAULT {}",
                self.q_table(table_name),
                self.q(&col.name),
                new_default,
            ));
        }

        stmts
    }

    /// Quote a PostgreSQL type identifier without folding its case.
    fn type_q(&self, name: &str) -> String {
        self.provider.quote_identifier(name)
    }
}
