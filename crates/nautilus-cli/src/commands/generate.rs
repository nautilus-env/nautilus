use std::path::PathBuf;

pub fn run_generate(
    schema: Option<String>,
    install: bool,
    no_install: bool,
    verbose: bool,
    standalone: bool,
) -> anyhow::Result<()> {
    let path_buf = schema.map(PathBuf::from);
    let path = nautilus_codegen::resolve_schema_path(path_buf)?;
    let install = match (install, no_install) {
        (true, true) => {
            return Err(anyhow::anyhow!("--install and --no-install conflict"));
        }
        (true, false) => nautilus_codegen::InstallMode::Always,
        (false, true) => nautilus_codegen::InstallMode::Never,
        (false, false) => nautilus_codegen::InstallMode::Auto,
    };
    nautilus_codegen::generate_command(
        &path,
        nautilus_codegen::GenerateOptions {
            install,
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
    use tempfile::TempDir;

    #[test]
    fn codegen_schema_resolution_auto_detects_first_nautilus_file() {
        let temp_dir = TempDir::new().expect("temp dir");

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

        let resolved = nautilus_codegen::resolve_schema_path_in(temp_dir.path(), None)
            .expect("schema should auto-resolve");
        assert_eq!(
            resolved.file_name().and_then(|name| name.to_str()),
            Some("alpha.nautilus")
        );
    }
}
