//! WASAPI Device Enumeration
//!
//! Enumerates audio devices using Windows Multimedia Device API.
//! Provides device discovery and default device detection.
//!
//! # Device Types
//!
//! - **Render**: Output devices (speakers, headphones)
//! - **Capture**: Input devices (microphones) - Gecko uses loopback instead
//!
//! # Virtual Devices
//!
//! Gecko cannot create virtual devices on Windows (requires kernel driver).
//! Instead, we detect existing virtual audio software:
//! - VB-Cable / VB-Audio
//! - Virtual Audio Cable (VAC)
//! - Voicemeeter

use crate::error::PlatformError;
use super::message::{DeviceFlow, DeviceInfo, DeviceState};

#[cfg(target_os = "windows")]
use windows::core::Interface;

/// Device enumerator for WASAPI
///
/// Wraps IMMDeviceEnumerator for audio endpoint discovery.
pub struct DeviceEnumerator {
    #[cfg(target_os = "windows")]
    enumerator: windows::Win32::Media::Audio::IMMDeviceEnumerator,
}

impl DeviceEnumerator {
    /// Create a new device enumerator
    ///
    /// # Requirements
    ///
    /// COM must be initialized on the calling thread before calling this.
    #[cfg(target_os = "windows")]
    pub fn new() -> Result<Self, PlatformError> {
        use windows::Win32::Media::Audio::MMDeviceEnumerator;
        use windows::Win32::System::Com::{CoCreateInstance, CLSCTX_ALL};

        let enumerator = unsafe {
            CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL).map_err(|e| {
                PlatformError::InitializationFailed(format!(
                    "Failed to create MMDeviceEnumerator: {}",
                    e
                ))
            })?
        };

        tracing::debug!("WASAPI device enumerator created");

        Ok(Self { enumerator })
    }

    #[cfg(not(target_os = "windows"))]
    pub fn new() -> Result<Self, PlatformError> {
        Err(PlatformError::FeatureNotAvailable(
            "WASAPI only available on Windows".into(),
        ))
    }

    /// Enumerate all active audio devices
    #[cfg(target_os = "windows")]
    pub fn enumerate_all(&self) -> Result<Vec<DeviceInfo>, PlatformError> {
        use windows::Win32::Media::Audio::{eAll, DEVICE_STATE_ACTIVE};

        self.enumerate_devices(eAll, DEVICE_STATE_ACTIVE)
    }

    /// Enumerate active render (output) devices only
    #[cfg(target_os = "windows")]
    pub fn enumerate_render_devices(&self) -> Result<Vec<DeviceInfo>, PlatformError> {
        use windows::Win32::Media::Audio::{eRender, DEVICE_STATE_ACTIVE};

        self.enumerate_devices(eRender, DEVICE_STATE_ACTIVE)
    }

    /// Enumerate devices with specific flow and state
    #[cfg(target_os = "windows")]
    fn enumerate_devices(
        &self,
        flow: windows::Win32::Media::Audio::EDataFlow,
        state_mask: windows::Win32::Media::Audio::DEVICE_STATE,
    ) -> Result<Vec<DeviceInfo>, PlatformError> {
        use windows::Win32::Media::Audio::{eCapture, eRender};

        let collection = unsafe {
            self.enumerator
                .EnumAudioEndpoints(flow, state_mask)
                .map_err(|e| {
                    PlatformError::Internal(format!("Failed to enumerate devices: {}", e))
                })?
        };

        let count = unsafe {
            collection.GetCount().map_err(|e| {
                PlatformError::Internal(format!("Failed to get device count: {}", e))
            })?
        };

        let mut devices = Vec::with_capacity(count as usize);

        // Get default devices for comparison
        let default_render_id = self.get_default_device_id(eRender).ok();
        let default_capture_id = self.get_default_device_id(eCapture).ok();

        for i in 0..count {
            if let Ok(device) = unsafe { collection.Item(i) } {
                if let Ok(info) = self.get_device_info(&device, &default_render_id, &default_capture_id) {
                    devices.push(info);
                }
            }
        }

        tracing::debug!("Enumerated {} audio devices", devices.len());

        Ok(devices)
    }

    /// Get the default render (output) device
    #[cfg(target_os = "windows")]
    pub fn get_default_render_device(&self) -> Result<DeviceInfo, PlatformError> {
        use windows::Win32::Media::Audio::{eConsole, eRender};

        let device = unsafe {
            self.enumerator
                .GetDefaultAudioEndpoint(eRender, eConsole)
                .map_err(|e| {
                    PlatformError::Internal(format!("Failed to get default render device: {}", e))
                })?
        };

        let mut info = self.get_device_info(&device, &None, &None)?;
        info.is_default = true;

        Ok(info)
    }

    /// Get the default capture (input) device
    #[cfg(target_os = "windows")]
    pub fn get_default_capture_device(&self) -> Result<DeviceInfo, PlatformError> {
        use windows::Win32::Media::Audio::{eCapture, eConsole};

        let device = unsafe {
            self.enumerator
                .GetDefaultAudioEndpoint(eCapture, eConsole)
                .map_err(|e| {
                    PlatformError::Internal(format!("Failed to get default capture device: {}", e))
                })?
        };

        let mut info = self.get_device_info(&device, &None, &None)?;
        info.is_default = true;

        Ok(info)
    }

    /// Get device ID string for a flow direction
    #[cfg(target_os = "windows")]
    fn get_default_device_id(
        &self,
        flow: windows::Win32::Media::Audio::EDataFlow,
    ) -> Result<String, PlatformError> {
        use windows::Win32::Media::Audio::eConsole;

        let device = unsafe {
            self.enumerator
                .GetDefaultAudioEndpoint(flow, eConsole)
                .map_err(|e| PlatformError::Internal(format!("No default device: {}", e)))?
        };

        self.get_device_id_string(&device)
    }

    /// Extract device information
    #[cfg(target_os = "windows")]
    fn get_device_info(
        &self,
        device: &windows::Win32::Media::Audio::IMMDevice,
        default_render_id: &Option<String>,
        default_capture_id: &Option<String>,
    ) -> Result<DeviceInfo, PlatformError> {
        let id = self.get_device_id_string(device)?;
        let name = self.get_device_friendly_name(device)?;
        let flow = self.get_device_flow(device)?;
        let state = self.get_device_state(device)?;

        let is_default = match flow {
            DeviceFlow::Render => default_render_id.as_ref() == Some(&id),
            DeviceFlow::Capture => default_capture_id.as_ref() == Some(&id),
        };

        Ok(DeviceInfo {
            id,
            name,
            is_default,
            flow,
            state,
        })
    }

    /// Get device ID string
    #[cfg(target_os = "windows")]
    fn get_device_id_string(
        &self,
        device: &windows::Win32::Media::Audio::IMMDevice,
    ) -> Result<String, PlatformError> {
        let id_pwstr = unsafe {
            device.GetId().map_err(|e| {
                PlatformError::Internal(format!("Failed to get device ID: {}", e))
            })?
        };

        let id = unsafe {
            id_pwstr.to_string().map_err(|e| {
                PlatformError::Internal(format!("Failed to convert device ID: {}", e))
            })?
        };

        // Free the COM-allocated string
        unsafe {
            windows::Win32::System::Com::CoTaskMemFree(Some(id_pwstr.as_ptr() as *mut _));
        }

        Ok(id)
    }

    /// Get device friendly name from property store
    #[cfg(target_os = "windows")]
    fn get_device_friendly_name(
        &self,
        device: &windows::Win32::Media::Audio::IMMDevice,
    ) -> Result<String, PlatformError> {
        use windows::core::GUID;
        use windows::Win32::System::Com::STGM_READ;
        use windows::Win32::UI::Shell::PropertiesSystem::PROPERTYKEY;
        use windows::Win32::System::Com::StructuredStorage::PropVariantToStringAlloc;

        let store = unsafe {
            device.OpenPropertyStore(STGM_READ).map_err(|e| {
                PlatformError::Internal(format!("Failed to open property store: {}", e))
            })?
        };

        // PKEY_Device_FriendlyName = {a45c254e-df1c-4efd-8020-67d146a850e0}, 14
        let pkey = PROPERTYKEY {
            fmtid: GUID::from_u128(0xa45c254e_df1c_4efd_8020_67d146a850e0),
            pid: 14,
        };

        let prop = unsafe {
            store.GetValue(&pkey).map_err(|e| {
                PlatformError::Internal(format!("Failed to get device name property: {}", e))
            })?
        };

        // Extract string from PROPVARIANT using helper function
        let name = unsafe {
            match PropVariantToStringAlloc(&prop) {
                Ok(pwstr) => {
                    let result = pwstr.to_string().unwrap_or_else(|_| "Unknown Device".into());
                    windows::Win32::System::Com::CoTaskMemFree(Some(pwstr.as_ptr() as *mut _));
                    result
                }
                Err(_) => "Unknown Device".into(),
            }
        };

        Ok(name)
    }

    /// Get device data flow direction
    #[cfg(target_os = "windows")]
    fn get_device_flow(
        &self,
        device: &windows::Win32::Media::Audio::IMMDevice,
    ) -> Result<DeviceFlow, PlatformError> {
        use windows::Win32::Media::Audio::{eRender, IMMEndpoint};

        let endpoint: IMMEndpoint = device.cast().map_err(|e| {
            PlatformError::Internal(format!("Failed to get endpoint interface: {}", e))
        })?;

        let flow = unsafe {
            endpoint.GetDataFlow().map_err(|e| {
                PlatformError::Internal(format!("Failed to get data flow: {}", e))
            })?
        };

        Ok(if flow == eRender {
            DeviceFlow::Render
        } else {
            DeviceFlow::Capture
        })
    }

    /// Get device state
    #[cfg(target_os = "windows")]
    fn get_device_state(
        &self,
        device: &windows::Win32::Media::Audio::IMMDevice,
    ) -> Result<DeviceState, PlatformError> {
        use windows::Win32::Media::Audio::{
            DEVICE_STATE_ACTIVE, DEVICE_STATE_DISABLED, DEVICE_STATE_NOTPRESENT,
            DEVICE_STATE_UNPLUGGED,
        };

        let state = unsafe {
            device.GetState().map_err(|e| {
                PlatformError::Internal(format!("Failed to get device state: {}", e))
            })?
        };

        Ok(match state {
            DEVICE_STATE_ACTIVE => DeviceState::Active,
            DEVICE_STATE_DISABLED => DeviceState::Disabled,
            DEVICE_STATE_NOTPRESENT => DeviceState::NotPresent,
            DEVICE_STATE_UNPLUGGED => DeviceState::Unplugged,
            _ => DeviceState::NotPresent,
        })
    }

    /// Find virtual audio devices (VB-Cable, VAC, Voicemeeter, etc.)
    ///
    /// Gecko cannot create virtual devices on Windows, but can use existing ones.
    #[cfg(target_os = "windows")]
    pub fn find_virtual_devices(&self) -> Result<Vec<DeviceInfo>, PlatformError> {
        let all_devices = self.enumerate_all()?;

        // Known virtual audio device name patterns (case-insensitive)
        let virtual_patterns = [
            "vb-cable",
            "vb-audio",
            "virtual audio cable",
            "voicemeeter",
            "cable input",
            "cable output",
            "vac ",
            "virtual cable",
            "blackhole",
        ];

        let virtual_devices: Vec<DeviceInfo> = all_devices
            .into_iter()
            .filter(|dev| {
                let name_lower = dev.name.to_lowercase();
                virtual_patterns
                    .iter()
                    .any(|pattern| name_lower.contains(pattern))
            })
            .collect();

        if !virtual_devices.is_empty() {
            tracing::info!("Found {} virtual audio devices", virtual_devices.len());
            for dev in &virtual_devices {
                tracing::info!("  - {}", dev.name);
            }
        }

        Ok(virtual_devices)
    }

    #[cfg(not(target_os = "windows"))]
    pub fn enumerate_all(&self) -> Result<Vec<DeviceInfo>, PlatformError> {
        Err(PlatformError::FeatureNotAvailable(
            "WASAPI only available on Windows".into(),
        ))
    }

    #[cfg(not(target_os = "windows"))]
    pub fn enumerate_render_devices(&self) -> Result<Vec<DeviceInfo>, PlatformError> {
        Err(PlatformError::FeatureNotAvailable(
            "WASAPI only available on Windows".into(),
        ))
    }

    #[cfg(not(target_os = "windows"))]
    pub fn get_default_render_device(&self) -> Result<DeviceInfo, PlatformError> {
        Err(PlatformError::FeatureNotAvailable(
            "WASAPI only available on Windows".into(),
        ))
    }

    #[cfg(not(target_os = "windows"))]
    pub fn get_default_capture_device(&self) -> Result<DeviceInfo, PlatformError> {
        Err(PlatformError::FeatureNotAvailable(
            "WASAPI only available on Windows".into(),
        ))
    }

    #[cfg(not(target_os = "windows"))]
    pub fn find_virtual_devices(&self) -> Result<Vec<DeviceInfo>, PlatformError> {
        Err(PlatformError::FeatureNotAvailable(
            "WASAPI only available on Windows".into(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_device_flow_values() {
        assert_ne!(DeviceFlow::Render, DeviceFlow::Capture);
    }

    #[test]
    fn test_device_state_values() {
        assert_ne!(DeviceState::Active, DeviceState::Disabled);
        assert_ne!(DeviceState::NotPresent, DeviceState::Unplugged);
    }

    #[test]
    #[cfg(target_os = "windows")]
    fn test_device_enumeration() {
        use super::super::com::ComGuard;

        let _com = ComGuard::new().expect("COM init failed");
        let enumerator = DeviceEnumerator::new().expect("Enumerator creation failed");

        let devices = enumerator.enumerate_all();
        assert!(devices.is_ok(), "Should enumerate devices");

        let device_list = devices.unwrap();
        println!("Found {} audio devices", device_list.len());

        for dev in &device_list {
            println!(
                "  {} - {} ({:?}, default: {})",
                dev.name, dev.id, dev.flow, dev.is_default
            );
        }
    }

    #[test]
    #[cfg(target_os = "windows")]
    fn test_default_render_device() {
        use super::super::com::ComGuard;

        let _com = ComGuard::new().expect("COM init failed");
        let enumerator = DeviceEnumerator::new().expect("Enumerator creation failed");

        let device = enumerator.get_default_render_device();
        assert!(device.is_ok(), "Should get default render device");

        let dev = device.unwrap();
        println!("Default render device: {}", dev.name);
        assert!(dev.is_default);
        assert_eq!(dev.flow, DeviceFlow::Render);
    }

    #[test]
    #[cfg(target_os = "windows")]
    fn test_virtual_device_detection() {
        use super::super::com::ComGuard;

        let _com = ComGuard::new().expect("COM init failed");
        let enumerator = DeviceEnumerator::new().expect("Enumerator creation failed");

        let virtual_devices = enumerator.find_virtual_devices();
        assert!(virtual_devices.is_ok());

        let devices = virtual_devices.unwrap();
        println!(
            "Found {} virtual devices: {:?}",
            devices.len(),
            devices.iter().map(|d| &d.name).collect::<Vec<_>>()
        );
    }
}
