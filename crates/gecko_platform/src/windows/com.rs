//! COM Initialization and Management
//!
//! Provides RAII-based COM (Component Object Model) initialization for WASAPI.
//!
//! # COM Threading Model
//!
//! WASAPI requires COM to be initialized on each thread that uses it.
//! We use apartment-threaded (STA) model which is required for most
//! Windows audio APIs.
//!
//! # Usage
//!
//! ```rust,no_run
//! use gecko_platform::windows::com::ComGuard;
//!
//! fn audio_thread() {
//!     // COM initialized for this thread
//!     let _com = ComGuard::new().expect("COM init failed");
//!
//!     // ... use WASAPI APIs ...
//!
//! } // COM automatically uninitialized when _com drops
//! ```

use crate::error::PlatformError;

/// RAII guard for COM initialization
///
/// Initializes COM when created, uninitializes when dropped.
/// Each thread that uses WASAPI must have its own ComGuard.
///
/// # Thread Safety
///
/// ComGuard is NOT Send or Sync - it must be created and dropped
/// on the same thread. This matches COM's threading requirements.
pub struct ComGuard {
    /// Marker to prevent Send/Sync (COM is thread-local)
    _not_send_sync: std::marker::PhantomData<*const ()>,
}

impl ComGuard {
    /// Initialize COM for the current thread
    ///
    /// Uses apartment-threaded model (COINIT_APARTMENTTHREADED) which is
    /// required for most Windows audio APIs including WASAPI.
    ///
    /// # Errors
    ///
    /// Returns error if COM initialization fails. Note that calling
    /// CoInitializeEx multiple times on the same thread is allowed
    /// (returns S_FALSE) as long as CoUninitialize is called the same
    /// number of times.
    #[cfg(target_os = "windows")]
    pub fn new() -> Result<Self, PlatformError> {
        use windows::Win32::System::Com::{
            CoInitializeEx, COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE,
        };

        // Rust pattern: unsafe block isolated and documented
        // SAFETY: CoInitializeEx is safe to call, and we track initialization
        // with the guard to ensure proper cleanup
        unsafe {
            // COINIT_APARTMENTTHREADED: Required for WASAPI
            // COINIT_DISABLE_OLE1DDE: Performance optimization (disables legacy DDE)
            let hr = CoInitializeEx(None, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE);

            // S_OK (0) = success, first init
            // S_FALSE (1) = success, already initialized (that's fine)
            // Negative = error
            if hr.is_err() {
                return Err(PlatformError::InitializationFailed(format!(
                    "COM initialization failed: {:?}",
                    hr
                )));
            }

            tracing::trace!("COM initialized for thread {:?}", std::thread::current().id());

            Ok(Self {
                _not_send_sync: std::marker::PhantomData,
            })
        }
    }

    /// Stub for non-Windows platforms
    #[cfg(not(target_os = "windows"))]
    pub fn new() -> Result<Self, PlatformError> {
        Err(PlatformError::FeatureNotAvailable(
            "COM only available on Windows".into(),
        ))
    }
}

impl Drop for ComGuard {
    #[cfg(target_os = "windows")]
    fn drop(&mut self) {
        use windows::Win32::System::Com::CoUninitialize;

        // SAFETY: We initialized COM in new(), so we must uninitialize
        unsafe {
            CoUninitialize();
        }

        tracing::trace!("COM uninitialized for thread {:?}", std::thread::current().id());
    }

    #[cfg(not(target_os = "windows"))]
    fn drop(&mut self) {
        // No-op on non-Windows
    }
}

/// Check if COM is initialized on the current thread
///
/// Useful for debugging/assertions.
#[cfg(target_os = "windows")]
pub fn is_com_initialized() -> bool {
    use windows::Win32::System::Com::{CoInitializeEx, CoUninitialize, COINIT_APARTMENTTHREADED};

    unsafe {
        // Try to initialize - if it returns S_FALSE, COM was already initialized
        let hr = CoInitializeEx(None, COINIT_APARTMENTTHREADED);

        if hr.is_ok() {
            // We just initialized it, so uninitialize and return false
            // (it wasn't initialized before our call)
            CoUninitialize();
            false
        } else {
            // S_FALSE means already initialized, which is what we want
            // But we still need to balance with CoUninitialize
            CoUninitialize();
            true
        }
    }
}

#[cfg(not(target_os = "windows"))]
pub fn is_com_initialized() -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[cfg(target_os = "windows")]
    fn test_com_initialization() {
        let guard = ComGuard::new();
        assert!(guard.is_ok(), "COM should initialize successfully");

        // Guard should be active
        let _g = guard.unwrap();

        // Creating another guard on the same thread should also succeed
        // (COM allows multiple init calls if balanced with uninit)
        let guard2 = ComGuard::new();
        assert!(guard2.is_ok(), "Second COM init should succeed");
    }

    #[test]
    #[cfg(target_os = "windows")]
    fn test_com_guard_drops_cleanly() {
        {
            let _guard = ComGuard::new().unwrap();
            // Guard in scope
        }
        // Guard dropped, COM uninitialized

        // Should be able to reinitialize
        let guard = ComGuard::new();
        assert!(guard.is_ok(), "Should reinitialize after drop");
    }

    #[test]
    #[cfg(not(target_os = "windows"))]
    fn test_com_not_available() {
        let result = ComGuard::new();
        assert!(result.is_err(), "COM should not be available on non-Windows");
    }
}
