//! PipeWire Thread Communication
//!
//! Defines the command and response types for communicating between
//! the main thread and the PipeWire event loop thread.
//!
//! # Per-App Audio Architecture
//!
//! Gecko creates a separate virtual sink for each detected audio application.
//! This enables TRUE per-app EQ processing where each app's audio is captured
//! and processed independently BEFORE mixing.
//!
//! ```text
//! Firefox ──→ Gecko-Firefox Sink ──→ Firefox EQ ──┐
//! Spotify ──→ Gecko-Spotify Sink ──→ Spotify EQ  ──┼──→ Mixer ──→ Master EQ ──→ Speakers
//! Discord ──→ Gecko-Discord Sink ──→ Discord EQ ──┘
//! ```

use crate::VirtualSinkConfig;

/// Commands sent from the main thread to the PipeWire thread
///
/// Each command that expects a response includes a `response_id` for correlation.
#[derive(Debug)]
pub enum PwCommand {
    /// Create a virtual audio sink
    CreateVirtualSink {
        config: VirtualSinkConfig,
        response_id: u64,
    },

    /// Destroy a virtual sink we created
    DestroyVirtualSink { node_id: u32, response_id: u64 },

    /// Create a link between two ports
    CreateLink {
        output_port: u32,
        input_port: u32,
        response_id: u64,
    },

    /// Destroy a link we created
    DestroyLink { link_id: u32, response_id: u64 },

    /// Request immediate state synchronization
    /// Note: Will be used when implementing real-time state sync
    #[allow(dead_code)]
    SyncState { response_id: u64 },

    /// Start audio streaming (capture from virtual sink, playback to speakers)
    StartStreaming {
        /// Node ID of the virtual sink to capture from (its monitor port)
        capture_target: u32,
        /// Optional node ID for playback (None = default output)
        playback_target: Option<u32>,
        response_id: u64,
    },

    /// Stop audio streaming
    StopStreaming { response_id: u64 },

    /// Switch playback to a new target device without stopping capture
    /// This is used during device hotplug to seamlessly switch output devices
    /// while keeping the virtual sink and capture stream alive.
    ///
    /// Uses target NAME instead of ID to avoid race conditions during hotplug.
    /// When a device is plugged in, its node ID changes, but the name stays the same.
    /// PipeWire resolves the name to the current ID at connection time.
    SwitchPlaybackTarget {
        /// Target device name (e.g., "alsa_output.usb-...")
        target_name: String,
        response_id: u64,
    },

    /// Update EQ band gain in real-time
    /// Note: Fire-and-forget, no response expected (processed via try_recv)
    UpdateEqBand { band: usize, gain_db: f32 },

    /// Set master volume
    /// Note: Fire-and-forget, no response expected
    SetVolume(f32),

    /// Set bypass state
    /// Note: Fire-and-forget, no response expected
    SetBypass(bool),

    /// Enforce stream routing by moving apps to Gecko Audio
    /// Note: Will be used when implementing automatic stream routing
    #[allow(dead_code)]
    EnforceStreamRouting {
        gecko_node_id: u32,
        hardware_sink_id: u32,
        response_id: u64,
    },

    // === Per-App Audio Commands ===
    // NOTE: These variants are stubbed for the per-app EQ feature (see AGENT.md).
    // They will be implemented when true per-app capture is added.

    /// Create a dedicated virtual sink for an application
    /// This enables true per-app EQ by capturing the app's audio independently
    #[allow(dead_code)]
    CreateAppSink {
        /// Application name (e.g., "Firefox", "Spotify")
        app_name: String,
        response_id: u64,
    },

    /// Destroy a per-app virtual sink
    #[allow(dead_code)]
    DestroyAppSink {
        /// Application name
        app_name: String,
        response_id: u64,
    },

    /// Start capture stream for a per-app sink
    /// Creates a dedicated capture stream and EQ instance for this app
    #[allow(dead_code)]
    StartAppCapture {
        /// Application name
        app_name: String,
        response_id: u64,
    },

    /// Stop capture stream for a per-app sink
    #[allow(dead_code)]
    StopAppCapture {
        /// Application name
        app_name: String,
        response_id: u64,
    },

    /// Update EQ band gain for a specific application
    /// Note: Fire-and-forget, no response expected
    UpdateAppEqBand {
        /// Application name
        app_name: String,
        /// Band index (0-9)
        band: usize,
        /// Gain in dB (-24 to +24)
        gain_db: f32,
    },

    /// Set bypass state for a specific application
    /// When bypassed, app audio passes through without EQ processing
    SetAppBypass {
        /// Application name
        app_name: String,
        /// Whether to bypass EQ for this app
        bypassed: bool,
    },

    /// Set per-app volume (0.0 - 2.0, where 1.0 is unity gain)
    /// This is applied after per-app EQ and before mixing
    SetAppVolume {
        /// Application name
        app_name: String,
        /// Volume level (0.0 = silent, 1.0 = unity, 2.0 = +6dB)
        volume: f32,
    },

    /// Shutdown the PipeWire thread gracefully
    Shutdown,
}

/// Responses from the PipeWire thread to the main thread
#[derive(Debug)]
pub enum PwResponse {
    /// Virtual sink was created successfully
    VirtualSinkCreated { response_id: u64, node_id: u32 },

    /// Link was created successfully
    LinkCreated { response_id: u64, link_id: u32 },

    /// Generic success response (for destroy operations)
    Ok { response_id: u64 },

    /// Operation failed with an error message
    Error { response_id: u64, message: String },

    /// State synchronization completed
    StateSynced { response_id: u64 },

    /// Audio streaming started successfully
    StreamingStarted { response_id: u64 },

    /// Audio streaming stopped successfully
    StreamingStopped { response_id: u64 },

    /// Playback target switched successfully
    PlaybackTargetSwitched { response_id: u64 },

    // === Per-App Audio Responses ===
    // NOTE: These variants are stubbed for the per-app EQ feature (see AGENT.md).
    // They will be used when true per-app capture is implemented.

    /// Per-app virtual sink was created successfully
    #[allow(dead_code)]
    AppSinkCreated {
        response_id: u64,
        /// Application name
        app_name: String,
        /// Node ID of the created sink
        sink_node_id: u32,
    },

    /// Per-app virtual sink was destroyed successfully
    #[allow(dead_code)]
    AppSinkDestroyed {
        response_id: u64,
        /// Application name
        app_name: String,
    },

    /// Per-app capture stream started successfully
    #[allow(dead_code)]
    AppCaptureStarted {
        response_id: u64,
        /// Application name
        app_name: String,
    },

    /// Per-app capture stream stopped successfully
    #[allow(dead_code)]
    AppCaptureStopped {
        response_id: u64,
        /// Application name
        app_name: String,
    },
}

impl PwResponse {
    /// Get the response ID for correlation
    pub fn response_id(&self) -> u64 {
        match self {
            PwResponse::VirtualSinkCreated { response_id, .. } => *response_id,
            PwResponse::LinkCreated { response_id, .. } => *response_id,
            PwResponse::Ok { response_id } => *response_id,
            PwResponse::Error { response_id, .. } => *response_id,
            PwResponse::StateSynced { response_id } => *response_id,
            PwResponse::StreamingStarted { response_id } => *response_id,
            PwResponse::StreamingStopped { response_id } => *response_id,
            PwResponse::PlaybackTargetSwitched { response_id } => *response_id,
            // Per-app responses
            PwResponse::AppSinkCreated { response_id, .. } => *response_id,
            PwResponse::AppSinkDestroyed { response_id, .. } => *response_id,
            PwResponse::AppCaptureStarted { response_id, .. } => *response_id,
            PwResponse::AppCaptureStopped { response_id, .. } => *response_id,
        }
    }

    /// Check if this is an error response
    /// Note: Will be used when implementing error handling in commands
    #[allow(dead_code)]
    pub fn is_error(&self) -> bool {
        matches!(self, PwResponse::Error { .. })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_response_id_extraction() {
        let response = PwResponse::VirtualSinkCreated {
            response_id: 42,
            node_id: 100,
        };
        assert_eq!(response.response_id(), 42);

        let error = PwResponse::Error {
            response_id: 99,
            message: "test error".to_string(),
        };
        assert_eq!(error.response_id(), 99);
        assert!(error.is_error());
    }

    #[test]
    fn test_command_creation() {
        let cmd = PwCommand::CreateVirtualSink {
            config: VirtualSinkConfig::default(),
            response_id: 1,
        };

        // Rust pattern: match to verify enum variant
        match cmd {
            PwCommand::CreateVirtualSink { response_id, .. } => {
                assert_eq!(response_id, 1);
            }
            _ => panic!("Wrong variant"),
        }
    }
}
