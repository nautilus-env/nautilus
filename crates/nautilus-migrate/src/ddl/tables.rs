use super::{field_is_autoincrement, managed_scalar_fields, DatabaseProvider, DdlGenerator};
use crate::error::Result;
use crate::live::model_table;
use crate::provider::check_constraint_name;
use nautilus_schema::ir::{ModelIr, ResolvedFieldType, SchemaIr};

impl DdlGenerator {
    /// Logical name of the model's single-column `autoincrement()` primary key,
    /// if it has one.
    ///
    /// Each provider spells this column differently — SQLite needs it inline as
    /// `INTEGER PRIMARY KEY AUTOINCREMENT`, PostgreSQL as `SERIAL`, MySQL as
    /// `AUTO_INCREMENT` — so they all start from this one check.
    pub(crate) fn autoincrement_primary_key(model: &ModelIr) -> Option<&str> {
        let pk_fields = model.primary_key.fields();
        let [pk_name] = pk_fields.as_slice() else {
            return None;
        };
        let pk_name = *pk_name;
        let field = model.find_field(pk_name)?;

        field_is_autoincrement(field).then_some(pk_name)
    }

    fn sqlite_inline_primary_key<'a>(&self, model: &'a ModelIr) -> Option<&'a str> {
        if self.provider != DatabaseProvider::Sqlite {
            return None;
        }

        Self::autoincrement_primary_key(model)
    }

    /// Create a table with its primary key, unique constraints and foreign keys.
    ///
    /// SQLite inlines generated primary keys and uses anonymous CHECKs. The other
    /// providers name CHECKs consistently with the applier so introspection can
    /// distinguish column constraints from table constraints.
    pub(crate) fn generate_create_table(
        &self,
        model: &ModelIr,
        schema: &SchemaIr,
    ) -> Result<String> {
        let mut lines = Vec::new();
        let sqlite_inline_pk = self.sqlite_inline_primary_key(model);
        let autoincrement_pk = Self::autoincrement_primary_key(model);

        for field in &model.fields {
            let is_autoincrement_pk =
                autoincrement_pk.is_some_and(|name| field.logical_name == name);
            if let Some(column_def) =
                self.generate_column_definition(field, schema, is_autoincrement_pk)?
            {
                lines.push(format!("  {}", column_def));
            }
        }

        let pk_fields = model.primary_key.fields();
        if !pk_fields.is_empty() && sqlite_inline_pk.is_none() {
            let pk_columns = pk_fields
                .iter()
                .map(|name| {
                    let field = model.find_field(name).unwrap();
                    self.quote_identifier(&field.db_name)
                })
                .collect::<Vec<_>>()
                .join(", ");
            lines.push(format!("  PRIMARY KEY ({})", pk_columns));
        }

        for unique_constraint in &model.unique_constraints {
            let unique_columns = unique_constraint
                .fields
                .iter()
                .map(|name| {
                    let field = model.find_field(name).unwrap();
                    self.quote_identifier(&field.db_name)
                })
                .collect::<Vec<_>>()
                .join(", ");
            lines.push(format!("  UNIQUE ({})", unique_columns));
        }

        for field in &model.fields {
            if let ResolvedFieldType::Relation(rel) = &field.field_type {
                if !rel.fields.is_empty() {
                    let fk_columns = rel
                        .fields
                        .iter()
                        .map(|name| {
                            let fk_field = model.find_field(name).unwrap();
                            self.quote_identifier(&fk_field.db_name)
                        })
                        .collect::<Vec<_>>()
                        .join(", ");

                    let target_model = schema.models.get(&rel.target_model).unwrap();
                    let ref_columns = rel
                        .references
                        .iter()
                        .map(|name| {
                            let ref_field = target_model.find_field(name).unwrap();
                            self.quote_identifier(&ref_field.db_name)
                        })
                        .collect::<Vec<_>>()
                        .join(", ");

                    let mut fk_def = format!(
                        "  FOREIGN KEY ({}) REFERENCES {} ({})",
                        fk_columns,
                        self.quote_table(&model_table(target_model)),
                        ref_columns
                    );

                    if let Some(action) = &rel.on_delete {
                        fk_def.push_str(&format!(
                            " ON DELETE {}",
                            self.referential_action_sql(action)
                        ));
                    }

                    if let Some(action) = &rel.on_update {
                        fk_def.push_str(&format!(
                            " ON UPDATE {}",
                            self.referential_action_sql(action)
                        ));
                    }

                    lines.push(fk_def);
                }
            }
        }
        if self.provider != DatabaseProvider::Sqlite {
            for field in managed_scalar_fields(model) {
                if let Some(ref check_expr) = field.check {
                    let cname = check_constraint_name(&model.db_name, Some(&field.db_name));
                    lines.push(format!(
                        "  CONSTRAINT {} CHECK ({})",
                        self.quote_identifier(&cname),
                        check_expr,
                    ));
                }
            }
        }
        for check_expr in &model.check_constraints {
            if self.provider == DatabaseProvider::Sqlite {
                lines.push(format!("  CHECK ({})", check_expr));
            } else {
                let cname = check_constraint_name(&model.db_name, None);
                lines.push(format!(
                    "  CONSTRAINT {} CHECK ({})",
                    self.quote_identifier(&cname),
                    check_expr,
                ));
            }
        }

        let table_sql = format!(
            "CREATE TABLE IF NOT EXISTS {} (\n{}\n)",
            self.quote_table(&model_table(model)),
            lines.join(",\n")
        );

        Ok(table_sql)
    }

    /// Generate referential action SQL
    fn referential_action_sql(
        &self,
        action: &nautilus_schema::ast::ReferentialAction,
    ) -> &'static str {
        use nautilus_schema::ast::ReferentialAction;
        match action {
            ReferentialAction::Cascade => "CASCADE",
            ReferentialAction::Restrict => "RESTRICT",
            ReferentialAction::NoAction => "NO ACTION",
            ReferentialAction::SetNull => "SET NULL",
            ReferentialAction::SetDefault => "SET DEFAULT",
        }
    }
}
