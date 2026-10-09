//! An enum field takes filter operators in the typed `where` input, as the
//! other scalars do. The engine already runs `in`, `notIn`, `not` and the null
//! check on enum columns; the generated types offered only a bare value.

use nautilus_codegen::{
    generate_command, js::generate_all_js_models, python::generate_all_python_models,
    GenerateOptions, InstallMode,
};
use nautilus_schema::validate_schema_source;
use std::fs;

const DATASOURCE: &str = r#"
datasource db {
  provider = "postgresql"
  url      = "postgres://localhost/unused"
}
"#;

/// `Bool` is named like a scalar filter on purpose: its own filter must not
/// redefine `BoolFilter`, which `done` uses.
const MODELS: &str = r#"
enum Status {
  PENDING
  DONE
}

enum Bool {
  YES
  NO
}

model Task {
  id       Int      @id @default(autoincrement())
  status   Status
  previous Status?
  history  Status[]
  answer   Bool
  done     Boolean
}
"#;

fn ir() -> nautilus_schema::ir::SchemaIr {
    validate_schema_source(&format!("{DATASOURCE}{MODELS}"))
        .expect("schema should validate")
        .ir
}

/// The lines of the declaration that `header` opens, up to the next one.
fn declaration<'a>(code: &'a str, header: &str) -> Vec<&'a str> {
    let mut lines = code.lines().skip_while(|line| !line.starts_with(header));
    let first = lines
        .next()
        .unwrap_or_else(|| panic!("{header} missing:\n{code}"));
    std::iter::once(first)
        .chain(lines.take_while(|line| !line.starts_with("class ") && !line.starts_with("export ")))
        .collect()
}

fn generated_python(name: &str) -> String {
    generate_all_python_models(&ir(), true, 3)
        .expect("generate_all_python_models failed")
        .into_iter()
        .find(|(file, _)| file == name)
        .unwrap_or_else(|| panic!("{name} missing"))
        .1
}

#[test]
fn the_python_where_input_takes_enum_filters() {
    let task = generated_python("task.py");
    let where_input = declaration(&task, "class TaskWhereInput(");
    for expected in [
        "    status: NotRequired[Union[Status, EnumStatusFilter]]",
        "    previous: NotRequired[Union[Status, None, EnumStatusFilter]]",
        "    answer: NotRequired[Union[Bool, EnumBoolFilter]]",
        "    done: NotRequired[Union[bool, BoolFilter]]",
    ] {
        assert!(where_input.contains(&expected), "{expected}:\n{task}");
    }
    assert!(
        where_input
            .iter()
            .all(|line| !line.starts_with("    history:")),
        "the operators compare one value, so an array field does not take them:\n{task}"
    );

    let filter = declaration(&task, "class EnumStatusFilter(TypedDict, total=False):");
    for expected in [
        "    equals: NotRequired[Status]",
        "    not_: NotRequired[Status]",
        "    in_: NotRequired[List[Status]]",
        "    not_in: NotRequired[List[Status]]",
        "    is_null: NotRequired[bool]",
    ] {
        assert!(filter.contains(&expected), "{expected}:\n{task}");
    }
    assert!(
        task.contains("class EnumBoolFilter(TypedDict, total=False):"),
        "{task}"
    );
    assert_eq!(task.matches("class BoolFilter(").count(), 1, "{task}");
}

#[test]
fn a_model_without_enums_gets_no_enum_filter() {
    let ir = validate_schema_source(&format!("{DATASOURCE}model Note {{\n  id Int @id\n}}\n"))
        .expect("schema should validate")
        .ir;
    let files = generate_all_python_models(&ir, true, 3).expect("generation failed");
    let (_, note) = files
        .iter()
        .find(|(name, _)| name == "note.py")
        .expect("note.py missing");
    assert!(!note.contains("class Enum"), "{note}");
}

#[test]
fn the_split_python_modules_define_and_export_enum_filters() {
    let fixture = tempfile::tempdir().unwrap();
    let output = fixture.path().join("client");
    let schema_path = fixture.path().join("schema.nautilus");
    let generator = format!(
        r#"
generator client {{
  provider  = "nautilus-client-py"
  interface = "async"
  output    = {:?}
}}
"#,
        output.to_str().unwrap(),
    );
    fs::write(&schema_path, format!("{DATASOURCE}{generator}{MODELS}")).unwrap();
    generate_command(
        &schema_path,
        GenerateOptions {
            install: InstallMode::Never,
            ..Default::default()
        },
    )
    .expect("generate should succeed");

    let read = |name: &str| fs::read_to_string(output.join("models").join(name)).unwrap();
    let inputs = read("_task_inputs.py");
    assert!(
        inputs.contains("class EnumStatusFilter(TypedDict, total=False):"),
        "{inputs}"
    );
    let where_input = declaration(&inputs, "class TaskWhereInput(");
    assert!(
        where_input.contains(&"    status: NotRequired[Union[Status, EnumStatusFilter]]"),
        "{inputs}"
    );
    let facade = read("task.py");
    assert!(facade.contains("    EnumStatusFilter,"), "{facade}");
    assert!(facade.contains("    \"EnumStatusFilter\","), "{facade}");
}

#[test]
fn the_typescript_where_input_takes_enum_filters() {
    let (_, declarations) = generate_all_js_models(&ir()).expect("generate_all_js_models failed");
    let (_, task) = declarations
        .iter()
        .find(|(name, _)| name.starts_with("task"))
        .expect("task declarations missing");
    let where_input = declaration(task, "export interface TaskWhereInput {");
    for expected in [
        "  status?: Status | EnumFilter<Status>;",
        "  previous?: Status | null | EnumFilter<Status>;",
        "  answer?: Bool | EnumFilter<Bool>;",
    ] {
        assert!(where_input.contains(&expected), "{expected}:\n{task}");
    }
    assert!(where_input.iter().all(|line| !line.contains("history")));
    let filter = declaration(task, "export interface EnumFilter<T> {");
    for expected in [
        "  in?:     T[];",
        "  notIn?:  T[];",
        "  not?:    T;",
        "  isNull?: boolean;",
    ] {
        assert!(filter.contains(&expected), "{expected}:\n{task}");
    }
}
