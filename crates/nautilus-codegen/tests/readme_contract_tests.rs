#[test]
fn readme_tracks_generated_js_and_python_install_story() {
    let readme = std::fs::read_to_string(format!("{}/README.md", env!("CARGO_MANIFEST_DIR")))
        .expect("failed to read codegen README");

    assert!(readme.contains("import the generated `output` package directly"));
    assert!(readme.contains("site-packages/nautilus"));
    assert!(readme.contains("not a PyPI publish step"));
    assert!(readme.contains("node_modules/nautilus"));
    assert!(readme.contains("not an npm publish step"));
    assert!(readme.contains("The checked-in examples show the intended consumption pattern today"));
}
