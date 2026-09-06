//! Resolved Java package, build and interface settings.

use crate::extension_types::ExtensionRegistry;

pub(crate) const JACKSON_VERSION: &str = "2.17.2";

pub(super) const DEFAULT_MAVEN_VERSION: &str = "0.1.0-SNAPSHOT";

#[derive(Debug, Clone)]
pub(super) struct JavaConfig {
    pub(super) root_package: String,
    pub(super) group_id: String,
    pub(super) artifact_id: String,
    pub(super) version: String,
    pub(super) schema_path: String,
    pub(super) is_async: bool,
    pub(super) extensions: ExtensionRegistry,
}
