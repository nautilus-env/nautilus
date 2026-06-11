//! Benchmarks for `Row` construction and lookup.
//!
//! `row_build_first_get/wide_16col` builds from owned `String` names (the
//! reshaping path, which also pays the `String -> Arc<str>` conversion). The
//! `_shared` variant builds the way decoders do: cloning per-statement
//! `Arc<str>` names.

use std::hint::black_box;
use std::sync::Arc;

use criterion::{criterion_group, criterion_main, BatchSize, Criterion};
use nautilus_connector::Row;
use nautilus_core::Value;

fn columns(n: usize) -> Vec<(String, Value)> {
    (0..n)
        .map(|i| {
            let value = match i % 3 {
                0 => Value::I64(i as i64),
                1 => Value::String(format!("value-{i}")),
                _ => Value::Bool(i % 2 == 0),
            };
            (format!("users__col_{i:02}"), value)
        })
        .collect()
}

/// Repeated named lookups on a 5-column row (linear-scan path).
fn bench_get_narrow(c: &mut Criterion) {
    let row = Row::new(columns(5));
    c.bench_function("row_get/narrow_5col", |b| {
        b.iter(|| black_box(row.get(black_box("users__col_03"))));
    });
}

/// Repeated named lookups on a 16-column row after the lazy index is built
/// (steady-state path used by `get` in include hydration and decoding).
fn bench_get_wide_indexed(c: &mut Criterion) {
    let row = Row::new(columns(16));
    // Force the lazy index build outside the measured loop.
    assert!(row.get("users__col_12").is_some());
    c.bench_function("row_get/wide_16col_indexed", |b| {
        b.iter(|| black_box(row.get(black_box("users__col_12"))));
    });
}

/// Build a fresh 16-column row from owned `String` names and do a first
/// lookup, paying the lazy index construction and the `String -> Arc<str>`
/// conversion; decoders no longer use this path.
fn bench_build_and_first_get(c: &mut Criterion) {
    c.bench_function("row_build_first_get/wide_16col", |b| {
        b.iter_batched(
            || columns(16),
            |cols| {
                let row = Row::new(cols);
                black_box(row.get("users__col_12").cloned())
            },
            BatchSize::SmallInput,
        );
    });
}

/// Build a fresh 16-column row the way decoders do: column names built once
/// per statement and shared across rows via `Arc` clones.
fn bench_build_shared_names_and_first_get(c: &mut Criterion) {
    let names: Vec<Arc<str>> = columns(16)
        .into_iter()
        .map(|(name, _)| Arc::from(name))
        .collect();
    let values: Vec<Value> = columns(16).into_iter().map(|(_, value)| value).collect();

    c.bench_function("row_build_first_get/wide_16col_shared", |b| {
        b.iter(|| {
            let mut row = Row::with_capacity(names.len());
            for (name, value) in names.iter().zip(&values) {
                row.push_column(Arc::clone(name), value.clone());
            }
            black_box(row.get("users__col_12").cloned())
        });
    });
}

criterion_group!(
    benches,
    bench_get_narrow,
    bench_get_wide_indexed,
    bench_build_and_first_get,
    bench_build_shared_names_and_first_get
);
criterion_main!(benches);
