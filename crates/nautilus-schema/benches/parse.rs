//! Benchmarks for the schema front-end.
//!
//! Lexing, parsing, and full validation of a synthetic 100-model schema.
//! These are the paths the LSP re-runs on every keystroke (mitigated by the
//! per-document cache) and the CLI runs on `validate` / `generate`.

use std::hint::black_box;

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion};
use nautilus_schema::{parse_schema_source, validate_schema_source, Lexer, TokenKind};

/// Generate a valid schema with `models` models sharing one enum.
fn synthetic_schema(models: usize) -> String {
    let mut source = String::with_capacity(models * 320 + 256);

    source.push_str(
        "datasource db {\n  provider = \"postgresql\"\n  url      = \"postgres://localhost:5432/bench\"\n}\n\n",
    );
    source.push_str("enum Status {\n  ACTIVE\n  SUSPENDED\n  DELETED\n}\n\n");

    for i in 0..models {
        source.push_str(&format!(
            "model Model{i:03} {{\n\
             \x20 id        Int      @id @default(autoincrement())\n\
             \x20 publicId  String   @unique\n\
             \x20 name      String\n\
             \x20 status    Status\n\
             \x20 score     Float?\n\
             \x20 payload   Json?\n\
             \x20 tags      String[]\n\
             \x20 createdAt DateTime @default(now())\n\
             }}\n\n"
        ));
    }

    source
}

fn lex_all(source: &str) -> usize {
    let mut lexer = Lexer::new(source);
    let mut count = 0usize;
    loop {
        let token = lexer.next_token().expect("benchmark schema should lex");
        if token.kind == TokenKind::Eof {
            break;
        }
        count += 1;
    }
    count
}

fn bench_schema_frontend(c: &mut Criterion) {
    let source = synthetic_schema(100);

    // Fail loudly up front if the synthetic schema ever stops being valid.
    validate_schema_source(&source).expect("synthetic benchmark schema should validate");

    let mut group = c.benchmark_group("schema_frontend/100_models");

    group.bench_with_input(BenchmarkId::from_parameter("lex"), &source, |b, src| {
        b.iter(|| black_box(lex_all(black_box(src))));
    });

    group.bench_with_input(BenchmarkId::from_parameter("parse"), &source, |b, src| {
        b.iter(|| black_box(parse_schema_source(black_box(src)).expect("parse")));
    });

    group.bench_with_input(
        BenchmarkId::from_parameter("validate"),
        &source,
        |b, src| {
            b.iter(|| black_box(validate_schema_source(black_box(src)).expect("validate")));
        },
    );

    group.finish();
}

criterion_group!(benches, bench_schema_frontend);
criterion_main!(benches);
