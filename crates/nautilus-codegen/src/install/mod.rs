//! Where a generated package goes once it exists.
//!
//! Generation produces files and a [`Delivery`] describing what has to happen
//! to them; this module is the only one that acts on it — writing the package
//! out, adding a crate to a Cargo workspace, copying a package into a
//! machine-wide location. Every language's rule about when installing is
//! appropriate is stated in the [`Delivery`] it asks for, not rediscovered
//! here.

pub(crate) mod js;
pub(crate) mod python;
pub(crate) mod rust;

use anyhow::{Context, Result};
use std::fs;
use std::path::{Path, PathBuf};

use crate::client::GeneratedClient;
use crate::java::build_java_bundle;
use crate::package::GeneratedPackage;
use crate::InstallMode;

/// What has to happen to a generated package.
pub(crate) enum Delivery {
    /// A Rust crate written to `path`, added to the nearest Cargo workspace
    /// when `integrate` — which is local, so it is not gated on the install
    /// mode being explicit.
    RustCrate { path: String, integrate: bool },
    /// A package that can also be installed into the machine-wide location its
    /// language imports from. Without a `path` there is nowhere to import it
    /// from, so installing is the only way to reach it.
    Installable {
        target: InstallTarget,
        path: Option<String>,
        install: InstallMode,
    },
    /// A Java module written to `path`, with the jar built when `bundle` names
    /// the artifact id to build it under.
    JavaModule {
        path: String,
        bundle: Option<String>,
    },
}

/// The machine-wide location a package installs into.
#[derive(Clone, Copy)]
pub(crate) enum InstallTarget {
    /// `site-packages/nautilus`.
    Python,
    /// `node_modules/nautilus`.
    JavaScript,
}

impl InstallTarget {
    /// The name of the staging directory used when there is no output path.
    fn staging_name(self) -> &'static str {
        match self {
            InstallTarget::Python => "nautilus_codegen_tmp",
            InstallTarget::JavaScript => "nautilus_codegen_js_tmp",
        }
    }

    fn install(self, package_path: &str, schema_path: &Path) -> Result<PathBuf> {
        match self {
            InstallTarget::Python => python::install(package_path),
            InstallTarget::JavaScript => js::install(package_path, schema_path),
        }
    }
}

/// A delivered client: the location to report, and what to tell the user.
///
/// `output` is `None` when the run had nowhere to write and nothing to install,
/// which the accompanying warning explains.
pub(crate) struct Delivered {
    pub(crate) output: Option<String>,
    pub(crate) warnings: Vec<String>,
}

/// Write `client` where its delivery says, installing when it asks.
pub(crate) fn deliver(client: &GeneratedClient, schema_path: &Path) -> Result<Delivered> {
    match &client.delivery {
        Delivery::RustCrate { path, integrate } => {
            client.package.publish(path)?;
            if *integrate {
                rust::integrate_package(path, schema_path)?;
            }
            Ok(Delivered {
                output: Some(path.clone()),
                warnings: Vec::new(),
            })
        }
        Delivery::Installable {
            target,
            path,
            install,
        } => deliver_installable(
            &client.package,
            *target,
            path.as_deref(),
            *install,
            schema_path,
        ),
        Delivery::JavaModule { path, bundle } => {
            client.package.publish(path)?;
            let output = match bundle {
                Some(artifact_id) => build_java_bundle(path, artifact_id)?.display().to_string(),
                None => path.clone(),
            };
            Ok(Delivered {
                output: Some(output),
                warnings: Vec::new(),
            })
        }
    }
}

/// Write the package and, when asked, install it.
///
/// Without a configured output path the package is built under a staging
/// directory of its own in the system temp directory, so two generations
/// running at once never share one, and the directory is removed whether or
/// not the install succeeded.
fn deliver_installable(
    package: &GeneratedPackage,
    target: InstallTarget,
    output_path: Option<&str>,
    install: InstallMode,
    schema_path: &Path,
) -> Result<Delivered> {
    let Some(output_path) = output_path else {
        if install == InstallMode::Never {
            return Ok(Delivered {
                output: None,
                warnings: vec![
                    "no output path specified and --no-install given; nothing written".to_string(),
                ],
            });
        }

        let staging = crate::publish::unique_dir(&std::env::temp_dir(), target.staging_name())?;
        let staging_path = staging.to_string_lossy().to_string();
        let installed = package
            .publish(&staging_path)
            .and_then(|()| target.install(&staging_path, schema_path));
        let _ = fs::remove_dir_all(&staging);
        return Ok(Delivered {
            output: Some(installed?.display().to_string()),
            warnings: Vec::new(),
        });
    };

    package.publish(output_path)?;
    if install != InstallMode::Always {
        return Ok(Delivered {
            output: Some(output_path.to_string()),
            warnings: Vec::new(),
        });
    }

    let installed = target.install(output_path, schema_path)?;
    Ok(Delivered {
        output: Some(installed.display().to_string()),
        warnings: vec![format!(
            "installed the generated client into {}, which every project on this machine shares",
            installed.display()
        )],
    })
}

/// Copy `src` onto `dst`, creating directories as needed.
pub(crate) fn copy_dir_recursive(src: &Path, dst: &Path) -> Result<()> {
    fs::create_dir_all(dst)
        .with_context(|| format!("Failed to create directory: {}", dst.display()))?;

    for entry in
        fs::read_dir(src).with_context(|| format!("Failed to read directory: {}", src.display()))?
    {
        let entry = entry.with_context(|| "Failed to read directory entry")?;
        let file_type = entry
            .file_type()
            .with_context(|| "Failed to get file type")?;
        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());

        if file_type.is_dir() {
            copy_dir_recursive(&src_path, &dst_path)?;
        } else {
            fs::copy(&src_path, &dst_path).with_context(|| {
                format!(
                    "Failed to copy {} -> {}",
                    src_path.display(),
                    dst_path.display()
                )
            })?;
        }
    }
    Ok(())
}
