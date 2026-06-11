//! Benchmarks for `Value` serialization paths.
//!
//! Covers the two hot conversions called per cell on the wire paths:
//! - the tagged serde representation (`impl Serialize for Value`, used for
//!   params and transaction payloads), which today round-trips through an
//!   owned `SerdeValue` mirror (deep clones);
//! - `to_json_plain`, used by the engine row serializer, which today builds
//!   an owned `serde_json::Value` tree per cell.

use std::collections::BTreeMap;
use std::hint::black_box;

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion};
use nautilus_core::Value;

fn sample_values(n: usize) -> Vec<Value> {
    let datetime = chrono::NaiveDate::from_ymd_opt(2026, 2, 18)
        .expect("valid date")
        .and_hms_opt(10, 30, 45)
        .expect("valid time");

    (0..n)
        .map(|i| match i % 10 {
            0 => Value::I64(i as i64),
            1 => Value::String(format!("user-{i}@example.com")),
            2 => Value::Bool(i % 2 == 0),
            3 => Value::F64(i as f64 * 0.5),
            4 => Value::Decimal(rust_decimal::Decimal::new(i as i64 * 100 + 45, 2)),
            5 => Value::DateTime(datetime),
            6 => Value::Uuid(uuid::Uuid::from_u128(i as u128)),
            7 => Value::Json(serde_json::json!({ "key": "value", "n": i })),
            8 => Value::Array(vec![
                Value::String("alpha".to_string()),
                Value::String("beta".to_string()),
                Value::I64(i as i64),
            ]),
            _ => Value::Hstore(BTreeMap::from([
                ("display_name".to_string(), Some(format!("User {i}"))),
                ("nickname".to_string(), None),
            ])),
        })
        .collect()
}

fn bench_serialize_tagged(c: &mut Criterion) {
    let mut group = c.benchmark_group("value_serialize_tagged");
    for &n in &[100usize, 1_000] {
        let values = sample_values(n);
        group.bench_with_input(BenchmarkId::from_parameter(n), &values, |b, values| {
            b.iter(|| {
                let json = serde_json::to_string(black_box(values)).expect("serialize");
                black_box(json)
            });
        });
    }
    group.finish();
}

fn bench_deserialize_tagged(c: &mut Criterion) {
    let values = sample_values(1_000);
    let json = serde_json::to_string(&values).expect("serialize");
    c.bench_function("value_deserialize_tagged/1000", |b| {
        b.iter(|| {
            let values: Vec<Value> = serde_json::from_str(black_box(&json)).expect("deserialize");
            black_box(values)
        });
    });
}

fn bench_to_json_plain(c: &mut Criterion) {
    let mut group = c.benchmark_group("value_to_json_plain");
    for &n in &[100usize, 1_000] {
        let values = sample_values(n);
        group.bench_with_input(BenchmarkId::from_parameter(n), &values, |b, values| {
            b.iter(|| {
                let plain: Vec<serde_json::Value> =
                    values.iter().map(Value::to_json_plain).collect();
                black_box(plain)
            });
        });
    }
    group.finish();
}

criterion_group!(
    benches,
    bench_serialize_tagged,
    bench_deserialize_tagged,
    bench_to_json_plain
);
criterion_main!(benches);
