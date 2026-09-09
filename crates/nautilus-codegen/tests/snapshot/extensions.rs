use super::support::{
    generated_java_file, generated_named_file, generated_python_file, python_runtime_codec,
    section_until, validate,
};
use nautilus_codegen::extension_types::{
    generate_java_extension_files, generate_js_extension_files, generate_python_extension_files,
    generate_rust_extension_files, ExtensionRegistry,
};
use nautilus_codegen::java::generate_java_client;
use nautilus_codegen::js::generate_all_js_models;
use nautilus_codegen::python::generate_all_python_models;

#[test]
fn test_generated_hstore_filters_are_typed_in_js_and_python() {
    let ir = validate(
        r#"
datasource db {
  provider   = "postgresql"
  url        = env("DATABASE_URL")
  extensions = [hstore]
}

model User {
  id   Int     @id @default(autoincrement())
  meta Hstore?
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
    assert!(js_dts.contains("export interface HstoreFilter {"));
    assert!(js_dts.contains("export type HstoreValue = Record<string, string | null>;"));
    // With the `hstore` extension declared, filter inputs accept the generated
    // wrapper or the raw `HstoreValue` payload via the `HstoreInput` union.
    assert!(js_dts.contains("equals?: HstoreInput;"));
    assert!(js_dts.contains("not?:    HstoreInput;"));
    assert!(js_dts.contains("isNull?: boolean;"));
    assert!(js_dts.contains("meta?: HstoreInput | HstoreFilter | null;"));

    let py_models = generate_all_python_models(&ir, false, 1)
        .expect("generate_all_python_models should succeed");
    let py_model = generated_python_file(&py_models, "user.py");
    assert!(py_model.contains("from .._internal.codec import HstoreValue,"));
    assert!(python_runtime_codec().contains("HstoreValue = Dict[str, Optional[str]]"));
    assert!(py_model.contains("class HstoreFilter(TypedDict, total=False):"));
    // With the `hstore` extension declared the filter accepts the wrapper too.
    assert!(py_model.contains("equals: NotRequired[HstoreInput]"));
    assert!(py_model.contains("not_: NotRequired[HstoreInput]"));
    assert!(py_model.contains("is_null: NotRequired[bool]"));
    assert!(py_model.contains("meta: NotRequired[Union[HstoreInput, HstoreFilter, None]]"));
}

#[test]
fn test_extension_input_builders_are_generated_across_codegens() {
    let ir = validate(
        r#"
datasource db {
  provider   = "postgresql"
  url        = env("DATABASE_URL")
  extensions = [citext, hstore, ltree, postgis, vector]
}

model Example {
  id          Int        @id @default(autoincrement())
  email       Citext
  path        Ltree?
  meta        Hstore?
  footprint   Geometry?
  serviceArea Geography?
  embedding   Vector(3)
}
"#,
    );

    let extensions = ExtensionRegistry::from_schema(&ir);

    let py_ext_files = generate_python_extension_files(&extensions)
        .expect("generate_python_extension_files should succeed");
    let citext_py = generated_named_file(&py_ext_files, "citext/types.py");
    let hstore_py = generated_named_file(&py_ext_files, "hstore/types.py");
    let ltree_py = generated_named_file(&py_ext_files, "ltree/types.py");
    let postgis_py = generated_named_file(&py_ext_files, "postgis/types.py");
    let vector_py = generated_named_file(&py_ext_files, "vector/types.py");
    assert!(citext_py.contains("CitextInput = Union[\"Citext\", str, CitextBuilderInput]"));
    assert!(citext_py.contains("class CitextValueInput(TypedDict):"));
    assert!(ltree_py.contains("LtreeInput = Union[\"Ltree\", str, LtreeBuilderInput]"));
    assert!(hstore_py
        .contains("HstoreInput = Union[\"Hstore\", HstoreSource, HstoreEntriesBuilderInput]"));
    assert!(hstore_py.contains("class HstoreEntriesBuilderInput(TypedDict):"));
    assert!(postgis_py.contains("class GeometryPointInput(TypedDict, total=False):"));
    assert!(postgis_py.contains("class GeographyPointInput(TypedDict, total=False):"));
    assert!(postgis_py.contains("GeometryInput = Union[\"Geometry\", str, GeometryBuilderInput]"));
    assert!(
        postgis_py.contains("GeographyInput = Union[\"Geography\", str, GeographyBuilderInput]")
    );
    assert!(vector_py.contains("class VectorValuesInput(TypedDict):"));
    assert!(vector_py.contains("VectorInput = Union[\"Vector\", VectorSource, VectorValuesInput]"));

    let (_, js_ext_dts) = generate_js_extension_files(&extensions)
        .expect("generate_js_extension_files should succeed");
    let citext_dts = generated_named_file(&js_ext_dts, "extensions/citext/types.d.ts");
    let hstore_dts = generated_named_file(&js_ext_dts, "extensions/hstore/types.d.ts");
    let ltree_dts = generated_named_file(&js_ext_dts, "extensions/ltree/types.d.ts");
    let postgis_dts = generated_named_file(&js_ext_dts, "extensions/postgis/types.d.ts");
    let vector_dts = generated_named_file(&js_ext_dts, "extensions/vector/types.d.ts");
    assert!(citext_dts.contains("export interface CitextValueInput {"));
    assert!(citext_dts.contains("export type CitextInput = Citext | string | CitextBuilderInput;"));
    assert!(ltree_dts.contains("export type LtreeInput = Ltree | string | LtreeBuilderInput;"));
    assert!(hstore_dts.contains("export interface HstoreEntriesBuilderInput {"));
    assert!(hstore_dts.contains("export type HstoreInput = Hstore | HstoreBuilderInput;"));
    assert!(postgis_dts.contains("export interface GeometryPointInput {"));
    assert!(postgis_dts.contains("export interface GeographyPointInput {"));
    assert!(postgis_dts
        .contains("export type GeometryInput = Geometry | string | GeometryBuilderInput;"));
    assert!(postgis_dts
        .contains("export type GeographyInput = Geography | string | GeographyBuilderInput;"));
    assert!(vector_dts.contains("export interface VectorValuesInput {"));
    assert!(vector_dts.contains("export type VectorInput = Vector | VectorBuilderInput;"));

    let py_models = generate_all_python_models(&ir, false, 1)
        .expect("generate_all_python_models should succeed");
    let py_model = generated_python_file(&py_models, "example.py");
    assert!(py_model.contains("email: Required[CitextInput]"));
    assert!(py_model.contains("path: NotRequired[Optional[LtreeInput]]"));
    assert!(py_model.contains("meta: NotRequired[Optional[HstoreInput]]"));
    assert!(py_model.contains("footprint: NotRequired[Optional[GeometryInput]]"));
    assert!(py_model.contains("serviceArea: NotRequired[Optional[GeographyInput]]"));
    assert!(py_model.contains("embedding: Required[VectorInput]"));
    assert!(py_model.contains("footprint: NotRequired[Union[GeometryInput, StringFilter, None]]"));
    assert!(
        py_model.contains("serviceArea: NotRequired[Union[GeographyInput, StringFilter, None]]")
    );
    assert!(py_model.contains("embedding: NotRequired[Union[VectorInput, VectorFilter]]"));

    let (_, js_models) =
        generate_all_js_models(&ir).expect("generate_all_js_models should succeed");
    let js_model = js_models
        .iter()
        .find(|(name, _)| name == "example.d.ts")
        .map(|(_, code)| code.as_str())
        .expect("example.d.ts missing");
    assert!(js_model.contains("email: CitextInput;"));
    assert!(js_model.contains("path?: LtreeInput | null;"));
    assert!(js_model.contains("meta?: HstoreInput | null;"));
    assert!(js_model.contains("footprint?: GeometryInput | null;"));
    assert!(js_model.contains("serviceArea?: GeographyInput | null;"));
    assert!(js_model.contains("embedding: VectorInput;"));
    assert!(js_model.contains("footprint?: GeometryInput | StringFilter | null;"));
    assert!(js_model.contains("serviceArea?: GeographyInput | StringFilter | null;"));
    assert!(js_model.contains("embedding?: VectorInput | VectorFilter;"));

    let java_ext_files = generate_java_extension_files(&extensions, "com.acme.db")
        .expect("generate_java_extension_files should succeed");
    let geometry_java = generated_java_file(&java_ext_files, "Geometry.java");
    let geography_java = generated_java_file(&java_ext_files, "Geography.java");
    let hstore_java = generated_java_file(&java_ext_files, "Hstore.java");
    let vector_java = generated_java_file(&java_ext_files, "Vector.java");
    let citext_java = generated_java_file(&java_ext_files, "Citext.java");
    let ltree_java = generated_java_file(&java_ext_files, "Ltree.java");
    assert!(citext_java.contains("public static Citext of(String value)"));
    assert!(ltree_java.contains("public static Ltree of(String value)"));
    assert!(geometry_java.contains("public static Geometry point(double x, double y)"));
    assert!(geography_java.contains("public static Geography point(double lon, double lat)"));
    assert!(hstore_java
        .contains("public static Hstore ofEntries(Map.Entry<String, String>... entries)"));
    assert!(vector_java.contains("public static Vector of(double... values)"));

    let rust_ext_files = generate_rust_extension_files(&extensions)
        .expect("generate_rust_extension_files should succeed");
    let postgis_rust = generated_named_file(&rust_ext_files, "extensions/postgis/types.rs");
    let hstore_rust = generated_named_file(&rust_ext_files, "extensions/hstore/types.rs");
    let vector_rust = generated_named_file(&rust_ext_files, "extensions/vector/types.rs");
    let citext_rust = generated_named_file(&rust_ext_files, "extensions/citext/types.rs");
    let ltree_rust = generated_named_file(&rust_ext_files, "extensions/ltree/types.rs");
    assert!(citext_rust.contains("pub fn of(value: impl Into<String>) -> Self"));
    assert!(ltree_rust.contains("pub fn of(value: impl Into<String>) -> Self"));
    assert!(postgis_rust.contains("impl Geometry {"));
    assert!(postgis_rust
        .contains("pub fn point(x: impl std::fmt::Display, y: impl std::fmt::Display) -> Self"));
    assert!(postgis_rust.contains("impl Geography {"));
    assert!(postgis_rust.contains(
        "pub fn point(lon: impl std::fmt::Display, lat: impl std::fmt::Display) -> Self"
    ));
    assert!(hstore_rust.contains("pub fn from_entries<K, V, I>(entries: I) -> Self"));
    assert!(vector_rust.contains("pub fn of<I, N>(values: I) -> Self"));
}

#[test]
fn test_generated_java_hstore_uses_runtime_type_that_preserves_null_values() {
    let ir = validate(
        r#"
datasource db {
  provider   = "postgresql"
  url        = env("DATABASE_URL")
  extensions = [hstore]
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
  id   Int     @id @default(autoincrement())
  meta Hstore?
}
"#,
    );

    let java_files =
        generate_java_client(&ir, "schema.nautilus", false).expect("generate_java_client failed");
    let user_model = generated_java_file(&java_files, "model/User.java");
    let json_support = generated_java_file(&java_files, "internal/JsonSupport.java");

    // With the `hstore` extension declared the model field uses the generated
    // `Hstore` wrapper class (which itself wraps `JsonSupport.Hstore` to
    // preserve null-aware key/value semantics on the wire).
    assert!(user_model.contains("Hstore meta"));
    assert!(user_model.contains("import com.acme.db.extensions.hstore.types.Hstore;"));
    assert!(json_support
        .contains("public static final class Hstore extends LinkedHashMap<String, String>"));
    assert!(json_support.contains("public static Hstore asHstore(JsonNode node)"));
}

#[test]
fn test_js_client_calls_the_array_extension_coercer() {
    let ir = validate(
        r#"
datasource db {
  provider   = "postgresql"
  url        = env("DATABASE_URL")
  extensions = [citext]
}

model Doc {
  id   Int      @id @default(autoincrement())
  tags Citext[]
}
"#,
    );

    let (models, _) = generate_all_js_models(&ir).expect("generate_all_js_models should succeed");
    let code = generated_named_file(&models, "doc.js");

    assert!(
        code.contains(
            "coerced = ((value) => Array.isArray(value) ? value.map(item => Citext.from(item)) : value)(value);"
        ),
        "the array coercer must be applied to the value, not assigned as a function:\n{code}"
    );
    assert!(
        !code.contains("coerced = (value) =>"),
        "assigning the coercer itself leaves a function on the model, which JSON.stringify \
         silently drops:\n{code}"
    );
}

#[test]
fn test_generated_clients_write_null_into_nullable_extension_columns() {
    let ir = validate(
        r#"
datasource db {
  provider   = "postgresql"
  url        = env("DATABASE_URL")
  extensions = [citext]
}

model Doc {
  id      Int     @id @default(autoincrement())
  altSlug Citext?
}
"#,
    );

    // Both the write path and the filter path need the guard, so pin each one
    // inside its own function: asserting on the whole file would let either
    // regress while the other kept the assertion green.
    let (js_models, _) =
        generate_all_js_models(&ir).expect("generate_all_js_models should succeed");
    let js = generated_named_file(&js_models, "doc.js");
    for function in [
        "function _serializeScalarInput",
        "function _serializeFilterInput",
    ] {
        let body = section_until(js, function, "\n}");
        assert!(
            body.contains("if (value == null || !serializer) return _toWireValue(value);"),
            "a null must bypass the extension coercer in {function}, which only knows how to \
             build a value:\n{body}"
        );
    }

    let py_models = generate_all_python_models(&ir, true, 0)
        .expect("generate_all_python_models should succeed");
    let py = generated_python_file(&py_models, "doc.py");
    assert!(
        py.contains("_serialize_scalar_input = _Doc_input_codec.scalar_input")
            && py.contains("_serialize_filter_input = _Doc_input_codec.filter_input"),
        "the model must serialize its inputs through the shared codec:\n{py}"
    );
    let runtime = python_runtime_codec();
    for method in ["def scalar_input", "def filter_input"] {
        let body = section_until(&runtime, method, "\n\n    def ");
        assert!(
            body.contains("if value is None or serializer is None:"),
            "the Python client must bypass the extension coercer for None in {method}:\n{body}"
        );
    }
}

#[test]
fn test_rust_citext_wrapper_tags_values_with_their_type() {
    let ir = validate(
        r#"
datasource db {
  provider   = "postgresql"
  url        = env("DATABASE_URL")
  extensions = [citext]
}

model Doc {
  id   Int    @id @default(autoincrement())
  slug Citext
}
"#,
    );

    let files = generate_rust_extension_files(&ExtensionRegistry::from_schema(&ir))
        .expect("generate_rust_extension_files should succeed");
    let citext = generated_named_file(&files, "extensions/citext/types.rs");

    assert!(
        citext.contains("nautilus_core::Value::Extension { value: value.into_inner(), type_name: \"citext\".to_string() }"),
        "a citext must carry its type name so the dialect can emit `$1::citext`; without the \
         cast PostgreSQL compares a citext column case sensitively:\n{citext}"
    );
}
