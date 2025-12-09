//! Linux Platform Backend - PipeWire
//!
//! Provides integration with PipeWire for:
//! - Runtime virtual sink creation (no kernel modules needed)
//! - Per-application audio routing via graph manipulation
//! - Application enumeration and monitoring
//!
//! # Architecture
//!
//! PipeWire objects are not `Send`/`Sync`, so all PipeWire operations run in a
//! dedicated thread. The `PipeWireBackend` communicates with this thread via
//! channels:
//!
//! ```text
//! Main Thread                     PipeWire Thread
//! ─────────────                   ───────────────
//! PipeWireBackend                 MainLoop::run()
//!   ├── state (Arc<RwLock>)  ◄──  Registry listener
//!   ├── command_tx ───────────►  Command handler
//!   └── response_rx ◄──────────  Response sender
//! ```

mod message;
mod state;

#[cfg(feature = "pipewire")]
mod audio_stream;
#[cfg(feature = "pipewire")]
mod filter;
#[cfg(feature = "pipewire")]
mod thread;

#[cfg(feature = "pipewire")]
pub use audio_stream::{AudioFormat, AudioProcessingState, StreamConfig};
#[cfg(feature = "pipewire")]
pub use filter::FilterState;

pub use state::{PipeWireState, PortDirection, PwLinkInfo, PwNodeInfo, PwPortInfo};

use crate::error::PlatformError;
use crate::traits::*;

/// Stub backend for when PipeWire feature is disabled
#[cfg(not(feature = "pipewire"))]
pub struct StubBackend;

#[cfg(not(feature = "pipewire"))]
impl StubBackend {
    pub fn new() -> Self {
        Self
    }
}

#[cfg(not(feature = "pipewire"))]
impl Default for StubBackend {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(not(feature = "pipewire"))]
impl PlatformBackend for StubBackend {
    fn name(&self) -> &'static str {
        "Linux Stub (PipeWire disabled)"
    }

    fn is_connected(&self) -> bool {
        false
    }

    fn list_applications(&self) -> Result<Vec<ApplicationInfo>, PlatformError> {
        Err(PlatformError::FeatureNotAvailable(
            "PipeWire feature not enabled".into(),
        ))
    }

    fn list_nodes(&self) -> Result<Vec<AudioNode>, PlatformError> {
        Err(PlatformError::FeatureNotAvailable(
            "PipeWire feature not enabled".into(),
        ))
    }

    fn list_ports(&self, _node_id: u32) -> Result<Vec<AudioPort>, PlatformError> {
        Err(PlatformError::FeatureNotAvailable(
            "PipeWire feature not enabled".into(),
        ))
    }

    fn list_links(&self) -> Result<Vec<LinkInfo>, PlatformError> {
        Err(PlatformError::FeatureNotAvailable(
            "PipeWire feature not enabled".into(),
        ))
    }

    fn create_virtual_sink(&mut self, _config: VirtualSinkConfig) -> Result<u32, PlatformError> {
        Err(PlatformError::FeatureNotAvailable(
            "PipeWire feature not enabled".into(),
        ))
    }

    fn destroy_virtual_sink(&mut self, _node_id: u32) -> Result<(), PlatformError> {
        Err(PlatformError::FeatureNotAvailable(
            "PipeWire feature not enabled".into(),
        ))
    }

    fn create_link(&mut self, _output_port: u32, _input_port: u32) -> Result<u32, PlatformError> {
        Err(PlatformError::FeatureNotAvailable(
            "PipeWire feature not enabled".into(),
        ))
    }

    fn destroy_link(&mut self, _link_id: u32) -> Result<(), PlatformError> {
        Err(PlatformError::FeatureNotAvailable(
            "PipeWire feature not enabled".into(),
        ))
    }

    fn route_application_to_sink(
        &mut self,
        _app_name: &str,
        _sink_node_id: u32,
    ) -> Result<Vec<u32>, PlatformError> {
        Err(PlatformError::FeatureNotAvailable(
            "PipeWire feature not enabled".into(),
        ))
    }

    fn default_output_node(&self) -> Result<u32, PlatformError> {
        Err(PlatformError::FeatureNotAvailable(
            "PipeWire feature not enabled".into(),
        ))
    }

    fn default_input_node(&self) -> Result<u32, PlatformError> {
        Err(PlatformError::FeatureNotAvailable(
            "PipeWire feature not enabled".into(),
        ))
    }
}

// ============================================================================
// PipeWire Backend Implementation
// ============================================================================

#[cfg(feature = "pipewire")]
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
#[cfg(feature = "pipewire")]
use std::sync::{Arc, RwLock};
#[cfg(feature = "pipewire")]
use std::thread::JoinHandle;
#[cfg(feature = "pipewire")]
use std::time::Duration;

#[cfg(feature = "pipewire")]
use crossbeam_channel::{bounded, Receiver};
#[cfg(feature = "pipewire")]
use pipewire as pw;

#[cfg(feature = "pipewire")]
use message::{PwCommand, PwResponse};

/// PipeWire backend implementation
///
/// This backend uses the PipeWire graph API to:
/// 1. Create virtual sinks at runtime (no drivers needed)
/// 2. Link application audio to Gecko's processing pipeline
/// 3. Monitor for new audio applications
/// 4. Stream audio through DSP and to output
///
/// # Thread Safety
///
/// PipeWire objects are not `Send`/`Sync`, so all operations happen in a
/// dedicated thread. This struct provides a thread-safe interface via channels.
#[cfg(feature = "pipewire")]
pub struct PipeWireBackend {
    /// Shared state snapshot (updated by PipeWire thread, read by trait methods)
    state: Arc<RwLock<PipeWireState>>,

    /// Shared audio processing state (volume, peaks, bypass)
    audio_state: Arc<AudioProcessingState>,

    /// Channel to send commands to PipeWire thread
    command_tx: pw::channel::Sender<PwCommand>,

    /// Channel to receive responses from PipeWire thread
    response_rx: Receiver<PwResponse>,

    /// Handle to the PipeWire thread
    thread_handle: Option<JoinHandle<()>>,

    /// Flag to signal shutdown
    shutdown: Arc<AtomicBool>,

    /// Counter for generating unique response IDs
    next_response_id: AtomicU64,

    /// IDs of virtual sinks we've created (for cleanup tracking)
    our_sinks: Vec<u32>,

    /// IDs of links we've created (for cleanup tracking)
    our_links: Vec<u32>,
}

#[cfg(feature = "pipewire")]
impl PipeWireBackend {
    /// Timeout for waiting on responses from the PipeWire thread
    const RESPONSE_TIMEOUT: Duration = Duration::from_secs(5);

    /// Create a new PipeWire backend
    ///
    /// This spawns a dedicated thread for PipeWire operations and connects
    /// to the PipeWire daemon. The backend will automatically detect audio
    /// applications and create per-app sinks for true per-app EQ processing.
    ///
    /// # Errors
    ///
    /// Returns an error if the PipeWire daemon is not running or connection fails.
    pub fn new() -> Result<Self, PlatformError> {
        Self::new_internal(false)
    }

    /// Create a query-only PipeWire backend
    ///
    /// Similar to `new()`, but does NOT create per-app sinks automatically.
    /// Use this when you only need to query the PipeWire graph (e.g., listing
    /// audio applications) without affecting the audio routing.
    ///
    /// # Errors
    ///
    /// Returns an error if the PipeWire daemon is not running or connection fails.
    pub fn new_query_only() -> Result<Self, PlatformError> {
        Self::new_internal(true)
    }

    /// Internal constructor with query_only flag
    fn new_internal(query_only: bool) -> Result<Self, PlatformError> {
        if query_only {
            tracing::debug!("Initializing PipeWire backend (query-only mode)");
        } else {
            tracing::info!("Initializing PipeWire backend");
        }

        // Create shared state
        let state = Arc::new(RwLock::new(PipeWireState::new()));

        // Create shared audio processing state
        let audio_state = Arc::new(AudioProcessingState::new());

        // Create shutdown flag
        let shutdown = Arc::new(AtomicBool::new(false));

        // Create channels for communication
        // Rust pattern: pipewire::channel integrates with MainLoop for wake-up
        let (pw_cmd_tx, pw_cmd_rx) = pw::channel::channel::<PwCommand>();

        // Rust pattern: bounded channel prevents unbounded memory growth
        let (response_tx, response_rx) = bounded::<PwResponse>(32);

        // Clone references for the thread
        let state_clone = Arc::clone(&state);
        let audio_state_clone = Arc::clone(&audio_state);
        let shutdown_clone = Arc::clone(&shutdown);

        // Spawn the PipeWire thread
        let thread_handle = std::thread::Builder::new()
            .name("pipewire-main".into())
            .spawn(move || {
                thread::pipewire_thread_main(
                    pw_cmd_rx,
                    response_tx,
                    state_clone,
                    audio_state_clone,
                    shutdown_clone,
                    query_only,
                );
            })
            .map_err(|e| PlatformError::Internal(format!("Failed to spawn thread: {}", e)))?;

        // Wait a moment for connection to establish
        std::thread::sleep(Duration::from_millis(100));

        // Check if connected
        let connected = state
            .read()
            .map(|s| s.connected)
            .unwrap_or(false);

        if !connected {
            tracing::warn!("PipeWire connection not yet established, continuing anyway");
        }

        Ok(Self {
            state,
            audio_state,
            command_tx: pw_cmd_tx,
            response_rx,
            thread_handle: Some(thread_handle),
            shutdown,
            next_response_id: AtomicU64::new(1),
            our_sinks: Vec::new(),
            our_links: Vec::new(),
        })
    }

    /// Generate a unique response ID for request correlation
    fn next_id(&self) -> u64 {
        self.next_response_id.fetch_add(1, Ordering::SeqCst)
    }

    /// Send a command and wait for the correlated response
    fn send_and_wait(&self, cmd: PwCommand, expected_id: u64) -> Result<PwResponse, PlatformError> {
        // Send the command
        self.command_tx
            .send(cmd)
            .map_err(|_| PlatformError::Internal("PipeWire thread channel closed".into()))?;

        // Wait for the response with timeout
        match self.response_rx.recv_timeout(Self::RESPONSE_TIMEOUT) {
            Ok(response) => {
                if response.response_id() == expected_id {
                    Ok(response)
                } else {
                    // Response ID mismatch - this shouldn't happen in normal operation
                    Err(PlatformError::Internal(format!(
                        "Response ID mismatch: expected {}, got {}",
                        expected_id,
                        response.response_id()
                    )))
                }
            }
            Err(crossbeam_channel::RecvTimeoutError::Timeout) => {
                Err(PlatformError::Internal("Timeout waiting for PipeWire response".into()))
            }
            Err(crossbeam_channel::RecvTimeoutError::Disconnected) => {
                Err(PlatformError::Internal("PipeWire thread disconnected".into()))
            }
        }
    }

    // === Audio Streaming Methods ===

    /// Start audio streaming (capture from virtual sink, process through DSP, output to speakers)
    ///
    /// # Arguments
    /// * `sink_id` - The node ID of the virtual sink to capture from
    /// * `playback_target` - Optional node ID to output to (None = default output)
    ///
    /// # Returns
    /// Ok(()) on success, error if streaming fails to start
    pub fn start_streaming(&self, sink_id: u32, playback_target: Option<u32>) -> Result<(), PlatformError> {
        let response_id = self.next_id();

        let response = self.send_and_wait(
            PwCommand::StartStreaming {
                capture_target: sink_id,
                playback_target,
                response_id,
            },
            response_id,
        )?;

        match response {
            PwResponse::StreamingStarted { .. } => Ok(()),
            PwResponse::Error { message, .. } => {
                Err(PlatformError::Internal(format!("Failed to start streaming: {}", message)))
            }
            _ => Err(PlatformError::Internal("Unexpected response type".into())),
        }
    }

    /// Stop audio streaming
    pub fn stop_streaming(&self) -> Result<(), PlatformError> {
        let response_id = self.next_id();

        let response = self.send_and_wait(
            PwCommand::StopStreaming { response_id },
            response_id,
        )?;

        match response {
            PwResponse::StreamingStopped { .. } | PwResponse::Ok { .. } => Ok(()),
            PwResponse::Error { message, .. } => {
                Err(PlatformError::Internal(format!("Failed to stop streaming: {}", message)))
            }
            _ => Err(PlatformError::Internal("Unexpected response type".into())),
        }
    }

    /// Switch playback to a new target device without stopping capture
    ///
    /// This is used during device hotplug (e.g., headphones plugged/unplugged)
    /// to seamlessly switch the output device while keeping the virtual sink
    /// and capture stream alive. Only the playback stream is reconnected.
    ///
    /// Uses device NAME instead of ID to avoid race conditions during hotplug.
    /// When a device is plugged in, it gets a new node ID, but PipeWire can
    /// resolve the name to the current ID at connection time.
    ///
    /// # Arguments
    /// * `target_name` - The name of the target device (e.g., "alsa_output.usb-...")
    ///
    /// # Returns
    /// Ok(()) on success, error if switch fails
    pub fn switch_playback_target(&self, target_name: &str) -> Result<(), PlatformError> {
        let response_id = self.next_id();

        let response = self.send_and_wait(
            PwCommand::SwitchPlaybackTarget {
                target_name: target_name.to_string(),
                response_id,
            },
            response_id,
        )?;

        match response {
            PwResponse::PlaybackTargetSwitched { .. } | PwResponse::Ok { .. } => Ok(()),
            PwResponse::Error { message, .. } => {
                Err(PlatformError::Internal(format!("Failed to switch playback target: {}", message)))
            }
            _ => Err(PlatformError::Internal("Unexpected response type".into())),
        }
    }

    /// Update EQ band gain (fire-and-forget, real-time safe)
    pub fn update_eq_band(&self, band: usize, gain_db: f32) {
        let _ = self.command_tx.send(PwCommand::UpdateEqBand { band, gain_db });
    }

    /// Update per-app EQ band gain (fire-and-forget, real-time safe)
    ///
    /// This is TRUE per-app EQ - each app has its own independent EQ instance
    /// that processes audio BEFORE mixing. This is NOT additive to master EQ.
    ///
    /// # Arguments
    /// * `stream_id` - Stream ID (format: "pid:name" or just "name")
    /// * `band` - EQ band index (0-9)
    /// * `gain_db` - Gain in dB (-24 to +24)
    pub fn update_stream_eq_band(&self, stream_id: &str, band: usize, gain_db: f32) {
        // Extract app name from stream_id (format: "pid:name" or just "name")
        // The app_captures HashMap is keyed by app name, not stream ID
        let app_name = if stream_id.contains(':') {
            // Format is "pid:name" - take everything after the first colon
            stream_id.split_once(':').map(|(_, name)| name).unwrap_or(stream_id)
        } else {
            stream_id
        };

        // Update shared state so future streams pick it up
        self.audio_state.set_stream_eq_offset(app_name, band, gain_db);

        // Send to PipeWire thread which updates the per-app capture's atomic EQ gains
        let _ = self.command_tx.send(PwCommand::UpdateAppEqBand {
            app_name: app_name.to_string(),
            band,
            gain_db,
        });
    }

    /// Set bypass state for a specific application (fire-and-forget, real-time safe)
    ///
    /// When bypassed, the app's audio passes through without EQ processing.
    /// Master EQ is still applied after mixing if not globally bypassed.
    ///
    /// # Arguments
    /// * `app_name` - Application name (e.g., "Firefox", "Spotify")
    /// * `bypassed` - Whether to bypass EQ for this app
    pub fn set_app_bypass(&self, app_name: &str, bypassed: bool) {
        // Update shared state so future streams pick it up
        self.audio_state.set_stream_bypass(app_name, bypassed);

        let _ = self.command_tx.send(PwCommand::SetAppBypass {
            app_name: app_name.to_string(),
            bypassed,
        });
    }

    /// Set per-app volume (fire-and-forget, real-time safe)
    ///
    /// This volume is applied after per-app EQ and before mixing.
    /// It's independent of master volume.
    ///
    /// # Arguments
    /// * `app_name` - Application name (e.g., "Firefox", "Spotify")
    /// * `volume` - Volume level (0.0 = silent, 1.0 = unity, 2.0 = +6dB boost)
    pub fn set_app_volume(&self, app_name: &str, volume: f32) {
        // Update shared state so future streams pick it up
        self.audio_state.set_stream_volume(app_name, volume);

        let _ = self.command_tx.send(PwCommand::SetAppVolume {
            app_name: app_name.to_string(),
            volume,
        });
    }

    /// Set master volume (fire-and-forget, real-time safe)
    pub fn set_volume(&self, volume: f32) {
        let _ = self.command_tx.send(PwCommand::SetVolume(volume));
    }

    /// Get the volume of the "Gecko Audio" PipeWire sink
    /// 
    /// Uses wpctl to query the sink volume. Returns volume as 0.0-1.0+ 
    /// (can exceed 1.0 if boosted above 100%)
    pub fn get_sink_volume(&self) -> Result<f32, PlatformError> {
        use std::process::Command;
        
        // First, find our Gecko Audio sink ID
        let list_output = Command::new("wpctl")
            .args(["status"])
            .output()
            .map_err(|e| PlatformError::Internal(format!("Failed to run wpctl status: {}", e)))?;
        
        let list_str = String::from_utf8_lossy(&list_output.stdout);
        
        // Find line containing "Gecko Audio" and extract ID
        // Format: "  42. Gecko Audio [vol: 1.00]"
        let mut sink_id: Option<u32> = None;
        for line in list_str.lines() {
            if line.contains("Gecko Audio") && !line.contains("Monitor") {
                // Extract the number at the start (format: "  42. Gecko Audio")
                let trimmed = line.trim();
                if let Some(dot_pos) = trimmed.find('.') {
                    if let Ok(id) = trimmed[..dot_pos].trim().parse::<u32>() {
                        sink_id = Some(id);
                        break;
                    }
                }
            }
        }
        
        let sink_id = sink_id.ok_or_else(|| {
            PlatformError::Internal("Gecko Audio sink not found in wpctl status".into())
        })?;
        
        // Get volume for this sink
        let vol_output = Command::new("wpctl")
            .args(["get-volume", &sink_id.to_string()])
            .output()
            .map_err(|e| PlatformError::Internal(format!("Failed to get volume: {}", e)))?;
        
        let vol_str = String::from_utf8_lossy(&vol_output.stdout);
        // Format: "Volume: 1.00" or "Volume: 0.50 [MUTED]"
        if let Some(vol_start) = vol_str.find("Volume:") {
            let after_colon = &vol_str[vol_start + 7..];
            let vol_part = after_colon.split_whitespace().next().unwrap_or("1.0");
            if let Ok(vol) = vol_part.parse::<f32>() {
                return Ok(vol);
            }
        }
        
        Ok(1.0) // Default to 1.0 if parsing fails
    }
    
    /// Set the volume of the "Gecko Audio" PipeWire sink
    /// 
    /// Uses wpctl to set sink volume. Also updates internal master volume.
    /// Volume is 0.0-1.0+ (can exceed 1.0 for boost)
    pub fn set_sink_volume(&self, volume: f32) -> Result<(), PlatformError> {
        use std::process::Command;
        
        // Find our Gecko Audio sink ID
        let list_output = Command::new("wpctl")
            .args(["status"])
            .output()
            .map_err(|e| PlatformError::Internal(format!("Failed to run wpctl status: {}", e)))?;
        
        let list_str = String::from_utf8_lossy(&list_output.stdout);
        
        let mut sink_id: Option<u32> = None;
        for line in list_str.lines() {
            if line.contains("Gecko Audio") && !line.contains("Monitor") {
                let trimmed = line.trim();
                if let Some(dot_pos) = trimmed.find('.') {
                    if let Ok(id) = trimmed[..dot_pos].trim().parse::<u32>() {
                        sink_id = Some(id);
                        break;
                    }
                }
            }
        }
        
        let sink_id = sink_id.ok_or_else(|| {
            PlatformError::Internal("Gecko Audio sink not found".into())
        })?;
        
        // Set volume (wpctl expects percentage like "1.0" for 100%, "0.5" for 50%)
        let vol_str = format!("{:.2}", volume.clamp(0.0, 1.5)); // Cap at 150% for safety
        let status = Command::new("wpctl")
            .args(["set-volume", &sink_id.to_string(), &vol_str])
            .status()
            .map_err(|e| PlatformError::Internal(format!("Failed to set volume: {}", e)))?;
        
        if !status.success() {
            return Err(PlatformError::Internal("wpctl set-volume failed".into()));
        }
        
        // Also update internal master volume for DSP processing
        self.audio_state.set_master_volume(volume);
        let _ = self.command_tx.send(PwCommand::SetVolume(volume));
        
        Ok(())
    }

    /// Set bypass state (fire-and-forget, real-time safe)
    pub fn set_bypass(&self, bypassed: bool) {
        let _ = self.command_tx.send(PwCommand::SetBypass(bypassed));
    }

    /// Enable/disable soft clipping (limiter)
    pub fn set_soft_clip_enabled(&self, enabled: bool) {
        self.audio_state.set_soft_clip_enabled(enabled);
    }

    /// Get current peak levels (left, right) from the audio processing state
    pub fn get_peaks(&self) -> (f32, f32) {
        self.audio_state.peaks()
    }

    /// Update the spectrum analyzer and check if new FFT data is available
    ///
    /// Call this periodically from the UI thread (e.g., 30fps).
    /// Returns true if new spectrum data is ready.
    pub fn update_spectrum(&self) -> bool {
        self.audio_state.update_spectrum()
    }

    /// Get the current spectrum data (32 bins, 0.0-1.0 magnitude)
    ///
    /// Returns logarithmically-spaced frequency bins from ~20Hz to 20kHz.
    pub fn get_spectrum(&self) -> [f32; 32] {
        self.audio_state.get_spectrum()
    }

    /// Get reference to the shared audio processing state
    pub fn audio_state(&self) -> &Arc<AudioProcessingState> {
        &self.audio_state
    }

    /// Get the list of apps currently being captured with per-app EQ
    pub fn get_captured_apps(&self) -> Vec<String> {
        self.audio_state.get_captured_apps()
    }

    /// Get the version counter for captured apps (for change detection)
    pub fn captured_apps_version(&self) -> u32 {
        self.audio_state.captured_apps_version()
    }

    /// Set the default audio sink to our virtual sink
    /// This makes all applications automatically output to Gecko Audio
    ///
    /// # Arguments
    /// * `sink_name` - The name of the sink to set as default (e.g., "Gecko Audio")
    ///
    /// # Returns
    /// The previous default sink name (so it can be restored on stop)
    pub fn set_default_sink(&self, sink_name: &str) -> Result<Option<String>, PlatformError> {
        use std::process::Command;

        // First, get the current default sink so we can restore it later
        let output = Command::new("pw-metadata")
            .args(["-n", "default"])
            .output()
            .map_err(|e| PlatformError::Internal(format!("Failed to run pw-metadata: {}", e)))?;

        let stdout = String::from_utf8_lossy(&output.stdout);
        let previous_sink = Self::parse_sink_from_metadata(&stdout, "default.audio.sink");

        tracing::debug!(
            "Changing default sink from {:?} to '{}'",
            previous_sink,
            sink_name
        );

        // Set the new default sink using pw-metadata
        // Format: pw-metadata -n default 0 default.audio.sink '{"name":"sink_name"}'
        let value = format!(r#"{{"name":"{}"}}"#, sink_name);

        let status = Command::new("pw-metadata")
            .args([
                "-n", "default",
                "0",
                "default.audio.sink",
                &value,
            ])
            .status()
            .map_err(|e| PlatformError::Internal(format!("Failed to set default sink: {}", e)))?;

        if !status.success() {
            return Err(PlatformError::Internal(format!(
                "pw-metadata failed with exit code: {:?}",
                status.code()
            )));
        }

        tracing::debug!("Successfully set default sink to '{}'", sink_name);
        Ok(previous_sink)
    }

    /// Restore the default audio sink to its original value
    pub fn restore_default_sink(&self, sink_name: &str) -> Result<(), PlatformError> {
        use std::process::Command;

        tracing::debug!("Restoring default sink to '{}'", sink_name);

        let value = format!(r#"{{"name":"{}"}}"#, sink_name);

        let status = Command::new("pw-metadata")
            .args([
                "-n", "default",
                "0",
                "default.audio.sink",
                &value,
            ])
            .status()
            .map_err(|e| PlatformError::Internal(format!("Failed to restore default sink: {}", e)))?;

        if !status.success() {
            return Err(PlatformError::Internal(format!(
                "pw-metadata failed with exit code: {:?}",
                status.code()
            )));
        }

        Ok(())
    }

    /// Parse a sink name from pw-metadata output for a given key
    fn parse_sink_from_metadata(output: &str, key: &str) -> Option<String> {
        // Look for: key:'<key>' value:'{"name":"..."}'
        for line in output.lines() {
            if line.contains(key) && line.contains("value:") {
                // Extract the JSON value
                if let Some(start) = line.find(r#"{"name":""#) {
                    let after_name = &line[start + 9..]; // Skip {"name":"
                    if let Some(end) = after_name.find('"') {
                        return Some(after_name[..end].to_string());
                    }
                }
            }
        }
        None
    }

    /// Get the current default sink name (what's currently active)
    pub fn get_default_sink_name(&self) -> Result<Option<String>, PlatformError> {
        use std::process::Command;

        let output = Command::new("pw-metadata")
            .args(["-n", "default"])
            .output()
            .map_err(|e| PlatformError::Internal(format!("Failed to run pw-metadata: {}", e)))?;

        let stdout = String::from_utf8_lossy(&output.stdout);
        Ok(Self::parse_sink_from_metadata(&stdout, "default.audio.sink"))
    }

    /// Get the user's configured default sink (their preference, ignoring temporary changes)
    ///
    /// This returns what the user actually configured as their preferred output device,
    /// regardless of what Gecko or other applications have temporarily set as the active sink.
    /// This is critical for restoring audio to the correct device when Gecko stops.
    pub fn get_configured_default_sink(&self) -> Result<Option<String>, PlatformError> {
        use std::process::Command;

        let output = Command::new("pw-metadata")
            .args(["-n", "default"])
            .output()
            .map_err(|e| PlatformError::Internal(format!("Failed to run pw-metadata: {}", e)))?;

        let stdout = String::from_utf8_lossy(&output.stdout);
        Ok(Self::parse_sink_from_metadata(
            &stdout,
            "default.configured.audio.sink",
        ))
    }

    /// Get node ID by name
    pub fn get_node_id_by_name(&self, name: &str) -> Result<Option<u32>, PlatformError> {
        let state = self
            .state
            .read()
            .map_err(|_| PlatformError::Internal("State lock poisoned".into()))?;

        Ok(state
            .nodes
            .values()
            .find(|n| n.name == name)
            .map(|n| n.id))
    }

    /// Move a specific stream to a target sink
    pub fn move_stream_to_sink(&self, stream_id: u32, sink_id: u32) -> Result<(), PlatformError> {
        use std::process::Command;

        // 1. Find output ports of the stream
        let stream_ports = self.get_node_ports(stream_id, "Output")?;
        if stream_ports.is_empty() {
            return Err(PlatformError::Internal(format!("Stream {} has no output ports", stream_id)));
        }

        // 2. Find input ports of the target sink
        let sink_ports = self.get_node_ports(sink_id, "Input")?;
        if sink_ports.is_empty() {
            return Err(PlatformError::Internal(format!("Sink {} has no input ports", sink_id)));
        }

        // 3. Early-out: Check if already correctly routed to avoid unnecessary link churn
        // This prevents aggressive enforcement from constantly recreating links
        let links = self.list_links()?;
        let expected_link_count = stream_ports.len().min(sink_ports.len());
        let mut correct_links = 0;

        for (i, stream_port) in stream_ports.iter().enumerate() {
            if i >= sink_ports.len() {
                break;
            }
            let sink_port = sink_ports[i];
            // Check if this exact link already exists
            if links.iter().any(|l| l.output_port == *stream_port && l.input_port == sink_port) {
                correct_links += 1;
            }
        }

        if correct_links == expected_link_count {
            // Already correctly routed, nothing to do
            return Ok(());
        }

        tracing::debug!("Moving stream {} to sink {} ({}/{} links correct)",
                       stream_id, sink_id, correct_links, expected_link_count);

        // 4. Disconnect existing links from these stream ports (Exclusive Routing)
        // We parse `pw-link -l` to find what these ports are currently connected to.
        if let Ok(output) = Command::new("pw-link").arg("-l").output() {
            let output_str = String::from_utf8_lossy(&output.stdout);
            for line in output_str.lines() {
                // Format: "out_node_id:out_port_id -> in_node_id:in_port_id"
                // Example: "93:113 -> 107:92"
                if let Some((left, right)) = line.split_once(" -> ") {
                    if let Some((node_str, port_str)) = left.split_once(':') {
                        if let Ok(node) = node_str.trim().parse::<u32>() {
                            if node == stream_id {
                                // This link originates from our stream.
                                // Check if it's already connected to the target sink?
                                // To do that we'd need to parse the right side node ID.
                                let should_disconnect = if let Some((target_node_str, _)) = right.split_once(':') {
                                    if let Ok(target_node) = target_node_str.trim().parse::<u32>() {
                                        target_node != sink_id
                                    } else {
                                        true
                                    }
                                } else {
                                    true
                                };

                                if should_disconnect {
                                    // Extract port IDs for disconnection
                                    if let (Ok(out_port), Ok(in_port)) = (
                                        port_str.trim().parse::<u32>(),
                                        right.split(':').nth(1).unwrap_or("0").trim().parse::<u32>()
                                    ) {
                                        tracing::debug!("Disconnecting existing link: {} -> {}", out_port, in_port);
                                        let _ = Command::new("pw-link")
                                            .args(["-d", &out_port.to_string(), &in_port.to_string()])
                                            .status();
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        // 4. Connect to new sink
        for (i, stream_port_id) in stream_ports.iter().enumerate() {
            if i >= sink_ports.len() {
                break; // No more sink ports to map to
            }
            let sink_port_id = sink_ports[i];

            let status = Command::new("pw-link")
                .args([&stream_port_id.to_string(), &sink_port_id.to_string()])
                .status()
                .map_err(|e| PlatformError::Internal(format!("Failed to run pw-link: {}", e)))?;

            if !status.success() {
                // It might fail if already connected, which is fine now that we cleaned up others.
                tracing::debug!("pw-link reported failure connecting {} -> {} (might already exist)", stream_port_id, sink_port_id);
            } else {
                tracing::debug!("Connected port {} -> {}", stream_port_id, sink_port_id);
            }
        }

        Ok(())
    }

    /// Helper to get ports of a node
    fn get_node_ports(&self, node_id: u32, direction: &str) -> Result<Vec<u32>, PlatformError> {
        let state = self.state.read().map_err(|_| PlatformError::Internal("State lock poisoned".into()))?;
        
        use crate::linux::state::PortDirection;
        let target_dir = match direction {
            "Input" => PortDirection::Input,
            "Output" => PortDirection::Output,
            _ => return Err(PlatformError::Internal(format!("Invalid direction: {}", direction))),
        };
        
        // Find ports belonging to this node with the given direction
        let mut ports: Vec<(u32, String)> = state.ports.values()
            .filter(|p| p.node_id == node_id && p.direction == target_dir)
            .map(|p| (p.id, p.channel.clone()))
            .collect();
            
        // Sort by channel to ensure consistent ordering (FL, FR)
        // Standard channel map: FL, FR, FC, LFE, RL, RR, ...
        // We prioritize FL and FR to be first.
        ports.sort_by(|a, b| {
            let order_a = channel_order(&a.1);
            let order_b = channel_order(&b.1);
            order_a.cmp(&order_b)
        });

        Ok(ports.into_iter().map(|(id, _)| id).collect())
    }

    /// Enforce stream routing:
    /// 1. Gecko Playback -> Hardware Sink (Active Correction)
    /// 2. Gecko Capture -> Gecko Audio Monitor (Exclusive)
    /// 
    /// WE NO LONGER move generic apps. We rely on "Set Default Sink" to handle that.
    /// This prevents fighting with WirePlumber which causes audio cutouts.
    pub fn enforce_stream_routing(&self, _gecko_sink_id: u32, hardware_sink_id: u32) -> Result<usize, PlatformError> {
        let nodes = self.list_nodes()?;
        let mut moved_count = 0;

        for node in nodes {
            // Check if it's an output stream and specifically Gecko Playback
            // Rust pattern: Combine conditions into a single `if` for clarity
            if node.media_class == "Stream/Output/Audio" && node.name == "Gecko Playback" {
                // ACTIVE CORRECTION: Force Gecko Playback to Hardware Sink
                // This fights WirePlumber if it tries to move us to Gecko Audio
                if let Err(e) = self.move_stream_to_sink(node.id, hardware_sink_id) {
                    tracing::warn!("Failed to enforce Gecko Playback routing: {}", e);
                } else {
                    tracing::debug!("Enforced Gecko Playback -> Hardware Sink ({})", hardware_sink_id);
                    moved_count += 1;
                }
                // NOTE: We do NOT move other apps anymore.
                // We assume `set_default_sink` has done its job.
            }
            // NOTE: We no longer enforce capture routing here.
            // The PipeWire thread handles capture link creation via pending_capture_links.
            // Calling enforce_capture_routing was causing race conditions where deleting
            // WirePlumber-created links would pause the capture stream.
        }
        
        Ok(moved_count)
    }

    /// Enforce capture routing:
    /// Ensure Gecko Capture is connected ONLY to Gecko Audio Monitor ports.
    /// Remove any other links (e.g. from Mic or crossed links).
    ///
    /// NOTE: This function is currently unused - capture link management is handled
    /// by the PipeWire thread via pending_capture_links. Keeping for potential future use.
    #[allow(dead_code)]
    fn enforce_capture_routing(&self, capture_node_id: u32, gecko_sink_id: u32) -> Result<(), PlatformError> {
        // Get capture ports (Input)
        let capture_ports = self.get_node_ports(capture_node_id, "Input")?;
        if capture_ports.is_empty() {
            return Ok(());
        }

        // Get Gecko Audio Monitor ports (Output)
        let monitor_ports = self.get_node_ports(gecko_sink_id, "Output")?;
        if monitor_ports.is_empty() {
            return Ok(());
        }

        // Get all links
        let links = self.list_links()?;

        // Identify valid links: Monitor -> Capture
        // We expect: Monitor_FL -> Capture_FL, Monitor_FR -> Capture_FR
        // Assuming sorted ports: [Mon_L, Mon_R] and [Cap_L, Cap_R]
        let mut valid_links = Vec::new();
        if monitor_ports.len() >= 2 && capture_ports.len() >= 2 {
             valid_links.push((monitor_ports[0], capture_ports[0])); // FL -> FL
             valid_links.push((monitor_ports[1], capture_ports[1])); // FR -> FR
        }

        // Early-out: Check if routing is already correct to avoid unnecessary churn
        let capture_links: Vec<_> = links.iter()
            .filter(|l| capture_ports.contains(&l.input_port))
            .collect();

        // If we have exactly the valid links and nothing else, we're done
        let all_valid = capture_links.len() == valid_links.len()
            && capture_links.iter().all(|l| valid_links.contains(&(l.output_port, l.input_port)));

        if all_valid {
            return Ok(());
        }

        // Find and remove incorrect links
        for link in links {
            // Check if this link connects TO one of our capture ports
            if capture_ports.contains(&link.input_port) {
                // Check if it comes FROM one of our monitor ports
                let is_from_monitor = monitor_ports.contains(&link.output_port);

                // Check if it is a "straight" link (L->L, R->R), not crossed
                let is_valid_mapping = valid_links.contains(&(link.output_port, link.input_port));

                if !is_from_monitor || !is_valid_mapping {
                    tracing::warn!(
                        "Removing unwanted capture link: {} -> {} (id={})",
                        link.output_port, link.input_port, link.id
                    );
                    // Destroy the unwanted link
                    let _ = std::process::Command::new("pw-link")
                        .arg("-d")
                        .arg(link.id.to_string())
                        .output();
                }
            }
        }
        Ok(())
    }
}

fn channel_order(channel: &str) -> i32 {
    match channel {
        "FL" => 0,
        "FR" => 1,
        "FC" => 2,
        "LFE" => 3,
        "RL" => 4,
        "RR" => 5,
        _ => 100, // Unknown channels go last
    }
}

#[cfg(feature = "pipewire")]
impl Drop for PipeWireBackend {
    fn drop(&mut self) {
        tracing::info!("Shutting down PipeWire backend");

        // Signal shutdown
        self.shutdown.store(true, Ordering::SeqCst);

        // Send shutdown command to wake the thread
        let _ = self.command_tx.send(PwCommand::Shutdown);

        // Wait for thread to finish
        if let Some(handle) = self.thread_handle.take() {
            if let Err(e) = handle.join() {
                tracing::error!("PipeWire thread panicked: {:?}", e);
            }
        }

        tracing::info!("PipeWire backend shut down");
    }
}

#[cfg(feature = "pipewire")]
impl PlatformBackend for PipeWireBackend {
    fn name(&self) -> &'static str {
        "PipeWire"
    }

    fn is_connected(&self) -> bool {
        self.state
            .read()
            .map(|s| s.connected)
            .unwrap_or(false)
    }

    fn list_applications(&self) -> Result<Vec<ApplicationInfo>, PlatformError> {
        let state = self
            .state
            .read()
            .map_err(|_| PlatformError::Internal("State lock poisoned".into()))?;

        // Filter nodes that are application streams
        // Rust pattern: filter_map combines filter and map in one pass
        Ok(state
            .nodes
            .values()
            .filter(|n| {
                n.media_class
                    .as_ref()
                    .map(|c| c.contains("Stream/Output"))
                    .unwrap_or(false)
            })
            .filter_map(|n| {
                n.application_name.as_ref().map(|name| ApplicationInfo {
                    pid: n.application_pid.unwrap_or(0),
                    name: name.clone(),
                    icon: None, // PipeWire doesn't provide icon paths directly
                    is_active: n.is_active,
                })
            })
            .collect())
    }

    fn list_nodes(&self) -> Result<Vec<AudioNode>, PlatformError> {
        let state = self
            .state
            .read()
            .map_err(|_| PlatformError::Internal("State lock poisoned".into()))?;

        Ok(state
            .nodes
            .values()
            .map(|n| AudioNode {
                id: n.id,
                name: n.name.clone(),
                media_class: n.media_class.clone().unwrap_or_default(),
                application: n.application_name.as_ref().map(|name| ApplicationInfo {
                    pid: n.application_pid.unwrap_or(0),
                    name: name.clone(),
                    icon: None,
                    is_active: n.is_active,
                }),
            })
            .collect())
    }

    fn list_ports(&self, node_id: u32) -> Result<Vec<AudioPort>, PlatformError> {
        let state = self
            .state
            .read()
            .map_err(|_| PlatformError::Internal("State lock poisoned".into()))?;

        Ok(state
            .ports
            .values()
            .filter(|p| p.node_id == node_id)
            .map(|p| AudioPort {
                id: p.id,
                node_id: p.node_id,
                name: p.name.clone(),
                direction: match p.direction {
                    PortDirection::Input => "input".to_string(),
                    PortDirection::Output => "output".to_string(),
                },
                channel: p.channel.clone(),
            })
            .collect())
    }

    fn list_links(&self) -> Result<Vec<LinkInfo>, PlatformError> {
        let state = self
            .state
            .read()
            .map_err(|_| PlatformError::Internal("State lock poisoned".into()))?;

        Ok(state
            .links
            .values()
            .map(|l| LinkInfo {
                id: l.id,
                output_port: l.output_port,
                input_port: l.input_port,
                active: l.is_active,
            })
            .collect())
    }

    fn create_virtual_sink(&mut self, config: VirtualSinkConfig) -> Result<u32, PlatformError> {
        let response_id = self.next_id();

        let response = self.send_and_wait(
            PwCommand::CreateVirtualSink { config, response_id },
            response_id,
        )?;

        match response {
            PwResponse::VirtualSinkCreated { node_id, .. } => {
                self.our_sinks.push(node_id);
                Ok(node_id)
            }
            PwResponse::Error { message, .. } => {
                Err(PlatformError::VirtualDeviceCreationFailed(message))
            }
            _ => Err(PlatformError::Internal("Unexpected response type".into())),
        }
    }

    fn destroy_virtual_sink(&mut self, node_id: u32) -> Result<(), PlatformError> {
        let response_id = self.next_id();

        let response = self.send_and_wait(
            PwCommand::DestroyVirtualSink { node_id, response_id },
            response_id,
        )?;

        match response {
            PwResponse::Ok { .. } => {
                self.our_sinks.retain(|&id| id != node_id);
                Ok(())
            }
            PwResponse::Error { message, .. } => Err(PlatformError::Internal(message)),
            _ => Err(PlatformError::Internal("Unexpected response type".into())),
        }
    }

    fn create_link(&mut self, output_port: u32, input_port: u32) -> Result<u32, PlatformError> {
        let response_id = self.next_id();

        let response = self.send_and_wait(
            PwCommand::CreateLink {
                output_port,
                input_port,
                response_id,
            },
            response_id,
        )?;

        match response {
            PwResponse::LinkCreated { link_id, .. } => {
                self.our_links.push(link_id);
                Ok(link_id)
            }
            PwResponse::Error { message, .. } => Err(PlatformError::LinkCreationFailed(message)),
            _ => Err(PlatformError::Internal("Unexpected response type".into())),
        }
    }

    fn destroy_link(&mut self, link_id: u32) -> Result<(), PlatformError> {
        let response_id = self.next_id();

        let response = self.send_and_wait(
            PwCommand::DestroyLink { link_id, response_id },
            response_id,
        )?;

        match response {
            PwResponse::Ok { .. } => {
                self.our_links.retain(|&id| id != link_id);
                Ok(())
            }
            PwResponse::Error { message, .. } => Err(PlatformError::Internal(message)),
            _ => Err(PlatformError::Internal("Unexpected response type".into())),
        }
    }

    fn route_application_to_sink(
        &mut self,
        app_name: &str,
        sink_node_id: u32,
    ) -> Result<Vec<u32>, PlatformError> {
        tracing::debug!("Routing application '{}' to sink {}", app_name, sink_node_id);

        // Read the current state
        let state = self
            .state
            .read()
            .map_err(|_| PlatformError::Internal("State lock poisoned".into()))?;

        // Find nodes for this application
        let app_nodes = state.nodes_for_application(app_name);
        if app_nodes.is_empty() {
            return Err(PlatformError::ApplicationNotFound(app_name.to_string()));
        }

        // Collect app node IDs
        let app_node_ids: Vec<u32> = app_nodes.iter().map(|n| n.id).collect();

        // Get output ports of application nodes
        let output_ports: Vec<(u32, String)> = state
            .ports
            .values()
            .filter(|p| app_node_ids.contains(&p.node_id) && p.direction == PortDirection::Output)
            .map(|p| (p.id, p.channel.clone()))
            .collect();

        // Get input ports of sink
        let input_ports: Vec<(u32, String)> = state
            .ports
            .values()
            .filter(|p| p.node_id == sink_node_id && p.direction == PortDirection::Input)
            .map(|p| (p.id, p.channel.clone()))
            .collect();

        // Release lock before creating links
        drop(state);

        if output_ports.is_empty() {
            return Err(PlatformError::Internal(format!(
                "No output ports found for application '{}'",
                app_name
            )));
        }

        if input_ports.is_empty() {
            return Err(PlatformError::Internal(format!(
                "No input ports found for sink {}",
                sink_node_id
            )));
        }

        // Create links matching channels (FL->FL, FR->FR, etc.)
        let mut created_links = Vec::new();

        for (out_port_id, out_channel) in &output_ports {
            for (in_port_id, in_channel) in &input_ports {
                // Match channels (or link if both are MONO)
                if out_channel == in_channel
                    || out_channel == "MONO"
                    || in_channel == "MONO"
                {
                    match self.create_link(*out_port_id, *in_port_id) {
                        Ok(link_id) => {
                            tracing::debug!(
                                "Created link {} -> {} ({})",
                                out_port_id,
                                in_port_id,
                                out_channel
                            );
                            created_links.push(link_id);
                        }
                        Err(e) => {
                            tracing::warn!(
                                "Failed to create link {} -> {}: {}",
                                out_port_id,
                                in_port_id,
                                e
                            );
                        }
                    }
                }
            }
        }

        if created_links.is_empty() {
            return Err(PlatformError::Internal(
                "No links could be created - channel mismatch".into(),
            ));
        }

        Ok(created_links)
    }

    fn default_output_node(&self) -> Result<u32, PlatformError> {
        let state = self
            .state
            .read()
            .map_err(|_| PlatformError::Internal("State lock poisoned".into()))?;

        state
            .default_sink_id
            .ok_or_else(|| PlatformError::Internal("Default sink not yet discovered".into()))
    }

    fn default_input_node(&self) -> Result<u32, PlatformError> {
        let state = self
            .state
            .read()
            .map_err(|_| PlatformError::Internal("State lock poisoned".into()))?;

        state
            .default_source_id
            .ok_or_else(|| PlatformError::Internal("Default source not yet discovered".into()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[cfg(not(feature = "pipewire"))]
    fn test_stub_backend() {
        let backend = StubBackend::new();
        assert!(!backend.is_connected());
        assert!(backend.list_applications().is_err());
    }

    #[test]
    #[cfg(feature = "pipewire")]
    fn test_parse_sink_from_metadata() {
        // Simulated pw-metadata output showing both default.audio.sink and default.configured.audio.sink
        let metadata_output = r#"
Found "default" metadata 35
update: id:0 key:'default.audio.sink' value:'{"name":"Gecko Audio"}' type:''
update: id:0 key:'default.configured.audio.sink' value:'{"name":"alsa_output.pci-0000_00_1f.3.analog-stereo"}' type:''
update: id:0 key:'default.audio.source' value:'{"name":"alsa_input.pci-0000_00_1f.3.analog-stereo"}' type:''
"#;

        // default.audio.sink (currently active) should be Gecko Audio
        let active = PipeWireBackend::parse_sink_from_metadata(metadata_output, "default.audio.sink");
        assert_eq!(active, Some("Gecko Audio".to_string()));

        // default.configured.audio.sink (user's preference) should be the real speakers
        let configured = PipeWireBackend::parse_sink_from_metadata(
            metadata_output,
            "default.configured.audio.sink",
        );
        assert_eq!(
            configured,
            Some("alsa_output.pci-0000_00_1f.3.analog-stereo".to_string())
        );

        // Non-existent key should return None
        let missing = PipeWireBackend::parse_sink_from_metadata(metadata_output, "nonexistent.key");
        assert_eq!(missing, None);
    }

    #[test]
    #[cfg(feature = "pipewire")]
    fn test_parse_sink_from_metadata_complex_names() {
        // Test with various sink name formats
        let metadata = r#"
update: id:0 key:'default.configured.audio.sink' value:'{"name":"bluez_output.AA_BB_CC_DD_EE_FF.a2dp-sink"}' type:''
"#;
        let result = PipeWireBackend::parse_sink_from_metadata(
            metadata,
            "default.configured.audio.sink",
        );
        assert_eq!(
            result,
            Some("bluez_output.AA_BB_CC_DD_EE_FF.a2dp-sink".to_string())
        );
    }

    #[test]
    #[cfg(feature = "pipewire")]
    #[ignore = "requires PipeWire daemon"]
    fn test_pipewire_connection() {
        let backend = PipeWireBackend::new();
        assert!(backend.is_ok(), "Should connect to PipeWire");

        let backend = backend.unwrap();
        // Give it a moment to connect
        std::thread::sleep(std::time::Duration::from_millis(200));
        assert!(backend.is_connected(), "Should be connected");
    }

    #[test]
    #[cfg(feature = "pipewire")]
    #[ignore = "requires PipeWire daemon"]
    fn test_list_nodes() {
        let backend = PipeWireBackend::new().expect("Should connect");
        std::thread::sleep(std::time::Duration::from_millis(500));

        let nodes = backend.list_nodes().expect("Should list nodes");
        // There should be at least one node (default sink)
        assert!(!nodes.is_empty(), "Should have at least one node");
    }

    #[test]
    #[cfg(feature = "pipewire")]
    #[ignore = "requires PipeWire daemon"]
    fn test_create_virtual_sink() {
        let mut backend = PipeWireBackend::new().expect("Should connect");
        std::thread::sleep(std::time::Duration::from_millis(500));

        let config = VirtualSinkConfig {
            name: "Gecko-Test-Sink".to_string(),
            channels: 2,
            sample_rate: 48000,
            persistent: false,
        };

        let node_id = backend
            .create_virtual_sink(config)
            .expect("Should create virtual sink");
        assert!(node_id > 0, "Should return valid node ID");

        // Verify it appears in node list
        std::thread::sleep(std::time::Duration::from_millis(200));
        let nodes = backend.list_nodes().expect("Should list nodes");
        let found = nodes.iter().any(|n| n.name.contains("Gecko-Test-Sink"));
        assert!(found, "Virtual sink should appear in node list");

        // Cleanup
        backend
            .destroy_virtual_sink(node_id)
            .expect("Should destroy sink");
    }
}
