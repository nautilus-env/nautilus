//! Assembling a schema from several `.nautilus` files.

use nautilus_schema::SchemaSet;
use std::fs;
use std::path::PathBuf;

fn write(dir: &std::path::Path, name: &str, contents: &str) -> PathBuf {
    let path = dir.join(name);
    fs::write(&path, contents).expect("failed to write schema file");
    path
}

#[test]
fn declarations_resolve_across_files() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "00-datasource.nautilus",
        "datasource db {\n  provider = \"postgresql\"\n  url = \"postgresql://localhost/x\"\n}\n",
    );
    write(
        dir.path(),
        "enums.nautilus",
        "enum Role {\n  USER\n  ADMIN\n}\n",
    );
    write(
        dir.path(),
        "user.nautilus",
        "model User {\n  id Int @id\n  role Role @default(USER)\n  posts Post[]\n}\n",
    );
    write(
        dir.path(),
        "post.nautilus",
        "model Post {\n  id Int @id\n  authorId Int\n  author User @relation(fields: [authorId], references: [id])\n}\n",
    );

    let set = SchemaSet::load_dir(dir.path()).unwrap();
    let ir = set.validate().unwrap().ir;

    assert_eq!(ir.models.len(), 2);
    assert_eq!(ir.enums.len(), 1);
    assert!(ir.datasource.is_some());
}

#[test]
fn a_file_without_a_trailing_newline_does_not_glue_onto_the_next() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "a.nautilus", "model A {\n  id Int @id\n}");
    write(dir.path(), "b.nautilus", "model B {\n  id Int @id\n}\n");

    let set = SchemaSet::load_dir(dir.path()).unwrap();
    let ir = set.validate().unwrap().ir;

    assert!(ir.models.contains_key("A"));
    assert!(ir.models.contains_key("B"));
}

#[test]
fn diagnostics_name_the_file_the_span_came_from() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "a.nautilus", "model A {\n  id Int @id\n}\n");
    write(
        dir.path(),
        "b.nautilus",
        "model B {\n  id      Int @id\n  missing Nope\n}\n",
    );

    let set = SchemaSet::load_dir(dir.path()).unwrap();
    let error = set.validate().unwrap_err();
    let rendered = set.format_error(&error);

    assert!(
        rendered.contains("b.nautilus:3:3"),
        "expected the diagnostic to point at b.nautilus line 3, got: {}",
        rendered
    );
}

#[test]
fn a_single_file_path_loads_as_a_one_file_set() {
    let dir = tempfile::tempdir().unwrap();
    let path = write(
        dir.path(),
        "schema.nautilus",
        "model A {\n  id Int @id\n}\n",
    );
    write(dir.path(), "other.nautilus", "model B {\n  id Int @id\n}\n");

    let set = SchemaSet::load_path(&path).unwrap();
    let ir = set.validate().unwrap().ir;

    assert_eq!(
        ir.models.len(),
        1,
        "a file path must not pull in its siblings"
    );
    assert_eq!(set.display_path(), path);
}

#[test]
fn a_directory_path_loads_every_file_in_it() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "a.nautilus", "model A {\n  id Int @id\n}\n");
    write(dir.path(), "b.nautilus", "model B {\n  id Int @id\n}\n");

    let set = SchemaSet::load_path(dir.path()).unwrap();

    assert_eq!(set.paths().count(), 2);
    assert_eq!(set.validate().unwrap().ir.models.len(), 2);
}

#[test]
fn duplicate_declarations_across_files_are_rejected() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "a.nautilus", "model User {\n  id Int @id\n}\n");
    write(dir.path(), "b.nautilus", "model User {\n  id Int @id\n}\n");

    let set = SchemaSet::load_dir(dir.path()).unwrap();
    let error = set.validate().unwrap_err();

    assert!(
        set.format_error(&error)
            .contains("Duplicate model name 'User'"),
        "got: {}",
        set.format_error(&error)
    );
}
