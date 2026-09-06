use nautilus_codegen::{
    enum_gen::generate_all_enums, generator::generate_all_models, writer::write_rust_code,
};
use nautilus_schema::validate_schema_source;
use std::{fs, path::Path, process::Command};

const SCHEMA: &str = include_str!("fixtures/path_equivalence/schema.nautilus");

#[test]
fn generated_rust_client_and_engine_paths_agree() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap();
    let fixture = tempfile::tempdir_in(root).unwrap();
    let ir = validate_schema_source(SCHEMA).unwrap().ir;
    write_rust_code(
        fixture.path().to_str().unwrap(),
        &generate_all_models(&ir, true).unwrap(),
        Some(generate_all_enums(&ir.enums).unwrap()),
        None,
        &[],
        SCHEMA,
        true,
    )
    .unwrap();
    let manifest_path = fixture.path().join("Cargo.toml");
    let mut manifest = fs::read_to_string(&manifest_path).unwrap();
    manifest.push_str(
        "\n[dev-dependencies]\n\
         nautilus-migrate = { path = \"../crates/nautilus-migrate\", package = \"nautilus-orm-migrate\" }\n\
         tempfile = \"3\"\n",
    );
    fs::write(&manifest_path, manifest).unwrap();
    fs::copy(root.join("Cargo.lock"), fixture.path().join("Cargo.lock")).unwrap();
    let tests = fixture.path().join("tests");
    fs::create_dir(&tests).unwrap();
    fs::write(tests.join("schema.nautilus"), SCHEMA).unwrap();
    for (path, source) in [
        (
            "equivalence.rs",
            include_str!("fixtures/path_equivalence/equivalence.rs"),
        ),
        (
            "engine/mod.rs",
            include_str!("fixtures/path_equivalence/engine.rs"),
        ),
        (
            "client/mod.rs",
            include_str!("fixtures/path_equivalence/client.rs"),
        ),
        (
            "common/mod.rs",
            include_str!("../../nautilus-engine/tests/common/mod.rs"),
        ),
    ] {
        let path = tests.join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, source).unwrap();
    }
    let output = Command::new("cargo")
        .args(["test", "--quiet", "--offline", "--manifest-path"])
        .arg(manifest_path)
        .args(["--test", "equivalence", "--", "--nocapture"])
        .env("CARGO_TARGET_DIR", root.join("target/path-equivalence"))
        .output()
        .expect("failed to execute the generated Rust consumer");
    assert!(
        output.status.success(),
        "generated consumer failed:\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
}
