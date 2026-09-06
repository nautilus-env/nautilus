//! A generated client held in memory, before anything reaches the disk.
//!
//! Every backend lays its output out as this: a list of files named by a path
//! relative to the output directory. Nothing here reads the schema, prints, or
//! installs, so a caller can generate a client and inspect it without touching
//! the filesystem, and [`publish`](GeneratedPackage::publish) is the single
//! step that writes one out.

use anyhow::{Context, Result};
use std::path::Path;

use crate::publish::publish_into;
use crate::GeneratedFile;

/// The files of one generated client, in a deterministic order.
#[derive(Debug, Default, Clone)]
pub struct GeneratedPackage {
    files: Vec<GeneratedFile>,
}

impl GeneratedPackage {
    /// Add one file, replacing what an earlier `add` put at the same path.
    ///
    /// Replacing rather than appending keeps a package the faithful picture of
    /// the directory it becomes: writing the same path twice used to leave the
    /// last contents on disk, and it still does.
    pub(crate) fn add(&mut self, path: impl Into<String>, contents: impl Into<String>) {
        let path = path.into();
        let contents = contents.into();
        match self.files.iter_mut().find(|(name, _)| *name == path) {
            Some(existing) => existing.1 = contents,
            None => self.files.push((path, contents)),
        }
    }

    /// Add files that already carry a path relative to `directory`.
    pub(crate) fn add_all<'a>(
        &mut self,
        directory: &str,
        files: impl IntoIterator<Item = &'a GeneratedFile>,
    ) {
        for (relative_path, contents) in files {
            self.add(join(directory, relative_path), contents.clone());
        }
    }

    /// The files, sorted by path.
    pub fn files(&self) -> &[GeneratedFile] {
        &self.files
    }

    /// Order the files by path, so the same schema always lays out the same
    /// package however the backends collected their pieces.
    pub(crate) fn sorted(mut self) -> Self {
        self.files.sort_by(|(left, _), (right, _)| left.cmp(right));
        self
    }

    /// Write the package into `output_path`, replacing what was there.
    ///
    /// The tree is built aside and swapped in, so a failure part-way leaves the
    /// previously generated client untouched.
    pub fn publish(&self, output_path: &str) -> Result<()> {
        publish_into(output_path, |directory| self.write_into(directory))
    }

    fn write_into(&self, directory: &Path) -> Result<()> {
        std::fs::create_dir_all(directory)
            .with_context(|| format!("Failed to create directory: {}", directory.display()))?;

        for (relative_path, contents) in &self.files {
            let file_path = directory.join(relative_path);
            if let Some(parent) = file_path.parent() {
                std::fs::create_dir_all(parent)
                    .with_context(|| format!("Failed to create directory: {}", parent.display()))?;
            }
            std::fs::write(&file_path, contents)
                .with_context(|| format!("Failed to write file: {}", file_path.display()))?;
        }

        Ok(())
    }
}

/// Join two relative path pieces with the separator generated paths use.
fn join(directory: &str, relative_path: &str) -> String {
    if directory.is_empty() {
        relative_path.to_string()
    } else {
        format!("{directory}/{relative_path}")
    }
}

#[cfg(test)]
mod tests {
    use super::GeneratedPackage;

    #[test]
    fn a_path_written_twice_keeps_the_last_contents() {
        let mut package = GeneratedPackage::default();
        package.add("models/user.py", "first");
        package.add("models/user.py", "second");

        assert_eq!(
            package.files(),
            &[("models/user.py".to_string(), "second".to_string())]
        );
    }

    #[test]
    fn files_are_ordered_by_path() {
        let mut package = GeneratedPackage::default();
        package.add("src/user.rs", "");
        package.add("Cargo.toml", "");
        package.add("src/lib.rs", "");

        let package = package.sorted();
        let names: Vec<&str> = package
            .files()
            .iter()
            .map(|(name, _)| name.as_str())
            .collect();
        assert_eq!(names, ["Cargo.toml", "src/lib.rs", "src/user.rs"]);
    }
}
