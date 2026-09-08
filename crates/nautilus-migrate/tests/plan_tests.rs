mod common;

use nautilus_core::TableName;
use nautilus_migrate::{
    ApplyFailure, ApplyPlan, Change, DatabaseProvider, DdlGenerator, DiffApplier, GroupStatus,
    LiveSchema, MigrationFileStore, RollbackBehavior, TransactionRequirement,
};

fn plan(provider: DatabaseProvider, changes: &[Change]) -> ApplyPlan {
    let schema = common::parse("model Box { id Int @id }").unwrap();
    let live = LiveSchema::default();
    let ddl = DdlGenerator::new(provider);
    let applier = DiffApplier::new(provider, &ddl, &schema, &live);
    ApplyPlan::from_changes(
        provider,
        changes
            .iter()
            .map(|c| applier.plan_for(c).unwrap())
            .collect(),
    )
}

#[test]
fn enum_phases_keep_change_origin_and_partial_outcomes_across_file_export() {
    let plan = plan(
        DatabaseProvider::Postgres,
        &[
            Change::CreateSchema { name: "app".into() },
            Change::AlterEnum {
                name: "shade".into(),
                added_variants: vec!["blue".into(), "green".into()],
                removed_variants: vec![],
            },
            Change::DefaultChanged {
                table: TableName::new("Box"),
                column: "shade".into(),
                from: None,
                to: Some("'green'".into()),
            },
        ],
    );
    let phases = plan.phases();
    assert_eq!(phases.len(), 4);
    assert_eq!(phases[0].transaction(), TransactionRequirement::Shared);
    assert_eq!(phases[1].transaction(), TransactionRequirement::Standalone);
    assert_eq!(phases[2].transaction(), TransactionRequirement::Standalone);
    assert_eq!(phases[3].transaction(), TransactionRequirement::Shared);
    assert_eq!(plan.changes()[1].statement_range(), 1..3);
    assert!(matches!(
        plan.changes()[1].change(),
        Change::AlterEnum { .. }
    ));
    assert!(!plan.changes()[0].reversal().is_automatic());
    assert!(plan.changes()[0].reversal().statements().is_empty());
    assert!(!plan.changes()[1].reversal().is_automatic());
    assert!(plan.changes()[2].reversal().is_automatic());

    let outcome = plan.stopped(
        &phases[2],
        0,
        ApplyFailure {
            statement: plan.statements()[2].clone(),
            message: "failure".into(),
        },
    );
    assert_eq!(
        (outcome.committed, outcome.rolled_back, outcome.not_applied),
        (2, 0, 2)
    );
    assert_eq!(
        plan.classify_changes(&outcome),
        vec![
            GroupStatus::Applied,
            GroupStatus::Failed { committed: 1 },
            GroupStatus::NotAttempted,
        ]
    );

    let migration = plan.clone().into_migration("enum_change".into());
    assert!(migration.verify_checksum());
    assert!(migration.down_sql[0].contains("DROP DEFAULT"));
    assert!(migration.down_sql[1].starts_with("-- Cannot auto-reverse"));
    let dir = tempfile::tempdir().unwrap();
    let store = MigrationFileStore::new(dir.path());
    store
        .write_migration(&migration.name, &migration.up_sql, &migration.down_sql)
        .unwrap();
    let loaded = store.load_migration(&migration.name).unwrap();
    assert_eq!(loaded.checksum, migration.checksum);
    assert_eq!(loaded.down_sql, migration.down_sql);
    let from_file = ApplyPlan::from_sql(DatabaseProvider::Postgres, &loaded.up_sql);
    assert_eq!(from_file.phases(), phases);
    assert_eq!(from_file.statements(), plan.statements());
    assert!(from_file.changes().is_empty());
}

#[test]
fn generated_composite_attributes_do_not_mistake_quoted_names_for_enum_additions() {
    let plan = plan(
        DatabaseProvider::Postgres,
        &[Change::AlterCompositeType {
            name: "contains ADD VALUE text".into(),
            added_fields: vec![
                ("first".into(), "TEXT".into()),
                ("second".into(), "INT".into()),
            ],
            dropped_fields: vec![],
            type_changed_fields: vec![],
        }],
    );
    assert_eq!(plan.phases().len(), 1);
    assert_eq!(plan.phases()[0].statement_range(), 0..2);
    assert_eq!(
        plan.phases()[0].transaction(),
        TransactionRequirement::Shared
    );
    assert_eq!(plan.phases()[0].rollback(), RollbackBehavior::Transactional);
    assert!(nautilus_migrate::requires_own_transaction(
        &plan.statements()[0]
    ));
}

#[test]
fn provider_rollback_contract_drives_failure_reporting() {
    let changes = [
        Change::DroppedTable {
            name: TableName::new("first"),
        },
        Change::DroppedTable {
            name: TableName::new("second"),
        },
    ];
    for provider in [
        DatabaseProvider::Postgres,
        DatabaseProvider::Mysql,
        DatabaseProvider::Sqlite,
    ] {
        let plan = plan(provider, &changes);
        assert_eq!(plan.phases().len(), 1);
        let phase = &plan.phases()[0];
        assert_eq!(phase.statement_range(), 0..2);
        let outcome = plan.stopped(
            phase,
            1,
            ApplyFailure {
                statement: plan.statements()[1].clone(),
                message: "failure".into(),
            },
        );
        let expected = if provider == DatabaseProvider::Mysql {
            assert_eq!(phase.rollback(), RollbackBehavior::KeepsStatements);
            (1, 0, true)
        } else {
            assert_eq!(phase.rollback(), RollbackBehavior::Transactional);
            (0, 1, false)
        };
        assert_eq!(
            (
                outcome.committed,
                outcome.rolled_back,
                outcome.left_partial_state()
            ),
            expected
        );
        assert!(!plan.changes()[0].reversal().is_automatic());
    }
}

#[test]
fn unsupported_user_types_remain_no_op_changes() {
    let plan = plan(
        DatabaseProvider::Sqlite,
        &[Change::AlterEnum {
            name: "shade".into(),
            added_variants: vec!["blue".into()],
            removed_variants: vec![],
        }],
    );
    assert!(plan.statements().is_empty());
    assert!(plan.phases().is_empty());
    assert_eq!(plan.changes().len(), 1);
    assert!(plan.changes()[0].reversal().is_automatic());
    assert_eq!(plan.changes()[0].statement_range(), 0..0);
}
