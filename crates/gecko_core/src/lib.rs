//! Gecko Core - Audio Engine
//!
//! This crate provides the core audio engine for Gecko, including:
//! - Audio device enumeration and stream management (via CPAL)
//! - Real-time audio processing pipeline
//! - Lock-free communication between UI and audio threads
//! - Platform-agnostic transport layer
//!
//! # Architecture
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────┐
//! │                        UI Thread                            │
//! │  (Tauri/Web) ──commands──▶ Engine ◀──events── (Tauri/Web)  │
//! └─────────────────────────────────────────────────────────────┘
//!                              │ crossbeam-channel
//!                              ▼
//! ┌─────────────────────────────────────────────────────────────┐
//! │                      Audio Thread                           │
//! │   Capture ──rtrb──▶ DSP Chain ──rtrb──▶ Output             │
//! │     │                   │                  │                │
//! │     └───────────────────┴──────────────────┘                │
//! │              (Zero allocation in this path)                 │
//! └─────────────────────────────────────────────────────────────┘
//! ```

mod config;
mod device;
mod engine;
mod error;
mod message;
mod settings;
mod stream;

pub use config::{EngineConfig, StreamConfig};
pub use device::{AudioDevice, DeviceType};
pub use engine::AudioEngine;
pub use error::EngineError;
pub use message::{Command, Event};
pub use settings::{GeckoSettings, UiSettings, UserPreset};
pub use stream::AudioStream;

// Re-export DSP types for convenience
pub use gecko_dsp::{Equalizer, EqConfig, Band, BandType, EQ_BANDS};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_crate_exports() {
        // Verify public API is accessible
        let _config = EngineConfig::default();
    }
}
