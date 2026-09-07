use super::{pgvector, DatabaseProvider, ProviderStrategy};
use nautilus_core::TableName;
use nautilus_schema::ir::{BasicIndexType, IndexKind};

pub(crate) struct CreateIndex<'a> {
    pub(crate) table: &'a TableName,
    pub(crate) name: &'a str,
    pub(crate) columns: &'a [String],
    pub(crate) unique: bool,
    pub(crate) kind: &'a IndexKind,
    pub(crate) if_not_exists: bool,
    /// Partial-index predicate, already rendered against physical column names.
    pub(crate) predicate: Option<&'a str>,
}

impl ProviderStrategy {
    pub(crate) fn create_index_sql(&self, index: CreateIndex<'_>) -> String {
        let q = |name: &str| self.provider.quote_identifier(name);
        let unique_kw = if index.unique { "UNIQUE " } else { "" };

        let opclass_suffix = match index.kind {
            IndexKind::Pgvector(p) => pgvector::opclass_suffix(p),
            _ => None,
        };

        let columns_sql = index
            .columns
            .iter()
            .enumerate()
            .map(|(idx, column)| {
                let mut rendered = q(column);
                if idx == 0 {
                    if let Some(suffix) = opclass_suffix {
                        rendered.push(' ');
                        rendered.push_str(suffix);
                    }
                }
                rendered
            })
            .collect::<Vec<_>>()
            .join(", ");

        let where_clause = match index.predicate {
            Some(predicate) if self.provider != DatabaseProvider::Mysql => {
                format!(" WHERE {}", predicate)
            }
            _ => String::new(),
        };

        if self.provider == DatabaseProvider::Mysql
            && matches!(index.kind, IndexKind::Basic(BasicIndexType::FullText))
        {
            return format!(
                "CREATE FULLTEXT INDEX {} ON {} ({})",
                q(index.name),
                self.quote_table(index.table),
                columns_sql,
            );
        }

        let using_clause = self.using_clause(index.kind);
        let with_clause = match (self.provider, index.kind) {
            (DatabaseProvider::Postgres, IndexKind::Pgvector(p)) => {
                pgvector::with_clause(p.method, &p.options)
            }
            _ => String::new(),
        };

        match self.provider {
            DatabaseProvider::Postgres | DatabaseProvider::Sqlite => {
                let if_not_exists = if index.if_not_exists {
                    " IF NOT EXISTS"
                } else {
                    ""
                };
                format!(
                    "CREATE {}INDEX{} {} ON {}{} ({})",
                    unique_kw,
                    if_not_exists,
                    q(index.name),
                    self.quote_table(index.table),
                    using_clause,
                    columns_sql,
                ) + &with_clause
                    + &where_clause
            }
            DatabaseProvider::Mysql => format!(
                "CREATE {}INDEX {} ON {} ({}){}",
                unique_kw,
                q(index.name),
                self.quote_table(index.table),
                columns_sql,
                using_clause,
            ),
        }
    }

    /// Omit the default BTree method. MySQL FULLTEXT uses dedicated CREATE syntax
    /// in [`Self::create_index_sql`] rather than a USING clause.
    fn using_clause(&self, kind: &IndexKind) -> String {
        match (self.provider, kind) {
            (_, IndexKind::Default) => String::new(),
            (_, IndexKind::Basic(BasicIndexType::BTree)) => String::new(),
            (DatabaseProvider::Postgres, IndexKind::Basic(b)) => {
                format!(" USING {}", b.as_ddl_str().to_uppercase())
            }
            (DatabaseProvider::Postgres, IndexKind::Pgvector(p)) => {
                format!(" USING {}", p.method.as_ddl_str().to_uppercase())
            }
            (DatabaseProvider::Mysql, IndexKind::Basic(BasicIndexType::Hash)) => {
                " USING HASH".to_string()
            }
            _ => String::new(),
        }
    }

    pub(crate) fn drop_index_sql(&self, table: &TableName, name: &str) -> String {
        let name = self.provider.quote_identifier(name);
        match self.provider {
            DatabaseProvider::Postgres | DatabaseProvider::Sqlite => {
                format!("DROP INDEX IF EXISTS {}", name)
            }
            DatabaseProvider::Mysql => {
                format!("DROP INDEX {} ON {}", name, self.quote_table(table))
            }
        }
    }
}
