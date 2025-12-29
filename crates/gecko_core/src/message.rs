//! Message Types for Thread Communication
//!
//! Commands flow from UI thread -> Audio thread
//! Events flow from Audio thread -> UI thread

use serde::{Deserialize, Serialize};

use crate::config::StreamConfig;
use gecko_dsp::EqConfig;

/// Commands sent from UI thread to Audio engine
#[derive(Debug, Clone)]
pub enum Command {
    /// Start audio processing
    Start,

    /// Stop audio processing
    Stop,

    /// Update EQ configuration
    UpdateEq(EqConfig),

    /// Set gain for a single master EQ band (band_index, gain_db)
    SetBandGain { band: usize, gain_db: f32 },

    /// Set per-app EQ band gain (TRUE per-app EQ, NOT additive to master)
    /// Each app has its own independent EQ instance that processes audio BEFORE mixing
    SetStreamBandGain { stream_id: String, band: usize, gain_db: f32 },

    /// Set bypass state for a specific application
    /// When bypassed, the app's audio passes through without EQ processing
    SetAppBypass { app_name: String, bypassed: bool },

    /// Start capturing audio from a specific application (macOS only)
    /// Uses Process Tap API to capture the app's audio stream
    StartAppCapture { pid: u32, app_name: String },

    /// Stop capturing audio from a specific application (macOS only)
    StopAppCapture { pid: u32 },

    /// Set per-app volume (0.0 - 2.0, where 1.0 is unity gain)
    /// This is independent of master volume and is applied before mixing
    SetStreamVolume { stream_id: String, volume: f32 },

    /// Set master volume (0.0 - 1.0)
    SetMasterVolume(f32),

    /// Bypass all processing
    SetBypass(bool),

    /// Enable/disable soft clipping (limiter to prevent harsh distortion)
    SetSoftClipEnabled(bool),

    /// Change input device
    SetInputDevice(String),

    /// Change output device
    SetOutputDevice(String),

    /// Update stream configuration
    UpdateStreamConfig(StreamConfig),

    /// Request current state (triggers StateUpdate event)
    RequestState,

    /// Shutdown the engine
    Shutdown,
}

/// Events sent from Audio engine to UI thread
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "payload")]
pub enum Event {
    /// Engine started successfully
    Started,

    /// Engine stopped
    Stopped,

    /// Error occurred
    Error { message: String },

    /// Audio level update (for meters)
    /// Contains peak levels: (left, right) in range 0.0 - 1.0
    LevelUpdate { left: f32, right: f32 },

    /// Current state snapshot
    StateUpdate {
        is_running: bool,
        is_bypassed: bool,
        master_volume: f32,
        input_device: Option<String>,
        output_device: Option<String>,
    },

    /// Device list changed (hot-plug)
    DevicesChanged,

    /// Buffer underrun detected (audio glitch)
    BufferUnderrun,

    /// Stream configuration changed
    ConfigChanged(StreamConfig),

    /// An audio application/stream was discovered
    /// This is sent when a new app starts producing audio and Gecko creates
    /// a dedicated per-app sink and capture stream for it.
    StreamDiscovered {
        /// Application name (e.g., "Firefox", "Spotify")
        app_name: String,
        /// Node ID of the application's audio stream
        node_id: u32,
    },

    /// An audio application/stream was removed
    /// This is sent when an app stops producing audio and Gecko cleans up
    /// the associated sink and capture stream.
    StreamRemoved {
        /// Application name
        app_name: String,
    },

    /// FFT spectrum data update for visualization
    /// Sent at ~30fps when audio is playing, containing logarithmically-spaced
    /// frequency bin magnitudes for display.
    SpectrumUpdate {
        /// Array of 32 frequency bin magnitudes (0.0 to 1.0)
        /// Bins are logarithmically spaced from ~20Hz to 20kHz
        bins: Vec<f32>,
    },
}

impl Event {
    /// Create an error event from any error type
    pub fn error<E: std::fmt::Display>(err: E) -> Self {
        Event::Error {
            message: err.to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_event_serialization() {
        let event = Event::LevelUpdate {
            left: 0.5,
            right: 0.7,
        };
        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains("LevelUpdate"));

        let deserialized: Event = serde_json::from_str(&json).unwrap();
        if let Event::LevelUpdate { left, right } = deserialized {
            assert_eq!(left, 0.5);
            assert_eq!(right, 0.7);
        } else {
            panic!("Deserialization produced wrong variant");
        }
    }

    #[test]
    fn test_error_event() {
        let event = Event::error("Test error message");
        if let Event::Error { message } = event {
            assert_eq!(message, "Test error message");
        } else {
            panic!("Should be Error variant");
        }
    }

    #[test]
    fn test_state_update_serialization() {
        let event = Event::StateUpdate {
            is_running: true,
            is_bypassed: false,
            master_volume: 0.8,
            input_device: Some("Microphone".to_string()),
            output_device: Some("Speakers".to_string()),
        };

        let json = serde_json::to_string(&event).unwrap();
        let deserialized: Event = serde_json::from_str(&json).unwrap();

        if let Event::StateUpdate { is_running, master_volume, .. } = deserialized {
            assert!(is_running);
            assert_eq!(master_volume, 0.8);
        } else {
            panic!("Wrong variant");
        }
    }
}
