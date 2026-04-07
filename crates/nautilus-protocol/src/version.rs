//! Protocol versioning and compatibility checking.

use crate::error::{ProtocolError, Result};

/// Current Nautilus protocol version.
///
/// This version must be included in all client requests.
pub const PROTOCOL_VERSION: u32 = 1;

/// Minimum protocol version accepted by the engine.
pub const MIN_PROTOCOL_VERSION: u32 = 1;

/// Protocol version wrapper with validation.
///
/// Provides structured version checking as an alternative to comparing
/// against [`PROTOCOL_VERSION`] directly. The engine currently validates
/// versions inline, but consumers that prefer a typed wrapper can use this.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProtocolVersion(u32);

impl ProtocolVersion {
    /// Create a new protocol version.
    pub fn new(version: u32) -> Self {
        Self(version)
    }

    /// Get the version number.
    pub fn version(&self) -> u32 {
        self.0
    }

    /// Check if this version is compatible with the current protocol.
    /// Accepts any version in the range [MIN_PROTOCOL_VERSION, PROTOCOL_VERSION].
    pub fn is_compatible(&self) -> bool {
        self.0 >= MIN_PROTOCOL_VERSION && self.0 <= PROTOCOL_VERSION
    }

    /// Validate that this version is compatible, returning an error if not.
    pub fn validate(&self) -> Result<()> {
        if self.is_compatible() {
            Ok(())
        } else {
            Err(ProtocolError::UnsupportedProtocolVersion {
                actual: self.0,
                expected: PROTOCOL_VERSION,
            })
        }
    }
}

impl From<u32> for ProtocolVersion {
    fn from(version: u32) -> Self {
        Self(version)
    }
}

impl From<ProtocolVersion> for u32 {
    fn from(version: ProtocolVersion) -> Self {
        version.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_current_version_is_compatible() {
        let version = ProtocolVersion::new(PROTOCOL_VERSION);
        assert!(version.is_compatible());
        assert!(version.validate().is_ok());
    }

    #[test]
    fn test_v1_is_still_compatible() {
        let version = ProtocolVersion::new(1);
        assert!(version.is_compatible());
        assert!(version.validate().is_ok());
    }

    #[test]
    fn test_incompatible_version() {
        let version = ProtocolVersion::new(999);
        assert!(!version.is_compatible());
        assert!(version.validate().is_err());
    }

    #[test]
    fn test_version_zero_incompatible() {
        let version = ProtocolVersion::new(0);
        assert!(!version.is_compatible());
        assert!(version.validate().is_err());
    }

    #[test]
    fn test_version_conversion() {
        let v1 = ProtocolVersion::from(42);
        assert_eq!(v1.version(), 42);

        let v2: u32 = v1.into();
        assert_eq!(v2, 42);
    }

    #[test]
    fn test_version_error_message() {
        let version = ProtocolVersion::new(5);
        let result = version.validate();
        assert!(result.is_err());

        let err = result.unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("5"));
        assert!(msg.contains(&PROTOCOL_VERSION.to_string()));
    }
}
