use super::support::{
    assert_codegen_snapshot, generated_java_file, generated_named_file, generated_python_file,
    validate,
};
use nautilus_codegen::generator::generate_all_models;
use nautilus_codegen::java::generate_java_client;
use nautilus_codegen::js::generate_all_js_models;
use nautilus_codegen::python::generate_all_python_models;

const JAVA_CLIENT_SCHEMA: &str = include_str!("../fixtures/schemas/java_client.nautilus");
const USER_MAPPED_SCHEMA: &str = include_str!("../fixtures/schemas/user_mapped.nautilus");
const USER_SCHEMA: &str = include_str!("../fixtures/schemas/user.nautilus");

#[test]
fn test_rust_generates_find_many_builder() {
    let ir = validate(USER_SCHEMA);
    let models = generate_all_models(&ir, false).expect("generate_all_models should succeed");
    let code = models.get("User").expect("User model missing");
    assert!(
        code.contains("FindMany"),
        "expected FindMany builder:\n{code}"
    );
    assert_codegen_snapshot!("rust_generates_find_many_builder", code);
}

#[test]
fn test_rust_generates_count_and_group_by_api() {
    let ir = validate(
        r#"
enum Role {
  ADMIN
  MEMBER
}

model User {
  id          Int    @id @default(autoincrement()) @map("user_id")
  displayName String @map("display_name")
  role        Role
  views       Int

  @@map("users")
}
"#,
    );
    let models = generate_all_models(&ir, false).expect("generate_all_models should succeed");
    let code = models.get("User").expect("User model missing");

    assert!(
        code.contains("pub struct UserCountArgs"),
        "expected generated Rust code to expose count args:\n{code}"
    );
    assert!(
        code.contains("pub fn count("),
        "expected generated Rust code to expose count():\n{code}"
    );
    assert!(
        code.contains("pub fn group_by("),
        "expected generated Rust code to expose group_by():\n{code}"
    );
    assert!(
        code.contains("pub enum UserScalarField"),
        "expected generated Rust code to expose scalar field enums for group_by():\n{code}"
    );
    assert!(
        code.contains("Self::DisplayName => \"displayName\""),
        "expected mapped fields to serialize through logical names in aggregate APIs:\n{code}"
    );
    assert!(
        code.contains("pub struct UserGroupByOutput"),
        "expected generated Rust code to expose a typed group_by output:\n{code}"
    );
}

#[test]
fn test_rust_generated_query_builders_use_static_column_markers() {
    let ir = validate(
        r#"
model User {
  id    Int    @id @default(autoincrement())
  email String @unique
  name  String?

  @@map("users")
}
"#,
    );
    let models = generate_all_models(&ir, false).expect("generate_all_models should succeed");
    let code = models.get("User").expect("User model missing");

    assert!(
        code.contains("ColumnMarker::from_static(\"users\", \"email\")"),
        "expected generated Rust code to use borrowed column metadata for known columns:\n{code}"
    );
    assert!(
        code.contains("ColumnMarker::from_static(\"users\", \"id\")"),
        "expected generated Rust code to reuse borrowed PK metadata in returning/select paths:\n{code}"
    );
}

#[test]
fn test_python_select_input_supports_projection_safe_models() {
    let ir = validate(USER_MAPPED_SCHEMA);
    let models = generate_all_python_models(&ir, false, 0)
        .expect("generate_all_python_models should succeed");
    let (_, code) = models
        .iter()
        .find(|(name, _)| name == "user.py")
        .expect("user model missing");

    assert!(
        code.contains("display_name: str"),
        "expected generated Python models to keep required schema fields required:\n{code}"
    );
    assert!(
        code.contains("class UserProjection(TypedDict, total=False):")
            && code.contains("display_name: NotRequired[str]"),
        "expected generated Python projection type to allow missing selected fields:\n{code}"
    );
    assert!(
        code.contains("class UserSelectInput(TypedDict, total=False):"),
        "expected a typed UserSelectInput to be generated:\n{code}"
    );
    assert!(
        code.contains("display_name: NotRequired[bool]"),
        "expected select input to expose the Python model field name:\n{code}"
    );
    assert!(
        code.contains("\"display_name\": \"displayName\""),
        "expected select serialization to map Python field names back to logical names:\n{code}"
    );
    assert!(
        code.contains("args[\"select\"] = _process_select_fields(select, _users_py_to_logical)"),
        "expected find_many() to forward select through the logical-name serializer:\n{code}"
    );
}

#[test]
fn test_python_single_row_finds_use_dedicated_engine_methods() {
    let ir = validate(USER_SCHEMA);
    let py_models = generate_all_python_models(&ir, false, 0)
        .expect("generate_all_python_models should succeed");
    let py_model = generated_python_file(&py_models, "user.py");

    assert!(
        py_model.contains(r#""query.findFirst", payload"#),
        "expected generated Python find_first() to call query.findFirst:\n{py_model}"
    );
    assert!(
        py_model.contains("from .._internal.protocol import PROTOCOL_VERSION")
            && py_model.contains("\"protocolVersion\": PROTOCOL_VERSION"),
        "expected generated Python delegates to reuse the shared protocol version constant:\n{py_model}"
    );
    assert!(
        py_model.contains(r#""query.findUnique", payload"#),
        "expected generated Python find_unique() to call query.findUnique when possible:\n{py_model}"
    );
    assert!(
        py_model.contains("if select is not None or include is not None:"),
        "expected generated Python find_unique() to fall back to the single-row projection path when select/include are used:\n{py_model}"
    );
    assert!(
        !py_model.contains("rows = self.find_many(where=where, order_by=order_by, take=1, select=select, include=include)"),
        "generated Python find_first() should no longer delegate to find_many():\n{py_model}"
    );
    assert!(
        !py_model
            .contains("rows = self.find_many(where=where, take=1, select=select, include=include)"),
        "generated Python find_unique() should no longer delegate to find_many():\n{py_model}"
    );
}

#[test]
fn test_js_select_input_supports_projection_safe_models() {
    let ir = validate(USER_MAPPED_SCHEMA);
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
    assert_codegen_snapshot!("js_user_mapped", js_code);
    assert_codegen_snapshot!("js_user_mapped_declarations", dts_code);

    assert!(
        dts_code.contains("displayName: string;"),
        "expected generated JS models to keep required schema fields required:\n{dts_code}"
    );
    assert!(
        dts_code.contains("export type UserSelected<S extends UserSelectInput>"),
        "expected generated JS declarations to expose select result mapped types:\n{dts_code}"
    );
    assert!(
        dts_code.contains("export interface UserSelectInput {"),
        "expected a typed UserSelectInput to be generated:\n{dts_code}"
    );
    assert!(
        dts_code.contains("displayName?: boolean;"),
        "expected select input to expose logical field names:\n{dts_code}"
    );
    assert!(
        dts_code.contains("select?:   UserSelectInput;"),
        "expected select to be exposed on generated query methods:\n{dts_code}"
    );
    assert!(
        js_code.contains("if (args?.select   != null) rpcArgs['select']  = args.select;"),
        "expected runtime delegate to forward select to the engine:\n{js_code}"
    );
}

#[test]
fn test_js_single_row_finds_use_dedicated_engine_methods() {
    let ir = validate(USER_SCHEMA);
    let (js_models, _dts_models) =
        generate_all_js_models(&ir).expect("generate_all_js_models should succeed");
    let js_model = generated_named_file(&js_models, "user.js");

    assert!(
        js_model.contains("this.client._rpc('query.findFirst', request)"),
        "expected generated JS findFirst() to call query.findFirst:\n{js_model}"
    );
    assert!(
        js_model.contains("import { PROTOCOL_VERSION } from '../_internal/_protocol.js';")
            && js_model.contains("protocolVersion: PROTOCOL_VERSION"),
        "expected generated JS delegates to reuse the shared protocol version constant:\n{js_model}"
    );
    assert!(
        js_model.contains("this.client._rpc('query.findUnique', request)"),
        "expected generated JS findUnique() to call query.findUnique when possible:\n{js_model}"
    );
    assert!(
        js_model.contains("if (args.select != null || args.include != null)"),
        "expected generated JS findUnique() to fall back to the single-row projection path when select/include are used:\n{js_model}"
    );
    assert!(
        !js_model.contains("const rows = await this.findMany({ where: args?.where, orderBy: args?.orderBy, take: 1, select: args?.select, include: args?.include });"),
        "generated JS findFirst() should no longer delegate to findMany():\n{js_model}"
    );
    assert!(
        !js_model.contains("const rows = await this.findMany({ where: args.where, take: 1, select: args.select, include: args.include });"),
        "generated JS findUnique() should no longer delegate to findMany():\n{js_model}"
    );
}

#[test]
fn test_java_single_row_finds_use_dedicated_engine_methods() {
    let ir = validate(JAVA_CLIENT_SCHEMA);
    let java_files =
        generate_java_client(&ir, "schema.nautilus", false).expect("generate_java_client failed");
    let delegate = generated_java_file(&java_files, "client/UserDelegate.java");

    let base_delegate = generated_java_file(&java_files, "internal/AbstractDelegate.java");

    assert!(
        delegate
            .contains("JsonNode result = rpc(\"query.findFirst\", singleRowRequest(argsNode));"),
        "expected generated Java findFirst() to call query.findFirst:\n{delegate}"
    );
    assert!(
        base_delegate.contains("request.put(\"protocolVersion\", JsonSupport.PROTOCOL_VERSION);"),
        "expected the Java base delegate to reuse the shared protocol version constant:\n{base_delegate}"
    );
    assert!(
        delegate.contains("JsonNode result = rpc(\"query.findUnique\", request);"),
        "expected generated Java findUnique() to call query.findUnique when possible:\n{delegate}"
    );
    assert!(
        delegate.contains("if (argsNode.size() == 1 && argsNode.has(\"where\"))"),
        "expected generated Java findUnique() to gate the unique-only fast path conservatively:\n{delegate}"
    );
    assert!(
        !delegate.contains("return findFirst(spec);"),
        "generated Java findUnique() should no longer alias directly to findFirst():\n{delegate}"
    );
}

#[test]
fn test_java_select_uses_projection_api_instead_of_model_records() {
    let ir = validate(JAVA_CLIENT_SCHEMA);
    let sync_files =
        generate_java_client(&ir, "schema.nautilus", false).expect("generate_java_client failed");
    let async_files =
        generate_java_client(&ir, "schema.nautilus", true).expect("generate_java_client failed");
    let sync_delegate = generated_java_file(&sync_files, "client/UserDelegate.java");
    let async_delegate = generated_java_file(&async_files, "client/UserDelegate.java");
    let projection = generated_java_file(&sync_files, "model/UserProjection.java");

    assert!(
        sync_delegate.contains("public List<UserProjection> findManySelect(")
            && sync_delegate.contains("public <R> List<R> findManySelect(")
            && sync_delegate.contains("public UserProjection findFirstSelect(")
            && sync_delegate.contains("public UserProjection findUniqueSelect(")
            && sync_delegate.contains("public Stream<UserProjection> streamManySelect(")
            && sync_delegate.contains("public List<JsonNode> findManySelectRaw("),
        "expected generated Java sync delegate to expose typed projection APIs and raw escape hatches:\n{sync_delegate}"
    );
    assert!(
        async_delegate.contains("public CompletableFuture<List<UserProjection>> findManySelect(")
            && async_delegate.contains("public <R> CompletableFuture<List<R>> findManySelect(")
            && async_delegate.contains("public CompletableFuture<UserProjection> findFirstSelect(")
            && async_delegate.contains("public CompletableFuture<UserProjection> findUniqueSelect(")
            && async_delegate.contains("public CompletableFuture<List<JsonNode>> findManySelectRaw("),
        "expected generated Java async delegate to expose CompletableFuture projection APIs:\n{async_delegate}"
    );
    let base_delegate = generated_java_file(&sync_files, "internal/AbstractDelegate.java");
    assert!(
        base_delegate.contains("\"select returns partial rows and cannot be decoded as a full \"")
            && base_delegate.contains(
                "\" record; use findManySelect, findFirstSelect, or findUniqueSelect instead\""
            )
            && base_delegate.contains(
                "\"select projection APIs require select(...); use findMany/findFirst/findUnique for full \""
            ),
        "expected the Java base delegate to own the select guards:\n{base_delegate}"
    );
    assert!(
        sync_delegate.contains("rejectSelect(argsNode);")
            && sync_delegate.contains("requireSelect(argsNode);"),
        "expected generated Java model and projection APIs to apply the select guards:\n{sync_delegate}"
    );
    assert!(
        sync_delegate.contains("return rows(result, row -> actualMapper.apply(UserProjection.fromJsonNode(row)));")
            && sync_delegate.contains(
                "return mapRow(JsonSupport.firstDataRow(result), UserProjection::fromJsonNode, mapper);"
            ),
        "expected generated Java projection APIs to return typed projection rows or mapped values:\n{sync_delegate}"
    );
    assert!(
        projection.contains("public final class UserProjection implements WireSerializable")
            && projection.contains("public boolean hasId()")
            && projection.contains("public Integer id()")
            && projection.contains("public boolean hasName()")
            && projection.contains("public String name()")
            && projection.contains("return this.row.deepCopy();"),
        "expected generated Java projection class to expose typed getters plus presence checks:\n{projection}"
    );
}
