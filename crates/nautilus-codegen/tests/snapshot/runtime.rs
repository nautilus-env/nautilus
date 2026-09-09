use super::support::{generated_java_file, validate};
use nautilus_codegen::java::generate_java_client;
use nautilus_codegen::js::{generate_js_client, js_runtime_files};
use nautilus_codegen::python::{generate_python_client, python_runtime_files};

const JAVA_CLIENT_SYNC_SCHEMA: &str = include_str!("../fixtures/schemas/java_client_sync.nautilus");
const USER_SCHEMA: &str = include_str!("../fixtures/schemas/user.nautilus");

#[test]
fn test_python_runtime_exposes_engine_pool_options() {
    let ir = validate(USER_SCHEMA);
    let client = generate_python_client(&ir.models, "schema.nautilus", false)
        .expect("generate_python_client should succeed");
    let runtime = python_runtime_files();
    let engine_runtime = runtime
        .iter()
        .find(|(name, _)| name == "_engine.py")
        .expect("missing Python runtime engine")
        .1
        .as_str();

    assert!(
        client.contains("pool_options: EnginePoolOptions | None = None"),
        "expected generated Python client to expose pool_options:\n{client}"
    );
    assert!(
        engine_runtime.contains("class EnginePoolOptions:")
            && engine_runtime.contains("--max-connections")
            && engine_runtime.contains("--disable-idle-timeout")
            && engine_runtime.contains("--test-before-acquire")
            && engine_runtime.contains("--statement-cache-capacity"),
        "expected Python runtime engine to forward pool options to the CLI:\n{engine_runtime}"
    );
}

#[test]
fn test_js_runtime_exposes_engine_pool_options() {
    let ir = validate(USER_SCHEMA);
    let (_client_js, client_dts) = generate_js_client(&ir.models, "schema.nautilus")
        .expect("generate_js_client should succeed");
    let runtime = js_runtime_files();
    let client_runtime = runtime
        .iter()
        .find(|(name, _)| name == "_client.d.ts")
        .expect("missing JS runtime client declarations")
        .1
        .as_str();
    let engine_runtime_dts = runtime
        .iter()
        .find(|(name, _)| name == "_engine.d.ts")
        .expect("missing JS runtime engine declarations")
        .1
        .as_str();
    let engine_runtime = runtime
        .iter()
        .find(|(name, _)| name == "_engine.js")
        .expect("missing JS runtime engine")
        .1
        .as_str();

    assert!(
        client_dts.contains("constructor(options?: NautilusClientOptions);")
            && client_runtime.contains("pool?: EnginePoolOptions;"),
        "expected generated JS declarations to expose engine pool options:\n{client_dts}"
    );
    assert!(
        engine_runtime_dts.contains("export interface EnginePoolOptions")
            && engine_runtime.contains("--max-connections")
            && engine_runtime.contains("--disable-idle-timeout")
            && engine_runtime.contains("--test-before-acquire")
            && engine_runtime.contains("--statement-cache-capacity"),
        "expected JS runtime engine to forward pool options to the CLI:\n{engine_runtime}"
    );
}

#[test]
fn test_java_runtime_loads_dotenv_before_spawning_engine() {
    let ir = validate(JAVA_CLIENT_SYNC_SCHEMA);
    let files =
        generate_java_client(&ir, "schema.nautilus", false).expect("generate_java_client failed");
    let engine_process = generated_java_file(&files, "internal/EngineProcess.java");

    assert!(
        engine_process.contains("loadDotenv(builder.environment(), schemaPath);"),
        "expected generated Java runtime to load .env before starting the engine:\n{engine_process}"
    );
    assert!(
        engine_process.contains("Path candidate = root.resolve(\".env\");"),
        "expected generated Java runtime to search for .env files near the schema:\n{engine_process}"
    );
    assert!(
        engine_process.contains("environment.putIfAbsent(key, value);"),
        "expected generated Java runtime to preserve pre-existing environment variables:\n{engine_process}"
    );
    assert!(
        engine_process.contains("Optional<String> localBinary = findLocalBinary(schemaPath);"),
        "expected generated Java runtime to prefer a local nautilus binary before PATH lookup:\n{engine_process}"
    );
}

#[test]
fn test_java_runtime_exposes_engine_pool_options() {
    let ir = validate(JAVA_CLIENT_SYNC_SCHEMA);
    let files =
        generate_java_client(&ir, "schema.nautilus", false).expect("generate_java_client failed");
    let options = generated_java_file(&files, "client/NautilusOptions.java");
    let engine_process = generated_java_file(&files, "internal/EngineProcess.java");

    assert!(
        options.contains("public NautilusOptions maxConnections(Integer maxConnections)")
            && options
                .contains("public NautilusOptions disableIdleTimeout(boolean disableIdleTimeout)")
            && options.contains("public Boolean testBeforeAcquire()"),
        "expected generated Java options to expose engine pool settings:\n{options}"
    );
    assert!(
        engine_process.contains("command.add(\"--max-connections\");")
            && engine_process.contains("command.add(\"--disable-idle-timeout\");")
            && engine_process.contains("command.add(\"--test-before-acquire\");"),
        "expected generated Java runtime engine to forward pool options to the CLI:\n{engine_process}"
    );
}
