use std::path::PathBuf;

pub fn run_generate(
    schema: Option<String>,
    no_install: bool,
    verbose: bool,
    standalone: bool,
) -> anyhow::Result<()> {
    let path_buf = schema.map(PathBuf::from);
    let path = nautilus_codegen::resolve_schema_path(path_buf)?;
    nautilus_codegen::generate_command(
        &path,
        nautilus_codegen::GenerateOptions {
            install: !no_install,
            verbose,
            standalone,
        },
    )
}

pub fn run_validate(schema: Option<String>) -> anyhow::Result<()> {
    let path_buf = schema.map(PathBuf::from);
    let path = nautilus_codegen::resolve_schema_path(path_buf)?;
    nautilus_codegen::validate_command(&path)
}

#[cfg(test)]
mod tests {
    use crate::test_support::{lock_working_dir, CurrentDirGuard};
    use tempfile::TempDir;

    #[test]
    fn codegen_schema_resolution_auto_detects_first_nautilus_file() {
        let _cwd_lock = lock_working_dir();
        let temp_dir = TempDir::new().expect("temp dir");
        let _dir_guard = CurrentDirGuard::set(temp_dir.path());

        std::fs::write(
            temp_dir.path().join("zeta.nautilus"),
            "model User { id Int @id }\n",
        )
        .expect("failed to write zeta schema");
        std::fs::write(
            temp_dir.path().join("alpha.nautilus"),
            "model Post { id Int @id }\n",
        )
        .expect("failed to write alpha schema");

        let resolved =
            nautilus_codegen::resolve_schema_path(None).expect("schema should auto-resolve");
        assert_eq!(
            resolved.file_name().and_then(|name| name.to_str()),
            Some("alpha.nautilus")
        );
    }
}
