//! Windows Platform Backend - WASAPI
//!
//! Provides integration with Windows Audio Session API (WASAPI) for:
//! - Per-process audio capture via Process Loopback API (Build 20348+)
//! - System-wide audio loopback capture (fallback)
//! - Application and audio session enumeration
//! - Device enumeration and virtual device detection
//!
//! # Architecture
//!
//! ```text
//! WasapiBackend (main thread)
//!   │
//!   ├── WasapiThreadHandle ──► WASAPI Thread
//!   │     └── LoopbackCapture ──► Ring Buffer ──► AudioOutput
//!   │                                    │
//!   ├── DeviceEnumerator                 └─ DSP (EQ)
//!   ├── SessionEnumerator
//!   └── ProcessEnumerator
//! ```
//!
//! # Per-App Audio Capture
//!
//! On Windows 10 Build 20348+, per-process audio capture is supported via
//! `AUDIOCLIENT_PROCESS_LOOPBACK_PARAMS`. On older versions, Gecko falls
//! back to system-wide loopback capture.
//!
//! # Virtual Devices
//!
//! Unlike Linux (PipeWire), Windows cannot create virtual audio devices
//! at runtime - this requires a kernel driver. Gecko detects existing
//! virtual audio software (VB-Cable, Voicemeeter, etc.) instead.

pub mod com;
pub mod device;
pub mod message;
pub mod process;
pub mod session;
pub mod thread;
pub mod version;

#[cfg(test)]
mod tests;

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use tracing::{debug, info, warn};

use crate::error::PlatformError;
use crate::traits::*;

pub use device::DeviceEnumerator;
pub use message::{AudioProcessingState, AudioSessionInfo, DeviceFlow, DeviceInfo, SessionState};
pub use process::{ProcessEnumerator, ProcessInfo};
pub use session::SessionEnumerator;
pub use thread::WasapiThreadHandle;
pub use version::WindowsVersion;

/// WASAPI backend for Windows
///
/// Provides per-application audio capture using WASAPI Process Loopback API
/// (Windows 10 Build 20348+) with fallback to system-wide loopback.
///
/// # Capabilities
///
/// - Per-process audio capture (Build 20348+)
/// - System-wide loopback capture (fallback)
/// - Process enumeration with audio session filtering
/// - Virtual device detection (VB-Cable, Voicemeeter, etc.)
/// - Real-time DSP processing (EQ)
///
/// # Limitations
///
/// - Cannot create virtual devices (requires kernel driver)
/// - Cannot modify system audio routing graph
/// - Per-process capture unavailable on older Windows
///
/// # Thread Safety
///
/// Device enumeration creates COM objects on-demand that are not Send/Sync.
/// The backend itself is Send+Sync because the WASAPI thread handles all
/// COM operations through message passing.
pub struct WasapiBackend {
    /// Windows version info
    version: WindowsVersion,
    /// Whether backend is connected
    connected: bool,
    /// WASAPI thread handle (Send + Sync via channels)
    thread_handle: Option<WasapiThreadHandle>,
    /// PID to app name mapping for active captures
    pid_to_name: parking_lot::RwLock<HashMap<u32, String>>,
    /// App name to PID mapping
    name_to_pid: parking_lot::RwLock<HashMap<String, u32>>,
    /// Shared audio processing state
    processing_state: Option<Arc<AudioProcessingState>>,
}

impl WasapiBackend {
    /// Create a new WASAPI backend
    ///
    /// Initializes COM, detects Windows version, and spawns the WASAPI thread.
    #[cfg(target_os = "windows")]
    pub fn new() -> Result<Self, PlatformError> {
        info!("Initializing WASAPI backend");

        // Detect Windows version
        let version = WindowsVersion::current()?;
        info!("Detected {}", version);

        // Check WASAPI support
        if !version.supports_wasapi() {
            return Err(PlatformError::InitializationFailed(format!(
                "{} does not support WASAPI (requires Windows Vista+)",
                version
            )));
        }

        // Require per-process capture support (Windows 10 Build 20348+ / Windows 11)
        if !version.supports_process_loopback() {
            return Err(PlatformError::InitializationFailed(format!(
                "Gecko requires Windows 10 Build {}+ or Windows 11 for per-app audio capture.\n\
                 Your version: {} (Build {})\n\n\
                 Please upgrade to Windows 11 or Windows 10 Build 20348+.",
                WindowsVersion::MIN_PROCESS_LOOPBACK_BUILD,
                version.display_name(),
                version.build
            )));
        }

        info!("✓ Per-app capture supported (Process Loopback API available)");

        // Spawn WASAPI thread
        let thread_handle = WasapiThreadHandle::spawn()?;
        let processing_state = Some(Arc::clone(thread_handle.state()));

        info!("WASAPI backend initialized successfully");

        Ok(Self {
            version,
            connected: true,
            thread_handle: Some(thread_handle),
            pid_to_name: parking_lot::RwLock::new(HashMap::new()),
            name_to_pid: parking_lot::RwLock::new(HashMap::new()),
            processing_state,
        })
    }

    /// Stub for non-Windows platforms
    #[cfg(not(target_os = "windows"))]
    pub fn new() -> Result<Self, PlatformError> {
        Err(PlatformError::FeatureNotAvailable(
            "WASAPI only available on Windows".into(),
        ))
    }

    /// Check if per-process loopback is supported
    pub fn supports_process_loopback(&self) -> bool {
        self.version.supports_process_loopback()
    }

    /// Get Windows version information
    pub fn version(&self) -> &WindowsVersion {
        &self.version
    }

    /// Get shared audio processing state
    pub fn processing_state(&self) -> Option<&Arc<AudioProcessingState>> {
        self.processing_state.as_ref()
    }

    /// Start audio capture for an application
    ///
    /// On Build 20348+, uses per-process loopback.
    /// On older builds, uses system-wide loopback.
    pub fn start_capture(&mut self, app_name: &str, pid: u32) -> Result<(), PlatformError> {
        let thread = self.thread_handle.as_ref().ok_or_else(|| {
            PlatformError::Internal("WASAPI thread not running".into())
        })?;

        // Track the mapping
        self.pid_to_name.write().insert(pid, app_name.to_string());
        self.name_to_pid.write().insert(app_name.to_string(), pid);

        // Send command to WASAPI thread
        let target_pid = if self.supports_process_loopback() {
            Some(pid)
        } else {
            None // System-wide loopback
        };

        thread.send_command(message::WasapiCommand::StartCapture {
            pid: target_pid,
            app_name: app_name.to_string(),
        })?;

        // Wait for response
        match thread.recv_response_timeout(Duration::from_secs(5)) {
            Some(message::WasapiResponse::CaptureStarted { .. }) => {
                info!("Capture started for {} (PID {})", app_name, pid);
                Ok(())
            }
            Some(message::WasapiResponse::Error(e)) => {
                Err(PlatformError::Internal(e))
            }
            _ => Err(PlatformError::Internal("Timeout waiting for capture start".into())),
        }
    }

    /// Stop audio capture
    pub fn stop_capture(&mut self, pid: u32) -> Result<(), PlatformError> {
        let thread = self.thread_handle.as_ref().ok_or_else(|| {
            PlatformError::Internal("WASAPI thread not running".into())
        })?;

        // Remove mappings
        if let Some(name) = self.pid_to_name.write().remove(&pid) {
            self.name_to_pid.write().remove(&name);
        }

        thread.send_command(message::WasapiCommand::StopCapture { pid })?;

        Ok(())
    }

    /// Start audio output
    pub fn start_output(&mut self) -> Result<(), PlatformError> {
        let thread = self.thread_handle.as_ref().ok_or_else(|| {
            PlatformError::Internal("WASAPI thread not running".into())
        })?;

        thread.send_command(message::WasapiCommand::StartOutput)?;

        match thread.recv_response_timeout(Duration::from_secs(5)) {
            Some(message::WasapiResponse::OutputStarted) => {
                info!("Audio output started");
                Ok(())
            }
            Some(message::WasapiResponse::Error(e)) => {
                Err(PlatformError::Internal(e))
            }
            _ => Err(PlatformError::Internal("Timeout waiting for output start".into())),
        }
    }

    /// Stop audio output
    pub fn stop_output(&mut self) -> Result<(), PlatformError> {
        if let Some(thread) = &self.thread_handle {
            thread.send_command(message::WasapiCommand::StopOutput)?;
        }
        Ok(())
    }

    /// Set master volume (0.0 - 2.0)
    pub fn set_master_volume(&self, volume: f32) {
        if let Some(thread) = &self.thread_handle {
            let _ = thread.send_command(message::WasapiCommand::SetMasterVolume(volume));
        }
    }

    /// Set master bypass state
    pub fn set_master_bypass(&self, bypass: bool) {
        if let Some(thread) = &self.thread_handle {
            let _ = thread.send_command(message::WasapiCommand::SetMasterBypass(bypass));
        }
    }

    /// Set master EQ gains (10 bands)
    pub fn set_master_eq_gains(&self, gains: [f32; 10]) {
        if let Some(thread) = &self.thread_handle {
            let _ = thread.send_command(message::WasapiCommand::SetMasterEqGains(gains));
        }
    }

    /// Enumerate audio devices
    ///
    /// Creates a device enumerator on-demand (COM objects not stored).
    #[cfg(target_os = "windows")]
    pub fn enumerate_devices(&self) -> Result<Vec<DeviceInfo>, PlatformError> {
        use com::ComGuard;

        // Create COM context for this thread if needed
        let _com = ComGuard::new()?;
        let enumerator = DeviceEnumerator::new()?;
        enumerator.enumerate_all()
    }

    #[cfg(not(target_os = "windows"))]
    pub fn enumerate_devices(&self) -> Result<Vec<DeviceInfo>, PlatformError> {
        Err(PlatformError::FeatureNotAvailable(
            "Device enumeration only on Windows".into(),
        ))
    }

    /// Find virtual audio devices
    ///
    /// Creates a device enumerator on-demand (COM objects not stored).
    #[cfg(target_os = "windows")]
    pub fn find_virtual_devices(&self) -> Result<Vec<DeviceInfo>, PlatformError> {
        use com::ComGuard;

        let _com = ComGuard::new()?;
        let enumerator = DeviceEnumerator::new()?;
        enumerator.find_virtual_devices()
    }

    #[cfg(not(target_os = "windows"))]
    pub fn find_virtual_devices(&self) -> Result<Vec<DeviceInfo>, PlatformError> {
        Err(PlatformError::FeatureNotAvailable(
            "Virtual device detection only on Windows".into(),
        ))
    }

    /// Enumerate audio sessions
    #[cfg(target_os = "windows")]
    pub fn enumerate_audio_sessions(&self) -> Result<Vec<AudioSessionInfo>, PlatformError> {
        use com::ComGuard;

        let _com = ComGuard::new()?;
        let enumerator = SessionEnumerator::new();
        enumerator.enumerate_sessions()
    }

    #[cfg(not(target_os = "windows"))]
    pub fn enumerate_audio_sessions(&self) -> Result<Vec<AudioSessionInfo>, PlatformError> {
        Err(PlatformError::FeatureNotAvailable(
            "Audio sessions only on Windows".into(),
        ))
    }

    /// Get applications with active audio
    #[cfg(target_os = "windows")]
    pub fn list_audio_apps(&self) -> Result<Vec<ApplicationInfo>, PlatformError> {
        use com::ComGuard;

        let _com = ComGuard::new()?;
        let session_enum = SessionEnumerator::new();
        let sessions = session_enum.enumerate_sessions()?;

        let apps: Vec<ApplicationInfo> = sessions
            .into_iter()
            .filter(|s| s.state != SessionState::Expired)
            .map(|s| ApplicationInfo {
                pid: s.pid,
                name: s.name,
                icon: s.icon_path,
                is_active: s.state == SessionState::Active,
            })
            .collect();

        debug!("Found {} apps with audio sessions", apps.len());

        Ok(apps)
    }

    #[cfg(not(target_os = "windows"))]
    pub fn list_audio_apps(&self) -> Result<Vec<ApplicationInfo>, PlatformError> {
        Err(PlatformError::FeatureNotAvailable(
            "Application listing only on Windows".into(),
        ))
    }
}

impl Drop for WasapiBackend {
    fn drop(&mut self) {
        if let Some(mut thread) = self.thread_handle.take() {
            if let Err(e) = thread.shutdown() {
                warn!("Error shutting down WASAPI thread: {}", e);
            }
        }
    }
}

// ============================================================================
// PlatformBackend Trait Implementation
// ============================================================================

impl PlatformBackend for WasapiBackend {
    fn name(&self) -> &'static str {
        "WASAPI"
    }

    fn is_connected(&self) -> bool {
        self.connected && self.thread_handle.is_some()
    }

    fn list_applications(&self) -> Result<Vec<ApplicationInfo>, PlatformError> {
        #[cfg(target_os = "windows")]
        {
            self.list_audio_apps()
        }
        #[cfg(not(target_os = "windows"))]
        {
            Err(PlatformError::FeatureNotAvailable(
                "Application listing only on Windows".into(),
            ))
        }
    }

    fn list_nodes(&self) -> Result<Vec<AudioNode>, PlatformError> {
        // WASAPI doesn't have PipeWire-style nodes
        // Return devices as nodes for API compatibility
        #[cfg(target_os = "windows")]
        {
            // We need a mutable reference for enumerate_devices, but trait gives us &self
            // Return empty for now - full device enumeration available via enumerate_devices()
            Ok(Vec::new())
        }
        #[cfg(not(target_os = "windows"))]
        {
            Ok(Vec::new())
        }
    }

    fn list_ports(&self, _node_id: u32) -> Result<Vec<AudioPort>, PlatformError> {
        // WASAPI doesn't expose ports like PipeWire
        Ok(Vec::new())
    }

    fn list_links(&self) -> Result<Vec<LinkInfo>, PlatformError> {
        // WASAPI doesn't expose routing as links
        Ok(Vec::new())
    }

    fn create_virtual_sink(&mut self, _config: VirtualSinkConfig) -> Result<u32, PlatformError> {
        // Check for existing virtual devices
        #[cfg(target_os = "windows")]
        {
            let virtual_devs = self.find_virtual_devices()?;

            if virtual_devs.is_empty() {
                Err(PlatformError::FeatureNotAvailable(
                    "Virtual sink creation requires a kernel driver on Windows.\n\
                     Gecko cannot create virtual devices, but can use existing ones.\n\n\
                     Please install one of these virtual audio drivers:\n\
                     - VB-CABLE: https://vb-audio.com/Cable/\n\
                     - Virtual Audio Cable: https://vac.muzychenko.net/\n\
                     - Voicemeeter: https://vb-audio.com/Voicemeeter/"
                        .into(),
                ))
            } else {
                let names: Vec<&str> = virtual_devs.iter().map(|d| d.name.as_str()).collect();
                Err(PlatformError::FeatureNotAvailable(format!(
                    "Gecko detected existing virtual audio device(s): {}.\n\
                     Use these instead of creating new ones.",
                    names.join(", ")
                )))
            }
        }
        #[cfg(not(target_os = "windows"))]
        {
            Err(PlatformError::FeatureNotAvailable(
                "Virtual sinks not available".into(),
            ))
        }
    }

    fn destroy_virtual_sink(&mut self, _node_id: u32) -> Result<(), PlatformError> {
        Err(PlatformError::FeatureNotAvailable(
            "Cannot destroy driver-level virtual sinks on Windows".into(),
        ))
    }

    fn create_link(&mut self, _output_port: u32, _input_port: u32) -> Result<u32, PlatformError> {
        Err(PlatformError::FeatureNotAvailable(
            "Windows uses Process Loopback for audio routing, not graph links.\n\
             Use start_capture() to capture application audio."
                .into(),
        ))
    }

    fn destroy_link(&mut self, _link_id: u32) -> Result<(), PlatformError> {
        Err(PlatformError::FeatureNotAvailable(
            "Links not supported on Windows".into(),
        ))
    }

    fn route_application_to_sink(
        &mut self,
        app_name: &str,
        _sink_node_id: u32,
    ) -> Result<Vec<u32>, PlatformError> {
        info!("Routing application via loopback: {}", app_name);

        // Find the PID for this application
        #[cfg(target_os = "windows")]
        {
            let process_enum = ProcessEnumerator::new();
            let matches = process_enum.find_by_name(app_name)?;

            if matches.is_empty() {
                return Err(PlatformError::ApplicationNotFound(app_name.into()));
            }

            // Use first match
            let pid = matches[0].pid;
            self.start_capture(app_name, pid)?;

            // Also start output if not already running
            if let Some(state) = &self.processing_state {
                if !state.running.load(std::sync::atomic::Ordering::Relaxed) {
                    self.start_output()?;
                }
            }

            Ok(vec![pid])
        }
        #[cfg(not(target_os = "windows"))]
        {
            let _ = app_name;
            Err(PlatformError::FeatureNotAvailable(
                "Routing only on Windows".into(),
            ))
        }
    }

    fn default_output_node(&self) -> Result<u32, PlatformError> {
        // Return a placeholder ID
        // Full device info available via enumerate_devices()
        Ok(0)
    }

    fn default_input_node(&self) -> Result<u32, PlatformError> {
        // Return a placeholder ID
        Ok(0)
    }
}

// Tests are in tests.rs (imported via `mod tests;` at top of file)
