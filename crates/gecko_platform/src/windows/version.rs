//! Windows Version Detection
//!
//! Determines Windows version to check for Process Loopback API support.
//! Uses RtlGetVersion to bypass manifest-based version lying.
//!
//! # Process Loopback API Requirements
//!
//! - Windows 10 Build 20348+ (Windows 11 / Server 2022)
//! - Uses `AUDIOCLIENT_PROCESS_LOOPBACK_PARAMS`
//!
//! # Why RtlGetVersion?
//!
//! GetVersionEx lies based on app manifest compatibility settings.
//! RtlGetVersion returns the true OS version regardless of manifest.

use crate::error::PlatformError;

/// Windows version information
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WindowsVersion {
    /// Major version (10 for Windows 10/11)
    pub major: u32,
    /// Minor version (0 for Windows 10/11)
    pub minor: u32,
    /// Build number (e.g., 19045, 22000, 22621)
    pub build: u32,
}

impl WindowsVersion {
    /// Minimum build for WASAPI Process Loopback API
    pub const MIN_PROCESS_LOOPBACK_BUILD: u32 = 20348;

    /// Get the current Windows version using RtlGetVersion
    ///
    /// This bypasses manifest-based version lying that affects GetVersionEx.
    #[cfg(target_os = "windows")]
    pub fn current() -> Result<Self, PlatformError> {
        use std::mem;
        use windows::Win32::Foundation::STATUS_SUCCESS;
        use windows::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress};
        use windows::Win32::System::SystemInformation::OSVERSIONINFOEXW;

        // Rust pattern: unsafe block isolated to minimize unsafe scope
        // We use RtlGetVersion instead of GetVersionEx because GetVersionEx
        // returns fake versions based on app manifest compatibility settings
        unsafe {
            let ntdll = GetModuleHandleW(windows::core::w!("ntdll.dll")).map_err(|e| {
                PlatformError::InitializationFailed(format!("Failed to load ntdll.dll: {}", e))
            })?;

            let rtl_get_version = GetProcAddress(ntdll, windows::core::s!("RtlGetVersion"))
                .ok_or_else(|| {
                    PlatformError::InitializationFailed("RtlGetVersion not found in ntdll".into())
                })?;

            // Rust pattern: transmute to convert raw function pointer to typed fn pointer
            type RtlGetVersionFn = unsafe extern "system" fn(*mut OSVERSIONINFOEXW) -> i32;
            let rtl_get_version: RtlGetVersionFn = mem::transmute(rtl_get_version);

            let mut version_info: OSVERSIONINFOEXW = mem::zeroed();
            version_info.dwOSVersionInfoSize = mem::size_of::<OSVERSIONINFOEXW>() as u32;

            let status = rtl_get_version(&mut version_info);
            if status != STATUS_SUCCESS.0 {
                return Err(PlatformError::InitializationFailed(format!(
                    "RtlGetVersion failed with NTSTATUS: 0x{:08X}",
                    status
                )));
            }

            Ok(WindowsVersion {
                major: version_info.dwMajorVersion,
                minor: version_info.dwMinorVersion,
                build: version_info.dwBuildNumber,
            })
        }
    }

    /// Stub for non-Windows platforms (allows cross-compilation)
    #[cfg(not(target_os = "windows"))]
    pub fn current() -> Result<Self, PlatformError> {
        Err(PlatformError::FeatureNotAvailable(
            "Windows version detection only available on Windows".into(),
        ))
    }

    /// Check if per-process audio loopback capture is supported
    ///
    /// Requires Windows 10 Build 20348+ (Server 2022 / Windows 11 21H2 base)
    pub fn supports_process_loopback(&self) -> bool {
        // Windows 10 Build 20348+ or Windows 11+
        (self.major == 10 && self.build >= Self::MIN_PROCESS_LOOPBACK_BUILD) || self.major > 10
    }

    /// Check if WASAPI is available (Windows Vista+)
    pub fn supports_wasapi(&self) -> bool {
        // WASAPI introduced in Windows Vista (6.0)
        self.major >= 6
    }

    /// Check if this is Windows 11 (Build 22000+)
    pub fn is_windows_11(&self) -> bool {
        self.major == 10 && self.build >= 22000
    }

    /// Get a human-readable Windows version name
    pub fn display_name(&self) -> &'static str {
        match (self.major, self.minor, self.build) {
            (10, _, b) if b >= 22000 => "Windows 11",
            (10, _, _) => "Windows 10",
            (6, 3, _) => "Windows 8.1",
            (6, 2, _) => "Windows 8",
            (6, 1, _) => "Windows 7",
            (6, 0, _) => "Windows Vista",
            _ => "Unknown Windows",
        }
    }
}

impl std::fmt::Display for WindowsVersion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} (Build {})",
            self.display_name(),
            self.build
        )
    }
}

impl Default for WindowsVersion {
    fn default() -> Self {
        Self {
            major: 10,
            minor: 0,
            build: 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_process_loopback_support_detection() {
        // Old Windows 10 - no process loopback
        let win10_old = WindowsVersion {
            major: 10,
            minor: 0,
            build: 19041, // 20H1
        };
        assert!(!win10_old.supports_process_loopback());
        assert!(win10_old.supports_wasapi());
        assert!(!win10_old.is_windows_11());

        // Windows 10 with process loopback
        let win10_new = WindowsVersion {
            major: 10,
            minor: 0,
            build: 20348, // Server 2022 / exact minimum
        };
        assert!(win10_new.supports_process_loopback());

        // Windows 11
        let win11 = WindowsVersion {
            major: 10,
            minor: 0,
            build: 22000,
        };
        assert!(win11.supports_process_loopback());
        assert!(win11.is_windows_11());
    }

    #[test]
    fn test_display_format() {
        let version = WindowsVersion {
            major: 10,
            minor: 0,
            build: 19045,
        };
        let display = version.to_string();
        assert!(display.contains("Windows 10"));
        assert!(display.contains("19045"));
    }

    #[test]
    fn test_display_names() {
        assert_eq!(
            WindowsVersion { major: 10, minor: 0, build: 22631 }.display_name(),
            "Windows 11"
        );
        assert_eq!(
            WindowsVersion { major: 10, minor: 0, build: 19045 }.display_name(),
            "Windows 10"
        );
        assert_eq!(
            WindowsVersion { major: 6, minor: 1, build: 7601 }.display_name(),
            "Windows 7"
        );
    }

    #[test]
    #[cfg(target_os = "windows")]
    fn test_current_version_detection() {
        let version = WindowsVersion::current();
        assert!(version.is_ok(), "Should detect Windows version");

        let ver = version.unwrap();
        assert!(ver.major >= 10, "Should be Windows 10 or later");
        assert!(ver.supports_wasapi(), "Should support WASAPI");

        println!("Detected: {}", ver);
        println!("Process Loopback supported: {}", ver.supports_process_loopback());
    }

    #[test]
    #[cfg(not(target_os = "windows"))]
    fn test_current_version_non_windows() {
        let version = WindowsVersion::current();
        assert!(version.is_err(), "Should fail on non-Windows");
    }
}
