//! Command and Response Messages for WASAPI Thread
//!
//! Defines the communication protocol between the main thread (WasapiBackend)
//! and the dedicated WASAPI thread. This pattern is similar to the Linux
//! PipeWire implementation.
//!
//! # Architecture
//!
//! ```text
//! Main Thread                    WASAPI Thread
//! ───────────                    ────────────
//! WasapiBackend                  WasapiThread
//!   │                              │
//!   ├── command_tx ────────────►   │ process commands
//!   │                              │
//!   └── response_rx ◄────────────  │ send responses
//! ```

/// Commands sent from main thread to WASAPI thread
#[derive(Debug)]
pub enum WasapiCommand {
    /// Initialize audio capture for a process
    StartCapture {
        /// Process ID to capture (None = system-wide loopback)
        pid: Option<u32>,
        /// Application name for tracking
        app_name: String,
    },

    /// Stop capturing audio from a process
    StopCapture {
        /// Process ID to stop capturing
        pid: u32,
    },

    /// Start audio output stream
    StartOutput,

    /// Stop audio output stream
    StopOutput,

    /// Set master volume (0.0 - 2.0)
    SetMasterVolume(f32),

    /// Set master bypass state
    SetMasterBypass(bool),

    /// Set per-app volume
    SetAppVolume {
        app_name: String,
        volume: f32,
    },

    /// Set per-app EQ gains
    SetAppEqGains {
        app_name: String,
        gains: [f32; 10],
    },

    /// Set per-app bypass state
    SetAppBypass {
        app_name: String,
        bypassed: bool,
    },

    /// Set master EQ gains
    SetMasterEqGains([f32; 10]),

    /// Request list of active audio sessions
    ListAudioSessions,

    /// Request list of audio devices
    ListDevices,

    /// Switch output device
    SwitchOutputDevice {
        device_id: String,
    },

    /// Shutdown the WASAPI thread
    Shutdown,
}

/// Responses sent from WASAPI thread back to main thread
#[derive(Debug)]
pub enum WasapiResponse {
    /// Capture started successfully
    CaptureStarted {
        pid: Option<u32>,
        app_name: String,
    },

    /// Capture stopped
    CaptureStopped {
        pid: u32,
    },

    /// Output started successfully
    OutputStarted,

    /// Output stopped
    OutputStopped,

    /// List of active audio sessions
    AudioSessions(Vec<AudioSessionInfo>),

    /// List of audio devices
    Devices(Vec<DeviceInfo>),

    /// Error occurred
    Error(String),

    /// Shutdown acknowledged
    ShutdownComplete,
}

/// Information about an audio session (process with active audio)
#[derive(Debug, Clone)]
pub struct AudioSessionInfo {
    /// Process ID
    pub pid: u32,
    /// Process name
    pub name: String,
    /// Display name (may differ from process name)
    pub display_name: String,
    /// Session state (active, inactive, expired)
    pub state: SessionState,
    /// Current volume (0.0 - 1.0)
    pub volume: f32,
    /// Whether muted
    pub muted: bool,
    /// Icon path (if available)
    pub icon_path: Option<String>,
}

/// Audio session state
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionState {
    /// Session is actively producing audio
    Active,
    /// Session exists but not producing audio
    Inactive,
    /// Session has expired (process closed)
    Expired,
}

/// Information about an audio device
#[derive(Debug, Clone)]
pub struct DeviceInfo {
    /// Device ID (WASAPI endpoint ID string)
    pub id: String,
    /// Friendly name (e.g., "Speakers (Realtek Audio)")
    pub name: String,
    /// Whether this is the default device
    pub is_default: bool,
    /// Data flow direction
    pub flow: DeviceFlow,
    /// Device state
    pub state: DeviceState,
}

/// Audio device data flow direction
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeviceFlow {
    /// Output device (speakers, headphones)
    Render,
    /// Input device (microphone) - not used by Gecko
    Capture,
}

/// Audio device state
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeviceState {
    /// Device is active and available
    Active,
    /// Device is disabled
    Disabled,
    /// Device is not present
    NotPresent,
    /// Device is unplugged
    Unplugged,
}

/// Shared state for audio processing
///
/// This mirrors the AudioProcessingState from Linux/macOS implementations.
/// Uses atomics for lock-free access from audio callbacks.
#[derive(Debug)]
pub struct AudioProcessingState {
    /// Master volume (0.0 - 2.0, stored as f32 bits)
    pub master_volume: std::sync::atomic::AtomicU32,
    /// Master bypass flag
    pub master_bypass: std::sync::atomic::AtomicBool,
    /// Master EQ gains (10 bands, stored as f32 bits)
    pub master_eq_gains: [std::sync::atomic::AtomicU32; 10],
    /// Master EQ update counter (for detecting changes)
    pub master_eq_update_counter: std::sync::atomic::AtomicU32,
    /// Peak levels for visualization [left, right]
    pub peak_levels: [std::sync::atomic::AtomicU32; 2],
    /// Running flag
    pub running: std::sync::atomic::AtomicBool,
}

impl AudioProcessingState {
    /// Create new audio processing state with default values
    pub fn new() -> Self {
        Self {
            master_volume: std::sync::atomic::AtomicU32::new(1.0f32.to_bits()),
            master_bypass: std::sync::atomic::AtomicBool::new(false),
            master_eq_gains: std::array::from_fn(|_| {
                std::sync::atomic::AtomicU32::new(0.0f32.to_bits())
            }),
            master_eq_update_counter: std::sync::atomic::AtomicU32::new(0),
            peak_levels: [
                std::sync::atomic::AtomicU32::new(0.0f32.to_bits()),
                std::sync::atomic::AtomicU32::new(0.0f32.to_bits()),
            ],
            running: std::sync::atomic::AtomicBool::new(false),
        }
    }

    /// Get master volume (lock-free)
    pub fn get_master_volume(&self) -> f32 {
        f32::from_bits(self.master_volume.load(std::sync::atomic::Ordering::Relaxed))
    }

    /// Set master volume (lock-free)
    pub fn set_master_volume(&self, volume: f32) {
        self.master_volume
            .store(volume.to_bits(), std::sync::atomic::Ordering::Relaxed);
    }

    /// Get master bypass state (lock-free)
    pub fn is_bypassed(&self) -> bool {
        self.master_bypass.load(std::sync::atomic::Ordering::Relaxed)
    }

    /// Set master bypass state (lock-free)
    pub fn set_bypass(&self, bypass: bool) {
        self.master_bypass
            .store(bypass, std::sync::atomic::Ordering::Relaxed);
    }

    /// Get master EQ gains (lock-free)
    pub fn get_master_eq_gains(&self) -> [f32; 10] {
        std::array::from_fn(|i| {
            f32::from_bits(self.master_eq_gains[i].load(std::sync::atomic::Ordering::Relaxed))
        })
    }

    /// Set master EQ gains (lock-free)
    pub fn set_master_eq_gains(&self, gains: &[f32; 10]) {
        for (i, &gain) in gains.iter().enumerate() {
            self.master_eq_gains[i].store(gain.to_bits(), std::sync::atomic::Ordering::Relaxed);
        }
        self.master_eq_update_counter
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }

    /// Get peak levels [left, right] (lock-free)
    pub fn get_peak_levels(&self) -> [f32; 2] {
        [
            f32::from_bits(self.peak_levels[0].load(std::sync::atomic::Ordering::Relaxed)),
            f32::from_bits(self.peak_levels[1].load(std::sync::atomic::Ordering::Relaxed)),
        ]
    }

    /// Update peak levels (called from audio callback, lock-free)
    pub fn update_peak_levels(&self, left: f32, right: f32) {
        self.peak_levels[0].store(left.to_bits(), std::sync::atomic::Ordering::Relaxed);
        self.peak_levels[1].store(right.to_bits(), std::sync::atomic::Ordering::Relaxed);
    }
}

impl Default for AudioProcessingState {
    fn default() -> Self {
        Self::new()
    }
}

// Rust pattern: Arc<AudioProcessingState> is our shared state handle
// Safe to share between threads due to atomic operations
unsafe impl Send for AudioProcessingState {}
unsafe impl Sync for AudioProcessingState {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_audio_processing_state_defaults() {
        let state = AudioProcessingState::new();
        assert!((state.get_master_volume() - 1.0).abs() < f32::EPSILON);
        assert!(!state.is_bypassed());
    }

    #[test]
    fn test_volume_set_get() {
        let state = AudioProcessingState::new();
        state.set_master_volume(0.5);
        assert!((state.get_master_volume() - 0.5).abs() < f32::EPSILON);
    }

    #[test]
    fn test_bypass_set_get() {
        let state = AudioProcessingState::new();
        state.set_bypass(true);
        assert!(state.is_bypassed());
        state.set_bypass(false);
        assert!(!state.is_bypassed());
    }

    #[test]
    fn test_eq_gains_set_get() {
        let state = AudioProcessingState::new();
        let gains = [1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0, 10.0];
        state.set_master_eq_gains(&gains);

        let retrieved = state.get_master_eq_gains();
        for i in 0..10 {
            assert!((retrieved[i] - gains[i]).abs() < f32::EPSILON);
        }
    }

    #[test]
    fn test_peak_levels() {
        let state = AudioProcessingState::new();
        state.update_peak_levels(0.7, 0.8);

        let peaks = state.get_peak_levels();
        assert!((peaks[0] - 0.7).abs() < f32::EPSILON);
        assert!((peaks[1] - 0.8).abs() < f32::EPSILON);
    }

    #[test]
    fn test_session_state_equality() {
        assert_eq!(SessionState::Active, SessionState::Active);
        assert_ne!(SessionState::Active, SessionState::Inactive);
    }

    #[test]
    fn test_device_flow_equality() {
        assert_eq!(DeviceFlow::Render, DeviceFlow::Render);
        assert_ne!(DeviceFlow::Render, DeviceFlow::Capture);
    }
}
