use futures::{FutureExt, TryStreamExt};
use nautilus_client::{
    Client, EngineMode, Order, OrderCreateEntry, OrderCreateInput, OrderDeleteArgs,
    OrderUpdateArgs, OrderUpdateInput,
};
use nautilus_connector::PgExecutor;
use nautilus_core::{FindManyArgs, FindUniqueArgs};
use nautilus_dialect::Sql;
use std::panic::AssertUnwindSafe;

async fn sql(client: &Client<PgExecutor>, text: &str) {
    nautilus_client::execute_all(
        client.executor(),
        &Sql {
            text: text.into(),
            params: vec![],
        },
    )
    .await
    .unwrap();
}

fn check(row: &Order, id: i32, empty: bool) {
    assert_eq!(row.id, id);
    assert_eq!(
        row.contact_names,
        if empty {
            vec![]
        } else {
            vec!["Alice", "", "O'Reilly"]
        }
    );
    assert_eq!(row.contact_namex, if empty { vec![] } else { vec!["Bob"] });
    assert_eq!(
        row.contact_notes.as_deref(),
        if empty { None } else { Some("caffè") }
    );
}

#[tokio::test]
async fn aliases_work_in_direct_and_embedded_modes() {
    let client = Client::postgres(&std::env::var("NAUTILUS_TEST_POSTGRES_URL").unwrap())
        .await
        .unwrap();
    sql(&client, "CREATE TABLE \"__TABLE__\" (order_identifier INTEGER PRIMARY KEY, contact_names TEXT[] NOT NULL, contact_namex TEXT[] NOT NULL, contact_notes TEXT)").await;
    let result = AssertUnwindSafe(async {
        for mode in [EngineMode::Auto, EngineMode::Always, EngineMode::Never] {
            sql(&client, "TRUNCATE TABLE \"__TABLE__\"").await;
            let client = client.clone().with_engine_mode(mode);
            let order = Order::nautilus(&client);
            let created = order
                .create(OrderCreateInput {
                    id: Some(1),
                    contact_names: Some(vec!["Alice".into(), "".into(), "O'Reilly".into()]),
                    contact_namex: Some(vec!["Bob".into()]),
                    contact_notes: Some(Some("caffè".into())),
                })
                .await
                .unwrap();
            check(&created, 1, false);
            for _ in 0..2 {
                check(
                    &order
                        .find_unique(FindUniqueArgs::new(Order::id().eq(1)))
                        .await
                        .unwrap()
                        .unwrap(),
                    1,
                    false,
                );
                let rows = order.find_many(FindManyArgs::default()).await.unwrap();
                assert_eq!(rows.len(), 1);
                check(&rows[0], 1, false);
            }
            let projection = order
                .find_many_select(FindManyArgs::default(), |c| {
                    (c.contact_names(), c.contact_namex(), c.contact_notes())
                })
                .await
                .unwrap();
            assert_eq!(
                projection,
                vec![(
                    created.contact_names.clone(),
                    created.contact_namex.clone(),
                    created.contact_notes.clone()
                )]
            );
            let projected = order
                .find_unique_select(FindUniqueArgs::new(Order::id().eq(1)), |c| {
                    (c.contact_names(), c.contact_namex())
                })
                .await
                .unwrap()
                .unwrap();
            assert_eq!(
                projected,
                (created.contact_names.clone(), created.contact_namex.clone())
            );
            let rows = order
                .create_many(vec![OrderCreateEntry {
                    id: 2,
                    contact_names: vec![],
                    contact_namex: vec![],
                    contact_notes: None,
                }])
                .await
                .unwrap();
            check(&rows[0], 2, true);
            let streamed = Box::pin(order.stream_many(FindManyArgs::default()).unwrap())
                .try_collect::<Vec<_>>()
                .await
                .unwrap();
            assert_eq!(streamed.len(), 2);
            for row in streamed {
                check(&row, row.id, row.id == 2);
            }
            let updated = order
                .update(OrderUpdateArgs {
                    where_: Some(Order::id().eq(1)),
                    data: OrderUpdateInput {
                        contact_names: Some(vec![]),
                        contact_namex: Some(vec![]),
                        contact_notes: Some(None),
                        ..Default::default()
                    },
                })
                .await
                .unwrap();
            check(&updated[0], 1, true);
            let deleted = order
                .delete(OrderDeleteArgs {
                    where_: Some(Order::id().eq(1)),
                })
                .await
                .unwrap()
                .unwrap();
            check(&deleted, 1, true);
        }
    })
    .catch_unwind()
    .await;
    sql(&client, "DROP TABLE \"__TABLE__\"").await;
    if let Err(panic) = result {
        std::panic::resume_unwind(panic);
    }
}
