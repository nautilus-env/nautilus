use super::support::{generated_java_file, generated_named_file, generated_python_file, validate};
use nautilus_codegen::generator::generate_all_models;
use nautilus_codegen::java::generate_java_client;
use nautilus_codegen::js::generate_all_js_models;
use nautilus_codegen::python::generate_all_python_models;

const JAVA_CLIENT_ASYNC_SCHEMA: &str =
    include_str!("../fixtures/schemas/java_client_async.nautilus");
const USER_SCHEMA: &str = include_str!("../fixtures/schemas/user.nautilus");

#[test]
fn test_rust_async_delegate_exposes_stream_many() {
    let ir = validate(USER_SCHEMA);
    let async_models = generate_all_models(&ir, true).expect("generate_all_models should succeed");
    let async_code = async_models.get("User").expect("User missing");

    assert!(
        async_code.contains("pub fn stream_many("),
        "expected async delegate to expose stream_many:\n{async_code}"
    );
    assert!(
        async_code.contains("execute_owned(sql)"),
        "expected stream_many to drive the executor's owned-stream path:\n{async_code}"
    );
    assert!(
        async_code.contains("stream_many does not support backward pagination"),
        "expected stream_many to reject backward pagination explicitly:\n{async_code}"
    );

    let sync_models = generate_all_models(&ir, false).expect("generate_all_models should succeed");
    let sync_code = sync_models.get("User").expect("User missing");
    assert!(
        !sync_code.contains("pub fn stream_many("),
        "stream_many should not be emitted for sync clients (the runtime would have to block on iteration); got:\n{sync_code}"
    );
}

#[test]
fn test_python_find_many_exposes_chunk_size() {
    let ir = validate(USER_SCHEMA);
    let models = generate_all_python_models(&ir, false, 0)
        .expect("generate_all_python_models should succeed");
    let (_, code) = models
        .iter()
        .find(|(name, _)| name == "user.py")
        .expect("user model missing");

    assert!(
        code.contains("chunk_size: Optional[int] = None"),
        "expected generated Python find_many() to expose chunk_size:\n{code}"
    );
    assert!(
        code.contains("payload[\"chunkSize\"] = chunk_size"),
        "expected generated Python find_many() to forward chunk_size as protocol chunkSize:\n{code}"
    );
}

#[test]
fn test_python_async_delegate_exposes_stream_many() {
    let ir = validate(USER_SCHEMA);
    let async_models = generate_all_python_models(&ir, true, 0)
        .expect("generate_all_python_models should succeed");
    let sync_models = generate_all_python_models(&ir, false, 0)
        .expect("generate_all_python_models should succeed");
    let async_code = generated_python_file(&async_models, "user.py");
    let sync_code = generated_python_file(&sync_models, "user.py");

    assert!(
        async_code.contains("async def stream_many("),
        "expected generated async Python delegate to expose stream_many():\n{async_code}"
    );
    assert!(
        async_code.contains(") -> AsyncIterator[User]:"),
        "expected generated async Python stream_many() to return an AsyncIterator:\n{async_code}"
    );
    assert!(
        async_code.contains(
            "async for chunk in self._client._stream_rpc(\"query.findMany\", payload):"
        ),
        "expected generated async Python stream_many() to consume chunked RPC frames:\n{async_code}"
    );
    assert!(
        async_code.contains("\"chunkSize\": chunk_size"),
        "expected generated async Python stream_many() to force protocol chunking:\n{async_code}"
    );
    assert!(
        !sync_code.contains("def stream_many("),
        "stream_many should not be emitted for sync Python clients:\n{sync_code}"
    );
}

#[test]
fn test_js_find_many_exposes_chunk_size() {
    let ir = validate(USER_SCHEMA);
    let (js_models, dts_models) =
        generate_all_js_models(&ir).expect("generate_all_js_models should succeed");
    let (_, dts_code) = dts_models
        .iter()
        .find(|(name, _)| name == "user.d.ts")
        .expect("user declaration missing");
    let (_, js_code) = js_models
        .iter()
        .find(|(name, _)| name == "user.js")
        .expect("user runtime missing");

    assert!(
        dts_code.contains("chunkSize?: number;"),
        "expected generated JS findMany() typings to expose chunkSize:\n{dts_code}"
    );
    assert!(
        js_code.contains("if (args?.chunkSize != null) request['chunkSize'] = args.chunkSize;"),
        "expected generated JS findMany() to forward chunkSize at the protocol level:\n{js_code}"
    );
}

#[test]
fn test_js_async_delegate_exposes_stream_many() {
    let ir = validate(USER_SCHEMA);
    let (js_models, dts_models) =
        generate_all_js_models(&ir).expect("generate_all_js_models should succeed");
    let js_code = generated_named_file(&js_models, "user.js");
    let dts_code = generated_named_file(&dts_models, "user.d.ts");

    assert!(
        dts_code.contains("streamMany(args?: Omit<UserFindManyArgs, 'select'>"),
        "expected generated JS typings to expose streamMany():\n{dts_code}"
    );
    assert!(
        dts_code.contains("): AsyncIterable<UserModel>;"),
        "expected generated JS streamMany() typings to return an AsyncIterable:\n{dts_code}"
    );
    assert!(
        js_code.contains("async *streamMany(args) {"),
        "expected generated JS delegate to expose streamMany():\n{js_code}"
    );
    assert!(
        js_code.contains(
            "for await (const chunk of this.client._streamRpc('query.findMany', payload)) {"
        ),
        "expected generated JS streamMany() to consume chunked RPC frames:\n{js_code}"
    );
    assert!(
        js_code.contains("chunkSize,"),
        "expected generated JS streamMany() to force protocol chunking:\n{js_code}"
    );
}

#[test]
fn test_java_generation_exposes_stream_many_over_chunked_rpc() {
    let ir = validate(JAVA_CLIENT_ASYNC_SCHEMA);
    let async_files =
        generate_java_client(&ir, "schema.nautilus", true).expect("generate_java_client failed");
    let sync_files =
        generate_java_client(&ir, "schema.nautilus", false).expect("generate_java_client failed");

    let async_delegate = generated_java_file(&async_files, "client/UserDelegate.java");
    let sync_delegate = generated_java_file(&sync_files, "client/UserDelegate.java");
    let dsl = generated_java_file(&async_files, "dsl/UserDsl.java");
    let rpc_caller = generated_java_file(&async_files, "internal/RpcCaller.java");
    let base_client = generated_java_file(&async_files, "internal/BaseNautilusClient.java");
    let base_tx_client = generated_java_file(&async_files, "internal/BaseTransactionClient.java");
    let base_delegate = generated_java_file(&async_files, "internal/AbstractDelegate.java");

    assert!(
        async_delegate.contains("public Stream<User> streamMany()")
            && sync_delegate.contains("public Stream<User> streamMany()"),
        "expected generated Java delegates to expose streamMany():\nasync:\n{async_delegate}\n\nsync:\n{sync_delegate}"
    );
    assert!(
        base_delegate.contains("DEFAULT_STREAM_CHUNK_SIZE = 128")
            && base_delegate.contains(
                "throw new IllegalArgumentException(operation + \" chunkSize must be a positive integer\");"
            ),
        "expected the Java base delegate to own the stream chunk size rule:\n{base_delegate}"
    );
    assert!(
        async_delegate
            .contains("int chunkSize = streamChunkSize(actual.chunkSize(), \"streamMany\");")
            && async_delegate.contains(
                "return rows(streamRpc(\"query.findMany\", request), User::fromJsonNode);"
            ),
        "expected generated Java async delegate to stream chunked findMany rows:\n{async_delegate}"
    );
    assert!(
        rpc_caller.contains("Stream<JsonNode> streamRpc(String method, ObjectNode params);"),
        "expected Java RpcCaller to expose streamRpc():\n{rpc_caller}"
    );
    assert!(
        dsl.contains("public ObjectNode whereNode()")
            && dsl.contains("values.add(orderBy.node());"),
        "expected Java FindManyArgs to expose whereNode() and serialize orderBy as an array:\n{dsl}"
    );
    assert!(
        base_client
            .contains("private final Map<Long, StreamState> streams = new ConcurrentHashMap<>();")
            && base_client.contains("request.put(\"method\", \"request.cancel\");")
            && base_client.contains(
                "return StreamSupport.stream(spliterator, false).onClose(cursor::close);"
            ),
        "expected Java runtime to stream chunked responses and cancel early closes:\n{base_client}"
    );
    assert!(
        base_tx_client.contains("return this.parent.streamRpc(method, actual);"),
        "expected transaction clients to forward streamRpc() through the parent client:\n{base_tx_client}"
    );
}
