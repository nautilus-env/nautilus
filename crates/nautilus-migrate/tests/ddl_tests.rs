use nautilus_core::TableName;
mod common;

use nautilus_migrate::live::{LiveCompositeField, LiveCompositeType, LiveSchema};
use nautilus_migrate::{DatabaseProvider, DdlGenerator};

#[test]
fn test_generate_postgres_ddl() {
    let source = r#"
enum Role {
  USER
  ADMIN
}

model User {
  id    Int    @id
  email String @unique
  role  Role   @default(USER)
}
"#;
    let ir = common::parse(source).unwrap();

    let generator = DdlGenerator::new(DatabaseProvider::Postgres);
    let statements = generator.generate_create_tables(&ir).unwrap();

    assert!(statements.len() >= 2);
    assert!(statements[0].contains("CREATE TYPE"));
    assert!(statements[1].contains("CREATE TABLE"));
    assert!(statements[1].contains("\"id\""));
    assert!(statements[1].contains("\"email\""));
}

#[test]
fn test_generate_postgres_ddl_includes_secondary_indexes() {
    let source = r#"
model User {
  id        Int      @id
  createdAt DateTime @map("created_at")

  @@map("users")
  @@index([createdAt])
}
"#;
    let ir = common::parse(source).unwrap();

    let generator = DdlGenerator::new(DatabaseProvider::Postgres);
    let statements = generator.generate_create_tables(&ir).unwrap();

    assert!(
        statements
            .iter()
            .any(|sql| sql.contains("CREATE INDEX IF NOT EXISTS \"idx_users_created_at\"")),
        "expected CREATE INDEX statement after CREATE TABLE: {:?}",
        statements
    );
}

#[test]
fn test_generate_postgres_ddl_with_array_default() {
    let source = r#"
model Post {
  id   Int      @id
  tags String[] @default(["TEST", "TEST2"])
}
"#;
    let ir = common::parse(source).unwrap();

    let generator = DdlGenerator::new(DatabaseProvider::Postgres);
    let statements = generator.generate_create_tables(&ir).unwrap();
    let table_stmt = statements
        .iter()
        .find(|sql| sql.contains("CREATE TABLE"))
        .expect("missing create table statement");

    assert!(
        table_stmt.contains(r#""tags" TEXT[] DEFAULT ARRAY['TEST', 'TEST2']::TEXT[]"#),
        "sql: {}",
        table_stmt
    );
}

#[test]
fn test_generate_postgres_ddl_with_uuidv7_default() {
    let source = r#"
model User {
  id Uuid @id @default(uuidv7())
}
"#;
    let ir = common::parse(source).unwrap();

    let generator = DdlGenerator::new(DatabaseProvider::Postgres);
    let statements = generator.generate_create_tables(&ir).unwrap();
    let table_stmt = statements
        .iter()
        .find(|sql| sql.contains("CREATE TABLE"))
        .expect("missing create table statement");

    assert!(
        table_stmt.contains(r#""id" UUID NOT NULL DEFAULT uuidv7()"#),
        "sql: {}",
        table_stmt
    );
}

#[test]
fn test_uuidv7_default_is_rejected_for_mysql_ddl() {
    let source = r#"
model User {
  id Uuid @id @default(uuidv7())
}
"#;
    let ir = common::parse(source).unwrap();

    let generator = DdlGenerator::new(DatabaseProvider::Mysql);
    let err = generator.generate_create_tables(&ir).unwrap_err();
    let msg = err.to_string();

    assert!(
        msg.contains("uuidv7()") && msg.contains("MySQL"),
        "error: {msg}"
    );
}

#[test]
fn test_generate_postgres_ddl_with_extension_backed_scalar_types() {
    let source = r#"
datasource db {
  provider   = "postgresql"
  url        = "postgres://localhost/test"
  extensions = [citext, hstore, ltree, postgis]
}

model User {
  id    Int    @id
  email Citext
  meta  Hstore
  path  Ltree
  geom  Geometry
  geog  Geography
}
"#;
    let ir = common::parse(source).unwrap();

    let generator = DdlGenerator::new(DatabaseProvider::Postgres);
    let statements = generator.generate_create_tables(&ir).unwrap();
    let table_stmt = statements
        .iter()
        .find(|sql| sql.contains("CREATE TABLE"))
        .expect("missing create table statement");

    assert!(
        table_stmt.contains("\"email\" CITEXT"),
        "sql: {}",
        table_stmt
    );
    assert!(
        table_stmt.contains("\"meta\" HSTORE"),
        "sql: {}",
        table_stmt
    );
    assert!(table_stmt.contains("\"path\" LTREE"), "sql: {}", table_stmt);
    assert!(
        table_stmt.contains("\"geom\" GEOMETRY"),
        "sql: {}",
        table_stmt
    );
    assert!(
        table_stmt.contains("\"geog\" GEOGRAPHY"),
        "sql: {}",
        table_stmt
    );
}

#[test]
fn test_generate_postgres_ddl_with_pgvector_type() {
    let source = r#"
datasource db {
  provider   = "postgresql"
  url        = "postgres://localhost/test"
  extensions = [vector]
}

model Embedding {
  id     Int @id
  vector Vector(1536)
}
"#;
    let ir = common::parse(source).unwrap();

    let generator = DdlGenerator::new(DatabaseProvider::Postgres);
    let statements = generator.generate_create_tables(&ir).unwrap();
    let table_stmt = statements
        .iter()
        .find(|sql| sql.contains("CREATE TABLE"))
        .expect("missing create table statement");

    assert!(
        statements
            .iter()
            .any(|sql| sql == "CREATE EXTENSION IF NOT EXISTS \"vector\""),
        "statements: {:?}",
        statements
    );
    assert!(
        table_stmt.contains("\"vector\" VECTOR(1536)"),
        "sql: {}",
        table_stmt
    );
}

#[test]
fn test_generate_postgres_ddl_with_pgvector_hnsw_index() {
    let source = r#"
datasource db {
  provider   = "postgresql"
  url        = "postgres://localhost/test"
  extensions = [vector]
}

model Embedding {
  id        Int @id
  embedding Vector(3)

  @@index([embedding], type: Hnsw, opclass: vector_cosine_ops, m: 16, ef_construction: 64)
}
"#;
    let ir = common::parse(source).unwrap();

    let generator = DdlGenerator::new(DatabaseProvider::Postgres);
    let statements = generator.generate_create_tables(&ir).unwrap();
    let index_stmt = statements
        .iter()
        .find(|sql| sql.contains("USING HNSW"))
        .expect("missing create index statement");

    assert!(index_stmt.contains("(\"embedding\" vector_cosine_ops)"));
    assert!(index_stmt.contains("WITH (m = 16, ef_construction = 64)"));
}

#[test]
fn test_generate_sqlite_ddl() {
    let source = r#"
model Post {
  id    Int    @id
  title String
}
"#;
    let ir = common::parse(source).unwrap();

    let generator = DdlGenerator::new(DatabaseProvider::Sqlite);
    let statements = generator.generate_create_tables(&ir).unwrap();

    assert_eq!(statements.len(), 1);
    assert!(statements[0].contains("CREATE TABLE"));
    assert!(statements[0].contains("\"Post\""));
}

#[test]
fn test_composite_type_postgres_ddl() {
    let source = r#"
datasource db {
  provider = "postgresql"
  url      = "postgres://localhost/test"
}

type Address {
  street String
  city   String
  zip    String
}

model User {
  id      Int     @id
  address Address
}
"#;
    let ir = common::parse(source).unwrap();
    let generator = DdlGenerator::new(DatabaseProvider::Postgres);
    let statements = generator.generate_create_tables(&ir).unwrap();

    let composite_stmt = statements
        .iter()
        .find(|s| s.contains("CREATE TYPE \"address\" AS"));
    assert!(
        composite_stmt.is_some(),
        "Missing CREATE TYPE statement for composite type"
    );
    let stmt = composite_stmt.unwrap();
    assert!(stmt.contains("\"street\" TEXT"));
    assert!(stmt.contains("\"city\" TEXT"));
    assert!(stmt.contains("\"zip\" TEXT"));

    let table_stmt = statements.iter().find(|s| s.contains("CREATE TABLE"));
    assert!(table_stmt.is_some());
    assert!(table_stmt.unwrap().contains("address"));
}

#[test]
fn test_composite_type_postgres_ddl_with_map_and_type_map() {
    let source = r#"
datasource db {
  provider = "postgresql"
  url      = "postgres://localhost/test"
}

type Address {
  street String
  zip    String @map("zip_code")
  @@map("address_t")
}

model User {
  id      Int     @id
  address Address
}
"#;
    let ir = common::parse(source).unwrap();
    let generator = DdlGenerator::new(DatabaseProvider::Postgres);
    let statements = generator.generate_create_tables(&ir).unwrap();

    // @@map renames the SQL composite type; @map renames the inner column.
    let composite_stmt = statements
        .iter()
        .find(|s| s.contains("CREATE TYPE \"address_t\" AS"))
        .expect("Missing CREATE TYPE statement using @@map name");
    assert!(composite_stmt.contains("\"street\" TEXT"));
    assert!(composite_stmt.contains("\"zip_code\" TEXT"));
    assert!(!composite_stmt.contains("\"zip\" TEXT"));

    // The model column references the mapped SQL type name, not the logical one.
    let table_stmt = statements
        .iter()
        .find(|s| s.contains("CREATE TABLE"))
        .unwrap();
    assert!(table_stmt.contains("address_t"));
    assert!(!table_stmt
        .to_lowercase()
        .contains("\"address\" \"address\""));
}

#[test]
fn test_composite_type_postgres_ddl_quotes_mapped_type_name() {
    let source = r#"
datasource db {
  provider = "postgresql"
  url      = "postgres://localhost/test"
}

type Address {
  street String
  @@map("AddressT")
}

model User {
  id        Int       @id
  address   Address
  addresses Address[]
}
"#;
    let ir = common::parse(source).unwrap();
    let generator = DdlGenerator::new(DatabaseProvider::Postgres);
    let statements = generator.generate_create_tables(&ir).unwrap();

    let composite_stmt = statements
        .iter()
        .find(|s| s.contains("CREATE TYPE \"AddressT\" AS"))
        .expect("Missing CREATE TYPE statement using quoted @@map name");
    assert!(composite_stmt.contains("\"street\" TEXT"));

    let table_stmt = statements
        .iter()
        .find(|s| s.contains("CREATE TABLE"))
        .unwrap();
    assert!(table_stmt.contains("\"address\" \"AddressT\""));
    assert!(table_stmt.contains("\"addresses\" \"AddressT\"[]"));
    assert!(!table_stmt.contains(" addressT"));
    assert!(!table_stmt.contains(" AddressT"));
}

#[test]
fn test_composite_type_sqlite_json_ddl() {
    let source = r#"
datasource db {
  provider = "sqlite"
  url      = "file:./dev.db"
}

type Address {
  street String
  city   String
}

model User {
  id      Int     @id
  address Address @store(json)
}
"#;
    let ir = common::parse(source).unwrap();
    let generator = DdlGenerator::new(DatabaseProvider::Sqlite);
    let statements = generator.generate_create_tables(&ir).unwrap();

    let table_stmt = statements
        .iter()
        .find(|s| s.contains("CREATE TABLE"))
        .unwrap();
    assert!(table_stmt.contains("\"address\" TEXT"));
}

#[test]
fn test_composite_type_mysql_json_ddl() {
    let source = r#"
datasource db {
  provider = "mysql"
  url      = "mysql://localhost/test"
}

type Address {
  street String
  city   String
}

model User {
  id      Int     @id
  address Address @store(json)
}
"#;
    let ir = common::parse(source).unwrap();
    let generator = DdlGenerator::new(DatabaseProvider::Mysql);
    let statements = generator.generate_create_tables(&ir).unwrap();

    let table_stmt = statements
        .iter()
        .find(|s| s.contains("CREATE TABLE"))
        .unwrap();
    assert!(table_stmt.contains("`address` JSON"));
}

#[test]
fn test_postgres_drop_tables_drops_composites_before_enums() {
    let source = r#"
datasource db {
  provider = "postgresql"
  url      = "postgres://localhost/test"
}

enum Status {
  DRAFT
  PUBLISHED
}

type Address {
  status Status
  street String
}

model User {
  id      Int     @id
  address Address
}
"#;
    let ir = common::parse(source).unwrap();
    let generator = DdlGenerator::new(DatabaseProvider::Postgres);
    let statements = generator.generate_drop_tables(&ir).unwrap();

    let composite_idx = statements
        .iter()
        .position(|s| s == "DROP TYPE IF EXISTS \"address\"")
        .unwrap();
    let enum_idx = statements
        .iter()
        .position(|s| s == "DROP TYPE IF EXISTS \"status\"")
        .unwrap();

    assert!(composite_idx < enum_idx);
}

#[test]
fn test_postgres_drop_live_tables_drops_composites_before_enums() {
    let mut live = LiveSchema::default();
    live.enums.insert(
        "status".to_string(),
        vec!["DRAFT".to_string(), "PUBLISHED".to_string()],
    );
    live.composite_types.insert(
        "address".to_string(),
        LiveCompositeType {
            name: "address".to_string(),
            fields: vec![
                LiveCompositeField {
                    name: "status".to_string(),
                    col_type: "status".to_string(),
                },
                LiveCompositeField {
                    name: "street".to_string(),
                    col_type: "text".to_string(),
                },
            ],
        },
    );

    let generator = DdlGenerator::new(DatabaseProvider::Postgres);
    let statements = generator.generate_drop_live_tables(&live);

    let composite_idx = statements
        .iter()
        .position(|s| s == "DROP TYPE IF EXISTS \"address\"")
        .unwrap();
    let enum_idx = statements
        .iter()
        .position(|s| s == "DROP TYPE IF EXISTS \"status\"")
        .unwrap();

    assert!(composite_idx < enum_idx);
}

#[test]
fn test_postgres_drop_live_tables_quotes_mixed_case_type_names() {
    let mut live = LiveSchema::default();
    live.enums.insert(
        "PostStatus".to_string(),
        vec!["DRAFT".to_string(), "PUBLISHED".to_string()],
    );
    live.composite_types.insert(
        "Address".to_string(),
        LiveCompositeType {
            name: "Address".to_string(),
            fields: vec![LiveCompositeField {
                name: "street".to_string(),
                col_type: "text".to_string(),
            }],
        },
    );

    let generator = DdlGenerator::new(DatabaseProvider::Postgres);
    let statements = generator.generate_drop_live_tables(&live);

    assert!(
        statements
            .iter()
            .any(|sql| sql == "DROP TYPE IF EXISTS \"Address\""),
        "expected quoted DROP TYPE for mixed-case composite: {:?}",
        statements
    );
    assert!(
        statements
            .iter()
            .any(|sql| sql == "DROP TYPE IF EXISTS \"PostStatus\""),
        "expected quoted DROP TYPE for mixed-case enum: {:?}",
        statements
    );
}

#[test]
fn test_postgres_extensions_emitted_before_tables() {
    let source = r#"
datasource db {
  provider   = "postgresql"
  url        = "postgres://localhost/test"
  extensions = [pg_trgm, "uuid-ossp"]
}

model Doc {
  id   String @id
  body String
}
"#;
    let ir = common::parse(source).unwrap();
    let generator = DdlGenerator::new(DatabaseProvider::Postgres);
    let statements = generator.generate_create_tables(&ir).unwrap();

    let ext_trgm = statements
        .iter()
        .position(|s| s == "CREATE EXTENSION IF NOT EXISTS \"pg_trgm\"")
        .expect("expected pg_trgm CREATE EXTENSION");
    let ext_uuid = statements
        .iter()
        .position(|s| s == "CREATE EXTENSION IF NOT EXISTS \"uuid-ossp\"")
        .expect("expected uuid-ossp CREATE EXTENSION");
    let create_table_idx = statements
        .iter()
        .position(|s| s.starts_with("CREATE TABLE"))
        .expect("expected CREATE TABLE");

    assert!(ext_trgm < create_table_idx, "{statements:?}");
    assert!(ext_uuid < create_table_idx, "{statements:?}");
}

#[test]
fn test_sqlite_ignores_extensions() {
    let source = r#"
datasource db {
  provider = "sqlite"
  url      = "file:./test.db"
}

model Doc { id Int @id body String }
"#;
    let ir = common::parse(source).unwrap();
    let generator = DdlGenerator::new(DatabaseProvider::Sqlite);
    let statements = generator.generate_create_tables(&ir).unwrap();

    assert!(
        !statements.iter().any(|s| s.contains("CREATE EXTENSION")),
        "SQLite output must not contain CREATE EXTENSION: {statements:?}"
    );
}

#[test]
fn test_placeholder_dialect_per_provider() {
    // Postgres uses numbered placeholders; MySQL/SQLite use anonymous `?`.
    assert_eq!(DatabaseProvider::Postgres.placeholder(1), "$1");
    assert_eq!(DatabaseProvider::Postgres.placeholder(4), "$4");
    assert_eq!(DatabaseProvider::Mysql.placeholder(1), "?");
    assert_eq!(DatabaseProvider::Sqlite.placeholder(3), "?");
}

#[test]
fn test_placeholders_list_per_provider() {
    assert_eq!(DatabaseProvider::Postgres.placeholders(4), "$1, $2, $3, $4");
    assert_eq!(DatabaseProvider::Mysql.placeholders(4), "?, ?, ?, ?");
    assert_eq!(DatabaseProvider::Sqlite.placeholders(1), "?");
    assert_eq!(DatabaseProvider::Postgres.placeholders(0), "");
}

/// MySQL has no `CREATE TYPE`, so an enum becomes a native column `ENUM(...)`.
/// Storing it as TEXT instead makes the column reject a DEFAULT (error 1101)
/// and refuse to be indexed without a key length (error 1170).
#[test]
fn mysql_renders_an_enum_column_as_a_native_enum() {
    let ir = common::parse(
        "enum Status { DRAFT PUBLISHED }\n\
         model Post { id Int @id  status Status @default(DRAFT) }",
    )
    .unwrap();

    let generator = DdlGenerator::new(DatabaseProvider::Mysql);
    let statements = generator.generate_create_tables(&ir).unwrap();
    let create_table = statements
        .iter()
        .find(|sql| sql.contains("CREATE TABLE"))
        .expect("a CREATE TABLE statement");

    assert!(
        create_table.contains("ENUM('DRAFT', 'PUBLISHED')"),
        "expected a native MySQL enum column:\n{create_table}"
    );
    assert!(
        create_table.contains("DEFAULT 'DRAFT'"),
        "a native enum column accepts a DEFAULT, unlike TEXT:\n{create_table}"
    );
}

/// The canonical type string is compared against what the server reports, and
/// MySQL echoes enum variants with the case they were declared with.
#[test]
fn mysql_enum_column_type_keeps_variant_case() {
    let ir = common::parse(
        "enum Status { DRAFT PUBLISHED }\n\
         model Post { id Int @id  status Status }",
    )
    .unwrap();

    let generator = DdlGenerator::new(DatabaseProvider::Mysql);
    let model = ir.models.get("Post").expect("Post model");
    let field = model
        .fields
        .iter()
        .find(|f| f.logical_name == "status")
        .expect("status field");

    assert_eq!(
        generator.column_type_sql(field).unwrap(),
        "enum('DRAFT', 'PUBLISHED')"
    );
}

#[test]
fn mysql_autoincrement_primary_key_gets_auto_increment() {
    let source = r#"
model User {
  id    Int    @id @default(autoincrement())
  email String @unique
}
"#;
    let ir = common::parse(source).unwrap();

    let statements = DdlGenerator::new(DatabaseProvider::Mysql)
        .generate_create_tables(&ir)
        .unwrap();
    let create_table = statements
        .iter()
        .find(|sql| sql.contains("CREATE TABLE"))
        .expect("expected a CREATE TABLE statement");

    assert!(
        create_table.contains("`id` INT NOT NULL AUTO_INCREMENT"),
        "expected an AUTO_INCREMENT primary key: {create_table}"
    );
    assert!(
        create_table.contains("PRIMARY KEY (`id`)"),
        "AUTO_INCREMENT requires the column to stay indexed: {create_table}"
    );
}

#[test]
fn mysql_bigint_autoincrement_primary_key_uses_bigint() {
    let source = r#"
model Event {
  id BigInt @id @default(autoincrement())
}
"#;
    let ir = common::parse(source).unwrap();

    let statements = DdlGenerator::new(DatabaseProvider::Mysql)
        .generate_create_tables(&ir)
        .unwrap();

    assert!(
        statements
            .iter()
            .any(|sql| sql.contains("`id` BIGINT NOT NULL AUTO_INCREMENT")),
        "expected a BIGINT AUTO_INCREMENT key: {statements:?}"
    );
}

#[test]
fn mysql_rejects_autoincrement_outside_the_primary_key() {
    let source = r#"
model Ticket {
  id     Int @id
  serial Int @default(autoincrement())
}
"#;
    let ir = common::parse(source).unwrap();

    let error = DdlGenerator::new(DatabaseProvider::Mysql)
        .generate_create_tables(&ir)
        .unwrap_err()
        .to_string();

    assert!(
        error.contains("single-column primary key"),
        "expected a clear MySQL limitation error, got: {error}"
    );
}

#[test]
fn mysql_rejects_autoincrement_on_a_composite_primary_key() {
    let source = r#"
model Reading {
  id       Int @default(autoincrement())
  sensorId Int

  @@id([id, sensorId])
}
"#;
    let ir = common::parse(source).unwrap();

    let error = DdlGenerator::new(DatabaseProvider::Mysql)
        .generate_create_tables(&ir)
        .unwrap_err()
        .to_string();

    assert!(
        error.contains("single-column primary key"),
        "expected a clear MySQL limitation error, got: {error}"
    );
}

#[test]
fn postgres_and_sqlite_autoincrement_spellings_are_unchanged() {
    let source = r#"
model User {
  id Int @id @default(autoincrement())
}
"#;
    let ir = common::parse(source).unwrap();

    let postgres = DdlGenerator::new(DatabaseProvider::Postgres)
        .generate_create_tables(&ir)
        .unwrap();
    assert!(
        postgres.iter().any(|sql| sql.contains("\"id\" SERIAL")),
        "expected PostgreSQL to keep SERIAL: {postgres:?}"
    );

    let sqlite = DdlGenerator::new(DatabaseProvider::Sqlite)
        .generate_create_tables(&ir)
        .unwrap();
    assert!(
        sqlite
            .iter()
            .any(|sql| sql.contains("\"id\" INTEGER PRIMARY KEY AUTOINCREMENT")),
        "expected SQLite to keep the inline autoincrement key: {sqlite:?}"
    );
}

#[test]
fn boolean_defaults_use_each_provider_own_spelling() {
    let source = r#"
model Flagged {
  id       Int     @id
  active   Boolean @default(true)
  archived Boolean @default(false)
}
"#;
    let ir = common::parse(source).unwrap();
    let model = ir.models.values().next().unwrap();
    let active = model.find_field("active").unwrap();
    let archived = model.find_field("archived").unwrap();

    let postgres = DdlGenerator::new(DatabaseProvider::Postgres);
    assert_eq!(
        postgres.column_default_sql(active).unwrap().as_deref(),
        Some("TRUE")
    );

    // MySQL stores booleans as tinyint(1) and SQLite as an integer; both report
    // the default back as 1/0, so writing TRUE would never compare equal.
    for provider in [DatabaseProvider::Mysql, DatabaseProvider::Sqlite] {
        let generator = DdlGenerator::new(provider);
        assert_eq!(
            generator.column_default_sql(active).unwrap().as_deref(),
            Some("1"),
            "{provider:?} should render a true default as 1"
        );
        assert_eq!(
            generator.column_default_sql(archived).unwrap().as_deref(),
            Some("0"),
            "{provider:?} should render a false default as 0"
        );
    }
}

#[test]
fn test_partial_index_emits_where_clause_on_postgres_and_sqlite() {
    let source = r#"
model Task {
  id      Int     @id
  done    Boolean
  ownerId Int     @map("owner_id")

  @@index([ownerId], where: done = false)
}
"#;
    let ir = common::parse(source).unwrap();

    for provider in [DatabaseProvider::Postgres, DatabaseProvider::Sqlite] {
        let generator = DdlGenerator::new(provider);
        let statements = generator.generate_create_tables(&ir).unwrap();
        let create_index = statements
            .iter()
            .find(|s| s.contains("CREATE INDEX"))
            .unwrap_or_else(|| panic!("no CREATE INDEX for {:?}: {:?}", provider, statements));

        assert!(
            create_index.contains("WHERE done = FALSE"),
            "{:?} produced: {}",
            provider,
            create_index
        );
    }
}

#[test]
fn test_index_without_predicate_has_no_where_clause() {
    let source = r#"
model Task {
  id      Int @id
  ownerId Int

  @@index([ownerId])
}
"#;
    let ir = common::parse(source).unwrap();
    let generator = DdlGenerator::new(DatabaseProvider::Postgres);
    let statements = generator.generate_create_tables(&ir).unwrap();
    let create_index = statements
        .iter()
        .find(|s| s.contains("CREATE INDEX"))
        .unwrap();

    assert!(!create_index.contains("WHERE"), "got: {}", create_index);
}

#[test]
fn test_ignored_declarations_are_left_out_of_create_tables() {
    let source = r#"
model Device {
  id     Int     @id
  name   String
  uptime String? @ignore
}

model Legacy {
  id     Int    @id
  opaque String @ignore

  @@ignore
}
"#;
    let ir = common::parse(source).unwrap();
    let generator = DdlGenerator::new(DatabaseProvider::Postgres);
    let statements = generator.generate_create_tables(&ir).unwrap();
    let sql = statements.join("\n");

    assert!(sql.contains("\"Device\""), "got: {}", sql);
    assert!(
        !sql.contains("uptime"),
        "ignored column was created: {}",
        sql
    );
    assert!(
        !sql.contains("Legacy"),
        "ignored model was created: {}",
        sql
    );
}

#[test]
fn test_sqlite_drop_live_tables_defers_foreign_key_checks() {
    let mut live = LiveSchema::default();
    for name in ["post", "user"] {
        live.tables.insert(
            TableName::new(name.to_string()),
            nautilus_migrate::live::LiveTable {
                name: TableName::new(name.to_string()),
                columns: vec![],
                primary_key: vec![],
                indexes: vec![],
                check_constraints: vec![],
                foreign_keys: vec![],
            },
        );
    }

    let statements = DdlGenerator::new(DatabaseProvider::Sqlite).generate_drop_live_tables(&live);

    assert_eq!(
        statements.first().map(String::as_str),
        Some("PRAGMA defer_foreign_keys = ON"),
        "dropping in name order would otherwise break a foreign key: {:?}",
        statements
    );
}
