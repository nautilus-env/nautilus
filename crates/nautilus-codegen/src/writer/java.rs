//! How a generated Java module is laid out.

use crate::package::GeneratedPackage;
use crate::GeneratedFile;

/// Lay out the generated Java module, preserving the relative file layout the
/// Java generator produced — `pom.xml` and the sources under their package
/// directories.
pub(crate) fn package(files: &[GeneratedFile]) -> GeneratedPackage {
    let mut package = GeneratedPackage::default();
    package.add_all("", files);
    package.sorted()
}
