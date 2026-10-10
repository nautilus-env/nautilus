use nautilus_codegen::{generate_command, GenerateOptions, InstallMode};
use std::{fs, path::Path, process::Command};

#[test]
#[ignore = "requires PostgreSQL; set NAUTILUS_TEST_POSTGRES_URL"]
fn generated_rust_postgres_result_aliases() {
    let database = std::env::var("NAUTILUS_TEST_POSTGRES_URL").expect("PostgreSQL test URL");
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap();
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos()
        .to_string();
    for table in [
        format!("orders_{nonce:a<42}"),
        format!("ordini_{}_{nonce}", "è".repeat(16)),
    ] {
        let fixture = tempfile::tempdir_in(root).unwrap();
        let schema = fixture.path().join("schema.nautilus");
        let output_path = fixture.path().to_string_lossy().replace('\\', "/");
        fs::write(
            &schema,
            format!(
                r#"
datasource db {{ provider = "postgresql" url = env("NAUTILUS_TEST_POSTGRES_URL") }}
generator client {{ provider = "nautilus-client-rs" interface = "async" output = "{output_path}" }}
model Order {{
  id Int @id @map("order_identifier")
  contactNames String[] @map("contact_names")
  contactNamex String[] @map("contact_namex")
  contactNotes String? @map("contact_notes")
  @@map("{table}")
}}
"#
            ),
        )
        .unwrap();
        generate_command(
            &schema,
            GenerateOptions {
                standalone: true,
                install: InstallMode::Never,
                ..Default::default()
            },
        )
        .unwrap();
        fs::copy(root.join("Cargo.lock"), fixture.path().join("Cargo.lock")).unwrap();
        let tests = fixture.path().join("tests");
        fs::create_dir(&tests).unwrap();
        fs::write(
            tests.join("result_aliases.rs"),
            include_str!("fixtures/result_aliases/rust.rs").replace("__TABLE__", &table),
        )
        .unwrap();
        let output = Command::new("cargo")
            .args(["test", "--quiet", "--offline", "--manifest-path"])
            .arg(fixture.path().join("Cargo.toml"))
            .args(["--test", "result_aliases", "--", "--nocapture"])
            .env("NAUTILUS_TEST_POSTGRES_URL", &database)
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
}
