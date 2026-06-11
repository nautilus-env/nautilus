//! Benchmark for the engine row -> JSON wire serialization.
//!
//! `rows_to_raw_json` is the hottest serialization path: every `findMany` /
//! `findUnique` response flows through it.

use std::hint::black_box;

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion};
use nautilus_connector::Row;
use nautilus_core::Value;
use nautilus_engine::conversion::rows_to_raw_json;

/// Build `rows` rows with a 10-column shape representative of a typical model
/// (ints, uuid, strings, bool, float, decimal, datetime, array, json).
fn build_rows(rows: usize) -> Vec<Row> {
    let datetime = chrono::NaiveDate::from_ymd_opt(2026, 2, 18)
        .expect("valid date")
        .and_hms_opt(10, 30, 45)
        .expect("valid time");

    (0..rows)
        .map(|r| {
            Row::new(vec![
                ("users__id".to_string(), Value::I64(r as i64)),
                (
                    "users__public_id".to_string(),
                    Value::Uuid(uuid::Uuid::from_u128(r as u128)),
                ),
                (
                    "users__email".to_string(),
                    Value::String(format!("user-{r}@example.com")),
                ),
                (
                    "users__bio".to_string(),
                    Value::String(
                        "Lorem ipsum dolor sit amet, consectetur adipiscing elit.".to_string(),
                    ),
                ),
                ("users__active".to_string(), Value::Bool(r % 2 == 0)),
                ("users__score".to_string(), Value::F64(r as f64 * 0.5)),
                (
                    "users__balance".to_string(),
                    Value::Decimal(rust_decimal::Decimal::new(r as i64 * 100 + 45, 2)),
                ),
                ("users__created_at".to_string(), Value::DateTime(datetime)),
                (
                    "users__tags".to_string(),
                    Value::Array(vec![
                        Value::String("alpha".to_string()),
                        Value::String("beta".to_string()),
                    ]),
                ),
                (
                    "users__meta".to_string(),
                    Value::Json(serde_json::json!({ "plan": "pro", "n": r })),
                ),
            ])
        })
        .collect()
}

fn bench_rows_to_raw_json(c: &mut Criterion) {
    let mut group = c.benchmark_group("rows_to_raw_json");
    for &rows in &[1_000usize, 10_000] {
        let data = build_rows(rows);
        // Criterion reports per-iteration time; one iteration = one full
        // result-set serialization.
        group.bench_with_input(BenchmarkId::from_parameter(rows), &data, |b, data| {
            b.iter(|| black_box(rows_to_raw_json(black_box(data)).expect("serialize")));
        });
    }
    group.finish();
}

criterion_group!(benches, bench_rows_to_raw_json);
criterion_main!(benches);
