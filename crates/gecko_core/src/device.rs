//! Audio Device Enumeration and Management

use cpal::traits::{DeviceTrait, HostTrait};
use serde::{Deserialize, Serialize};

use crate::error::{EngineError, EngineResult};

/// Type of audio device
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DeviceType {
    Input,
    Output,
}

/// Represents an audio device (input or output)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AudioDevice {
    /// Unique identifier for this device
    pub id: String,

    /// Human-readable device name
    pub name: String,

    /// Whether this is an input or output device
    pub device_type: DeviceType,

    /// Whether this is the system default device
    pub is_default: bool,

    /// Supported sample rates (may be empty if querying failed)
    pub sample_rates: Vec<u32>,

    /// Maximum supported channels
    pub max_channels: u16,
}

impl AudioDevice {
    /// Enumerate all available audio devices
    pub fn enumerate_all() -> EngineResult<Vec<AudioDevice>> {
        // Rust pattern: `?` early-returns errors, cleaner than try/catch
        let host = cpal::default_host();

        let mut devices = Vec::new();

        // Get default device names for comparison
        let default_input_name = host
            .default_input_device()
            .and_then(|d| d.name().ok());
        let default_output_name = host
            .default_output_device()
            .and_then(|d| d.name().ok());

        // Enumerate input devices
        if let Ok(input_devices) = host.input_devices() {
            for device in input_devices {
                if let Ok(audio_device) = Self::from_cpal_device(
                    &device,
                    DeviceType::Input,
                    default_input_name.as_deref(),
                ) {
                    devices.push(audio_device);
                }
            }
        }

        // Enumerate output devices
        if let Ok(output_devices) = host.output_devices() {
            for device in output_devices {
                if let Ok(audio_device) = Self::from_cpal_device(
                    &device,
                    DeviceType::Output,
                    default_output_name.as_deref(),
                ) {
                    devices.push(audio_device);
                }
            }
        }

        if devices.is_empty() {
            return Err(EngineError::NoDevicesFound);
        }

        Ok(devices)
    }

    /// Get only input devices
    pub fn enumerate_inputs() -> EngineResult<Vec<AudioDevice>> {
        Ok(Self::enumerate_all()?
            .into_iter()
            .filter(|d| d.device_type == DeviceType::Input)
            .collect())
    }

    /// Get only output devices
    pub fn enumerate_outputs() -> EngineResult<Vec<AudioDevice>> {
        Ok(Self::enumerate_all()?
            .into_iter()
            .filter(|d| d.device_type == DeviceType::Output)
            .collect())
    }

    /// Get the default input device
    pub fn default_input() -> EngineResult<AudioDevice> {
        let host = cpal::default_host();
        let device = host
            .default_input_device()
            .ok_or(EngineError::NoDevicesFound)?;

        Self::from_cpal_device(&device, DeviceType::Input, None)
            .map(|mut d| {
                d.is_default = true;
                d
            })
    }

    /// Get the default output device
    pub fn default_output() -> EngineResult<AudioDevice> {
        let host = cpal::default_host();
        let device = host
            .default_output_device()
            .ok_or(EngineError::NoDevicesFound)?;

        Self::from_cpal_device(&device, DeviceType::Output, None)
            .map(|mut d| {
                d.is_default = true;
                d
            })
    }

    /// Create AudioDevice from CPAL device
    fn from_cpal_device(
        device: &cpal::Device,
        device_type: DeviceType,
        default_name: Option<&str>,
    ) -> EngineResult<Self> {
        let name = device
            .name()
            .map_err(|e| EngineError::DeviceNotFound(e.to_string()))?;

        // Use name as ID (CPAL doesn't provide separate IDs)
        let id = name.clone();

        let is_default = default_name.map(|d| d == name).unwrap_or(false);

        // Query supported configurations
        let (sample_rates, max_channels) = match device_type {
            DeviceType::Input => Self::query_input_config(device),
            DeviceType::Output => Self::query_output_config(device),
        };

        Ok(AudioDevice {
            id,
            name,
            device_type,
            is_default,
            sample_rates,
            max_channels,
        })
    }

    fn query_input_config(device: &cpal::Device) -> (Vec<u32>, u16) {
        if let Ok(configs) = device.supported_input_configs() {
            Self::extract_config_info(configs)
        } else {
            (vec![], 2)
        }
    }

    fn query_output_config(device: &cpal::Device) -> (Vec<u32>, u16) {
        if let Ok(configs) = device.supported_output_configs() {
            Self::extract_config_info(configs)
        } else {
            (vec![], 2)
        }
    }

    fn extract_config_info(
        configs: impl Iterator<Item = cpal::SupportedStreamConfigRange>,
    ) -> (Vec<u32>, u16) {
        let mut sample_rates = Vec::new();
        let mut max_channels = 0u16;

        // Common sample rates to check
        const COMMON_RATES: [u32; 6] = [44100, 48000, 88200, 96000, 176400, 192000];

        for config in configs {
            max_channels = max_channels.max(config.channels());

            // Check which common rates are supported
            let min = config.min_sample_rate().0;
            let max = config.max_sample_rate().0;

            for &rate in &COMMON_RATES {
                if rate >= min && rate <= max && !sample_rates.contains(&rate) {
                    sample_rates.push(rate);
                }
            }
        }

        sample_rates.sort_unstable();
        (sample_rates, max_channels)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_device_type_serialization() {
        let input = DeviceType::Input;
        let json = serde_json::to_string(&input).unwrap();
        let deserialized: DeviceType = serde_json::from_str(&json).unwrap();
        assert_eq!(input, deserialized);
    }

    #[test]
    fn test_audio_device_serialization() {
        let device = AudioDevice {
            id: "test-id".to_string(),
            name: "Test Device".to_string(),
            device_type: DeviceType::Output,
            is_default: true,
            sample_rates: vec![44100, 48000],
            max_channels: 2,
        };

        let json = serde_json::to_string(&device).unwrap();
        let deserialized: AudioDevice = serde_json::from_str(&json).unwrap();

        assert_eq!(device.id, deserialized.id);
        assert_eq!(device.name, deserialized.name);
        assert_eq!(device.device_type, deserialized.device_type);
    }

    // Note: Hardware-dependent tests are marked with #[ignore]
    // Run them with: cargo test -- --ignored

    #[test]
    #[ignore = "requires audio hardware"]
    fn test_enumerate_all_devices() {
        let devices = AudioDevice::enumerate_all();
        // On most systems, at least one device should exist
        assert!(devices.is_ok());
    }

    #[test]
    #[ignore = "requires audio hardware"]
    fn test_default_output() {
        let device = AudioDevice::default_output();
        if let Ok(d) = device {
            assert!(d.is_default);
            assert_eq!(d.device_type, DeviceType::Output);
        }
    }

    #[test]
    #[ignore = "requires audio hardware"]
    fn test_default_input() {
        let device = AudioDevice::default_input();
        if let Ok(d) = device {
            assert!(d.is_default);
            assert_eq!(d.device_type, DeviceType::Input);
        }
    }
}
