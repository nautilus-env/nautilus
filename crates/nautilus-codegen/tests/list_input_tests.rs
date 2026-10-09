//! List columns have a value type of their own, never their element's filters
//! or arithmetic operators. Scalar columns must retain those operations.

use nautilus_codegen::{js::generate_all_js_models, python::generate_all_python_models};
use nautilus_schema::validate_schema_source;

const SCHEMA: &str = include_str!("fixtures/schemas/python_typecheck.nautilus");
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

fn declaration<'a>(code: &'a str, header: &str) -> Vec<&'a str> {
    let mut lines = code.lines().skip_while(|line| !line.starts_with(header));
    let first = lines.next().unwrap_or_else(|| panic!("missing {header}"));
    std::iter::once(first)
        .chain(lines.take_while(|line| !line.starts_with("class ") && !line.starts_with("export ")))
        .collect()
}

#[test]
fn python_lists_are_writable_and_countable_but_have_no_scalar_operations() {
    let ir = validate_schema_source(SCHEMA).unwrap().ir;
    let files = generate_all_python_models(&ir, true, 3).unwrap();
    let code = &files.iter().find(|(name, _)| name == "order.py").unwrap().1;
    for header in [
        "class OrderWhereInput(",
        "class OrderAvgAggregateInput(",
        "class OrderSumAggregateInput(",
        "class OrderMinAggregateInput(",
        "class OrderMaxAggregateInput(",
    ] {
        let section = declaration(code, header);
        for field in LIST_FIELDS {
            assert!(
                section
                    .iter()
                    .all(|line| !line.starts_with(&format!("    {field}:"))),
                "{header} offers {field}"
            );
        }
    }
    let ordering = declaration(code, "OrderOrderByInput = TypedDict(");
    for field in LIST_FIELDS {
        assert!(ordering
            .iter()
            .all(|line| !line.contains(&format!("\"{field}\":"))));
    }
    let update = declaration(code, "class OrderUpdateInput(");
    let create = declaration(code, "class OrderCreateInput(");
    let count = declaration(code, "class OrderCountAggregateInput(");
    for field in LIST_FIELDS {
        let line = update
            .iter()
            .find(|line| line.starts_with(&format!("    {field}:")))
            .unwrap();
        assert!(
            line.contains("List[") && !line.contains("FieldUpdate"),
            "{line}"
        );
        assert!(create
            .iter()
            .any(|line| line.starts_with(&format!("    {field}:"))));
        assert!(count
            .iter()
            .any(|line| line.starts_with(&format!("    {field}:"))));
    }
    assert!(update.contains(&"    weight: NotRequired[Union[Optional[float], FloatFieldUpdate]]"));
}

#[test]
fn javascript_lists_are_writable_and_countable_but_have_no_scalar_operations() {
    let ir = validate_schema_source(SCHEMA).unwrap().ir;
    let (_, files) = generate_all_js_models(&ir).unwrap();
    let code = &files
        .iter()
        .find(|(name, _)| name.starts_with("order."))
        .unwrap()
        .1;
    for header in [
        "export interface OrderWhereInput {",
        "export interface OrderAvgAggregateInput {",
        "export interface OrderSumAggregateInput {",
        "export interface OrderMinAggregateInput {",
        "export interface OrderMaxAggregateInput {",
        "export interface OrderOrderByInput {",
    ] {
        let section = declaration(code, header);
        for field in LIST_FIELDS {
            assert!(
                section
                    .iter()
                    .all(|line| !line.starts_with(&format!("  {field}?"))),
                "{header} offers {field}"
            );
        }
    }
    let update = declaration(code, "export interface OrderUpdateInput {");
    let create = declaration(code, "export interface OrderCreateInput {");
    let count = declaration(code, "export interface OrderCountAggregateInput {");
    for field in LIST_FIELDS {
        let line = update
            .iter()
            .find(|line| line.starts_with(&format!("  {field}?")))
            .unwrap();
        assert!(
            !line.contains("increment") && !line.contains("set?"),
            "{line}"
        );
        assert!(create
            .iter()
            .any(|line| line.starts_with(&format!("  {field}"))));
        assert!(count
            .iter()
            .any(|line| line.starts_with(&format!("  {field}?"))));
    }
    assert!(update
        .iter()
        .any(|line| line.starts_with("  weight?") && line.contains("increment")));
}

#[test]
fn rust_and_java_numeric_aggregates_exclude_lists() {
    let source = format!("{SCHEMA}\ngenerator client {{\n provider = \"nautilus-client-java\"\n output = \"./generated-java\"\n package = \"com.acme.db\"\n group_id = \"com.acme\"\n artifact_id = \"db-client\"\n interface = \"sync\"\n}}\n");
    let ir = validate_schema_source(&source).unwrap().ir;
    let rust = nautilus_codegen::generator::generate_all_models(&ir, false).unwrap();
    let java = nautilus_codegen::java::generate_java_client(&ir, "schema.nautilus", false).unwrap();
    let java_order = &java
        .iter()
        .find(|(name, _)| name.ends_with("/dsl/OrderDsl.java"))
        .unwrap()
        .1;
    for field in ["weights", "prices", "quantities", "sequences"] {
        for name in ["Avg", "Sum"] {
            let rust_input = declaration(
                &rust["Order"],
                &format!("pub struct Order{name}AggregateInput {{"),
            );
            // Rust declarations do not use Python/JS section delimiters.
            assert!(rust_input
                .iter()
                .take_while(|line| !line.starts_with('}'))
                .all(|line| !line.contains(&format!("pub {field}:"))));
            let java_input = java_order
                .split(&format!("public static final class {name}Aggregate"))
                .nth(1)
                .unwrap()
                .split("public static final class")
                .next()
                .unwrap();
            assert!(!java_input.contains(&format!(" {field}(")));
        }
    }
    for field in LIST_FIELDS {
        let where_section = java_order
            .split("public static final class Where")
            .nth(1)
            .unwrap()
            .split("public static final class")
            .next()
            .unwrap();
        assert!(!where_section.contains(&format!("Where {field}(")));
    }
}
