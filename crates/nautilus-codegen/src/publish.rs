//! Publishing a generated package: build it aside, then swap it into place.
//!
//! The generator owns its output directory outright and replaces it wholesale,
//! so the previous run's files never linger next to the new ones. Replacing it
//! is only safe if the new tree already exists, which is why every file is
//! written into a staging directory beside the target first: a template that
//! fails to render, or a write that fails half-way, leaves the client that was
//! already published untouched.

use anyhow::{bail, Context, Result};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_STAGE_ID: AtomicU64 = AtomicU64::new(0);

/// A name no two generations share, even when they run at the same time.
fn unique_suffix() -> String {
    format!(
        "{}-{}",
        std::process::id(),
        NEXT_STAGE_ID.fetch_add(1, Ordering::Relaxed)
    )
}

/// Create a uniquely named directory under `parent` whose name starts with
/// `prefix`.
pub(crate) fn unique_dir(parent: &Path, prefix: &str) -> Result<PathBuf> {
    let path = parent.join(format!("{prefix}-{}", unique_suffix()));
    std::fs::create_dir_all(&path)
        .with_context(|| format!("Failed to create directory: {}", path.display()))?;
    Ok(path)
}

/// Build the package for `output_path` in a staging directory, then swap it in.
///
/// `write` receives the staging directory and must produce the complete tree;
/// only once it returns does anything under `output_path` change. Every file
/// under `output_path` belongs to the generator: whatever was there before is
/// removed by the swap, so hand-written files do not survive a generation and
/// belong outside the output directory.
pub(crate) fn publish_into(
    output_path: &str,
    write: impl FnOnce(&Path) -> Result<()>,
) -> Result<()> {
    let target = validated_target(output_path)?;
    let parent = target
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));
    std::fs::create_dir_all(&parent).with_context(|| {
        format!(
            "Failed to create parent of output directory: {}",
            parent.display()
        )
    })?;

    let staging = Staging::beside(&target)?;
    write(staging.path())?;
    staging.swap_into(&target)
}

/// Reject an output path the generator must not replace wholesale.
fn validated_target(output_path: &str) -> Result<PathBuf> {
    if output_path.trim().is_empty() {
        bail!("generator output path is empty");
    }
    let target = PathBuf::from(output_path);
    if target.file_name().is_none() {
        bail!(
            "generator output path {} names a filesystem root, which cannot be replaced",
            target.display()
        );
    }
    if target.exists() && !target.is_dir() {
        bail!(
            "generator output path {} exists and is not a directory",
            target.display()
        );
    }
    Ok(target)
}

/// A directory being filled in beside the location it will take over.
///
/// Staging beside the target keeps both on one filesystem, so the swap is a
/// pair of renames rather than a copy — which also means Windows never has to
/// delete the live directory before the replacement exists.
struct Staging {
    path: PathBuf,
    published: bool,
}

impl Staging {
    fn beside(target: &Path) -> Result<Self> {
        let parent = target.parent().unwrap_or_else(|| Path::new("."));
        let name = target
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("output");
        let path = unique_dir(parent, &format!(".{name}.nautilus-stage"))?;
        Ok(Self {
            path,
            published: false,
        })
    }

    fn path(&self) -> &Path {
        &self.path
    }

    /// Move the staged tree onto `target`, putting the previous tree back if
    /// the move cannot be completed.
    fn swap_into(mut self, target: &Path) -> Result<()> {
        if !target.exists() {
            std::fs::rename(&self.path, target).with_context(|| {
                format!("Failed to publish generated files to {}", target.display())
            })?;
            self.published = true;
            return Ok(());
        }

        let parent = target.parent().unwrap_or_else(|| Path::new("."));
        let name = target
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("output");
        let previous = parent.join(format!(".{name}.nautilus-old-{}", unique_suffix()));

        std::fs::rename(target, &previous).with_context(|| {
            format!(
                "Failed to move the previously generated files out of {}; \
                 is a file in it still open?",
                target.display()
            )
        })?;

        match std::fs::rename(&self.path, target) {
            Ok(()) => {
                self.published = true;
                let _ = std::fs::remove_dir_all(&previous);
                Ok(())
            }
            Err(e) => {
                let restored = std::fs::rename(&previous, target);
                Err(anyhow::Error::new(e).context(if restored.is_ok() {
                    format!(
                        "Failed to publish generated files to {}; \
                         the previously generated files were put back",
                        target.display()
                    )
                } else {
                    format!(
                        "Failed to publish generated files to {}; \
                         the previously generated files are left in {}",
                        target.display(),
                        previous.display()
                    )
                }))
            }
        }
    }
}

impl Drop for Staging {
    fn drop(&mut self) {
        if !self.published {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read(path: &Path) -> String {
        std::fs::read_to_string(path).expect("generated file")
    }

    #[test]
    fn a_failed_write_leaves_the_previous_output_in_place() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("client");
        std::fs::create_dir_all(&out).unwrap();
        std::fs::write(out.join("model.rs"), "previous").unwrap();

        let err = publish_into(&out.to_string_lossy(), |staging| {
            std::fs::write(staging.join("model.rs"), "new").unwrap();
            bail!("rendering failed")
        })
        .unwrap_err();

        assert!(err.to_string().contains("rendering failed"));
        assert_eq!(read(&out.join("model.rs")), "previous");
    }

    #[test]
    fn publishing_replaces_the_whole_directory() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("client");
        std::fs::create_dir_all(out.join("src")).unwrap();
        std::fs::write(out.join("src").join("stale.rs"), "stale").unwrap();

        publish_into(&out.to_string_lossy(), |staging| {
            std::fs::create_dir_all(staging.join("src"))?;
            std::fs::write(staging.join("src").join("fresh.rs"), "fresh")?;
            Ok(())
        })
        .unwrap();

        assert_eq!(read(&out.join("src").join("fresh.rs")), "fresh");
        assert!(
            !out.join("src").join("stale.rs").exists(),
            "the output directory belongs to the generator and is replaced whole"
        );
    }

    #[test]
    fn a_file_dropped_into_the_output_does_not_survive_a_generation() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("client");
        std::fs::create_dir_all(&out).unwrap();
        std::fs::write(out.join("notes.md"), "mine").unwrap();

        publish_into(&out.to_string_lossy(), |staging| {
            std::fs::write(staging.join("model.rs"), "new")?;
            Ok(())
        })
        .unwrap();

        assert!(!out.join("notes.md").exists());
        assert_eq!(read(&out.join("model.rs")), "new");
    }

    #[test]
    fn staging_directories_of_two_generations_never_collide() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("client");
        let first = Staging::beside(&target).unwrap();
        let second = Staging::beside(&target).unwrap();
        assert_ne!(first.path(), second.path());
    }

    #[test]
    fn a_staging_directory_is_removed_when_it_is_not_published() {
        let dir = tempfile::tempdir().unwrap();
        let staged = {
            let staging = Staging::beside(&dir.path().join("client")).unwrap();
            staging.path().to_path_buf()
        };
        assert!(!staged.exists());
    }

    #[test]
    fn an_output_path_that_is_a_file_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("client");
        std::fs::write(&file, "not a directory").unwrap();

        let err = publish_into(&file.to_string_lossy(), |_| Ok(())).unwrap_err();
        assert!(err.to_string().contains("is not a directory"), "{err}");
        assert_eq!(read(&file), "not a directory");
    }

    #[test]
    fn an_empty_output_path_is_refused() {
        let err = publish_into("  ", |_| Ok(())).unwrap_err();
        assert!(err.to_string().contains("empty"), "{err}");
    }

    #[test]
    fn a_filesystem_root_is_refused() {
        let root = if cfg!(windows) { "C:\\" } else { "/" };
        let err = publish_into(root, |_| Ok(())).unwrap_err();
        assert!(err.to_string().contains("filesystem root"), "{err}");
    }
}
