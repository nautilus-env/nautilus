//! Benchmark for the pure include-hydration grouping path (plan.md, item 1.5,
//! deviazione di fase 0).
//!
//! Covers the in-memory half of the batched include path: grouping child rows
//! by FK and producing the per-parent JSON payloads. The child query itself is
//! out of scope (network/DB bound).

use std::collections::HashMap;
use std::hint::black_box;

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion};
use nautilus_connector::Row;
use nautilus_core::Value;
use nautilus_engine::handlers::{build_include_values, group_key, GroupKey, IncludeProjection};

/// Child rows shaped like a typical 4-column relation target, `children` rows
/// per parent, FK pointing back at the parent index.
fn build_child_rows(parents: usize, children_per_parent: usize) -> Vec<Row> {
    (0..parents)
        .flat_map(|p| {
            (0..children_per_parent).map(move |c| {
                Row::new(vec![
                    (
                        "blog_posts__post_id".to_string(),
                        Value::I64((p * children_per_parent + c) as i64),
                    ),
                    (
                        "blog_posts__title".to_string(),
                        Value::String(format!("post-{p}-{c}")),
                    ),
                    ("blog_posts__sort_index".to_string(), Value::I32(c as i32)),
                    ("blog_posts__author_id".to_string(), Value::I64(p as i64)),
                ])
            })
        })
        .collect()
}

fn bench_hydrate_includes(c: &mut Criterion) {
    let projection = IncludeProjection::new(vec![
        ("blog_posts__post_id".to_string(), "id".to_string()),
        ("blog_posts__title".to_string(), "title".to_string()),
        ("blog_posts__sort_index".to_string(), "sort".to_string()),
        ("blog_posts__author_id".to_string(), "authorId".to_string()),
    ]);

    let mut group = c.benchmark_group("hydrate_includes");
    for &(parents, children) in &[(100usize, 10usize), (1_000, 10)] {
        let child_rows = build_child_rows(parents, children);
        let row_keys: Vec<Option<GroupKey>> = (0..parents)
            .map(|p| Some(group_key(&Value::I64(p as i64))))
            .collect();
        let key_counts: HashMap<GroupKey, usize> = row_keys
            .iter()
            .flatten()
            .map(|key| (key.clone(), 1usize))
            .collect();

        // Criterion reports per-iteration time; one iteration = grouping one
        // full child result set. row_keys/key_counts are consumed by the call,
        // so their clone cost is part of each iteration (small vs. the JSON
        // conversion work being measured).
        group.bench_with_input(
            BenchmarkId::from_parameter(format!("{parents}x{children}")),
            &child_rows,
            |b, child_rows| {
                b.iter(|| {
                    black_box(build_include_values(
                        black_box(row_keys.clone()),
                        black_box(key_counts.clone()),
                        child_rows,
                        "blog_posts__author_id",
                        &projection,
                        true,
                    ))
                });
            },
        );
    }
    group.finish();
}

criterion_group!(benches, bench_hydrate_includes);
criterion_main!(benches);
