//! Bounded PostgreSQL projection aliases and their original result names.

use nautilus_core::{ColumnMarker, Select, SelectItem};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

#[derive(Default)]
pub(crate) struct OutputAliases {
    names: HashMap<String, String>,
    computed: HashSet<String>,
    restored: Vec<(String, Arc<str>)>,
}

impl OutputAliases {
    pub(crate) fn for_select(select: &Select) -> Self {
        let items = select
            .items
            .iter()
            .chain(select.joins.iter().flat_map(|join| &join.items));
        if !items.clone().any(|item| match item {
            SelectItem::Column(column) => column.table.len() + 2 + column.name.len() > 63,
            SelectItem::Computed { alias, .. } => alias.len() > 63,
        }) {
            return Self::default();
        }
        let mut result = Self::new(items.clone().map(|item| match item {
            SelectItem::Column(column) => column_name(column),
            SelectItem::Computed { alias, .. } => alias.clone(),
        }));
        result.computed = items
            .filter_map(|item| match item {
                SelectItem::Computed { alias, .. } if alias.len() > 63 => Some(alias.clone()),
                _ => None,
            })
            .collect();
        result
    }

    pub(crate) fn for_returning(columns: &[ColumnMarker]) -> Self {
        if !columns
            .iter()
            .any(|column| column.table.len() + 2 + column.name.len() > 63)
        {
            return Self::default();
        }
        Self::new(columns.iter().map(column_name))
    }

    fn new(names: impl Iterator<Item = String>) -> Self {
        let names: Vec<_> = names.collect();
        // Reserve all original names, including user-specified computed aliases,
        // so generated aliases cannot collide with a short projection name.
        let mut used: HashSet<_> = names.iter().cloned().collect();
        used.insert(crate::ident::WINDOW_ROW_NUMBER_ALIAS.into());
        let mut result = Self::default();
        let mut next = 0;
        for name in names {
            // String::len counts UTF-8 bytes, as PostgreSQL does for identifiers.
            if name.len() <= 63 || result.names.contains_key(&name) {
                continue;
            }
            let alias = loop {
                let candidate = format!("__nautilus_col_{next}");
                next += 1;
                if used.insert(candidate.clone()) {
                    break candidate;
                }
            };
            result
                .restored
                .push((alias.clone(), Arc::from(name.as_str())));
            result.names.insert(name, alias);
        }
        result
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.names.is_empty()
    }

    pub(crate) fn sql_name<'a>(&'a self, original: &'a str) -> &'a str {
        self.names.get(original).map_or(original, String::as_str)
    }

    pub(crate) fn computed_sql_name(&self, original: &str) -> Option<&str> {
        self.computed
            .contains(original)
            .then(|| self.names.get(original).map(String::as_str))
            .flatten()
    }

    pub(crate) fn into_restored(self) -> Vec<(String, Arc<str>)> {
        self.restored
    }
}

pub(crate) fn column_name(column: &ColumnMarker) -> String {
    format!("{}__{}", column.table, column.name)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn aliases_reserve_short_names_and_preserve_distinct_long_names() {
        let prefix = "a".repeat(63);
        let first = format!("{prefix}x");
        let second = format!("{prefix}y");
        let unicode = "è".repeat(32);
        let aliases = OutputAliases::new(
            [
                first.clone(),
                second.clone(),
                unicode.clone(),
                first.clone(),
                "__nautilus_col_0".into(),
                "a".repeat(63),
            ]
            .into_iter(),
        );
        assert_eq!(aliases.sql_name(&first), "__nautilus_col_1");
        assert_eq!(aliases.sql_name(&second), "__nautilus_col_2");
        assert_eq!(aliases.sql_name(&unicode), "__nautilus_col_3");
        assert_eq!(aliases.sql_name(&"a".repeat(63)), "a".repeat(63));
        assert_eq!(aliases.sql_name("__nautilus_col_0"), "__nautilus_col_0");
        assert_eq!(aliases.restored.len(), 3);
    }
}
