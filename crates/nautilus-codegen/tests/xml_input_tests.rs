//! XML stays writable and countable, but PostgreSQL cannot compare it.

use nautilus_codegen::{js::generate_all_js_models, python::generate_all_python_models};
use nautilus_schema::validate_schema_source;

const SCHEMA: &str = r#"
datasource db { provider = "postgresql" url = "postgresql://unused/unused" }
model Order {
  id Int @id
  invoice Xml? @map("invoice_xml")
  receipt Xml
  region String
}
"#;

#[test]
fn python_xml_inputs_only_offer_null_filters_and_comparable_keys() {
    let ir = validate_schema_source(SCHEMA).unwrap().ir;
    let files = generate_all_python_models(&ir, true, 3).unwrap();
    let code = &files.iter().find(|(name, _)| name == "order.py").unwrap().1;
    assert!(code.contains("invoice: NotRequired[NullFilter]"));
    assert!(code.contains("receipt: NotRequired[NullFilter]"));
    assert!(code.contains("invoice: NotRequired[Optional[str]]"));
    assert!(code.contains("receipt: Required[str]"));
    assert!(code.contains("OrderComparableFieldKeys = Literal[\"id\", \"region\"]"));
    assert!(code.contains("by: List[OrderComparableFieldKeys]"));
    assert!(code.contains("distinct: Optional[List[OrderComparableFieldKeys]]"));
    let ordering = code
        .split("class OrderOrderByInput(")
        .nth(1)
        .unwrap()
        .split("class OrderSelectInput(")
        .next()
        .unwrap();
    assert!(!ordering.contains("invoice") && !ordering.contains("receipt"));
}

#[test]
fn javascript_xml_inputs_only_offer_null_filters_and_comparable_keys() {
    let ir = validate_schema_source(SCHEMA).unwrap().ir;
    let (_, files) = generate_all_js_models(&ir).unwrap();
    let code = &files
        .iter()
        .find(|(name, _)| name == "order.d.ts")
        .unwrap()
        .1;
    assert!(code.contains("invoice?: NullFilter;"));
    assert!(code.contains("receipt?: NullFilter;"));
    assert!(code.contains("invoice?: string | null;"));
    assert!(code.contains("receipt: string;"));
    assert!(code.contains("OrderComparableFieldKeys = 'id' | 'region';"));
    assert!(code.contains("by:      OrderComparableFieldKeys[];"));
    assert!(code.contains("distinct?: OrderComparableFieldKeys[];"));
    let ordering = code
        .split("export interface OrderOrderByInput {")
        .nth(1)
        .unwrap()
        .split('}')
        .next()
        .unwrap();
    assert!(!ordering.contains("invoice") && !ordering.contains("receipt"));
}

#[test]
fn java_xml_filter_dsl_offers_null_checks_without_equality_or_ordering() {
    let source = format!("{SCHEMA}\ngenerator client {{ provider = \"nautilus-client-java\" output = \"./generated-java\" package = \"com.acme.db\" group_id = \"com.acme\" artifact_id = \"db-client\" interface = \"sync\" }}");
    let ir = validate_schema_source(&source).unwrap().ir;
    let files =
        nautilus_codegen::java::generate_java_client(&ir, "schema.nautilus", false).unwrap();
    let code = &files
        .iter()
        .find(|(name, _)| name.ends_with("/dsl/OrderDsl.java"))
        .unwrap()
        .1;
    assert!(code.contains("Where invoiceIsNull(Boolean value)"));
    assert!(code.contains("Where receiptIsNull(Boolean value)"));
    assert!(!code.contains("Where invoice(String value)"));
    assert!(!code.contains("Where invoiceContains("));
    assert!(!code.contains("OrderBy invoice("));
    assert!(code.contains("CountAggregate invoice()"));
}
