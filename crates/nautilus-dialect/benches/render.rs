//! Baseline benchmarks for SQL rendering (plan.md, fase 0).
//!
//! Rendering consumes the AST (`render_*_owned`), so every iteration receives
//! a fresh clone built in the `iter_batched` setup closure (not measured).

use std::hint::black_box;

use criterion::{criterion_group, criterion_main, BatchSize, BenchmarkId, Criterion};
use nautilus_core::{ColumnMarker, Expr, Insert, Select, SelectItem, Update, Value};
use nautilus_dialect::{Dialect, MysqlDialect, PostgresDialect, SqliteDialect};

fn marker(i: usize) -> ColumnMarker {
    ColumnMarker::new("users", format!("col_{i:02}"))
}

/// SELECT with `columns` projected items, a 3-predicate AND filter,
/// ORDER BY + LIMIT/OFFSET — the shape produced by a typical `findMany`.
fn select_ast(columns: usize) -> Select {
    let mut builder = Select::from_table("users");
    for i in 0..columns {
        builder = builder.item(SelectItem::column(marker(i)));
    }

    let filter = Expr::column("users__tenant_id")
        .eq(Expr::param(Value::I64(42)))
        .and(Expr::column("users__status").eq(Expr::param(Value::String("active".to_string()))))
        .and(Expr::column("users__score").gt(Expr::param(Value::F64(0.5))));

    builder
        .filter(filter)
        .order_by_desc("users__created_at")
        .take(50)
        .skip(100)
        .build()
        .expect("benchmark select AST should build")
}

/// INSERT of `rows` rows x `columns` columns with RETURNING on all columns —
/// the shape produced by `create` / `createMany`.
fn insert_ast(rows: usize, columns: usize) -> Insert {
    let markers: Vec<ColumnMarker> = (0..columns).map(marker).collect();

    let data: Vec<Vec<Value>> = (0..rows)
        .map(|r| {
            (0..columns)
                .map(|c| match c % 4 {
                    0 => Value::I64((r * columns + c) as i64),
                    1 => Value::String(format!("value-{r}-{c}")),
                    2 => Value::Bool(r % 2 == 0),
                    _ => Value::F64(r as f64 + c as f64 * 0.25),
                })
                .collect()
        })
        .collect();

    Insert::into_table("users")
        .columns(markers.clone())
        .rows(data)
        .returning(markers)
        .build()
        .expect("benchmark insert AST should build")
}

/// UPDATE of `columns` assignments with a single-PK filter and RETURNING —
/// the shape produced by `update`.
fn update_ast(columns: usize) -> Update {
    let mut builder = Update::table("users");
    for i in 0..columns {
        builder = builder.set(marker(i), Value::String(format!("value-{i}")));
    }

    builder
        .filter(Expr::column("users__id").eq(Expr::param(Value::I64(7))))
        .returning(vec![ColumnMarker::new("users", "id")])
        .build()
        .expect("benchmark update AST should build")
}

fn bench_render_select(c: &mut Criterion) {
    let mut group = c.benchmark_group("render_select/postgres");
    for &cols in &[5usize, 20, 50] {
        let ast = select_ast(cols);
        group.bench_with_input(BenchmarkId::from_parameter(cols), &ast, |b, ast| {
            b.iter_batched(
                || ast.clone(),
                |owned| black_box(PostgresDialect.render_select_owned(owned)),
                BatchSize::SmallInput,
            );
        });
    }
    group.finish();

    let ast = select_ast(20);
    let mut group = c.benchmark_group("render_select_20col");
    group.bench_function("postgres", |b| {
        b.iter_batched(
            || ast.clone(),
            |owned| black_box(PostgresDialect.render_select_owned(owned)),
            BatchSize::SmallInput,
        );
    });
    group.bench_function("mysql", |b| {
        b.iter_batched(
            || ast.clone(),
            |owned| black_box(MysqlDialect.render_select_owned(owned)),
            BatchSize::SmallInput,
        );
    });
    group.bench_function("sqlite", |b| {
        b.iter_batched(
            || ast.clone(),
            |owned| black_box(SqliteDialect.render_select_owned(owned)),
            BatchSize::SmallInput,
        );
    });
    group.finish();
}

fn bench_render_insert(c: &mut Criterion) {
    let mut group = c.benchmark_group("render_insert/postgres");
    for &(rows, cols) in &[(1usize, 10usize), (100, 10)] {
        let ast = insert_ast(rows, cols);
        group.bench_with_input(
            BenchmarkId::from_parameter(format!("{rows}x{cols}")),
            &ast,
            |b, ast| {
                b.iter_batched(
                    || ast.clone(),
                    |owned| black_box(PostgresDialect.render_insert_owned(owned)),
                    BatchSize::SmallInput,
                );
            },
        );
    }
    group.finish();
}

fn bench_render_update(c: &mut Criterion) {
    let ast = update_ast(10);
    c.bench_function("render_update/postgres/10col", |b| {
        b.iter_batched(
            || ast.clone(),
            |owned| black_box(PostgresDialect.render_update_owned(owned)),
            BatchSize::SmallInput,
        );
    });
}

criterion_group!(
    benches,
    bench_render_select,
    bench_render_insert,
    bench_render_update
);
criterion_main!(benches);
