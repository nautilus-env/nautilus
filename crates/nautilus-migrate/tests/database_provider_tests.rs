use nautilus_migrate::DatabaseProvider;
use nautilus_schema::ir::DatabaseProvider as SchemaDatabaseProvider;

#[test]
fn provider_parsing_preserves_canonical_names_and_the_legacy_alias() {
    for (name, provider, canonical) in [
        ("postgresql", DatabaseProvider::Postgres, "postgresql"),
        ("postgres", DatabaseProvider::Postgres, "postgresql"),
        ("mysql", DatabaseProvider::Mysql, "mysql"),
        ("sqlite", DatabaseProvider::Sqlite, "sqlite"),
    ] {
        let parsed = DatabaseProvider::from_schema_provider(name).unwrap();
        assert_eq!(parsed, provider);
        assert_eq!(parsed.schema_provider_name(), canonical);
        assert_eq!(
            SchemaDatabaseProvider::from(parsed),
            canonical.parse::<SchemaDatabaseProvider>().unwrap()
        );
    }

    assert!("postgres".parse::<SchemaDatabaseProvider>().is_err());
    for invalid in ["", "Postgres", "POSTGRESQL", " mysql", "sqlite ", "mariadb"] {
        assert_eq!(DatabaseProvider::from_schema_provider(invalid), None);
    }
}
