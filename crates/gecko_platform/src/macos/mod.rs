//! macOS Platform Backend - CoreAudio Process Tap API
//!
//! On macOS 14.4+, Gecko uses the native `AudioHardwareCreateProcessTap` API
//! for per-app audio capture. This requires no driver installation.
//!
//! # Architecture
//!
//! ```text
//! Process Tap API ──► Direct per-app capture ──► DSP ──► Speakers
//! ```
//!
//! # Requirements
//!
//! - macOS 14.4 (Sonoma) or later
//! - User must grant audio capture permission when prompted
//!
//! # Submodules
//!
//! - `process_tap`: macOS 14.4+ Process Tap API for per-app capture
//! - `process_tap_ffi`: Raw FFI bindings to CoreAudio Process Tap functions
//! - `coreaudio`: Device enumeration and application listing
//! - `audio_output`: cpal-based audio output stream

// Make submodules public so they can be accessed from Tauri commands
pub mod audio_output;
pub mod coreaudio;
pub mod process_tap;
pub mod process_tap_ffi;
pub mod tap_description;
pub mod permissions;

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use tracing::{debug, info, trace, warn};

use crate::error::PlatformError;
use crate::traits::*;

// Re-export commonly used items at the module level for convenience
pub use audio_output::{
    AudioFormat, AudioMixer, AudioOutputStream, AudioProcessingState,
    get_callback_count, get_total_samples_mixed, get_mixer_debug_stats, reset_debug_counters,
};
pub use coreaudio::{
    get_system_volume, set_system_volume,
};
pub use process_tap::{
    AudioRingBuffer, ProcessTapCapture,
    is_process_tap_available, macos_version,
};
pub use permissions::{
    has_microphone_permission, request_microphone_permission,
};
pub use process_tap::{
    has_screen_recording_permission, probe_system_audio_permission,
    request_screen_recording_permission,
};

/// Get list of PIDs currently playing audio (safe wrapper around FFI)
///
/// Only processes returned by this function can be tapped with ProcessTapCapture.
/// This is the key filter - attempting to tap a process NOT in this list will fail.
pub fn get_audio_active_pids() -> Vec<i32> {
    unsafe { process_tap_ffi::get_audio_active_pids() }
}

/// Check if a specific PID is currently playing audio
///
/// Returns true if the process can be tapped, false otherwise.
pub fn is_pid_audio_active(pid: u32) -> bool {
    unsafe { process_tap_ffi::translate_pid_to_audio_object(pid as i32).is_some() }
}

/// CoreAudio backend for macOS
///
/// This backend uses the Process Tap API (macOS 14.4+) for per-app audio capture.
/// No driver installation is required.
///
/// # Per-App Audio Support
///
/// On macOS 14.4+, per-app audio capture is supported via the native Process Tap API.
/// On older macOS versions, per-app audio is NOT available - only master EQ works.
///
/// # Limitations
///
/// Safari, FaceTime, iMessage, and system sounds cannot be individually routed
/// due to macOS sandboxing. They DO receive master EQ when Gecko is default output.
pub struct CoreAudioBackend {
    /// Whether backend is connected and ready
    connected: bool,

    /// Whether Process Tap API is available (true on macOS 14.4+)
    process_tap_available: bool,

    /// Process Tap captures (macOS 14.4+ only)
    /// Key: process ID
    process_taps: HashMap<u32, ProcessTapCapture>,

    /// PID to app name mapping for captured apps
    /// Allows looking up app name by PID
    pid_to_name: HashMap<u32, String>,

    /// App name to PID mapping (reverse lookup)
    /// Allows looking up PID by app name
    name_to_pid: HashMap<String, u32>,

    /// Shutdown flag for background threads
    shutdown: Arc<AtomicBool>,

    /// Master volume (0.0 - 2.0)
    master_volume: f32,

    /// Global bypass state
    bypassed: bool,

    /// Per-app EQ gains (app_name -> [10 bands])
    app_eq_gains: HashMap<String, [f32; 10]>,

    /// Per-app volume (app_name -> volume 0.0-2.0)
    app_volumes: HashMap<String, f32>,

    /// Per-app bypass state
    app_bypassed: HashMap<String, bool>,

    /// Shared audio processing state (for DSP, spectrum analyzer, etc.)
    processing_state: Arc<AudioProcessingState>,
}

impl CoreAudioBackend {
    /// Create a new CoreAudio backend
    ///
    /// Checks if macOS 14.4+ is available for Process Tap API support.
    pub fn new() -> Result<Self, PlatformError> {
        let process_tap_available = process_tap::is_process_tap_available();

        if process_tap_available {
            info!("macOS 14.4+ detected - Process Tap API available for per-app capture");
        } else {
            let version = process_tap::macos_version();
            warn!(
                "macOS {}.{}.{} detected - Process Tap API requires macOS 14.4+. \
                 Per-app audio capture is NOT available. Only master EQ will work.",
                version.0, version.1, version.2
            );
        }

        Ok(Self {
            connected: true,
            process_tap_available,
            process_taps: HashMap::new(),
            pid_to_name: HashMap::new(),
            name_to_pid: HashMap::new(),
            shutdown: Arc::new(AtomicBool::new(false)),
            master_volume: 1.0,
            bypassed: false,
            app_eq_gains: HashMap::new(),
            app_volumes: HashMap::new(),
            app_bypassed: HashMap::new(),
            processing_state: Arc::new(AudioProcessingState::new()),
        })
    }

    /// Check if Process Tap API is available (macOS 14.4+)
    pub fn uses_process_tap(&self) -> bool {
        self.process_tap_available
    }

    /// Check if per-app audio capture is supported
    pub fn supports_per_app_capture(&self) -> bool {
        self.process_tap_available
    }

    /// Start audio capture for a specific application
    ///
    /// Requires macOS 14.4+ with Process Tap API.
    /// Returns error on older macOS versions.
    pub fn start_app_capture(&mut self, app_name: &str, pid: u32) -> Result<u32, PlatformError> {
        if !self.process_tap_available {
            return Err(PlatformError::FeatureNotAvailable(
                "Per-app audio capture requires macOS 14.4+. Please update your macOS.".into(),
            ));
        }

        // Check if already capturing this app
        if self.process_taps.contains_key(&pid) {
            debug!("Already capturing PID {}, skipping", pid);
            return Ok(self.process_taps.get(&pid).map(|t| t.tap_id()).unwrap_or(0));
        }

        // Create Process Tap and start capturing
        let mut tap = ProcessTapCapture::new(pid)?;
        let tap_id = tap.tap_id();

        // Start audio capture - this registers the IO proc and begins receiving audio
        tap.start()?;

        // Track PID-to-name mappings
        self.pid_to_name.insert(pid, app_name.to_string());
        self.name_to_pid.insert(app_name.to_string(), pid);
        self.process_taps.insert(pid, tap);

        debug!(
            "Started Process Tap capture for {} (PID: {}, Tap ID: {})",
            app_name, pid, tap_id
        );
        Ok(tap_id)
    }

    /// Stop audio capture for a specific application
    pub fn stop_app_capture(&mut self, pid: u32) -> Result<(), PlatformError> {
        if let Some(mut tap) = self.process_taps.remove(&pid) {
            // Explicitly stop before dropping (Drop also stops, but explicit is clearer)
            if let Err(e) = tap.stop() {
                warn!("Error stopping tap for PID {}: {}", pid, e);
            }
            drop(tap); // ProcessTapCapture::drop handles final cleanup

            // Clean up PID-name mappings
            if let Some(app_name) = self.pid_to_name.remove(&pid) {
                self.name_to_pid.remove(&app_name);
                debug!("Stopped Process Tap capture for {} (PID: {})", app_name, pid);
            } else {
                debug!("Stopped Process Tap capture for PID: {}", pid);
            }
        } else {
            debug!("No active tap found for PID: {}", pid);
        }
        Ok(())
    }

    /// Stop capture by app name (convenience method)
    pub fn stop_app_capture_by_name(&mut self, app_name: &str) -> Result<(), PlatformError> {
        if let Some(&pid) = self.name_to_pid.get(app_name) {
            self.stop_app_capture(pid)
        } else {
            debug!("No active capture found for app: {}", app_name);
            Ok(())
        }
    }

    /// Get app name for a PID
    pub fn get_app_name(&self, pid: u32) -> Option<&str> {
        self.pid_to_name.get(&pid).map(|s| s.as_str())
    }

    /// Get PID for an app name
    pub fn get_app_pid(&self, app_name: &str) -> Option<u32> {
        self.name_to_pid.get(app_name).copied()
    }

    /// Get list of captured app names
    pub fn captured_app_names(&self) -> Vec<&str> {
        self.pid_to_name.values().map(|s| s.as_str()).collect()
    }

    /// Set master volume (0.0 - 2.0)
    pub fn set_volume(&mut self, volume: f32) {
        self.master_volume = volume.clamp(0.0, 2.0);
    }

    /// Set global bypass state
    pub fn set_bypass(&mut self, bypassed: bool) {
        self.bypassed = bypassed;
    }

    /// Update master EQ band
    pub fn update_eq_band(&mut self, band: usize, gain_db: f32) {
        // TODO: Forward to audio processing thread
        trace!("Master EQ band {} set to {}dB", band, gain_db);
    }

    /// Update per-app EQ band
    pub fn update_stream_eq_band(&mut self, app_name: &str, band: usize, gain_db: f32) {
        if band < 10 {
            let gains = self.app_eq_gains.entry(app_name.to_string()).or_insert([0.0; 10]);
            gains[band] = gain_db;
            trace!("App '{}' EQ band {} set to {}dB", app_name, band, gain_db);
        }
    }

    /// Set per-app volume
    pub fn set_app_volume(&mut self, app_name: &str, volume: f32) {
        self.app_volumes.insert(app_name.to_string(), volume.clamp(0.0, 2.0));
    }

    /// Set per-app bypass
    pub fn set_app_bypass(&mut self, app_name: &str, bypassed: bool) {
        self.app_bypassed.insert(app_name.to_string(), bypassed);
    }

    /// Get current audio peaks (for level meters)
    pub fn get_peaks(&self) -> (f32, f32) {
        // TODO: Implement peak tracking from audio processing
        (0.0, 0.0)
    }

    /// Read audio samples from a specific app's tap
    ///
    /// Returns the number of samples actually read. Audio is interleaved stereo float.
    /// Returns 0 if the app is not being captured or no audio is available.
    pub fn read_app_audio(&self, pid: u32, buffer: &mut [f32]) -> usize {
        if let Some(tap) = self.process_taps.get(&pid) {
            tap.read_samples(buffer)
        } else {
            buffer.fill(0.0);
            0
        }
    }

    /// Read and mix audio from all active Process Taps into a single buffer
    ///
    /// This is useful for master processing - reads from all apps, mixes together.
    /// Returns the number of samples written (may be less than buffer size).
    pub fn read_all_apps_audio(&self, buffer: &mut [f32]) -> usize {
        buffer.fill(0.0);

        if self.process_taps.is_empty() {
            return 0;
        }

        // Temporary buffer for reading from each tap
        let mut tap_buffer = vec![0.0f32; buffer.len()];
        let mut max_samples = 0;

        for tap in self.process_taps.values() {
            let samples_read = tap.read_samples(&mut tap_buffer);
            if samples_read > 0 {
                // Mix into output buffer (additive mixing)
                for (out, &sample) in buffer.iter_mut().zip(tap_buffer.iter()) {
                    *out += sample;
                }
                max_samples = max_samples.max(samples_read);
            }
        }

        max_samples
    }

    /// Get number of active Process Tap captures
    pub fn active_tap_count(&self) -> usize {
        self.process_taps.len()
    }

    /// Get list of PIDs currently being captured
    pub fn captured_pids(&self) -> Vec<u32> {
        self.process_taps.keys().copied().collect()
    }

    /// Check if a specific PID is being captured
    pub fn is_capturing(&self, pid: u32) -> bool {
        self.process_taps.contains_key(&pid)
    }

    /// Get all active ring buffers with their PIDs
    ///
    /// Returns a vector of (PID, Arc<AudioRingBuffer>) tuples for all active captures.
    /// This allows the audio mixer to read from all captures and mix them.
    pub fn get_ring_buffers(&self) -> Vec<(u32, Arc<process_tap::AudioRingBuffer>)> {
        self.process_taps
            .iter()
            .map(|(&pid, tap)| (pid, tap.ring_buffer()))
            .collect()
    }

    /// Get a ring buffer for a specific PID
    pub fn get_ring_buffer(&self, pid: u32) -> Option<Arc<process_tap::AudioRingBuffer>> {
        self.process_taps.get(&pid).map(|tap| tap.ring_buffer())
    }

    /// Get debug stats for all active taps
    ///
    /// Returns a vector of debug strings showing callback/sample counts for each tap.
    /// Used to diagnose audio flow issues.
    pub fn get_tap_debug_stats(&self) -> Vec<String> {
        self.process_taps
            .values()
            .map(|tap| tap.debug_stats())
            .collect()
    }

    /// Log debug stats for all active taps (at trace level to avoid spam)
    pub fn log_tap_debug_stats(&self) {
        if !self.process_taps.is_empty() {
            let stats: Vec<String> = self.get_tap_debug_stats();
            for stat in stats {
                trace!("Tap stats: {}", stat);
            }
        }
    }

    /// Get the shared audio processing state
    ///
    /// This can be used to access DSP settings, spectrum data, etc.
    /// The engine can use this to create an AudioOutputStream with shared state.
    pub fn processing_state(&self) -> Arc<AudioProcessingState> {
        Arc::clone(&self.processing_state)
    }

    /// Get spectrum data for visualization
    pub fn get_spectrum(&self) -> [f32; gecko_dsp::NUM_BINS] {
        self.processing_state.get_spectrum()
    }

    /// Shutdown the backend
    pub fn shutdown(&mut self) {
        self.shutdown.store(true, Ordering::SeqCst);

        // Clean up Process Taps
        self.process_taps.clear();
        self.pid_to_name.clear();
        self.name_to_pid.clear();

        self.connected = false;
        info!("CoreAudio backend shut down");
    }
}

impl Default for CoreAudioBackend {
    fn default() -> Self {
        Self::new().expect("Failed to create CoreAudioBackend")
    }
}

impl Drop for CoreAudioBackend {
    fn drop(&mut self) {
        self.shutdown();
    }
}

impl PlatformBackend for CoreAudioBackend {
    fn name(&self) -> &'static str {
        if self.process_tap_available {
            "CoreAudio (Process Tap)"
        } else {
            "CoreAudio (No Per-App Support)"
        }
    }

    fn is_connected(&self) -> bool {
        self.connected
    }

    fn list_applications(&self) -> Result<Vec<ApplicationInfo>, PlatformError> {
        coreaudio::list_audio_applications()
    }

    fn list_nodes(&self) -> Result<Vec<AudioNode>, PlatformError> {
        coreaudio::list_audio_devices()
    }

    fn list_ports(&self, _node_id: u32) -> Result<Vec<AudioPort>, PlatformError> {
        // CoreAudio uses "streams" not "ports"
        Ok(Vec::new())
    }

    fn list_links(&self) -> Result<Vec<LinkInfo>, PlatformError> {
        // CoreAudio doesn't expose routing as links
        Ok(Vec::new())
    }

    fn create_virtual_sink(&mut self, config: VirtualSinkConfig) -> Result<u32, PlatformError> {
        if self.process_tap_available {
            // macOS 14.4+: No virtual sink needed - Process Tap captures directly
            trace!(
                "Virtual sink '{}' not needed on macOS 14.4+ (using Process Tap)",
                config.name
            );
            Ok(0) // Placeholder ID for Process Tap mode
        } else {
            Err(PlatformError::FeatureNotAvailable(
                "Virtual sink creation requires macOS 14.4+ with Process Tap API.".into(),
            ))
        }
    }

    fn destroy_virtual_sink(&mut self, _node_id: u32) -> Result<(), PlatformError> {
        Ok(())
    }

    fn create_link(&mut self, _output_port: u32, _input_port: u32) -> Result<u32, PlatformError> {
        Err(PlatformError::FeatureNotAvailable(
            "macOS uses Process Tap instead of links for audio routing.".into(),
        ))
    }

    fn destroy_link(&mut self, _link_id: u32) -> Result<(), PlatformError> {
        Err(PlatformError::FeatureNotAvailable(
            "Links not supported on macOS".into(),
        ))
    }

    fn route_application_to_sink(
        &mut self,
        app_name: &str,
        _sink_node_id: u32,
    ) -> Result<Vec<u32>, PlatformError> {
        if self.process_tap_available {
            trace!(
                "App '{}' will be captured via Process Tap",
                app_name
            );
            // Actual capture starts when we know the PID via start_app_capture()
            Ok(Vec::new())
        } else {
            Err(PlatformError::FeatureNotAvailable(
                "Per-app routing requires macOS 14.4+ with Process Tap API.".into(),
            ))
        }
    }

    fn default_output_node(&self) -> Result<u32, PlatformError> {
        coreaudio::get_default_output_device()
    }

    fn default_input_node(&self) -> Result<u32, PlatformError> {
        coreaudio::get_default_input_device()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_coreaudio_backend_creation() {
        let backend = CoreAudioBackend::new();
        assert!(backend.is_ok());
        let backend = backend.unwrap();
        assert!(backend.is_connected());
    }

    #[test]
    fn test_process_tap_detection() {
        // This test verifies the detection logic runs without panicking
        let available = process_tap::is_process_tap_available();
        println!("Process Tap API available: {}", available);
    }

    #[test]
    fn test_backend_name() {
        let backend = CoreAudioBackend::new().unwrap();
        let name = backend.name();
        assert!(name.contains("CoreAudio"));
    }

    #[test]
    fn test_set_volume() {
        let mut backend = CoreAudioBackend::new().unwrap();
        backend.set_volume(0.5);
        assert!((backend.master_volume - 0.5).abs() < 0.001);

        // Test clamping
        backend.set_volume(3.0);
        assert!((backend.master_volume - 2.0).abs() < 0.001);

        backend.set_volume(-1.0);
        assert!(backend.master_volume.abs() < 0.001);
    }

    #[test]
    fn test_per_app_eq() {
        let mut backend = CoreAudioBackend::new().unwrap();
        backend.update_stream_eq_band("Firefox", 0, 3.0);
        backend.update_stream_eq_band("Firefox", 5, -2.0);

        let gains = backend.app_eq_gains.get("Firefox").unwrap();
        assert!((gains[0] - 3.0).abs() < 0.001);
        assert!((gains[5] - (-2.0)).abs() < 0.001);
    }

    #[test]
    fn test_active_tap_count() {
        let backend = CoreAudioBackend::new().unwrap();
        assert_eq!(backend.active_tap_count(), 0);
        assert!(backend.captured_pids().is_empty());
    }

    #[test]
    fn test_is_capturing() {
        let backend = CoreAudioBackend::new().unwrap();
        assert!(!backend.is_capturing(12345));
    }

    #[test]
    fn test_read_app_audio_no_tap() {
        let backend = CoreAudioBackend::new().unwrap();
        let mut buffer = vec![1.0f32; 1024];
        let samples = backend.read_app_audio(99999, &mut buffer);
        assert_eq!(samples, 0);
        // Buffer should be zeroed
        assert!(buffer.iter().all(|&x| x == 0.0));
    }

    #[test]
    fn test_read_all_apps_no_taps() {
        let backend = CoreAudioBackend::new().unwrap();
        let mut buffer = vec![1.0f32; 1024];
        let samples = backend.read_all_apps_audio(&mut buffer);
        assert_eq!(samples, 0);
        // Buffer should be zeroed
        assert!(buffer.iter().all(|&x| x == 0.0));
    }
}
