//! Gecko Platform - OS-Specific Audio Routing
//!
//! This crate provides platform-specific implementations for:
//! - Virtual audio device creation
//! - Per-application audio capture
//! - Audio graph manipulation
//!
//! # Platform Support
//!
//! | Platform | Backend    | Virtual Devices | Per-App Capture |
//! |----------|------------|-----------------|-----------------|
//! | Linux    | PipeWire   | Yes (runtime)   | Yes (linking)   |
//! | Windows  | WASAPI     | No (driver)     | Yes (API)       |
//! | macOS    | CoreAudio  | No (HAL plugin) | No              |
//!
//! # Architecture
//!
//! Each platform module implements the `PlatformBackend` trait, providing
//! a unified interface for the core engine while handling OS-specific details.

mod error;
mod traits;

// Linux module is always compiled (contains stub for non-pipewire builds)
#[cfg(target_os = "linux")]
pub mod linux;

#[cfg(target_os = "windows")]
pub mod windows;

#[cfg(target_os = "macos")]
pub mod macos;

pub use error::PlatformError;
pub use traits::{
    ApplicationInfo, AudioNode, AudioPort, LinkInfo, PlatformBackend, VirtualSinkConfig,
};

/// Get the platform backend for the current OS
///
/// Returns a boxed trait object that can be used for platform-specific operations.
pub fn get_backend() -> Result<Box<dyn PlatformBackend>, PlatformError> {
    #[cfg(target_os = "linux")]
    {
        #[cfg(feature = "pipewire")]
        {
            Ok(Box::new(linux::PipeWireBackend::new()?))
        }
        #[cfg(not(feature = "pipewire"))]
        {
            Ok(Box::new(linux::StubBackend::new()))
        }
    }

    #[cfg(target_os = "windows")]
    {
        Ok(Box::new(windows::WasapiBackend::new()?))
    }

    #[cfg(target_os = "macos")]
    {
        Ok(Box::new(macos::CoreAudioBackend::new()?))
    }

    #[cfg(not(any(target_os = "linux", target_os = "windows", target_os = "macos")))]
    {
        Err(PlatformError::UnsupportedPlatform)
    }
}

/// Check if the current platform supports virtual audio devices
pub fn supports_virtual_devices() -> bool {
    #[cfg(target_os = "linux")]
    {
        true // PipeWire can create virtual sinks at runtime
    }
    #[cfg(target_os = "windows")]
    {
        false // Requires kernel driver
    }
    #[cfg(target_os = "macos")]
    {
        false // Requires HAL plugin installation
    }
    #[cfg(not(any(target_os = "linux", target_os = "windows", target_os = "macos")))]
    {
        false
    }
}

/// Check if the current platform supports per-application audio capture
pub fn supports_per_app_capture() -> bool {
    #[cfg(target_os = "linux")]
    {
        true // PipeWire allows linking to specific nodes
    }
    #[cfg(target_os = "windows")]
    {
        true // AUDIOCLIENT_PROCESS_LOOPBACK_PARAMS (Win10 20348+)
    }
    #[cfg(target_os = "macos")]
    {
        false // No native support
    }
    #[cfg(not(any(target_os = "linux", target_os = "windows", target_os = "macos")))]
    {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_platform_capabilities() {
        // These should compile and return reasonable values
        let _ = supports_virtual_devices();
        let _ = supports_per_app_capture();
    }
}
