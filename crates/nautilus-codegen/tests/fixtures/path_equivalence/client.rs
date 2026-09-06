use super::{assert_user, fixture, input, user_value};
use nautilus_client::{
    EngineMode, EventControl, EventPhase, User, UserCountArgs, UserCreateEventContext,
    UserUpdateArgs, UserUpdateEventContext, UserUpdateInput, UserUpsertArgs,
};
use nautilus_core::{FindManyArgs, FindUniqueArgs};
use serde_json::json;
use std::sync::{Arc, Mutex};

#[tokio::test]
async fn generated_mutations_preserve_values_and_upsert_return_data_in_every_mode() {
    let mut transcripts = Vec::new();
    for mode in [EngineMode::Auto, EngineMode::Always, EngineMode::Never] {
        let (_state, client, _dir) = fixture().await;
        assert_eq!(client.engine_mode(), EngineMode::Auto);
        let client = client.with_engine_mode(mode);
        let created = User::nautilus(&client).create(input()).await.unwrap();
        assert_user(&created);
        let mut transcript = vec![user_value(&created)];
        for return_data in [true, false] {
            let upserted = User::nautilus(&client)
                .upsert(UserUpsertArgs {
                    where_: Some(User::email().eq(created.email.clone())),
                    create: input(),
                    update: UserUpdateInput {
                        name: Some(Some("Updated".into())),
                        balance: Some(nautilus_client::NumericUpdate::Increment(
                            "0.25".parse().unwrap(),
                        )),
                        ..Default::default()
                    },
                    return_data,
                })
                .await
                .unwrap();
            assert_eq!(upserted.is_some(), return_data);
            transcript.push(
                upserted
                    .as_ref()
                    .map(user_value)
                    .unwrap_or(serde_json::Value::Null),
            );
        }
        let mut second = input();
        second.email = Some("second@example.com".into());
        let inserted = User::nautilus(&client)
            .upsert(UserUpsertArgs {
                where_: Some(User::email().eq("second@example.com")),
                create: second,
                return_data: true,
                ..Default::default()
            })
            .await
            .unwrap()
            .unwrap();
        assert!(inserted.id > created.id);
        let reloaded = User::nautilus(&client)
            .find_unique(FindUniqueArgs::new(User::id().eq(inserted.id)))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(user_value(&reloaded), user_value(&inserted));
        let mut inserted_value = user_value(&inserted);
        inserted_value.as_object_mut().unwrap().remove("id");
        transcript.push(inserted_value);
        let updated = User::nautilus(&client)
            .update(UserUpdateArgs {
                where_: Some(User::id().eq(1)),
                data: UserUpdateInput {
                    name: Some(None),
                    ..Default::default()
                },
            })
            .await
            .unwrap();
        assert_eq!(updated.len(), 1);
        assert_eq!(updated[0].name, None);
        assert_eq!(updated[0].balance, "1235.00".parse().unwrap());
        transcript.push(user_value(&updated[0]));
        match mode {
            EngineMode::Never => assert!(User::nautilus(&client)
                .count(UserCountArgs::default())
                .await
                .unwrap_err()
                .to_string()
                .contains("count queries require the embedded engine")),
            _ => assert_eq!(
                User::nautilus(&client)
                    .count(UserCountArgs::default())
                    .await
                    .unwrap(),
                2
            ),
        }
        transcripts.push(transcript);
    }
    assert_eq!(transcripts[0], transcripts[1], "Auto versus Always");
    assert_eq!(transcripts[0], transcripts[2], "Auto versus Never");
}

#[tokio::test]
async fn create_and_stopped_update_events_match_direct_and_engine() {
    let mut transcripts = Vec::new();
    for mode in [EngineMode::Auto, EngineMode::Always, EngineMode::Never] {
        let (_state, client, _dir) = fixture().await;
        let client = client.with_engine_mode(mode);
        let events = Arc::new(Mutex::new(Vec::new()));
        let before = Arc::clone(&events);
        client
            .events()
            .on_create::<UserCreateEventContext, User, _>("User", EventPhase::Before, move |ctx| {
                let before = Arc::clone(&before);
                Box::pin(async move {
                    before.lock().unwrap().push("before:create".to_string());
                    ctx.state.insert("marker".into(), json!("shared"));
                    Ok(EventControl::Continue)
                })
            });
        let after = Arc::clone(&events);
        client
            .events()
            .on_create::<UserCreateEventContext, User, _>("User", EventPhase::After, move |ctx| {
                let after = Arc::clone(&after);
                Box::pin(async move {
                    assert_eq!(ctx.state["marker"], "shared");
                    assert_user(ctx.result.as_ref().unwrap());
                    after.lock().unwrap().push("after:create".to_string());
                    Ok(EventControl::Continue)
                })
            });
        let stopped = Arc::clone(&events);
        client
            .events()
            .on_update_with_priority::<UserUpdateEventContext, Vec<User>, _>(
                "User",
                EventPhase::Before,
                2,
                move |_| {
                    let stopped = Arc::clone(&stopped);
                    Box::pin(async move {
                        stopped.lock().unwrap().push("stop:update".to_string());
                        Ok(EventControl::StopPropagation(Vec::new()))
                    })
                },
            );
        client
            .events()
            .on_update_with_priority::<UserUpdateEventContext, Vec<User>, _>(
                "User",
                EventPhase::Before,
                1,
                |_| Box::pin(async { panic!("lower priority hook ran after stop") }),
            );
        let created = User::nautilus(&client).create(input()).await.unwrap();
        let stopped = User::nautilus(&client)
            .update(UserUpdateArgs {
                where_: Some(User::id().eq(created.id)),
                data: UserUpdateInput {
                    name: Some(Some("Blocked".into())),
                    ..Default::default()
                },
            })
            .await
            .unwrap();
        assert!(stopped.is_empty());
        let reloaded = User::nautilus(&client)
            .find_unique(FindUniqueArgs::new(User::id().eq(created.id)))
            .await
            .unwrap()
            .unwrap();
        assert_user(&reloaded);
        let events = events.lock().unwrap().clone();
        assert_eq!(events, ["before:create", "after:create", "stop:update"]);
        transcripts.push(events);
    }
    assert_eq!(transcripts[0], transcripts[1]);
    assert_eq!(transcripts[0], transcripts[2]);
}

#[tokio::test]
async fn transaction_rollback_is_visible_in_direct_and_engine_reads() {
    for mode in [EngineMode::Auto, EngineMode::Always, EngineMode::Never] {
        let (_state, client, _dir) = fixture().await;
        let client = client.with_engine_mode(mode);
        let result = client
            .transaction(
                nautilus_client::TransactionOptions::default(),
                |tx| async move {
                    let created = User::nautilus(&tx).create(input()).await?;
                    assert_user(&created);
                    let inside = User::nautilus(&tx)
                        .find_unique(FindUniqueArgs::new(User::id().eq(created.id)))
                        .await?;
                    assert!(inside.is_some());
                    Err::<(), _>(nautilus_connector::ConnectorError::from(
                        nautilus_core::Error::Other("rollback probe".into()),
                    ))
                },
            )
            .await;
        assert!(result.unwrap_err().to_string().contains("rollback probe"));
        assert!(
            User::nautilus(&client)
                .find_many(FindManyArgs::default())
                .await
                .unwrap()
                .is_empty(),
            "{mode:?}"
        );
    }
}
