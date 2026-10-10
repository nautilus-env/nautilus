use nautilus_core::{
    ColumnMarker, Delete, Expr, Insert, OrderDir, PartitionWindow, Select, SelectItem, Update,
    Value,
};
use nautilus_dialect::{Dialect, MysqlDialect, PostgresDialect, SqliteDialect};

#[test]
fn postgres_bounded_aliases_cover_projection_joins_windows_and_returning() {
    let table = format!("orders_{}", "a".repeat(42));
    let first = ColumnMarker::new(&table, "contact_names");
    let joined_table = format!("orders_{}", "b".repeat(42));
    let second = ColumnMarker::new(&joined_table, "contact_namex");
    let computed = "è".repeat(32);
    let select = Select::from_table(&table)
        .item(SelectItem::Column(first.clone()))
        .item(SelectItem::computed(Expr::param(Value::I32(7)), &computed))
        .item(SelectItem::computed(
            Expr::param(Value::I32(9)),
            "__nautilus_col_0",
        ))
        .left_join(
            &joined_table,
            Expr::param(Value::Bool(true)),
            vec![SelectItem::Column(second.clone())],
        )
        .partition_window(PartitionWindow {
            partition_by: vec![],
            skip: 0,
            take: Some(2),
        })
        .build()
        .unwrap();
    let output = PostgresDialect.render_select_output_owned(select).unwrap();
    assert_eq!(output.aliases.len(), 3);
    assert_eq!(output.aliases[0].0, "__nautilus_col_1");
    assert_eq!(
        output.aliases[0].1.as_ref(),
        format!("{table}__contact_names")
    );
    assert_eq!(output.aliases[1].1.as_ref(), computed);
    assert_eq!(
        output.aliases[2].1.as_ref(),
        format!("{joined_table}__contact_namex")
    );
    assert!(output.text.starts_with("SELECT \"__nautilus_col_1\", \"__nautilus_col_2\", \"__nautilus_col_0\", \"__nautilus_col_3\" FROM ("));
    for (alias, _) in &output.aliases {
        assert!(alias.len() <= 63);
    }
    assert_eq!(
        output.params,
        vec![Value::I32(7), Value::I32(9), Value::Bool(true)]
    );

    let second = ColumnMarker::new(&table, "contact_namex");
    let insert = Insert::into_table(&table)
        .columns(vec![first.clone()])
        .values(vec![Value::String("Alice".into())])
        .returning(vec![first.clone(), second.clone()])
        .build()
        .unwrap();
    let update = Update::table(&table)
        .set(first.clone(), Value::String("Bob".into()))
        .returning(vec![first.clone(), second.clone()])
        .build()
        .unwrap();
    let delete = Delete::from_table(&table)
        .returning(vec![first, second])
        .build()
        .unwrap();
    for output in [
        PostgresDialect.render_insert_output_owned(insert).unwrap(),
        PostgresDialect.render_update_output_owned(update).unwrap(),
        PostgresDialect.render_delete_output_owned(delete).unwrap(),
    ] {
        assert_eq!(output.aliases.len(), 2);
        assert!(output.text.contains(" AS \"__nautilus_col_0\""));
        assert!(output.text.contains(" AS \"__nautilus_col_1\""));
    }
}

#[test]
fn postgres_order_by_and_distinct_use_the_bounded_computed_alias() {
    let alias = "è".repeat(32);
    let select = Select::from_table("orders")
        .item(SelectItem::computed(Expr::param(Value::I32(7)), &alias))
        .distinct(vec![alias.clone()])
        .order_by(&alias, OrderDir::Asc)
        .build()
        .unwrap();
    let output = PostgresDialect.render_select_output_owned(select).unwrap();
    assert!(output
        .text
        .starts_with("SELECT DISTINCT ON (\"__nautilus_col_0\") "));
    assert!(output.text.ends_with(" ORDER BY \"__nautilus_col_0\" ASC"));
    assert_eq!(output.aliases[0].1.as_ref(), alias);
}

#[test]
fn short_aliases_and_other_providers_keep_their_sql() {
    let select = Select::from_table("orders")
        .item(SelectItem::Column(ColumnMarker::new("orders", "id")))
        .build()
        .unwrap();
    for dialect in [
        &PostgresDialect as &dyn Dialect,
        &SqliteDialect,
        &MysqlDialect,
    ] {
        let sql = dialect.render_select(&select).unwrap();
        let output = dialect.render_select_output_owned(select.clone()).unwrap();
        assert_eq!(output.sql, sql);
        assert!(output.aliases.is_empty());
    }
}
