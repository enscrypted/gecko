//! Platform Error Types

use thiserror::Error;

/// Errors from platform-specific operations
#[derive(Error, Debug)]
pub enum PlatformError {
    #[error("Platform not supported")]
    UnsupportedPlatform,

    #[error("Feature not available on this platform: {0}")]
    FeatureNotAvailable(String),

    #[error("Failed to connect to audio server: {0}")]
    ConnectionFailed(String),

    #[error("Failed to create virtual device: {0}")]
    VirtualDeviceCreationFailed(String),

    #[error("Application not found: {0}")]
    ApplicationNotFound(String),

    #[error("Failed to create link: {0}")]
    LinkCreationFailed(String),

    #[error("Node not found: {0}")]
    NodeNotFound(u32),

    #[error("Port not found: {0}")]
    PortNotFound(u32),

    #[error("Permission denied: {0}")]
    PermissionDenied(String),

    #[error("Backend initialization failed: {0}")]
    InitializationFailed(String),

    #[error("Internal error: {0}")]
    Internal(String),

    #[error("Invalid configuration: {0}")]
    InvalidConfiguration(String),

    #[error("Command failed: {0}")]
    CommandFailed(String),

    #[error("Serialization error: {0}")]
    SerializationError(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_display() {
        let err = PlatformError::ApplicationNotFound("Spotify".into());
        assert!(err.to_string().contains("Spotify"));
    }
}
