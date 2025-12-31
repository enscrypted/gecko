//! WASAPI Audio Session Enumeration
//!
//! Enumerates active audio sessions using IAudioSessionManager2.
//! This identifies which processes are currently producing audio.
//!
//! # Usage
//!
//! Unlike process enumeration which shows ALL running processes,
//! session enumeration only shows processes with active audio sessions.
//! This is useful for:
//!
//! - Filtering the process list to audio-producing apps only
//! - Getting session volume/mute state
//! - Identifying audio sources for per-app capture

use crate::error::PlatformError;
use super::message::{AudioSessionInfo, SessionState};
use std::collections::HashSet;

#[cfg(target_os = "windows")]
use windows::core::Interface;

/// Audio session enumerator
///
/// Uses IAudioSessionManager2 to enumerate active audio sessions.
pub struct SessionEnumerator {
    /// Placeholder for non-Windows builds
    #[cfg(not(target_os = "windows"))]
    _phantom: std::marker::PhantomData<()>,
}

impl SessionEnumerator {
    /// Create a new session enumerator
    ///
    /// # Requirements
    ///
    /// COM must be initialized on the calling thread.
    pub fn new() -> Self {
        #[cfg(target_os = "windows")]
        {
            Self {}
        }
        #[cfg(not(target_os = "windows"))]
        {
            Self {
                _phantom: std::marker::PhantomData,
            }
        }
    }

    /// Enumerate all audio sessions on the default render device
    ///
    /// Returns sessions for all processes producing audio.
    #[cfg(target_os = "windows")]
    pub fn enumerate_sessions(&self) -> Result<Vec<AudioSessionInfo>, PlatformError> {
        use windows::Win32::Media::Audio::{
            eConsole, eRender, AudioSessionStateActive, AudioSessionStateExpired,
            AudioSessionStateInactive, IAudioSessionControl2, IAudioSessionManager2,
            IMMDeviceEnumerator, ISimpleAudioVolume, MMDeviceEnumerator,
        };
        use windows::Win32::System::Com::{CoCreateInstance, CoTaskMemFree, CLSCTX_ALL};

        // Get device enumerator
        let device_enumerator: IMMDeviceEnumerator = unsafe {
            CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL).map_err(|e| {
                PlatformError::Internal(format!("Failed to create device enumerator: {}", e))
            })?
        };

        // Get default render device
        let device = unsafe {
            device_enumerator
                .GetDefaultAudioEndpoint(eRender, eConsole)
                .map_err(|e| {
                    PlatformError::Internal(format!("Failed to get default device: {}", e))
                })?
        };

        // Activate session manager
        let session_manager: IAudioSessionManager2 = unsafe {
            device.Activate(CLSCTX_ALL, None).map_err(|e| {
                PlatformError::Internal(format!("Failed to activate session manager: {}", e))
            })?
        };

        // Get session enumerator
        let session_enumerator = unsafe {
            session_manager.GetSessionEnumerator().map_err(|e| {
                PlatformError::Internal(format!("Failed to get session enumerator: {}", e))
            })?
        };

        // Get session count
        let count = unsafe {
            session_enumerator.GetCount().map_err(|e| {
                PlatformError::Internal(format!("Failed to get session count: {}", e))
            })?
        };

        tracing::debug!("Found {} audio sessions", count);

        let mut sessions = Vec::new();

        for i in 0..count {
            let session_control = match unsafe { session_enumerator.GetSession(i) } {
                Ok(s) => s,
                Err(_) => continue,
            };

            // Get extended session control for PID access
            let session_control2: IAudioSessionControl2 = match session_control.cast() {
                Ok(s) => s,
                Err(_) => continue,
            };

            // Get process ID
            let pid = match unsafe { session_control2.GetProcessId() } {
                Ok(p) => p,
                Err(_) => continue, // System session or error
            };

            // Skip system sessions (PID 0, 4)
            if pid == 0 || pid == 4 {
                continue;
            }

            // Get session state
            let state_raw = unsafe {
                session_control2
                    .GetState()
                    .unwrap_or(AudioSessionStateInactive)
            };

            let state = if state_raw == AudioSessionStateActive {
                SessionState::Active
            } else if state_raw == AudioSessionStateInactive {
                SessionState::Inactive
            } else if state_raw == AudioSessionStateExpired {
                SessionState::Expired
            } else {
                SessionState::Inactive
            };

            // Get display name (may be empty)
            let display_name = unsafe {
                match session_control2.GetDisplayName() {
                    Ok(name_pwstr) => {
                        let name = name_pwstr.to_string().unwrap_or_default();
                        CoTaskMemFree(Some(name_pwstr.as_ptr() as *mut _));
                        name
                    }
                    Err(_) => String::new(),
                }
            };

            // Get icon path (may be empty)
            let icon_path = unsafe {
                match session_control2.GetIconPath() {
                    Ok(path_pwstr) => {
                        let path = path_pwstr.to_string().ok().filter(|s| !s.is_empty());
                        CoTaskMemFree(Some(path_pwstr.as_ptr() as *mut _));
                        path
                    }
                    Err(_) => None,
                }
            };

            // Get volume control
            let (volume, muted) = unsafe {
                match session_control.cast::<ISimpleAudioVolume>() {
                    Ok(vol_control) => {
                        let vol: f32 = vol_control.GetMasterVolume().unwrap_or(1.0);
                        let mute: bool = vol_control
                            .GetMute()
                            .map(|m: windows::Win32::Foundation::BOOL| m.as_bool())
                            .unwrap_or(false);
                        (vol, mute)
                    }
                    Err(_) => (1.0, false),
                }
            };

            // Get process name from PID
            let name = get_process_name(pid).unwrap_or_else(|| format!("PID {}", pid));

            sessions.push(AudioSessionInfo {
                pid,
                name,
                display_name,
                state,
                volume,
                muted,
                icon_path,
            });
        }

        tracing::debug!(
            "Enumerated {} audio sessions (excluding system)",
            sessions.len()
        );

        Ok(sessions)
    }

    /// Enumerate only active audio sessions (currently producing audio)
    #[cfg(target_os = "windows")]
    pub fn enumerate_active_sessions(&self) -> Result<Vec<AudioSessionInfo>, PlatformError> {
        let all_sessions = self.enumerate_sessions()?;

        let active: Vec<AudioSessionInfo> = all_sessions
            .into_iter()
            .filter(|s| s.state == SessionState::Active)
            .collect();

        tracing::debug!("Found {} active audio sessions", active.len());

        Ok(active)
    }

    /// Get set of PIDs with active audio sessions
    ///
    /// Useful for filtering a process list to only audio-producing apps.
    #[cfg(target_os = "windows")]
    pub fn get_active_pids(&self) -> Result<HashSet<u32>, PlatformError> {
        let sessions = self.enumerate_active_sessions()?;
        let pids: HashSet<u32> = sessions.into_iter().map(|s| s.pid).collect();

        tracing::debug!("Found {} PIDs with active audio", pids.len());

        Ok(pids)
    }

    /// Get all PIDs with audio sessions (active or inactive)
    #[cfg(target_os = "windows")]
    pub fn get_all_audio_pids(&self) -> Result<HashSet<u32>, PlatformError> {
        let sessions = self.enumerate_sessions()?;
        let pids: HashSet<u32> = sessions
            .into_iter()
            .filter(|s| s.state != SessionState::Expired)
            .map(|s| s.pid)
            .collect();

        Ok(pids)
    }

    #[cfg(not(target_os = "windows"))]
    pub fn enumerate_sessions(&self) -> Result<Vec<AudioSessionInfo>, PlatformError> {
        Err(PlatformError::FeatureNotAvailable(
            "Audio sessions only available on Windows".into(),
        ))
    }

    #[cfg(not(target_os = "windows"))]
    pub fn enumerate_active_sessions(&self) -> Result<Vec<AudioSessionInfo>, PlatformError> {
        Err(PlatformError::FeatureNotAvailable(
            "Audio sessions only available on Windows".into(),
        ))
    }

    #[cfg(not(target_os = "windows"))]
    pub fn get_active_pids(&self) -> Result<HashSet<u32>, PlatformError> {
        Err(PlatformError::FeatureNotAvailable(
            "Audio sessions only available on Windows".into(),
        ))
    }

    #[cfg(not(target_os = "windows"))]
    pub fn get_all_audio_pids(&self) -> Result<HashSet<u32>, PlatformError> {
        Err(PlatformError::FeatureNotAvailable(
            "Audio sessions only available on Windows".into(),
        ))
    }
}

impl Default for SessionEnumerator {
    fn default() -> Self {
        Self::new()
    }
}

/// Get process name from PID using snapshot API
#[cfg(target_os = "windows")]
fn get_process_name(pid: u32) -> Option<String> {
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W,
        TH32CS_SNAPPROCESS,
    };

    unsafe {
        let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0).ok()?;

        let mut entry = PROCESSENTRY32W {
            dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
            ..Default::default()
        };

        let mut result = None;

        if Process32FirstW(snapshot, &mut entry).is_ok() {
            loop {
                if entry.th32ProcessID == pid {
                    // Convert wide string to Rust String
                    let name_end = entry
                        .szExeFile
                        .iter()
                        .position(|&c| c == 0)
                        .unwrap_or(entry.szExeFile.len());
                    let name = String::from_utf16_lossy(&entry.szExeFile[..name_end]);

                    // Strip .exe extension for cleaner display
                    result = Some(
                        name.strip_suffix(".exe")
                            .unwrap_or(&name)
                            .to_string(),
                    );
                    break;
                }

                if Process32NextW(snapshot, &mut entry).is_err() {
                    break;
                }
            }
        }

        let _ = CloseHandle(snapshot);
        result
    }
}

#[cfg(not(target_os = "windows"))]
fn get_process_name(_pid: u32) -> Option<String> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_session_enumerator_creation() {
        let _enumerator = SessionEnumerator::new();
    }

    #[test]
    fn test_session_state_equality() {
        assert_eq!(SessionState::Active, SessionState::Active);
        assert_ne!(SessionState::Active, SessionState::Inactive);
        assert_ne!(SessionState::Inactive, SessionState::Expired);
    }

    #[test]
    #[cfg(target_os = "windows")]
    fn test_session_enumeration() {
        use super::super::com::ComGuard;

        let _com = ComGuard::new().expect("COM init failed");
        let enumerator = SessionEnumerator::new();

        let sessions = enumerator.enumerate_sessions();
        assert!(sessions.is_ok(), "Should enumerate sessions");

        let session_list = sessions.unwrap();
        println!("Found {} audio sessions", session_list.len());

        for session in &session_list {
            println!(
                "  PID {}: {} (state: {:?}, vol: {:.2}, muted: {})",
                session.pid, session.name, session.state, session.volume, session.muted
            );
        }
    }

    #[test]
    #[cfg(target_os = "windows")]
    fn test_active_session_enumeration() {
        use super::super::com::ComGuard;

        let _com = ComGuard::new().expect("COM init failed");
        let enumerator = SessionEnumerator::new();

        let active = enumerator.enumerate_active_sessions();
        assert!(active.is_ok(), "Should enumerate active sessions");

        let session_list = active.unwrap();
        println!("Found {} active audio sessions", session_list.len());

        // All returned sessions should be active
        for session in &session_list {
            assert_eq!(session.state, SessionState::Active);
        }
    }

    #[test]
    #[cfg(target_os = "windows")]
    fn test_get_active_pids() {
        use super::super::com::ComGuard;

        let _com = ComGuard::new().expect("COM init failed");
        let enumerator = SessionEnumerator::new();

        let pids = enumerator.get_active_pids();
        assert!(pids.is_ok(), "Should get active PIDs");

        let pid_set = pids.unwrap();
        println!("Active audio PIDs: {:?}", pid_set);
    }
}
