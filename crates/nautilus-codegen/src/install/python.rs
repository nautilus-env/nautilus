//! Installing the generated Python package into `site-packages`.

use anyhow::{Context, Result};
use std::fs;
use std::path::{Path, PathBuf};

use super::copy_dir_recursive;

/// Copy the package at `output_path` over `site-packages/nautilus`.
pub(crate) fn install(output_path: &str) -> Result<PathBuf> {
    let site_packages = detect_site_packages()?;
    let src = Path::new(output_path);
    let dst = site_packages.join("nautilus");

    install_into(src, &dst)?;
    Ok(dst)
}

fn detect_site_packages() -> Result<PathBuf> {
    use std::process::Command;

    let script = "import sysconfig; print(sysconfig.get_path('purelib'))";
    for exe in &["python", "python3"] {
        if let Ok(out) = Command::new(exe).arg("-c").arg(script).output() {
            if out.status.success() {
                let path_str = String::from_utf8_lossy(&out.stdout).trim().to_string();
                if !path_str.is_empty() {
                    return Ok(PathBuf::from(path_str));
                }
            }
        }
    }

    Err(anyhow::anyhow!(
        "Could not detect Python site-packages directory.\n\
        Make sure Python is installed and available as 'python' or 'python3'."
    ))
}

/// What a generated Python package consists of, and so what an install
/// replaces. Everything else in the target directory is left alone.
const GENERATED_PACKAGE_ENTRIES: &[&str] = &[
    "__init__.py",
    "client.py",
    "transaction.py",
    "py.typed",
    "models",
    "enums",
    "errors",
    "_internal",
    "types",
    "extensions",
];

fn install_into(src: &Path, dst: &Path) -> Result<()> {
    if dst.exists() {
        if !dst.is_dir() {
            return Err(anyhow::anyhow!(
                "Python install target exists but is not a directory: {}",
                dst.display()
            ));
        }

        // Keep the CLI wrapper files that pip installs (`__main__.py`,
        // `nautilus`, `nautilus.exe`) and refresh only the generated client tree.
        clear_generated_package(dst)?;
    }

    copy_dir_recursive(src, dst)
}

fn clear_generated_package(dst: &Path) -> Result<()> {
    for entry in GENERATED_PACKAGE_ENTRIES {
        let path = dst.join(entry);
        if path.is_dir() {
            fs::remove_dir_all(&path).with_context(|| {
                format!(
                    "Failed to remove generated directory from Python install: {}",
                    path.display()
                )
            })?;
        } else if path.exists() {
            fs::remove_file(&path).with_context(|| {
                format!(
                    "Failed to remove generated file from Python install: {}",
                    path.display()
                )
            })?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::install_into;

    #[test]
    fn python_install_preserves_cli_wrapper_files() {
        let src_root = tempfile::TempDir::new().expect("temp src dir");
        let dst_root = tempfile::TempDir::new().expect("temp dst dir");
        let src = src_root.path().join("generated");
        let dst = dst_root.path().join("nautilus");

        std::fs::create_dir_all(src.join("models")).expect("create generated models dir");
        std::fs::write(src.join("__init__.py"), "from .client import Nautilus\n")
            .expect("write generated __init__.py");
        std::fs::write(src.join("client.py"), "class Nautilus: ...\n")
            .expect("write generated client.py");
        std::fs::write(src.join("py.typed"), "").expect("write generated py.typed");
        std::fs::write(src.join("models").join("user.py"), "class User: ...\n")
            .expect("write generated model");

        std::fs::create_dir_all(dst.join("models")).expect("create installed models dir");
        std::fs::write(dst.join("__main__.py"), "def main(): ...\n")
            .expect("write cli __main__.py");
        std::fs::write(dst.join("nautilus"), "binary").expect("write cli binary");
        std::fs::write(dst.join("nautilus.exe"), "binary").expect("write cli windows binary");
        std::fs::write(dst.join("__init__.py"), "old generated package\n")
            .expect("write stale generated __init__.py");
        std::fs::write(dst.join("client.py"), "old client\n").expect("write stale client.py");
        std::fs::write(dst.join("models").join("legacy.py"), "old model\n")
            .expect("write stale model");

        install_into(&src, &dst).expect("overlay install should succeed");

        assert_eq!(
            std::fs::read_to_string(dst.join("__main__.py")).expect("read cli __main__.py"),
            "def main(): ...\n"
        );
        assert_eq!(
            std::fs::read_to_string(dst.join("nautilus")).expect("read cli binary"),
            "binary"
        );
        assert_eq!(
            std::fs::read_to_string(dst.join("nautilus.exe")).expect("read cli windows binary"),
            "binary"
        );
        assert_eq!(
            std::fs::read_to_string(dst.join("__init__.py")).expect("read generated __init__.py"),
            "from .client import Nautilus\n"
        );
        assert_eq!(
            std::fs::read_to_string(dst.join("client.py")).expect("read generated client.py"),
            "class Nautilus: ...\n"
        );
        assert!(
            !dst.join("models").join("legacy.py").exists(),
            "stale generated model should be removed"
        );
        assert!(
            dst.join("models").join("user.py").exists(),
            "new generated model should be installed"
        );
    }

    #[test]
    fn python_install_removes_generated_entries_absent_from_new_output() {
        let src_root = tempfile::TempDir::new().expect("temp src dir");
        let dst_root = tempfile::TempDir::new().expect("temp dst dir");
        let src = src_root.path().join("generated");
        let dst = dst_root.path().join("nautilus");

        std::fs::create_dir_all(src.join("_internal")).expect("create generated runtime dir");
        std::fs::write(src.join("__init__.py"), "fresh init\n").expect("write generated init");
        std::fs::write(src.join("_internal").join("__init__.py"), "").expect("write runtime init");

        std::fs::create_dir_all(dst.join("types")).expect("create stale types dir");
        std::fs::write(dst.join("types").join("__init__.py"), "stale types\n")
            .expect("write stale types init");

        install_into(&src, &dst).expect("overlay install should succeed");

        assert!(
            !dst.join("types").exists(),
            "stale generated types dir should be removed when no longer generated"
        );
        assert!(
            dst.join("_internal").join("__init__.py").exists(),
            "fresh generated runtime files should be installed"
        );
    }
}
