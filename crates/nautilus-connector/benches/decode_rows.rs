//! Benchmark for the row decode loop (plan.md, item 2.1 — bench rimandato
//! dalla fase 0).
//!
//! Decodes real sqlx rows fetched from an in-memory SQLite database through
//! the same batch entry point the executors use (`decode_rows`). SQLite is the
//! only backend whose rows can be produced without a running server; the
//! PostgreSQL-specific column-plan classification is covered by unit tests in
//! `postgres_stream.rs`, while this bench tracks the per-row cost the decode
//! loop shares across backends (name `String`s, NULL checks, value extraction)
//! — the same loop item 2.2 targets.

use std::hint::black_box;

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion};
use nautilus_connector::bench::decode_sqlite_rows;
use sqlx::sqlite::SqliteRow;

fn fetch_rows(count: usize) -> Vec<SqliteRow> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("failed to build tokio runtime");

    runtime.block_on(async move {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("failed to open in-memory sqlite");

        sqlx::query(
            r#"
            CREATE TABLE users (
                id         INTEGER PRIMARY KEY,
                name       TEXT NOT NULL,
                email      TEXT NOT NULL,
                bio        TEXT,
                age        INTEGER NOT NULL,
                visits     INTEGER NOT NULL,
                score      REAL NOT NULL,
                rating     REAL,
                country    TEXT NOT NULL,
                created_at TEXT NOT NULL
            )
            "#,
        )
        .execute(&pool)
        .await
        .expect("failed to create table");

        sqlx::query(
            r#"
            WITH RECURSIVE seq(x) AS (
                SELECT 1 UNION ALL SELECT x + 1 FROM seq WHERE x < ?1
            )
            INSERT INTO users
            SELECT
                x,
                printf('User %05d', x),
                printf('user%05d@example.com', x),
                CASE WHEN x % 7 = 0 THEN NULL ELSE printf('bio for user %d', x) END,
                20 + (x % 50),
                x * 3,
                x * 0.5,
                CASE WHEN x % 5 = 0 THEN NULL ELSE x * 0.25 END,
                CASE x % 3 WHEN 0 THEN 'IT' WHEN 1 THEN 'DE' ELSE 'FR' END,
                '2024-01-01 00:00:00'
            FROM seq
            "#,
        )
        .bind(count as i64)
        .execute(&pool)
        .await
        .expect("failed to seed rows");

        sqlx::query("SELECT * FROM users ORDER BY id")
            .fetch_all(&pool)
            .await
            .expect("failed to fetch rows")
    })
}

fn bench_decode_rows(c: &mut Criterion) {
    let mut group = c.benchmark_group("decode_rows");
    for count in [1_000usize, 10_000] {
        let rows = fetch_rows(count);
        group.bench_with_input(BenchmarkId::new("sqlite_10col", count), &rows, |b, rows| {
            b.iter(|| black_box(decode_sqlite_rows(black_box(rows)).expect("decode failed")));
        });
    }
    group.finish();
}

criterion_group!(benches, bench_decode_rows);
criterion_main!(benches);
