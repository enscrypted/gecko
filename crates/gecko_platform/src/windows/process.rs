//! Windows Process Enumeration
//!
//! Enumerates running processes using the Toolhelp32 snapshot API.
//! Used in conjunction with session enumeration to identify audio-producing apps.
//!
//! # Process Filtering
//!
//! The raw process list includes many system processes. Gecko filters to:
//! 1. Processes with audio sessions (via SessionEnumerator)
//! 2. Or all windowed applications if requested

use crate::error::PlatformError;
use crate::traits::ApplicationInfo;

/// Information about a running process
#[derive(Debug, Clone)]
pub struct ProcessInfo {
    /// Process ID
    pub pid: u32,
    /// Parent process ID
    pub parent_pid: u32,
    /// Executable name (e.g., "chrome.exe")
    pub exe_name: String,
    /// Number of threads
    pub thread_count: u32,
}

impl ProcessInfo {
    /// Convert to ApplicationInfo for the platform trait
    pub fn to_application_info(&self, is_active: bool) -> ApplicationInfo {
        // Strip .exe extension for cleaner display
        let name = self
            .exe_name
            .strip_suffix(".exe")
            .unwrap_or(&self.exe_name)
            .to_string();

        ApplicationInfo {
            pid: self.pid,
            name,
            icon: None, // Icon extraction requires additional Windows API calls
            is_active,
        }
    }
}

/// Process enumerator using Toolhelp32 snapshot
pub struct ProcessEnumerator;

impl ProcessEnumerator {
    /// Create a new process enumerator
    pub fn new() -> Self {
        Self
    }

    /// Enumerate all running processes
    #[cfg(target_os = "windows")]
    pub fn enumerate_all(&self) -> Result<Vec<ProcessInfo>, PlatformError> {
        use windows::Win32::Foundation::CloseHandle;
        use windows::Win32::System::Diagnostics::ToolHelp::{
            CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W,
            TH32CS_SNAPPROCESS,
        };

        let snapshot = unsafe {
            CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0).map_err(|e| {
                PlatformError::Internal(format!("Failed to create process snapshot: {}", e))
            })?
        };

        // RAII guard for snapshot handle
        struct SnapshotGuard(windows::Win32::Foundation::HANDLE);
        impl Drop for SnapshotGuard {
            fn drop(&mut self) {
                unsafe {
                    let _ = CloseHandle(self.0);
                }
            }
        }
        let _guard = SnapshotGuard(snapshot);

        let mut entry = PROCESSENTRY32W {
            dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
            ..Default::default()
        };

        let mut processes = Vec::new();

        unsafe {
            if Process32FirstW(snapshot, &mut entry).is_ok() {
                loop {
                    // Skip system processes (PID 0 = System Idle, PID 4 = System)
                    if entry.th32ProcessID != 0 && entry.th32ProcessID != 4 {
                        // Convert wide string to Rust String
                        let name_end = entry
                            .szExeFile
                            .iter()
                            .position(|&c| c == 0)
                            .unwrap_or(entry.szExeFile.len());
                        let exe_name = String::from_utf16_lossy(&entry.szExeFile[..name_end]);

                        processes.push(ProcessInfo {
                            pid: entry.th32ProcessID,
                            parent_pid: entry.th32ParentProcessID,
                            exe_name,
                            thread_count: entry.cntThreads,
                        });
                    }

                    if Process32NextW(snapshot, &mut entry).is_err() {
                        break;
                    }
                }
            }
        }

        tracing::debug!("Enumerated {} processes", processes.len());

        Ok(processes)
    }

    /// Enumerate processes, filtered to those with audio sessions
    #[cfg(target_os = "windows")]
    pub fn enumerate_audio_processes(&self) -> Result<Vec<ProcessInfo>, PlatformError> {
        use super::session::SessionEnumerator;

        let session_enum = SessionEnumerator::new();
        let audio_pids = session_enum.get_all_audio_pids()?;

        if audio_pids.is_empty() {
            tracing::debug!("No processes with audio sessions found");
            return Ok(Vec::new());
        }

        let all_processes = self.enumerate_all()?;

        let audio_processes: Vec<ProcessInfo> = all_processes
            .into_iter()
            .filter(|p| audio_pids.contains(&p.pid))
            .collect();

        tracing::debug!(
            "Found {} processes with audio sessions",
            audio_processes.len()
        );

        Ok(audio_processes)
    }

    /// Get information about a specific process by PID
    #[cfg(target_os = "windows")]
    pub fn get_process(&self, pid: u32) -> Result<ProcessInfo, PlatformError> {
        use windows::Win32::Foundation::CloseHandle;
        use windows::Win32::System::Diagnostics::ToolHelp::{
            CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W,
            TH32CS_SNAPPROCESS,
        };

        let snapshot = unsafe {
            CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0).map_err(|e| {
                PlatformError::Internal(format!("Failed to create process snapshot: {}", e))
            })?
        };

        struct SnapshotGuard(windows::Win32::Foundation::HANDLE);
        impl Drop for SnapshotGuard {
            fn drop(&mut self) {
                unsafe {
                    let _ = CloseHandle(self.0);
                }
            }
        }
        let _guard = SnapshotGuard(snapshot);

        let mut entry = PROCESSENTRY32W {
            dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
            ..Default::default()
        };

        unsafe {
            if Process32FirstW(snapshot, &mut entry).is_ok() {
                loop {
                    if entry.th32ProcessID == pid {
                        let name_end = entry
                            .szExeFile
                            .iter()
                            .position(|&c| c == 0)
                            .unwrap_or(entry.szExeFile.len());
                        let exe_name = String::from_utf16_lossy(&entry.szExeFile[..name_end]);

                        return Ok(ProcessInfo {
                            pid: entry.th32ProcessID,
                            parent_pid: entry.th32ParentProcessID,
                            exe_name,
                            thread_count: entry.cntThreads,
                        });
                    }

                    if Process32NextW(snapshot, &mut entry).is_err() {
                        break;
                    }
                }
            }
        }

        Err(PlatformError::ApplicationNotFound(format!("PID {}", pid)))
    }

    /// Find process by executable name (case-insensitive)
    #[cfg(target_os = "windows")]
    pub fn find_by_name(&self, name: &str) -> Result<Vec<ProcessInfo>, PlatformError> {
        let all = self.enumerate_all()?;
        let name_lower = name.to_lowercase();

        let matches: Vec<ProcessInfo> = all
            .into_iter()
            .filter(|p| p.exe_name.to_lowercase().contains(&name_lower))
            .collect();

        Ok(matches)
    }

    #[cfg(not(target_os = "windows"))]
    pub fn enumerate_all(&self) -> Result<Vec<ProcessInfo>, PlatformError> {
        Err(PlatformError::FeatureNotAvailable(
            "Process enumeration only available on Windows".into(),
        ))
    }

    #[cfg(not(target_os = "windows"))]
    pub fn enumerate_audio_processes(&self) -> Result<Vec<ProcessInfo>, PlatformError> {
        Err(PlatformError::FeatureNotAvailable(
            "Process enumeration only available on Windows".into(),
        ))
    }

    #[cfg(not(target_os = "windows"))]
    pub fn get_process(&self, _pid: u32) -> Result<ProcessInfo, PlatformError> {
        Err(PlatformError::FeatureNotAvailable(
            "Process enumeration only available on Windows".into(),
        ))
    }

    #[cfg(not(target_os = "windows"))]
    pub fn find_by_name(&self, _name: &str) -> Result<Vec<ProcessInfo>, PlatformError> {
        Err(PlatformError::FeatureNotAvailable(
            "Process enumeration only available on Windows".into(),
        ))
    }
}

impl Default for ProcessEnumerator {
    fn default() -> Self {
        Self::new()
    }
}

/// Check if a process is still running
#[cfg(target_os = "windows")]
pub fn is_process_running(pid: u32) -> bool {
    use windows::Win32::Foundation::{CloseHandle, STILL_ACTIVE};
    use windows::Win32::System::Threading::{
        GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
    };

    unsafe {
        let handle = match OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) {
            Ok(h) => h,
            Err(_) => return false,
        };

        let mut exit_code = 0u32;
        let result = GetExitCodeProcess(handle, &mut exit_code);
        let _ = CloseHandle(handle);

        result.is_ok() && exit_code == STILL_ACTIVE.0 as u32
    }
}

#[cfg(not(target_os = "windows"))]
pub fn is_process_running(_pid: u32) -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_process_enumerator_creation() {
        let _enumerator = ProcessEnumerator::new();
    }

    #[test]
    #[cfg(target_os = "windows")]
    fn test_enumerate_all_processes() {
        let enumerator = ProcessEnumerator::new();
        let processes = enumerator.enumerate_all();

        assert!(processes.is_ok(), "Should enumerate processes");

        let proc_list = processes.unwrap();
        assert!(!proc_list.is_empty(), "Should find at least one process");

        println!("Found {} processes", proc_list.len());

        // Should include common processes
        let has_system = proc_list.iter().any(|p| {
            p.exe_name.to_lowercase().contains("explorer")
                || p.exe_name.to_lowercase().contains("svchost")
        });
        println!("Found system processes: {}", has_system);
    }

    #[test]
    #[cfg(target_os = "windows")]
    fn test_enumerate_audio_processes() {
        use super::super::com::ComGuard;

        let _com = ComGuard::new().expect("COM init failed");
        let enumerator = ProcessEnumerator::new();

        let audio_processes = enumerator.enumerate_audio_processes();
        assert!(audio_processes.is_ok(), "Should enumerate audio processes");

        let proc_list = audio_processes.unwrap();
        println!("Found {} processes with audio", proc_list.len());

        for proc in &proc_list {
            println!("  PID {}: {}", proc.pid, proc.exe_name);
        }
    }

    #[test]
    #[cfg(target_os = "windows")]
    fn test_to_application_info() {
        let proc = ProcessInfo {
            pid: 1234,
            parent_pid: 1,
            exe_name: "chrome.exe".to_string(),
            thread_count: 10,
        };

        let app_info = proc.to_application_info(true);

        assert_eq!(app_info.pid, 1234);
        assert_eq!(app_info.name, "chrome"); // .exe stripped
        assert!(app_info.is_active);
    }

    #[test]
    #[cfg(target_os = "windows")]
    fn test_is_process_running() {
        // PID 4 is always the System process on Windows
        assert!(!is_process_running(4), "System process check");

        // Our own process should be running
        let our_pid = std::process::id();
        assert!(is_process_running(our_pid), "Own process should be running");

        // Non-existent PID should not be running
        assert!(!is_process_running(0xFFFFFFFF), "Invalid PID should not be running");
    }

    #[test]
    #[cfg(target_os = "windows")]
    fn test_find_by_name() {
        let enumerator = ProcessEnumerator::new();

        // Should find at least svchost
        let results = enumerator.find_by_name("svchost");
        assert!(results.is_ok());

        let procs = results.unwrap();
        // svchost typically has many instances
        println!("Found {} svchost instances", procs.len());
    }
}
