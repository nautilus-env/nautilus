use super::{assert_user, common, data, fixture, user_value};
use nautilus_client::{EngineMode, FromRow, User};
use nautilus_connector::Row;
use nautilus_core::{FindManyArgs, FindUniqueArgs, IncludeRelation, OrderBy};
use nautilus_engine::{
    handlers::{self, EmbeddedResponse},
    EngineState,
};
use nautilus_protocol::{ProtocolError, RpcError, RpcRequest, PROTOCOL_VERSION};
use serde_json::{json, Value};

fn params(model: &str) -> Value {
    json!({"protocolVersion": PROTOCOL_VERSION, "model": model})
}

fn request(method: &str, params: &Value) -> RpcRequest {
    RpcRequest {
        jsonrpc: "2.0".into(),
        id: Some(nautilus_protocol::RpcId::Number(17)),
        method: method.into(),
        params: serde_json::value::to_raw_value(params).unwrap(),
    }
}

fn rows_json(rows: &[Row]) -> Value {
    json!({"data": rows.iter().map(|row| {
        row.iter().map(|(key, value)| (key.to_string(), value.to_json_plain())).collect::<serde_json::Map<_, _>>()
    }).collect::<Vec<_>>()})
}

fn embedded_json(response: EmbeddedResponse) -> Value {
    match response {
        EmbeddedResponse::Rows(rows) => rows_json(&rows),
        EmbeddedResponse::Count(count) => json!({"count": count}),
        EmbeddedResponse::Json(value) => serde_json::from_str(value.get()).unwrap(),
    }
}

#[derive(Clone, Copy, Debug)]
enum Path {
    Rpc,
    Embedded,
    Typed,
}

async fn call(state: &EngineState, path: Path, method: &str, params: Value) -> Value {
    let mut result = match path {
        Path::Rpc => common::call_rpc_json(state, method, params).await,
        Path::Embedded => embedded_json(common::call_embedded(state, method, params).await),
        Path::Typed => match method {
            "query.create" => rows_json(
                &handlers::handle_create_typed(state, serde_json::from_value(params).unwrap())
                    .await
                    .unwrap(),
            ),
            "query.upsert" => rows_json(
                &handlers::handle_upsert_typed(state, serde_json::from_value(params).unwrap())
                    .await
                    .unwrap(),
            ),
            "query.updateMany" => {
                json!({"count": handlers::handle_update_many_typed(state, serde_json::from_value(params).unwrap()).await.unwrap()})
            }
            "query.deleteMany" => {
                json!({"count": handlers::handle_delete_many_typed(state, serde_json::from_value(params).unwrap()).await.unwrap()})
            }
            "query.count" => {
                json!({"count": handlers::handle_count_typed(state, serde_json::from_value(params).unwrap()).await.unwrap()})
            }
            _ => panic!("unsupported test operation: {method}"),
        },
    };
    if matches!(method, "query.create" | "query.upsert") {
        let count = json!(result["data"].as_array().unwrap().len());
        if matches!(path, Path::Rpc) {
            assert_eq!(result["count"], count);
        } else {
            result["count"] = count;
        }
    }
    result
}

#[tokio::test]
async fn mutations_and_counts_match_rpc_embedded_and_typed() {
    let mut transcripts = Vec::new();
    for path in [Path::Rpc, Path::Embedded, Path::Typed] {
        let (state, _client, _dir) = fixture().await;
        let mut transcript = Vec::new();
        let mut create = params("User");
        create["data"] = data();
        let created = call(&state, path, "query.create", create).await;
        assert_eq!(created["data"][0]["app_users__user_id"], 1);
        assert_eq!(created["data"][0]["app_users__display_name"], Value::Null);
        assert_eq!(created["data"][0]["app_users__account_role"], "ADMIN");
        transcript.push(created);

        for email in ["second@example.com", "second@example.com"] {
            let mut upsert = params("User");
            upsert["filter"] = json!({"email": email});
            upsert["create"] = data();
            upsert["create"]["email"] = json!(email);
            upsert["update"] = json!({"name": "Updated", "balance": {"increment": "0.25"}});
            upsert["returnData"] = json!(true);
            let result = call(&state, path, "query.upsert", upsert).await;
            transcript.push(result);
        }

        let mut update = params("User");
        update["filter"] = json!({"role": "ADMIN"});
        update["data"] = json!({"name": null});
        let result = call(&state, path, "query.updateMany", update).await;
        assert_eq!(result, json!({"count": 2}));
        transcript.push(result);
        for args in [
            json!({}),
            json!({"where": {"name": null}, "skip": 1, "take": 1}),
            json!({"where": {"email": "absent"}}),
        ] {
            let mut count = params("User");
            count["args"] = args;
            transcript.push(call(&state, path, "query.count", count).await);
        }
        assert_eq!(
            &transcript[4..],
            &[
                json!({"count": 2}),
                json!({"count": 1}),
                json!({"count": 0})
            ]
        );
        let rows = handlers::handle_find_many_typed(
            &state,
            "User",
            &FindManyArgs {
                order_by: vec![OrderBy::asc("id")],
                ..Default::default()
            },
            None,
        )
        .await
        .unwrap();
        transcript.push(rows_json(&rows));
        let mut delete = params("User");
        delete["filter"] = json!({"email": "second@example.com"});
        for expected in [1, 0] {
            let deleted = call(&state, path, "query.deleteMany", delete.clone()).await;
            assert_eq!(deleted, json!({"count": expected}));
            transcript.push(deleted);
        }
        transcripts.push(transcript);
    }
    assert_eq!(transcripts[0], transcripts[1], "RPC versus embedded");
    assert_eq!(transcripts[0], transcripts[2], "RPC versus typed");
}

#[tokio::test]
async fn mapped_scalar_reads_match_all_five_surfaces() {
    let (state, client, _dir) = fixture().await;
    let mut create = params("User");
    create["data"] = data();
    common::call_rpc_json(&state, "query.create", create).await;
    let args = FindManyArgs {
        where_: Some(User::email().eq("o'hara@example.com")),
        ..Default::default()
    };
    let mut read = params("User");
    read["args"] = json!({"where": {"email": "o'hara@example.com"}});
    let rpc = common::call_rpc_json(&state, "query.findMany", read.clone()).await;
    assert_eq!(
        rpc,
        embedded_json(common::call_embedded(&state, "query.findMany", read).await)
    );
    let rows = handlers::handle_find_many_typed(&state, "User", &args, None)
        .await
        .unwrap();
    assert_eq!(rpc, rows_json(&rows));
    assert!(matches!(
        rows[0].get("app_users__account_balance"),
        Some(nautilus_core::Value::Decimal(_))
    ));
    assert!(matches!(
        rows[0].get("app_users__joined_at"),
        Some(nautilus_core::Value::DateTime(_))
    ));
    let user = User::from_row(&rows[0]).unwrap();
    assert_user(&user);
    for mode in [EngineMode::Auto, EngineMode::Always, EngineMode::Never] {
        let client = client.clone().with_engine_mode(mode);
        let users = User::nautilus(&client)
            .find_many(args.clone())
            .await
            .unwrap();
        assert_eq!(users.len(), 1);
        assert_eq!(user_value(&users[0]), user_value(&user), "{mode:?}");
        let unique = User::nautilus(&client)
            .find_unique(FindUniqueArgs::new(User::email().eq("o'hara@example.com")))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(user_value(&unique), user_value(&user));
        assert!(User::nautilus(&client)
            .find_unique(FindUniqueArgs::new(User::email().eq("missing")))
            .await
            .unwrap()
            .is_none());
    }
}

#[tokio::test]
async fn includes_keep_mapping_filter_and_order_across_engine_surfaces() {
    let (state, client, _dir) = fixture().await;
    User::nautilus(&client)
        .create(super::input())
        .await
        .unwrap();
    for title in ["C", "A", "B"] {
        nautilus_client::Post::nautilus(&client)
            .create(nautilus_client::PostCreateInput {
                title: Some(title.into()),
                author_id: Some(1),
                ..Default::default()
            })
            .await
            .unwrap();
    }
    let include = IncludeRelation::with_filter(
        nautilus_core::Expr::column("blog_posts__post_title").lt(nautilus_core::Expr::param("C")),
    )
    .with_order_by(OrderBy::desc("title"))
    .with_skip(1)
    .with_take(1);
    let args = FindManyArgs {
        include: std::collections::HashMap::from([("posts".into(), include.clone())]),
        ..Default::default()
    };
    let mut read = params("User");
    read["args"] = json!({"include": {"posts": {"where": {"title": {"lt": "C"}}, "orderBy": [{"title": "desc"}], "skip": 1, "take": 1}}});
    let rpc = common::call_rpc_json(&state, "query.findMany", read.clone()).await;
    assert_eq!(
        rpc,
        embedded_json(common::call_embedded(&state, "query.findMany", read).await)
    );
    let typed = handlers::handle_find_many_typed(&state, "User", &args, None)
        .await
        .unwrap();
    assert_eq!(rpc, rows_json(&typed));
    assert_eq!(rpc["data"][0]["posts_json"][0]["title"], "A");
    for mode in [EngineMode::Auto, EngineMode::Always] {
        let users = User::nautilus(&client.clone().with_engine_mode(mode))
            .find_many(args.clone())
            .await
            .unwrap();
        let posts = &users[0].posts;
        assert_eq!(posts.len(), 1);
        assert_eq!(posts[0].title, "A");
    }
    assert!(User::nautilus(&client.with_engine_mode(EngineMode::Never))
        .find_many(args)
        .await
        .unwrap_err()
        .to_string()
        .contains("include queries require the embedded engine"));
}

#[tokio::test]
async fn error_codes_messages_and_details_match_all_engine_adapters() {
    let (state, _client, _dir) = fixture().await;
    for (model, data) in [
        ("Missing", data()),
        ("User", json!({"email": "x", "balance": "not-a-decimal"})),
        ("User", json!({"unknown": true})),
    ] {
        let mut create = params(model);
        create["data"] = data;
        let rpc = handlers::handle_request_inline(&state, request("query.create", &create))
            .await
            .error
            .unwrap();
        let embedded = handlers::handle_request_embedded(&state, request("query.create", &create))
            .await
            .unwrap_err();
        let typed = handlers::handle_create_typed(&state, serde_json::from_value(create).unwrap())
            .await
            .unwrap_err();
        for error in [embedded, typed] {
            let error: RpcError = error.into();
            assert_eq!(error.code, rpc.code);
            assert_eq!(error.message, rpc.message);
            assert_eq!(error.data, rpc.data);
        }
        assert_ne!(rpc.code, ProtocolError::Internal(String::new()).code());
    }
}
