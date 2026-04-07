mod common;

use nautilus_migrate::live::{
    ComputedKind, LiveColumn, LiveCompositeField, LiveCompositeType, LiveIndex, LiveSchema,
    LiveTable,
};
use nautilus_migrate::{serialize_live_schema, DatabaseProvider};
use nautilus_schema::ir::{ResolvedFieldType, ScalarType};

#[test]
fn serialises_single_table() {
    let live = common::make_live_schema(vec![LiveTable {
        name: "users".to_string(),
        columns: vec![
            LiveColumn {
                name: "id".to_string(),
                col_type: "integer".to_string(),
                nullable: false,
                default_value: None,
                generated_expr: None,
                computed_kind: None,
                check_expr: None,
            },
            LiveColumn {
                name: "email".to_string(),
                col_type: "text".to_string(),
                nullable: false,
                default_value: None,
                generated_expr: None,
                computed_kind: None,
                check_expr: None,
            },
        ],
        primary_key: vec!["id".to_string()],
        indexes: vec![],
        check_constraints: vec![],
        foreign_keys: vec![],
    }]);

    let out = serialize_live_schema(
        &live,
        DatabaseProvider::Postgres,
        "postgres://localhost/test",
    );

    assert!(out.contains("datasource db {"));
    assert!(out.contains("provider = \"postgresql\""));
    assert!(out.contains("model Users {"));
    assert!(out.contains("@id"));
    assert!(out.contains("@@map(\"users\")"));
}

#[test]
fn serialises_nullable_column() {
    let live = common::make_live_schema(vec![LiveTable {
        name: "posts".to_string(),
        columns: vec![
            LiveColumn {
                name: "id".to_string(),
                col_type: "integer".to_string(),
                nullable: false,
                default_value: None,
                generated_expr: None,
                computed_kind: None,
                check_expr: None,
            },
            LiveColumn {
                name: "body".to_string(),
                col_type: "text".to_string(),
                nullable: true,
                default_value: None,
                generated_expr: None,
                computed_kind: None,
                check_expr: None,
            },
        ],
        primary_key: vec!["id".to_string()],
        indexes: vec![],
        check_constraints: vec![],
        foreign_keys: vec![],
    }]);

    let out = serialize_live_schema(&live, DatabaseProvider::Sqlite, "sqlite:test.db");

    assert!(out.contains("String?"));
}

#[test]
fn serialises_composite_pk() {
    let live = common::make_live_schema(vec![LiveTable {
        name: "order_items".to_string(),
        columns: vec![
            LiveColumn {
                name: "order_id".to_string(),
                col_type: "integer".to_string(),
                nullable: false,
                default_value: None,
                generated_expr: None,
                computed_kind: None,
                check_expr: None,
            },
            LiveColumn {
                name: "product_id".to_string(),
                col_type: "integer".to_string(),
                nullable: false,
                default_value: None,
                generated_expr: None,
                computed_kind: None,
                check_expr: None,
            },
        ],
        primary_key: vec!["order_id".to_string(), "product_id".to_string()],
        indexes: vec![],
        check_constraints: vec![],
        foreign_keys: vec![],
    }]);

    let out = serialize_live_schema(&live, DatabaseProvider::Postgres, "postgres://localhost/db");

    assert!(out.contains("@@id([order_id, product_id])"));
    let lines: Vec<&str> = out.lines().collect();
    let has_standalone_id = lines.iter().any(|l| {
        let trimmed = l.trim();
        trimmed.contains("@id") && !trimmed.starts_with("@@id")
    });
    assert!(!has_standalone_id, "should not have per-column @id:\n{out}");
}

#[test]
fn serialises_indexes() {
    let live = common::make_live_schema(vec![LiveTable {
        name: "users".to_string(),
        columns: vec![LiveColumn {
            name: "id".to_string(),
            col_type: "integer".to_string(),
            nullable: false,
            default_value: None,
            generated_expr: None,
            computed_kind: None,
            check_expr: None,
        }],
        primary_key: vec!["id".to_string()],
        indexes: vec![
            LiveIndex {
                name: "idx_User_email".to_string(),
                columns: vec!["email".to_string()],
                unique: true,
                method: None,
            },
            LiveIndex {
                name: "idx_User_name".to_string(),
                columns: vec!["name".to_string()],
                unique: false,
                method: None,
            },
        ],
        check_constraints: vec![],
        foreign_keys: vec![],
    }]);

    let out = serialize_live_schema(&live, DatabaseProvider::Postgres, "postgres://localhost/db");

    assert!(out.contains("@@unique([email])"));
    assert!(out.contains("@@index([name])"));
}

#[test]
fn serialises_default_value() {
    let live = common::make_live_schema(vec![LiveTable {
        name: "config".to_string(),
        columns: vec![
            LiveColumn {
                name: "id".to_string(),
                col_type: "integer".to_string(),
                nullable: false,
                default_value: None,
                generated_expr: None,
                computed_kind: None,
                check_expr: None,
            },
            LiveColumn {
                name: "active".to_string(),
                col_type: "boolean".to_string(),
                nullable: false,
                default_value: Some("true".to_string()),
                generated_expr: None,
                computed_kind: None,
                check_expr: None,
            },
        ],
        primary_key: vec!["id".to_string()],
        indexes: vec![],
        check_constraints: vec![],
        foreign_keys: vec![],
    }]);

    let out = serialize_live_schema(&live, DatabaseProvider::Postgres, "postgres://localhost/db");

    assert!(out.contains("@default(true)"));
}

#[test]
fn serialises_computed_column() {
    let live = common::make_live_schema(vec![LiveTable {
        name: "orders".to_string(),
        columns: vec![
            LiveColumn {
                name: "id".to_string(),
                col_type: "integer".to_string(),
                nullable: false,
                default_value: None,
                generated_expr: None,
                computed_kind: None,
                check_expr: None,
            },
            LiveColumn {
                name: "total".to_string(),
                col_type: "integer".to_string(),
                nullable: true,
                default_value: None,
                generated_expr: Some("price * quantity".to_string()),
                computed_kind: Some(ComputedKind::Stored),
                check_expr: None,
            },
        ],
        primary_key: vec!["id".to_string()],
        indexes: vec![],
        check_constraints: vec![],
        foreign_keys: vec![],
    }]);

    let out = serialize_live_schema(&live, DatabaseProvider::Postgres, "postgres://localhost/db");

    assert!(out.contains("@computed(price * quantity, Stored)"));
    assert!(!out.contains("@default"));
}

#[test]
fn serialises_column_check_constraint() {
    let live = common::make_live_schema(vec![LiveTable {
        name: "products".to_string(),
        columns: vec![
            LiveColumn {
                name: "id".to_string(),
                col_type: "integer".to_string(),
                nullable: false,
                default_value: None,
                generated_expr: None,
                computed_kind: None,
                check_expr: None,
            },
            LiveColumn {
                name: "price".to_string(),
                col_type: "integer".to_string(),
                nullable: false,
                default_value: None,
                generated_expr: None,
                computed_kind: None,
                check_expr: Some("price > 0".to_string()),
            },
        ],
        primary_key: vec!["id".to_string()],
        indexes: vec![],
        check_constraints: vec![],
        foreign_keys: vec![],
    }]);

    let out = serialize_live_schema(&live, DatabaseProvider::Postgres, "postgres://localhost/db");

    assert!(out.contains("@check(price > 0)"));
}

#[test]
fn serialises_table_check_constraint() {
    let live = common::make_live_schema(vec![LiveTable {
        name: "events".to_string(),
        columns: vec![
            LiveColumn {
                name: "id".to_string(),
                col_type: "integer".to_string(),
                nullable: false,
                default_value: None,
                generated_expr: None,
                computed_kind: None,
                check_expr: None,
            },
            LiveColumn {
                name: "start_date".to_string(),
                col_type: "timestamp".to_string(),
                nullable: false,
                default_value: None,
                generated_expr: None,
                computed_kind: None,
                check_expr: None,
            },
            LiveColumn {
                name: "end_date".to_string(),
                col_type: "timestamp".to_string(),
                nullable: false,
                default_value: None,
                generated_expr: None,
                computed_kind: None,
                check_expr: None,
            },
        ],
        primary_key: vec!["id".to_string()],
        indexes: vec![],
        check_constraints: vec!["start_date < end_date".to_string()],
        foreign_keys: vec![],
    }]);

    let out = serialize_live_schema(&live, DatabaseProvider::Postgres, "postgres://localhost/db");

    assert!(out.contains("@@check(start_date < end_date)"));
}

#[test]
fn serialises_and_reparses_computed_column_with_sql_string_literal() {
    let live = common::make_live_schema(vec![LiveTable {
        name: "users".to_string(),
        columns: vec![
            LiveColumn {
                name: "id".to_string(),
                col_type: "integer".to_string(),
                nullable: false,
                default_value: None,
                generated_expr: None,
                computed_kind: None,
                check_expr: None,
            },
            LiveColumn {
                name: "first_name".to_string(),
                col_type: "text".to_string(),
                nullable: false,
                default_value: None,
                generated_expr: None,
                computed_kind: None,
                check_expr: None,
            },
            LiveColumn {
                name: "last_name".to_string(),
                col_type: "text".to_string(),
                nullable: false,
                default_value: None,
                generated_expr: None,
                computed_kind: None,
                check_expr: None,
            },
            LiveColumn {
                name: "display_name".to_string(),
                col_type: "text".to_string(),
                nullable: true,
                default_value: None,
                generated_expr: Some("first_name || ' ' || last_name".to_string()),
                computed_kind: Some(ComputedKind::Stored),
                check_expr: None,
            },
        ],
        primary_key: vec!["id".to_string()],
        indexes: vec![],
        check_constraints: vec![],
        foreign_keys: vec![],
    }]);

    let out = serialize_live_schema(&live, DatabaseProvider::Postgres, "postgres://localhost/db");
    let ir = common::parse(&out).unwrap();
    let users = ir.models.get("Users").unwrap();
    let display_name = users.find_field("display_name").unwrap();

    assert!(matches!(
        &display_name.computed,
        Some((expr, ComputedKind::Stored)) if expr == "first_name || \" \" || last_name"
    ));
}

#[test]
fn serialises_and_reparses_check_constraints_with_sql_string_literals() {
    let live = common::make_live_schema(vec![LiveTable {
        name: "accounts".to_string(),
        columns: vec![
            LiveColumn {
                name: "id".to_string(),
                col_type: "integer".to_string(),
                nullable: false,
                default_value: None,
                generated_expr: None,
                computed_kind: None,
                check_expr: None,
            },
            LiveColumn {
                name: "status".to_string(),
                col_type: "text".to_string(),
                nullable: false,
                default_value: None,
                generated_expr: None,
                computed_kind: None,
                check_expr: Some("status IN ['Draft', 'PUBLISHED']".to_string()),
            },
            LiveColumn {
                name: "role".to_string(),
                col_type: "text".to_string(),
                nullable: false,
                default_value: None,
                generated_expr: None,
                computed_kind: None,
                check_expr: None,
            },
        ],
        primary_key: vec!["id".to_string()],
        indexes: vec![],
        check_constraints: vec!["role IN ['ADMIN', 'User']".to_string()],
        foreign_keys: vec![],
    }]);

    let out = serialize_live_schema(&live, DatabaseProvider::Postgres, "postgres://localhost/db");
    let ir = common::parse(&out).unwrap();
    let accounts = ir.models.get("Accounts").unwrap();
    let status = accounts.find_field("status").unwrap();

    assert_eq!(
        status.check.as_deref(),
        Some("status IN ('Draft', 'PUBLISHED')")
    );
    assert_eq!(
        accounts.check_constraints,
        vec!["role IN ('ADMIN', 'User')"]
    );
}

#[test]
fn serialises_and_reparses_postgres_composites_and_arrays() {
    let mut live = LiveSchema::default();

    live.enums.insert(
        "status".to_string(),
        vec!["ACTIVE".to_string(), "INACTIVE".to_string()],
    );
    live.composite_types.insert(
        "address".to_string(),
        LiveCompositeType {
            name: "address".to_string(),
            fields: vec![
                LiveCompositeField {
                    name: "street".to_string(),
                    col_type: "text".to_string(),
                },
                LiveCompositeField {
                    name: "zip_code".to_string(),
                    col_type: "integer".to_string(),
                },
                LiveCompositeField {
                    name: "status".to_string(),
                    col_type: "status".to_string(),
                },
            ],
        },
    );
    live.tables.insert(
        "profiles".to_string(),
        LiveTable {
            name: "profiles".to_string(),
            columns: vec![
                LiveColumn {
                    name: "id".to_string(),
                    col_type: "integer".to_string(),
                    nullable: false,
                    default_value: None,
                    generated_expr: None,
                    computed_kind: None,
                    check_expr: None,
                },
                LiveColumn {
                    name: "primary_address".to_string(),
                    col_type: "address".to_string(),
                    nullable: false,
                    default_value: None,
                    generated_expr: None,
                    computed_kind: None,
                    check_expr: None,
                },
                LiveColumn {
                    name: "previous_addresses".to_string(),
                    col_type: "address[]".to_string(),
                    nullable: false,
                    default_value: None,
                    generated_expr: None,
                    computed_kind: None,
                    check_expr: None,
                },
                LiveColumn {
                    name: "status_history".to_string(),
                    col_type: "status[]".to_string(),
                    nullable: false,
                    default_value: None,
                    generated_expr: None,
                    computed_kind: None,
                    check_expr: None,
                },
                LiveColumn {
                    name: "lucky_numbers".to_string(),
                    col_type: "integer[]".to_string(),
                    nullable: false,
                    default_value: None,
                    generated_expr: None,
                    computed_kind: None,
                    check_expr: None,
                },
            ],
            primary_key: vec!["id".to_string()],
            indexes: vec![],
            check_constraints: vec![],
            foreign_keys: vec![],
        },
    );

    let out = serialize_live_schema(&live, DatabaseProvider::Postgres, "postgres://localhost/db");
    let ir = common::parse(&out).unwrap();

    let address = ir.composite_types.get("Address").unwrap();
    assert_eq!(address.fields.len(), 3);
    assert!(matches!(
        &address.fields[2].field_type,
        ResolvedFieldType::Enum { enum_name } if enum_name == "Status"
    ));

    let profiles = ir.models.get("Profiles").unwrap();

    let primary_address = profiles.find_field("primary_address").unwrap();
    assert!(matches!(
        &primary_address.field_type,
        ResolvedFieldType::CompositeType { type_name } if type_name == "Address"
    ));
    assert!(!primary_address.is_array);

    let previous_addresses = profiles.find_field("previous_addresses").unwrap();
    assert!(matches!(
        &previous_addresses.field_type,
        ResolvedFieldType::CompositeType { type_name } if type_name == "Address"
    ));
    assert!(previous_addresses.is_array);

    let status_history = profiles.find_field("status_history").unwrap();
    assert!(matches!(
        &status_history.field_type,
        ResolvedFieldType::Enum { enum_name } if enum_name == "Status"
    ));
    assert!(status_history.is_array);

    let lucky_numbers = profiles.find_field("lucky_numbers").unwrap();
    assert!(matches!(
        &lucky_numbers.field_type,
        ResolvedFieldType::Scalar(ScalarType::Int)
    ));
    assert!(lucky_numbers.is_array);
}
