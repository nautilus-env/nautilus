use super::support::{assert_codegen_snapshot, generated_java_file, validate};
use nautilus_codegen::generator::generate_all_models;
use nautilus_codegen::java::generate_java_client;
use nautilus_codegen::js::{generate_js_client, js_runtime_files};
use nautilus_codegen::python::{
    generate_all_python_models, generate_python_client, python_runtime_files,
};

const JAVA_CLIENT_ASYNC_SCHEMA: &str =
    include_str!("../fixtures/schemas/java_client_async.nautilus");
const USER_POST_SCHEMA: &str = include_str!("../fixtures/schemas/user_post.nautilus");
const USER_SCHEMA: &str = include_str!("../fixtures/schemas/user.nautilus");

#[test]
fn test_rust_async_generates_async_fns() {
    let ir = validate(USER_SCHEMA);
    let sync_models = generate_all_models(&ir, false).expect("generate_all_models should succeed");
    let async_models = generate_all_models(&ir, true).expect("generate_all_models should succeed");
    let sync_code = sync_models.get("User").unwrap();
    let async_code = async_models.get("User").unwrap();
    assert!(
        async_code.contains("async"),
        "expected async in async mode:\n{async_code}"
    );
    assert_ne!(sync_code, async_code, "sync and async should differ");
    assert_codegen_snapshot!("rust_user_async", async_code);
}

#[test]
fn test_python_async_generates_async_defs() {
    let ir = validate(USER_SCHEMA);
    let sync_models = generate_all_python_models(&ir, false, 0)
        .expect("generate_all_python_models should succeed");
    let async_models = generate_all_python_models(&ir, true, 0)
        .expect("generate_all_python_models should succeed");
    let (_, sync_code) = sync_models.iter().find(|(n, _)| n == "user.py").unwrap();
    let (_, async_code) = async_models.iter().find(|(n, _)| n == "user.py").unwrap();
    assert!(
        async_code.contains("async def"),
        "expected async def:\n{async_code}"
    );
    assert_ne!(sync_code, async_code, "sync and async should differ");
    assert_codegen_snapshot!("python_user_async", async_code);
}

/// Exercises `generate_python_client`: verifies the output contains the top-level
/// `NautilusClient` class and per-model delegate attributes.
#[test]
fn test_python_client_generation() {
    let ir = validate(USER_POST_SCHEMA);
    let client_sync = generate_python_client(&ir.models, "schema.nautilus", false)
        .expect("generate_python_client should succeed");
    let client_async = generate_python_client(&ir.models, "schema.nautilus", true)
        .expect("generate_python_client should succeed");
    assert!(
        client_sync.contains("NautilusClient"),
        "expected NautilusClient:\n{client_sync}"
    );
    assert!(
        client_async.contains("async def") || client_async.contains("async"),
        "expected async keyword in async client:\n{client_async}"
    );
    assert_ne!(
        client_sync, client_async,
        "sync and async clients should differ"
    );
    assert_codegen_snapshot!("python_client_sync", &client_sync);
}

#[test]
fn test_js_client_exposes_batch_transactions_and_runtime_stays_on_protocol_v1() {
    let ir = validate(USER_SCHEMA);
    let (client_js, client_dts) = generate_js_client(&ir.models, "schema.nautilus")
        .expect("generate_js_client should succeed");
    assert_codegen_snapshot!("js_client", client_js);
    assert_codegen_snapshot!("js_client_declarations", client_dts);
    let runtime = js_runtime_files();
    let client_runtime = runtime
        .iter()
        .find(|(name, _)| name == "_client.js")
        .expect("missing JS runtime client")
        .1
        .as_str();
    let protocol_runtime = runtime
        .iter()
        .find(|(name, _)| name == "_protocol.js")
        .expect("missing JS runtime protocol")
        .1
        .as_str();
    let error_runtime = runtime
        .iter()
        .find(|(name, _)| name == "_errors.js")
        .expect("missing JS runtime errors")
        .1
        .as_str();
    let tx_runtime = runtime
        .iter()
        .find(|(name, _)| name == "_transaction.js")
        .expect("missing JS runtime transaction")
        .1
        .as_str();

    assert!(
        client_js.contains("async $transactionBatch(operations, options)"),
        "expected generated JS client to expose $transactionBatch():\n{client_js}"
    );
    assert!(
        client_dts.contains("$transactionBatch("),
        "expected generated JS declarations to expose $transactionBatch():\n{client_dts}"
    );
    assert!(
        protocol_runtime.contains("export const PROTOCOL_VERSION = 1;")
            && client_runtime.contains("protocolVersion: PROTOCOL_VERSION")
            && client_runtime.contains("client expects ${PROTOCOL_VERSION}")
            && client_runtime.contains("transaction.batch")
            && client_runtime.contains("async *_streamRpc(")
            && client_runtime.contains("method: 'request.cancel'"),
        "expected JS runtime client to reuse the shared protocol version constant and expose transaction.batch:\n{client_runtime}\n\nProtocol:\n{protocol_runtime}"
    );
    assert!(
        error_runtime.contains("this.data = details?.data"),
        "expected JS runtime errors to retain error.data from the engine:\n{error_runtime}"
    );
    assert!(
        !tx_runtime.contains("snapshot"),
        "expected JS runtime isolation levels to match the protocol exactly:\n{tx_runtime}"
    );
}

#[test]
fn test_python_runtime_stays_on_protocol_v1_and_preserves_error_data() {
    let runtime = python_runtime_files();
    let client_runtime = runtime
        .iter()
        .find(|(name, _)| name == "_client.py")
        .expect("missing Python runtime client")
        .1
        .as_str();
    let protocol_runtime = runtime
        .iter()
        .find(|(name, _)| name == "_protocol.py")
        .expect("missing Python runtime protocol")
        .1
        .as_str();
    let error_runtime = runtime
        .iter()
        .find(|(name, _)| name == "_errors.py")
        .expect("missing Python runtime errors")
        .1
        .as_str();
    let tx_runtime = runtime
        .iter()
        .find(|(name, _)| name == "_transaction.py")
        .expect("missing Python runtime transaction")
        .1
        .as_str();

    assert!(
        protocol_runtime.contains("PROTOCOL_VERSION = 1")
            && client_runtime.contains("\"protocolVersion\": PROTOCOL_VERSION")
            && client_runtime.contains("client expects {PROTOCOL_VERSION}")
            && client_runtime.contains("async def transaction_batch(")
            && client_runtime.contains("async def _stream_rpc(")
            && client_runtime.contains("method=\"request.cancel\""),
        "expected Python runtime client to reuse the shared protocol version constant and keep transaction_batch():\n{client_runtime}\n\nProtocol:\n{protocol_runtime}"
    );
    assert!(
        protocol_runtime.contains("self.error.data"),
        "expected Python runtime protocol to preserve error.data:\n{protocol_runtime}"
    );
    assert!(
        error_runtime.contains("self.data = data"),
        "expected Python runtime errors to retain error.data from the engine:\n{error_runtime}"
    );
    assert!(
        !tx_runtime.contains("SNAPSHOT"),
        "expected Python runtime isolation levels to match the protocol exactly:\n{tx_runtime}"
    );
}

#[test]
fn test_java_sync_generation_exposes_model_delegate_and_autoregister_accessor() {
    let ir = validate(
        r#"
generator client {
  provider    = "nautilus-client-java"
  output      = "./generated-java"
  package     = "com.acme.db"
  group_id    = "com.acme"
  artifact_id = "db-client"
  interface   = "sync"
}

enum Role {
  ADMIN
  MEMBER
}

model User {
  id   Int    @id @default(autoincrement())
  name String
  role Role
}
"#,
    );
    let files =
        generate_java_client(&ir, "schema.nautilus", false).expect("generate_java_client failed");
    let user_model = generated_java_file(&files, "model/User.java");
    let nautilus_client = generated_java_file(&files, "client/Nautilus.java");

    assert!(
        user_model.contains("public static UserDelegate nautilus()"),
        "expected generated Java model to expose static nautilus() accessor:\n{user_model}"
    );
    assert!(
        user_model.contains("GlobalNautilusRegistry.require()"),
        "expected generated Java model to resolve the auto-registered client:\n{user_model}"
    );
    assert!(
        nautilus_client.contains("GlobalNautilusRegistry.register(this);"),
        "expected generated Java client to auto-register itself when configured:\n{nautilus_client}"
    );

    assert_codegen_snapshot!("java_user_model_sync", user_model);
}

#[test]
fn test_java_async_generation_exposes_completable_future_transaction_api() {
    let ir = validate(JAVA_CLIENT_ASYNC_SCHEMA);
    let files =
        generate_java_client(&ir, "schema.nautilus", true).expect("generate_java_client failed");
    let delegate = generated_java_file(&files, "client/UserDelegate.java");
    let nautilus_client = generated_java_file(&files, "client/Nautilus.java");

    assert!(
        delegate.contains("CompletableFuture<List<User>> findMany()"),
        "expected generated Java async delegate to expose CompletableFuture APIs:\n{delegate}"
    );
    assert!(
        nautilus_client.contains(
            "public <T> CompletableFuture<T> transaction(Function<TransactionClient, CompletableFuture<T>> callback)"
        ),
        "expected generated Java async client to expose CompletableFuture transaction API:\n{nautilus_client}"
    );

    assert_codegen_snapshot!("java_nautilus_async", nautilus_client);
}
