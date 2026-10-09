//! Check the generated package itself, as when it lives in an application's tree.
//! Silencing imported modules would hide errors in delegates and runtime helpers.

use nautilus_codegen::{generate_command, GenerateOptions, InstallMode};
use std::{env, fs, process::Command};

const SCHEMA: &str = include_str!("fixtures/schemas/python_typecheck.nautilus");
const CONSUMER: &str = include_str!("fixtures/python_typecheck_consumer.py.tpl");
const SIMPLE_SCHEMA: &str = r#"
datasource db {
  provider = "sqlite"
  url = "sqlite://unused.db"
}
model Order {
  id Int @id @default(autoincrement())
  note String
}
"#;

const LIST_FIELDS: &[&str] = &[
    "history",
    "notes",
    "prices",
    "times",
    "identifiers",
    "files",
    "weights",
    "flags",
    "quantities",
    "sequences",
    "addresses",
];

fn python() -> Option<String> {
    let candidates = env::var("NAUTILUS_PYTHON")
        .map(|value| vec![value])
        .unwrap_or_else(|_| vec!["python3".into(), "python".into()]);
    for candidate in candidates {
        if Command::new(&candidate)
            .args(["-c", "import mypy.version, pydantic"])
            .output()
            .is_ok_and(|output| output.status.success())
        {
            return Some(candidate);
        }
    }
    assert!(
        env::var_os("NAUTILUS_REQUIRE_E2E").is_none(),
        "Python with mypy and pydantic is required"
    );
    eprintln!("skipping Python type check: install mypy and pydantic");
    None
}

fn check_generated_client(interface: &str, source: &str) {
    let Some(python) = python() else { return };
    let fixture = tempfile::tempdir().unwrap();
    let schema = fixture.path().join("schema.nautilus");
    let generated = fixture.path().join("generated");
    fs::write(
        &schema,
        format!(
            "{source}\ngenerator client {{\n  provider = \"nautilus-client-py\"\n  interface = \"{interface}\"\n  output = {:?}\n}}\n", generated.to_str().unwrap()
        ),
    )
    .unwrap();
    generate_command(
        &schema,
        GenerateOptions {
            install: InstallMode::Never,
            ..Default::default()
        },
    )
    .expect("generate client");
    let await_call = if interface == "async" { "await " } else { "" };
    let async_def = if interface == "async" { "async " } else { "" };
    let mut consumer = CONSUMER
        .replace("__ASYNC__", async_def)
        .replace("__AWAIT__", await_call)
        .replace(
            "__TRANSACTION__",
            if interface == "async" {
                "transaction"
            } else {
                "sync_transaction"
            },
        );
    if source == SCHEMA {
        consumer.push_str(&format!(
            "\n\n{async_def}def lists(client: Nautilus) -> None:\n"
        ));
        for field in LIST_FIELDS {
            consumer.push_str(&format!(
                "    {await_call}client.order.update(data={{\"{field}\": []}})\n"
            ));
        }
        consumer.push_str(&format!("    {await_call}client.order.aggregate(avg={{\"weight\": True}}, sum={{\"quantity\": True}}, count={{\"weights\": True}})\n"));
    }
    fs::write(fixture.path().join("app.py"), consumer).unwrap();
    let imports = Command::new(&python)
        .args(["-c", "import app; from generated.models import Order; assert 'nautilus' not in Order.model_fields"])
        .current_dir(fixture.path())
        .output()
        .expect("import generated client and event handlers");
    assert!(
        imports.status.success(),
        "generated {interface} package must import with Pydantic:\n{}",
        String::from_utf8_lossy(&imports.stderr)
    );
    for plugin in [false, true] {
        fs::write(
            fixture.path().join("mypy.ini"),
            if plugin {
                "[mypy]\nplugins = pydantic.mypy\n"
            } else {
                "[mypy]\n"
            },
        )
        .unwrap();
        let output = Command::new(&python)
            .args([
                "-m",
                "mypy",
                "--strict",
                "--no-incremental",
                "--config-file",
                "mypy.ini",
                ".",
            ])
            .current_dir(fixture.path())
            .output()
            .expect("run mypy");
        assert!(
            output.status.success(),
            "{interface} client (pydantic plugin: {plugin}) failed mypy:\n{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
    // The package must still reject invalid calls: making delegates Any would
    // make the positive test pass while removing their useful type contract.
    fs::write(
        fixture.path().join("invalid.py"),
        format!(
            "from generated import Nautilus\n\n{async_def}def invalid(client: Nautilus) -> None:\n    {await_call}client.order.find_unique(where={{\"id\": \"wrong\"}})\n    {await_call}client.order.find_many(unknown=True)\n    async with client.transaction() as tx:\n        {await_call}tx.order.find_many(unknown=True)\n"
        ).replace("    async with client.transaction()", &format!("    {async_def}with client.{}()", if interface == "async" { "transaction" } else { "sync_transaction" })),
    ).unwrap();
    let mut expected_errors = 3;
    if source == SCHEMA {
        let invalid_path = fixture.path().join("invalid.py");
        let mut invalid = fs::read_to_string(&invalid_path).unwrap();
        for field in LIST_FIELDS {
            for call in [
                format!("find_many(where={{\"{field}\": 1}})"),
                format!("find_many(where={{\"{field}\": {{\"equals\": 1}}}})"),
                format!("update(data={{\"{field}\": {{\"set\": []}}}})"),
                format!("update(data={{\"{field}\": {{\"increment\": 1}}}})"),
                format!("update(data={{\"{field}\": 1.5}})"),
                format!("aggregate(avg={{\"{field}\": True}})"),
                format!("aggregate(sum={{\"{field}\": True}})"),
            ] {
                invalid.push_str(&format!("    {await_call}client.order.{call}\n"));
                expected_errors += 1;
            }
        }
        fs::write(invalid_path, invalid).unwrap();
    }
    let output = Command::new(&python)
        .args([
            "-m",
            "mypy",
            "--strict",
            "--no-incremental",
            "--config-file",
            "mypy.ini",
            "invalid.py",
        ])
        .current_dir(fixture.path())
        .output()
        .expect("check invalid calls");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        !output.status.success()
            && stdout.contains(&format!("Found {expected_errors} errors in 1 file")),
        "invalid calls must report {expected_errors} errors:\n{stdout}\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn generated_async_client_passes_mypy_strict() {
    check_generated_client("async", SCHEMA);
}

#[test]
fn generated_sync_client_passes_mypy_strict() {
    check_generated_client("sync", SCHEMA);
}

#[test]
fn generated_simple_clients_pass_mypy_strict() {
    for interface in ["async", "sync"] {
        check_generated_client(interface, SIMPLE_SCHEMA);
    }
}
