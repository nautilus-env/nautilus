use nautilus_codegen::python::python_runtime_files;
use nautilus_schema::validate_schema_source;

/// Keep baseline identities independent of the feature module containing a test.
macro_rules! assert_codegen_snapshot {
    ($name:literal, $value:expr $(,)?) => {{
        let snapshot_value = $value.replace("\r\n", "\n");
        assert!(
            !snapshot_value.is_empty(),
            "generated snapshot content should not be empty"
        );
        let mut settings = insta::Settings::clone_current();
        settings.set_snapshot_path(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/snapshots"));
        settings.set_prepend_module_to_snapshot(false);
        settings.bind(|| {
            insta::assert_snapshot!(concat!("snapshot_tests__", $name), snapshot_value);
        });
    }};
}

pub(super) use assert_codegen_snapshot;

pub(super) fn validate(source: &str) -> nautilus_schema::ir::SchemaIr {
    validate_schema_source(source)
        .expect("validation failed")
        .ir
}

pub(super) fn generated_java_file<'a>(files: &'a [(String, String)], suffix: &str) -> &'a str {
    files
        .iter()
        .find(|(path, _)| path.ends_with(suffix))
        .map(|(_, code)| code.as_str())
        .unwrap_or_else(|| panic!("missing generated Java file ending with '{suffix}'"))
}

pub(super) fn generated_python_file<'a>(files: &'a [(String, String)], file_name: &str) -> &'a str {
    files
        .iter()
        .find(|(path, _)| path == file_name)
        .map(|(_, code)| code.as_str())
        .unwrap_or_else(|| panic!("missing generated Python file '{file_name}'"))
}

/// The runtime module holding the codec rules every generated model shares.
pub(super) fn python_runtime_codec() -> String {
    python_runtime_files()
        .into_iter()
        .find(|(name, _)| name == "_codec.py")
        .map(|(_, code)| code)
        .expect("missing Python runtime file '_codec.py'")
}

pub(super) fn generated_named_file<'a>(files: &'a [(String, String)], file_name: &str) -> &'a str {
    files
        .iter()
        .find(|(path, _)| path == file_name)
        .map(|(_, code)| code.as_str())
        .unwrap_or_else(|| panic!("missing generated file '{file_name}'"))
}

pub(super) fn section_until<'a>(code: &'a str, start_marker: &str, end_marker: &str) -> &'a str {
    let start = code
        .find(start_marker)
        .unwrap_or_else(|| panic!("missing section start '{start_marker}'"));
    let rest = &code[start..];
    let end = rest.find(end_marker).unwrap_or(rest.len());
    &rest[..end]
}
