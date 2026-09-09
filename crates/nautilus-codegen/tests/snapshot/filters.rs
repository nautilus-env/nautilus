use super::support::{generated_java_file, generated_python_file, python_runtime_codec, validate};
use nautilus_codegen::generator::generate_all_models;
use nautilus_codegen::java::generate_java_client;
use nautilus_codegen::js::generate_all_js_models;
use nautilus_codegen::python::generate_all_python_models;

#[test]
fn test_generated_clients_exclude_non_orderable_fields_from_order_by() {
    let ir = validate(
        r#"
datasource db {
  provider   = "postgresql"
  url        = env("DATABASE_URL")
  extensions = [hstore, vector]
}

generator client {
  provider    = "nautilus-client-java"
  output      = "./generated-java"
  package     = "com.acme.db"
  group_id    = "com.acme"
  artifact_id = "db-client"
  interface   = "sync"
}

model User {
  id      Int      @id @default(autoincrement())
  title   String
  active  Boolean
  meta    Hstore?
  payload Json?
  embedding Vector(3)
}
"#,
    );

    let (_, dts_models) =
        generate_all_js_models(&ir).expect("generate_all_js_models should succeed");
    let js_dts = dts_models
        .iter()
        .find(|(name, _)| name == "user.d.ts")
        .map(|(_, code)| code.as_str())
        .expect("user declaration missing");
    assert!(js_dts.contains("title?: SortOrder;"));
    assert!(!js_dts.contains("active?: SortOrder;"));
    assert!(!js_dts.contains("meta?: SortOrder;"));
    assert!(!js_dts.contains("payload?: SortOrder;"));
    assert!(!js_dts.contains("embedding?: SortOrder;"));

    let py_models = generate_all_python_models(&ir, false, 1)
        .expect("generate_all_python_models should succeed");
    let py_model = generated_python_file(&py_models, "user.py");
    assert!(py_model.contains("title: NotRequired[Literal[\"asc\", \"desc\"]]"));
    assert!(!py_model.contains("active: NotRequired[Literal[\"asc\", \"desc\"]]"));
    assert!(!py_model.contains("meta: NotRequired[Literal[\"asc\", \"desc\"]]"));
    assert!(!py_model.contains("payload: NotRequired[Literal[\"asc\", \"desc\"]]"));
    assert!(!py_model.contains("embedding: NotRequired[Literal[\"asc\", \"desc\"]]"));

    let java_files =
        generate_java_client(&ir, "schema.nautilus", false).expect("generate_java_client failed");
    let user_dsl = generated_java_file(&java_files, "dsl/UserDsl.java");
    assert!(user_dsl.contains("public OrderBy title(SortOrder order)"));
    assert!(!user_dsl.contains("public OrderBy active(SortOrder order)"));
    assert!(!user_dsl.contains("public OrderBy meta(SortOrder order)"));
    assert!(!user_dsl.contains("public OrderBy payload(SortOrder order)"));
    assert!(!user_dsl.contains("public OrderBy embedding(SortOrder order)"));
}

#[test]
fn test_generated_clients_type_composite_field_order_by_paths() {
    let ir = validate(
        r#"
datasource db {
  provider = "sqlite"
  url      = "sqlite::memory:"
}

generator client {
  provider    = "nautilus-client-java"
  output      = "./generated-java"
  package     = "com.acme.db"
  group_id    = "com.acme"
  artifact_id = "db-client"
  interface   = "sync"
}

type DeliveryEstimate {
  etaMinutes      Int @map("eta_minutes_db")
  weekendDelivery Boolean
  carrierMetadata Json
}

model Shipment {
  id           Int              @id @default(autoincrement())
  trackingCode String           @unique
  delivery     DeliveryEstimate @store(json)
}
"#,
    );

    let rust_models = generate_all_models(&ir, false).expect("generate_all_models should succeed");
    let rust_shipment = rust_models.get("Shipment").expect("Shipment model missing");
    assert!(rust_shipment
        .contains("pub fn delivery_eta_minutes(&self) -> nautilus_core::OrderField<i32>"));
    assert!(rust_shipment
        .contains("nautilus_core::OrderField::new(\"Shipment\", \"delivery.etaMinutes\")"));
    assert!(rust_shipment.contains("nautilus_core::JsonPathCast::Signed"));
    assert!(!rust_shipment
        .contains("pub fn delivery_weekend_delivery(&self) -> nautilus_core::OrderField"));
    assert!(!rust_shipment
        .contains("pub fn delivery_carrier_metadata(&self) -> nautilus_core::OrderField"));

    let (_, dts_models) =
        generate_all_js_models(&ir).expect("generate_all_js_models should succeed");
    let js_dts = dts_models
        .iter()
        .find(|(name, _)| name == "shipment.d.ts")
        .map(|(_, code)| code.as_str())
        .expect("shipment declaration missing");
    assert!(js_dts.contains("'delivery.etaMinutes'?: SortOrder;"));
    assert!(!js_dts.contains("delivery?: SortOrder;"));
    assert!(!js_dts.contains("'delivery.weekendDelivery'?: SortOrder;"));
    assert!(!js_dts.contains("'delivery.carrierMetadata'?: SortOrder;"));

    let py_models = generate_all_python_models(&ir, false, 1)
        .expect("generate_all_python_models should succeed");
    let py_model = generated_python_file(&py_models, "shipment.py");
    assert!(py_model.contains("ShipmentOrderByInput = TypedDict("));
    assert!(py_model.contains("\"delivery.etaMinutes\": NotRequired[Literal[\"asc\", \"desc\"]]"));
    assert!(!py_model.contains("delivery: NotRequired[Literal[\"asc\", \"desc\"]]"));
    assert!(
        !py_model.contains("\"delivery.weekendDelivery\": NotRequired[Literal[\"asc\", \"desc\"]]")
    );
    assert!(
        !py_model.contains("\"delivery.carrierMetadata\": NotRequired[Literal[\"asc\", \"desc\"]]")
    );

    let java_files =
        generate_java_client(&ir, "schema.nautilus", false).expect("generate_java_client failed");
    let shipment_dsl = generated_java_file(&java_files, "dsl/ShipmentDsl.java");
    assert!(shipment_dsl.contains("public OrderBy deliveryEtaMinutes(SortOrder order)"));
    assert!(shipment_dsl.contains("this.node.put(\"delivery.etaMinutes\", order.wireValue());"));
    assert!(!shipment_dsl.contains("public OrderBy delivery(SortOrder order)"));
    assert!(!shipment_dsl.contains("public OrderBy deliveryWeekendDelivery(SortOrder order)"));
    assert!(!shipment_dsl.contains("public OrderBy deliveryCarrierMetadata(SortOrder order)"));
}

#[test]
fn test_python_filter_operator_names_are_normalized_for_engine() {
    let ir = validate(
        r#"
model User {
  id    Int     @id @default(autoincrement())
  title String?
}
"#,
    );

    let py_models = generate_all_python_models(&ir, false, 1)
        .expect("generate_all_python_models should succeed");
    let py_model = generated_python_file(&py_models, "user.py");
    assert!(py_model.contains("_process_where_filters = _User_input_codec.where_filters"));

    let runtime = python_runtime_codec();
    assert!(runtime.contains("\"in_\": \"in\""));
    assert!(runtime.contains("\"not_\": \"not\""));
    assert!(runtime.contains("\"not_in\": \"notIn\""));
    assert!(runtime.contains("\"startswith\": \"startsWith\""));
    assert!(runtime.contains("\"endswith\": \"endsWith\""));
    assert!(runtime.contains("\"is_null\": \"isNull\""));
}

#[test]
fn test_generated_object_like_where_values_require_explicit_equals_in_js_and_python() {
    let ir = validate(
        r#"
datasource db {
  provider   = "postgresql"
  url        = env("DATABASE_URL")
  extensions = [hstore]
}

model User {
  id      Int    @id @default(autoincrement())
  payload Jsonb?
  meta    Hstore?
}
"#,
    );

    let (js_models, dts_models) =
        generate_all_js_models(&ir).expect("generate_all_js_models should succeed");
    let js_model = js_models
        .iter()
        .find(|(name, _)| name == "user.js")
        .map(|(_, code)| code.as_str())
        .expect("user runtime missing");
    let js_dts = dts_models
        .iter()
        .find(|(name, _)| name == "user.d.ts")
        .map(|(_, code)| code.as_str())
        .expect("user declaration missing");
    assert!(js_model.contains("ObjectValueDbFields = new Set(["));
    assert!(js_model.contains("_objectEqualityRequiresExplicitEquals"));
    assert!(js_model.contains("Use { equals: ... } for object equality filters."));
    assert!(js_model.contains("const actualOp = op === 'equals' ? 'eq' : op;"));
    assert!(js_dts.contains("export type JsonValue = JsonPrimitive | JsonObject | JsonValue[];"));
    assert!(js_dts.contains("export interface JsonFilter {"));
    assert!(js_dts.contains("equals?: JsonValue;"));
    assert!(js_dts.contains("payload?: JsonScalarOrArray | JsonFilter;"));

    let py_models = generate_all_python_models(&ir, false, 1)
        .expect("generate_all_python_models should succeed");
    let py_model = generated_python_file(&py_models, "user.py");
    let py_runtime = python_runtime_codec();
    assert!(py_runtime.contains("JsonValue = Union[JsonPrimitive, Dict[str, Any], List[Any]]"));
    assert!(py_model.contains("_object_value_db_fields: frozenset = frozenset({"));
    assert!(py_model.contains("    _User_object_value_db_fields,\n)"));
    assert!(py_runtime.contains("object_equality_requires_explicit_equals"));
    assert!(py_runtime.contains("Use {'equals': ...} for object equality filters."));
    assert!(py_runtime.contains("\"equals\": \"eq\""));
    assert!(py_model.contains("class JsonFilter(TypedDict, total=False):"));
    assert!(py_model.contains("equals: NotRequired[JsonValue]"));
    assert!(py_model.contains("payload: NotRequired[Union[JsonScalarOrArray, JsonFilter]]"));
}
