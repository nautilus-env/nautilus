use super::DdlGenerator;
use crate::live::model_table;
use crate::provider::{CreateIndex, ProviderStrategy};
use nautilus_schema::ir::ModelIr;

impl DdlGenerator {
    /// Generate standalone CREATE INDEX statements for the model's `@@index` declarations.
    ///
    /// Unique constraints are emitted inline in `generate_create_table`, so this
    /// only covers explicit secondary indexes.
    pub(crate) fn generate_create_indexes_for_model(&self, model: &ModelIr) -> Vec<String> {
        let strategy = ProviderStrategy::new(self.provider);
        model
            .indexes
            .iter()
            .map(|idx| {
                let columns: Vec<String> = idx
                    .fields
                    .iter()
                    .map(|name| {
                        model
                            .find_field(name)
                            .map(|f| f.db_name.clone())
                            .unwrap_or_else(|| name.clone())
                    })
                    .collect();
                let mut name_parts = columns.clone();
                name_parts.sort();
                let index_name = idx
                    .map
                    .clone()
                    .unwrap_or_else(|| format!("idx_{}_{}", model.db_name, name_parts.join("_")));
                strategy.create_index_sql(CreateIndex {
                    table: &model_table(model),
                    name: &index_name,
                    columns: &columns,
                    unique: false,
                    kind: &idx.kind,
                    if_not_exists: true,
                    predicate: idx.predicate.as_deref(),
                })
            })
            .collect()
    }
}
