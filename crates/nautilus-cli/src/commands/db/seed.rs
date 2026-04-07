use std::time::Instant;

use anyhow::Context;

use super::connection::{detect_provider, obfuscate_url, resolve_url, Connection};
use crate::tui;

/// Execute `nautilus db seed <file>` — run a SQL seed script against the database.
///
/// The file is executed as a raw SQL script inside a single transaction
/// (all-or-nothing), so statement boundaries are determined by the database
/// parser rather than by client-side string splitting.
pub async fn run(file: String, db_url_arg: Option<String>) -> anyhow::Result<()> {
    tui::print_header("db seed");

    let raw_url = db_url_arg
        .or_else(|| std::env::var("DATABASE_URL").ok())
        .context(
            "No database URL found. \
            Use --database-url or set the DATABASE_URL environment variable.",
        )?;

    let database_url = resolve_url(&raw_url)?;

    let sp = tui::spinner("Connecting to database…");
    let provider = detect_provider(&database_url)?;
    let conn = Connection::connect(&database_url, provider)
        .await
        .with_context(|| format!("Failed to connect to {}", database_url))?;
    tui::spinner_ok(sp, &format!("Connected  {}", obfuscate_url(&database_url)));

    let sp = tui::spinner(&format!("Reading {}…", file));
    let contents = std::fs::read_to_string(&file)
        .with_context(|| format!("Cannot read seed file: {}", file))?;

    if contents.trim().is_empty() {
        tui::spinner_ok(sp, "Seed file read — no SQL found");
        tui::print_summary_ok(
            "Nothing to seed",
            "The seed file contained no SQL statements",
        );
        return Ok(());
    }

    tui::spinner_ok(sp, "Seed file read");

    let sp = tui::spinner("Seeding database…");
    let start = Instant::now();

    conn.execute_script_in_transaction(&contents)
        .await
        .context("Seed failed — transaction was rolled back")?;

    let elapsed = start.elapsed();
    tui::spinner_ok(sp, "Seed transaction committed");

    tui::print_summary_ok(
        "Seeded",
        &format!(
            "SQL script applied  {:.0}ms",
            elapsed.as_secs_f64() * 1000.0
        ),
    );

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::run;
    use crate::test_support::sqlite_url;
    use sqlx::Row;
    use tempfile::TempDir;

    #[tokio::test]
    async fn run_executes_seed_script_without_client_side_semicolon_splitting() {
        let temp_dir = TempDir::new().expect("temp dir");
        let db_path = temp_dir.path().join("seed.db");
        let seed_path = temp_dir.path().join("seed.sql");
        let database_url = sqlite_url(&db_path);

        let seed = r#"CREATE TABLE notes (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  body TEXT NOT NULL
);

CREATE TABLE audit (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  body TEXT NOT NULL
);

CREATE TRIGGER notes_after_insert
AFTER INSERT ON notes
BEGIN
  INSERT INTO audit(body) VALUES('trigger; fired');
END;

INSERT INTO notes(body) VALUES('hello; world');
"#;
        std::fs::write(&seed_path, seed).expect("failed to write seed script");

        run(
            seed_path.to_string_lossy().to_string(),
            Some(database_url.clone()),
        )
        .await
        .expect("seed script should execute");

        let pool = sqlx::SqlitePool::connect(&database_url)
            .await
            .expect("failed to connect to sqlite db");

        let note = sqlx::query("SELECT body FROM notes")
            .fetch_one(&pool)
            .await
            .expect("note row should exist");
        let audit = sqlx::query("SELECT body FROM audit")
            .fetch_one(&pool)
            .await
            .expect("audit row should exist");

        assert_eq!(note.get::<String, _>("body"), "hello; world");
        assert_eq!(audit.get::<String, _>("body"), "trigger; fired");
    }
}
