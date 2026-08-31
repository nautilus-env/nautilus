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

#[test]
fn an_import_pulls_in_the_file_it_names() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "enums.nautilus",
        "enum Role {\n  USER\n  ADMIN\n}\n",
    );
    let root = write(
        dir.path(),
        "user.nautilus",
        "import \"./enums.nautilus\"\n\nmodel User {\n  id Int @id\n  role Role @default(USER)\n}\n",
    );

    let set = SchemaSet::load_path(&root).unwrap();
    let ir = set.validate().unwrap().ir;

    assert_eq!(set.paths().count(), 2);
    assert_eq!(ir.models.len(), 1);
    assert_eq!(ir.enums.len(), 1);
}

#[test]
fn imports_are_followed_transitively_and_each_file_is_joined_once() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "enums.nautilus", "enum Role {\n  USER\n}\n");
    write(
        dir.path(),
        "post.nautilus",
        "import \"./enums.nautilus\"\n\nmodel Post {\n  id Int @id\n  authorId Int\n  author User @relation(fields: [authorId], references: [id])\n}\n",
    );
    let root = write(
        dir.path(),
        "user.nautilus",
        "import \"./enums.nautilus\"\nimport \"./post.nautilus\"\n\nmodel User {\n  id Int @id\n  role Role\n  posts Post[]\n}\n",
    );

    let set = SchemaSet::load_path(&root).unwrap();
    let ir = set.validate().unwrap().ir;

    assert_eq!(set.paths().count(), 3, "enums.nautilus is joined once");
    assert_eq!(ir.models.len(), 2);
}

#[test]
fn a_cycle_between_two_files_terminates() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "b.nautilus",
        "import \"./a.nautilus\"\n\nmodel B {\n  id Int @id\n}\n",
    );
    let root = write(
        dir.path(),
        "a.nautilus",
        "import \"./b.nautilus\"\n\nmodel A {\n  id Int @id\n}\n",
    );

    let set = SchemaSet::load_path(&root).unwrap();
    let ir = set.validate().unwrap().ir;

    assert_eq!(set.paths().count(), 2);
    assert!(ir.models.contains_key("A"));
    assert!(ir.models.contains_key("B"));
}

#[test]
fn importing_a_directory_joins_every_schema_file_in_it() {
    let dir = tempfile::tempdir().unwrap();
    let domain = dir.path().join("domain");
    fs::create_dir(&domain).unwrap();
    write(&domain, "enums.nautilus", "enum Role {\n  USER\n}\n");
    write(&domain, "post.nautilus", "model Post {\n  id Int @id\n}\n");
    let root = write(
        dir.path(),
        "schema.nautilus",
        "import \"./domain\"\n\nmodel User {\n  id Int @id\n  role Role\n}\n",
    );

    let set = SchemaSet::load_path(&root).unwrap();
    let ir = set.validate().unwrap().ir;

    assert_eq!(set.paths().count(), 3);
    assert!(ir.models.contains_key("Post"));
}

#[test]
fn importing_a_directory_without_schema_files_is_rejected() {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join("domain")).unwrap();
    let root = write(
        dir.path(),
        "schema.nautilus",
        "import \"./domain\"\n\nmodel User {\n  id Int @id\n}\n",
    );

    let set = SchemaSet::load_path(&root).unwrap();
    let error = set.validate().unwrap_err();
    let rendered = set.format_error(&error);

    assert!(rendered.contains("schema.nautilus:1:1"), "{rendered}");
    assert!(rendered.contains("holds no .nautilus file"), "{rendered}");
    assert_eq!(set.paths().count(), 1);
}

#[test]
fn importing_a_file_without_the_nautilus_extension_is_rejected() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "models.txt", "model Hidden { id Int @id }");
    let root = write(
        dir.path(),
        "schema.nautilus",
        "import \"./models.txt\"\n\nmodel User {\n  id Int @id\n}\n",
    );

    let set = SchemaSet::load_path(&root).unwrap();
    let error = set.validate().unwrap_err();
    let rendered = set.format_error(&error);

    assert!(rendered.contains("schema.nautilus:1:1"), "{rendered}");
    assert!(rendered.contains(".nautilus extension"), "{rendered}");
    assert_eq!(set.paths().count(), 1);
}

#[test]
fn an_unresolved_import_fails_validation_at_the_import_statement() {
    let dir = tempfile::tempdir().unwrap();
    let root = write(
        dir.path(),
        "schema.nautilus",
        "import \"./missing.nautilus\"\n\nmodel User {\n  id Int @id\n}\n",
    );

    let set = SchemaSet::load_path(&root).unwrap();
    let error = set.validate().unwrap_err();
    let rendered = set.format_error(&error);

    assert!(rendered.contains("schema.nautilus:1:1"), "{}", rendered);
    assert!(rendered.contains("missing.nautilus"), "{}", rendered);
    assert_eq!(set.import_errors().len(), 1);
}

#[test]
fn an_unresolved_import_still_analyzes_the_files_that_were_reached() {
    let dir = tempfile::tempdir().unwrap();
    let root = write(
        dir.path(),
        "schema.nautilus",
        "import \"./missing.nautilus\"\n\nmodel User {\n  id Int @id\n  tag Nope\n}\n",
    );

    let set = SchemaSet::load_path(&root).unwrap();
    let messages: Vec<String> = set
        .analyze()
        .diagnostics
        .iter()
        .map(|d| d.message.clone())
        .collect();

    assert!(
        messages.iter().any(|m| m.contains("missing.nautilus")),
        "{:?}",
        messages
    );
    assert!(
        messages.iter().any(|m| m.contains("Nope")),
        "{:?}",
        messages
    );
}

#[test]
fn an_overlay_replaces_what_is_on_disk() {
    let dir = tempfile::tempdir().unwrap();
    let enums = write(dir.path(), "enums.nautilus", "enum Role {\n  USER\n}\n");
    let root = write(
        dir.path(),
        "user.nautilus",
        "import \"./enums.nautilus\"\n\nmodel User {\n  id Int @id\n  status Status\n}\n",
    );

    let set = SchemaSet::load_path_with(&root, &|path| {
        (path == enums).then(|| "enum Status {\n  ACTIVE\n}\n".to_string())
    })
    .unwrap();

    let ir = set.validate().unwrap().ir;
    assert!(ir.enums.contains_key("Status"));
}

#[test]
fn a_directory_set_still_follows_the_imports_of_its_files() {
    let dir = tempfile::tempdir().unwrap();
    let shared = dir.path().join("shared");
    fs::create_dir(&shared).unwrap();
    write(&shared, "enums.nautilus", "enum Role {\n  USER\n}\n");
    let schema = dir.path().join("schema");
    fs::create_dir(&schema).unwrap();
    write(
        &schema,
        "user.nautilus",
        "import \"../shared/enums.nautilus\"\n\nmodel User {\n  id Int @id\n  role Role\n}\n",
    );

    let set = SchemaSet::load_dir(&schema).unwrap();
    let ir = set.validate().unwrap().ir;

    assert_eq!(set.paths().count(), 2);
    assert!(ir.enums.contains_key("Role"));
}

#[test]
fn a_file_reached_through_two_spellings_is_joined_once() {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join("nested")).unwrap();
    write(dir.path(), "enums.nautilus", "enum Role {\n  USER\n}\n");
    write(
        dir.path(),
        "post.nautilus",
        "import \"./nested/../enums.nautilus\"\n\nmodel Post {\n  id Int @id\n}\n",
    );
    let root = write(
        dir.path(),
        "user.nautilus",
        "import \"./enums.nautilus\"\nimport \"./post.nautilus\"\n\nmodel User {\n  id Int @id\n  role Role\n}\n",
    );

    let set = SchemaSet::load_path(&root).unwrap();

    assert_eq!(set.paths().count(), 3);
    assert!(set.validate().is_ok());
}

#[test]
fn importing_a_second_schema_root_is_rejected_at_the_duplicate_block() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "other.nautilus",
        "datasource other {\n  provider = \"sqlite\"\n  url = \"sqlite:other.db\"\n}\n\nmodel Other {\n  id Int @id\n}\n",
    );
    let root = write(
        dir.path(),
        "schema.nautilus",
        "import \"./other.nautilus\"\n\ndatasource db {\n  provider = \"sqlite\"\n  url = \"sqlite:main.db\"\n}\n\nmodel User {\n  id Int @id\n}\n",
    );

    let set = SchemaSet::load_path(&root).unwrap();
    let error = set.validate().unwrap_err();
    let rendered = set.format_error(&error);

    assert!(rendered.contains("Duplicate datasource"), "{}", rendered);
    assert!(
        rendered.contains("other.nautilus:1:12"),
        "the block that came in through the import is the one flagged: {}",
        rendered
    );
}

#[test]
fn a_second_generator_reached_through_an_import_is_rejected() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "client.nautilus",
        "generator extra {\n  provider = \"nautilus-client-js\"\n  output = \"./extra\"\n}\n",
    );
    let root = write(
        dir.path(),
        "schema.nautilus",
        "import \"./client.nautilus\"\n\ngenerator client {\n  provider = \"nautilus-client-js\"\n  output = \"./db\"\n}\n\nmodel User {\n  id Int @id\n}\n",
    );

    let set = SchemaSet::load_path(&root).unwrap();
    let rendered = set.format_error(&set.validate().unwrap_err());

    assert!(rendered.contains("Duplicate generator"), "{}", rendered);
    assert!(rendered.contains("client.nautilus:1:11"), "{}", rendered);
}
