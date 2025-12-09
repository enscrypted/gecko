//! Engine and Stream Configuration

use serde::{Deserialize, Serialize};

/// Audio stream configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StreamConfig {
    /// Sample rate in Hz (e.g., 44100, 48000, 96000)
    pub sample_rate: u32,

    /// Number of audio channels (1 = mono, 2 = stereo)
    pub channels: u16,

    /// Buffer size in frames (lower = less latency, higher = more stability)
    pub buffer_size: u32,
}

impl Default for StreamConfig {
    fn default() -> Self {
        Self {
            sample_rate: 48000,
            channels: 2,
            buffer_size: 512,
        }
    }
}

impl StreamConfig {
    /// Calculate latency in milliseconds for this configuration
    pub fn latency_ms(&self) -> f32 {
        (self.buffer_size as f32 / self.sample_rate as f32) * 1000.0
    }

    /// Calculate bytes per frame (for buffer sizing)
    pub fn bytes_per_frame(&self) -> usize {
        // f32 samples * channels
        4 * self.channels as usize
    }

    /// Validate configuration
    pub fn validate(&self) -> Result<(), String> {
        if self.sample_rate < 8000 || self.sample_rate > 192000 {
            return Err(format!("Invalid sample rate: {}", self.sample_rate));
        }
        if self.channels == 0 || self.channels > 8 {
            return Err(format!("Invalid channel count: {}", self.channels));
        }
        if self.buffer_size < 32 || self.buffer_size > 8192 {
            return Err(format!("Invalid buffer size: {}", self.buffer_size));
        }
        Ok(())
    }
}

/// Overall engine configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EngineConfig {
    /// Stream configuration
    pub stream: StreamConfig,

    /// Ring buffer capacity in frames (should be multiple of buffer_size)
    pub ring_buffer_frames: usize,

    /// Whether to start capturing on engine start
    pub auto_start: bool,
}

impl Default for EngineConfig {
    fn default() -> Self {
        Self {
            stream: StreamConfig::default(),
            // 4 buffers worth of ring buffer capacity
            ring_buffer_frames: 512 * 4,
            auto_start: false,
        }
    }
}

impl EngineConfig {
    /// Create config optimized for low latency
    pub fn low_latency() -> Self {
        Self {
            stream: StreamConfig {
                sample_rate: 48000,
                channels: 2,
                buffer_size: 128, // ~2.6ms latency
            },
            ring_buffer_frames: 128 * 8,
            auto_start: false,
        }
    }

    /// Create config optimized for stability
    pub fn stable() -> Self {
        Self {
            stream: StreamConfig {
                sample_rate: 48000,
                channels: 2,
                buffer_size: 1024, // ~21ms latency
            },
            ring_buffer_frames: 1024 * 4,
            auto_start: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = StreamConfig::default();
        assert_eq!(config.sample_rate, 48000);
        assert_eq!(config.channels, 2);
        assert_eq!(config.buffer_size, 512);
    }

    #[test]
    fn test_latency_calculation() {
        let config = StreamConfig {
            sample_rate: 48000,
            channels: 2,
            buffer_size: 480, // Exactly 10ms at 48kHz
        };
        let latency = config.latency_ms();
        assert!((latency - 10.0).abs() < 0.01);
    }

    #[test]
    fn test_bytes_per_frame() {
        let stereo = StreamConfig {
            sample_rate: 48000,
            channels: 2,
            buffer_size: 512,
        };
        assert_eq!(stereo.bytes_per_frame(), 8); // 2 channels * 4 bytes (f32)

        let mono = StreamConfig {
            sample_rate: 48000,
            channels: 1,
            buffer_size: 512,
        };
        assert_eq!(mono.bytes_per_frame(), 4);
    }

    #[test]
    fn test_validation() {
        let valid = StreamConfig::default();
        assert!(valid.validate().is_ok());

        let invalid_rate = StreamConfig {
            sample_rate: 100,
            ..Default::default()
        };
        assert!(invalid_rate.validate().is_err());

        let invalid_channels = StreamConfig {
            channels: 0,
            ..Default::default()
        };
        assert!(invalid_channels.validate().is_err());

        let invalid_buffer = StreamConfig {
            buffer_size: 10,
            ..Default::default()
        };
        assert!(invalid_buffer.validate().is_err());
    }

    #[test]
    fn test_preset_configs() {
        let low_latency = EngineConfig::low_latency();
        let stable = EngineConfig::stable();

        assert!(low_latency.stream.buffer_size < stable.stream.buffer_size);
        assert!(low_latency.stream.latency_ms() < stable.stream.latency_ms());
    }

    #[test]
    fn test_config_serialization() {
        let config = EngineConfig::default();
        let json = serde_json::to_string(&config).unwrap();
        let deserialized: EngineConfig = serde_json::from_str(&json).unwrap();

        assert_eq!(config.stream.sample_rate, deserialized.stream.sample_rate);
        assert_eq!(config.stream.channels, deserialized.stream.channels);
    }
}
