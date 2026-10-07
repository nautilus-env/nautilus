//! A model that relates to itself is defined in its own module, so that module
//! must not import it again: Python would shadow the class for type checkers,
//! Rust would define the type twice.

use nautilus_codegen::{
    generate_command, generator::generate_all_models, python::generate_all_python_models,
    GenerateOptions, InstallMode,
};
use nautilus_schema::validate_schema_source;
use std::fs;

const MODELS: &str = r#"
model Revision {
  id           Int        @id @default(autoincrement())
  title        String
  supersedesId Int?       @unique
  supersedes   Revision?  @relation(name: "Revisions", fields: [supersedesId], references: [id])
  successor    Revision?  @relation(name: "Revisions")
  parentId     Int?
  parent       Revision?  @relation(name: "Tree", fields: [parentId], references: [id])
  children     Revision[] @relation(name: "Tree")
  authorId     Int
  author       Author     @relation(fields: [authorId], references: [id])
}

model Author {
  id        Int        @id @default(autoincrement())
  revisions Revision[]
}
"#;

fn ir() -> nautilus_schema::ir::SchemaIr {
    validate_schema_source(MODELS)
        .expect("a model may relate to itself")
        .ir
}

/// The other model is still imported: only the model's own name is left out.
fn assert_imports_only_the_other_model(code: &str, file: &str) {
    assert!(!code.contains("from . import Revision"), "{file}:\n{code}");
    assert!(!code.contains("from .revision import ("), "{file}:\n{code}");
    assert!(code.contains("from . import Author"), "{file}:\n{code}");
    assert!(code.contains("from .author import ("), "{file}:\n{code}");
}

#[test]
fn the_rust_model_does_not_import_itself() {
    let models = generate_all_models(&ir(), false).expect("generate_all_models should succeed");
    let revision = models.get("Revision").expect("Revision missing");
    assert!(!revision.contains("use super::Revision;"), "{revision}");
    assert!(revision.contains("use super::Author;"), "{revision}");
}

#[test]
fn the_python_module_does_not_import_itself() {
    let files =
        generate_all_python_models(&ir(), true, 3).expect("generate_all_python_models failed");
    let (_, revision) = files
        .iter()
        .find(|(name, _)| name == "revision.py")
        .expect("revision.py missing");
    assert_imports_only_the_other_model(revision, "revision.py");
}

#[test]
fn the_split_python_modules_do_not_import_their_own_names() {
    let fixture = tempfile::tempdir().unwrap();
    let output = fixture.path().join("client");
    let schema_path = fixture.path().join("schema.nautilus");
    let source = format!(
        "datasource db {{\n  provider = \"sqlite\"\n  url      = \"sqlite://./repro.db\"\n}}\n\ngenerator client {{\n  provider  = \"nautilus-client-py\"\n  interface = \"async\"\n  output    = {:?}\n}}\n{MODELS}",
        output.to_str().unwrap(),
    );
    fs::write(&schema_path, source).unwrap();
    generate_command(
        &schema_path,
        GenerateOptions {
            install: InstallMode::Never,
            ..Default::default()
        },
    )
    .expect("generate should succeed");

    let read = |name: &str| fs::read_to_string(output.join("models").join(name)).unwrap();
    assert_imports_only_the_other_model(&read("revision.py"), "revision.py");
    let inputs = read("_revision_inputs.py");
    assert!(!inputs.contains("from .revision import ("), "{inputs}");
    assert!(inputs.contains("from .author import ("), "{inputs}");
    assert!(inputs.contains("class RevisionWhereInput("), "{inputs}");
    let codec = read("_revision_codec.py");
    assert!(
        codec.contains("from .revision import Revision, RevisionProjection"),
        "{codec}"
    );
}
