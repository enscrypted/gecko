//! CoreAudio Device and Application Enumeration
//!
//! This module provides safe Rust wrappers around CoreAudio APIs for:
//! - Listing audio devices (for `list_nodes()`)
//! - Listing audio-producing applications (for `list_applications()`)
//!
//! # CoreAudio Concepts
//!
//! - AudioObject: Everything in CoreAudio is an object with properties
//! - AudioDevice: An audio endpoint (speakers, headphones, virtual device)
//! - AudioStream: A flow of audio data on a device
//!
//! # Safety
//!
//! These functions use unsafe FFI calls but wrap them in safe Rust interfaces.

use crate::error::PlatformError;
use crate::traits::{ApplicationInfo, AudioNode};

// Rust pattern: coreaudio-sys provides raw bindings to CoreAudio C APIs
// We wrap these in safe Rust functions that handle memory management
use coreaudio_sys::{
    kAudioDevicePropertyDeviceNameCFString, kAudioDevicePropertyStreams,
    kAudioDevicePropertyVolumeScalar, kAudioHardwarePropertyDefaultOutputDevice,
    kAudioHardwarePropertyDevices, kAudioObjectPropertyElementMain,
    kAudioObjectPropertyElementMaster, kAudioObjectPropertyScopeGlobal,
    kAudioObjectPropertyScopeOutput, kAudioObjectSystemObject, AudioDeviceID,
    AudioObjectGetPropertyData, AudioObjectGetPropertyDataSize, AudioObjectPropertyAddress,
    AudioObjectSetPropertyData,
};

// Rust pattern: Use the core-foundation crate for safe CFString handling
// This provides proper memory management and automatic bridging to Rust strings
use core_foundation::base::TCFType;
use core_foundation::string::CFString;

use std::ffi::c_void;
use std::mem;
use std::ptr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::thread;
use std::time::{Duration, Instant};

use tracing::{debug, trace, warn};

// ============================================================================
// GUI Apps Cache - Async background refresh to never block main thread
// ============================================================================

/// Cached GUI apps list with async background refresh
struct GuiAppsCache {
    /// Cached list of (pid, name) tuples
    apps: Vec<(u32, String)>,
    /// When the cache was last updated
    last_update: Option<Instant>,
}

impl GuiAppsCache {
    const fn new() -> Self {
        Self {
            apps: Vec::new(),
            last_update: None,
        }
    }
}

/// Global cache for GUI apps - refreshed asynchronously in background
/// Uses Mutex for thread safety since this is called from the Tauri command thread
static GUI_APPS_CACHE: Mutex<GuiAppsCache> = Mutex::new(GuiAppsCache::new());

/// Flag to track if cache refresher thread has been started
static CACHE_REFRESHER_STARTED: AtomicBool = AtomicBool::new(false);

/// How long to cache the GUI apps list before triggering a refresh
/// 3 seconds is a good balance - apps don't change that often
const GUI_APPS_CACHE_TTL: Duration = Duration::from_secs(3);

/// Start the background cache refresher thread (called once on first access)
///
/// This function blocks until the initial cache is populated, ensuring
/// the first caller gets valid data. Subsequent calls return immediately.
fn start_cache_refresher() {
    // Only start once - use compare_exchange for thread safety
    if CACHE_REFRESHER_STARTED
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_err()
    {
        // Already started - but we need to wait for initial population
        // Spin-wait until the cache has been populated at least once
        loop {
            {
                let cache = GUI_APPS_CACHE.lock().unwrap();
                if cache.last_update.is_some() {
                    return; // Cache is ready
                }
            }
            // Brief sleep to avoid busy-waiting
            thread::sleep(Duration::from_millis(10));
        }
    }

    // We're the first caller - do the initial synchronous fetch
    // Other callers will wait above until we're done
    let initial_apps = refresh_gui_apps_cache_sync();
    {
        let mut cache = GUI_APPS_CACHE.lock().unwrap();
        cache.apps = initial_apps;
        cache.last_update = Some(Instant::now());
    }

    trace!("GUI apps cache initialized ({} apps)", GUI_APPS_CACHE.lock().unwrap().apps.len());

    // Spawn background thread that periodically refreshes the cache
    thread::spawn(|| {
        loop {
            // Sleep for the cache TTL
            thread::sleep(GUI_APPS_CACHE_TTL);

            // Refresh the cache in the background
            let apps = refresh_gui_apps_cache_sync();

            // Update the cache
            if let Ok(mut cache) = GUI_APPS_CACHE.lock() {
                cache.apps = apps;
                cache.last_update = Some(Instant::now());
                trace!("Background cache refresh complete ({} apps)", cache.apps.len());
            }
        }
    });

    trace!("Started GUI apps cache background refresher");
}

/// Convert a raw CFStringRef to a Rust String
///
/// # Safety
///
/// The cf_string must be a valid CFStringRef. This function takes ownership
/// and releases the CFString after conversion.
unsafe fn cf_string_to_string(cf_string_ref: *const c_void) -> Option<String> {
    if cf_string_ref.is_null() {
        return None;
    }

    // Rust pattern: Wrap raw CFStringRef in safe CFString wrapper
    // which handles memory management automatically
    let cf_string = CFString::wrap_under_create_rule(cf_string_ref as _);
    Some(cf_string.to_string())
}

/// List all audio devices on the system
///
/// This enumerates CoreAudio devices including:
/// - Built-in speakers/headphones
/// - External audio devices (USB DACs, etc.)
/// - Virtual audio devices (BlackHole, Loopback, etc.)
/// - Gecko's own virtual devices (if HAL plugin is installed)
pub fn list_audio_devices() -> Result<Vec<AudioNode>, PlatformError> {
    let mut devices = Vec::new();

    // Rust pattern: Use unsafe block for FFI, then wrap result in safe Rust
    unsafe {
        // Get the number of audio devices
        let property_address = AudioObjectPropertyAddress {
            mSelector: kAudioHardwarePropertyDevices,
            mScope: kAudioObjectPropertyScopeGlobal,
            mElement: kAudioObjectPropertyElementMain,
        };

        let mut size: u32 = 0;
        let status = AudioObjectGetPropertyDataSize(
            kAudioObjectSystemObject,
            &property_address,
            0,
            ptr::null(),
            &mut size,
        );

        if status != 0 {
            warn!("Failed to get audio devices size: OSStatus {}", status);
            return Ok(devices);
        }

        // Calculate number of devices
        let device_count = size as usize / mem::size_of::<AudioDeviceID>();
        if device_count == 0 {
            return Ok(devices);
        }

        // Allocate buffer for device IDs
        let mut device_ids: Vec<AudioDeviceID> = vec![0; device_count];

        let status = AudioObjectGetPropertyData(
            kAudioObjectSystemObject,
            &property_address,
            0,
            ptr::null(),
            &mut size,
            device_ids.as_mut_ptr() as *mut c_void,
        );

        if status != 0 {
            warn!("Failed to get audio devices: OSStatus {}", status);
            return Ok(devices);
        }

        trace!("Found {} audio devices", device_count);

        // Get info for each device
        for device_id in device_ids {
            if let Some(node) = get_device_info(device_id) {
                devices.push(node);
            }
        }
    }

    Ok(devices)
}

/// Get information about a specific audio device
///
/// Returns None if the device info cannot be retrieved.
fn get_device_info(device_id: AudioDeviceID) -> Option<AudioNode> {
    // Get device name
    let name = get_device_name(device_id)?;

    // Determine media class based on whether device has output streams
    let media_class = if has_output_streams(device_id) {
        "Audio/Sink".to_string()
    } else {
        "Audio/Source".to_string()
    };

    Some(AudioNode {
        id: device_id,
        name,
        media_class,
        application: None, // Devices don't have associated applications
    })
}

/// Get the name of an audio device
fn get_device_name(device_id: AudioDeviceID) -> Option<String> {
    unsafe {
        let property_address = AudioObjectPropertyAddress {
            mSelector: kAudioDevicePropertyDeviceNameCFString,
            mScope: kAudioObjectPropertyScopeGlobal,
            mElement: kAudioObjectPropertyElementMain,
        };

        let mut name_cf: *const c_void = ptr::null();
        let mut size = mem::size_of::<*const c_void>() as u32;

        let status = AudioObjectGetPropertyData(
            device_id,
            &property_address,
            0,
            ptr::null(),
            &mut size,
            &mut name_cf as *mut *const c_void as *mut c_void,
        );

        if status != 0 {
            return None;
        }

        cf_string_to_string(name_cf)
    }
}

/// Check if a device has output streams (is a sink/speaker)
fn has_output_streams(device_id: AudioDeviceID) -> bool {
    unsafe {
        let property_address = AudioObjectPropertyAddress {
            mSelector: kAudioDevicePropertyStreams,
            mScope: kAudioObjectPropertyScopeOutput,
            mElement: kAudioObjectPropertyElementMain,
        };

        let mut size: u32 = 0;
        let status = AudioObjectGetPropertyDataSize(
            device_id,
            &property_address,
            0,
            ptr::null(),
            &mut size,
        );

        // If we can get the size and it's > 0, device has output streams
        status == 0 && size > 0
    }
}

// Import for default input device
use coreaudio_sys::kAudioHardwarePropertyDefaultInputDevice;

/// Get the default output device ID
pub fn get_default_output_device() -> Result<AudioDeviceID, PlatformError> {
    unsafe {
        let property_address = AudioObjectPropertyAddress {
            mSelector: kAudioHardwarePropertyDefaultOutputDevice,
            mScope: kAudioObjectPropertyScopeGlobal,
            mElement: kAudioObjectPropertyElementMain,
        };

        let mut device_id: AudioDeviceID = 0;
        let mut size = mem::size_of::<AudioDeviceID>() as u32;

        let status = AudioObjectGetPropertyData(
            kAudioObjectSystemObject,
            &property_address,
            0,
            ptr::null(),
            &mut size,
            &mut device_id as *mut AudioDeviceID as *mut c_void,
        );

        if status != 0 {
            return Err(PlatformError::Internal(format!(
                "Failed to get default output device: OSStatus {}",
                status
            )));
        }

        Ok(device_id)
    }
}

/// Get the default input device ID
pub fn get_default_input_device() -> Result<AudioDeviceID, PlatformError> {
    unsafe {
        let property_address = AudioObjectPropertyAddress {
            mSelector: kAudioHardwarePropertyDefaultInputDevice,
            mScope: kAudioObjectPropertyScopeGlobal,
            mElement: kAudioObjectPropertyElementMain,
        };

        let mut device_id: AudioDeviceID = 0;
        let mut size = mem::size_of::<AudioDeviceID>() as u32;

        let status = AudioObjectGetPropertyData(
            kAudioObjectSystemObject,
            &property_address,
            0,
            ptr::null(),
            &mut size,
            &mut device_id as *mut AudioDeviceID as *mut c_void,
        );

        if status != 0 {
            return Err(PlatformError::Internal(format!(
                "Failed to get default input device: OSStatus {}",
                status
            )));
        }

        Ok(device_id)
    }
}

/// List audio-producing applications
///
/// # Implementation Notes
///
/// On macOS 14.4+, we can use the Process Tap API to enumerate audio-producing processes.
/// On older macOS, we need to get this information from the helper daemon which monitors
/// applications that have registered with our virtual audio devices.
///
/// For now, this returns a basic list of known audio apps found via process enumeration.
/// Full implementation requires either:
/// - macOS 14.4+: AudioHardwareCreateProcessTap enumeration
/// - Older macOS: Helper daemon communication
pub fn list_audio_applications() -> Result<Vec<ApplicationInfo>, PlatformError> {
    // Rust pattern: Check macOS version to determine available APIs
    let (major, minor, _) = super::process_tap::macos_version();

    if major > 14 || (major == 14 && minor >= 4) {
        // macOS 14.4+: Can use Process Tap enumeration
        // TODO: Implement using AudioHardwareTapDescription
        trace!("Process Tap API available - using for app enumeration");
        list_apps_via_process_tap()
    } else {
        // Older macOS: Get from helper daemon
        trace!("Using helper daemon for app enumeration");
        list_apps_via_helper_daemon()
    }
}

/// List audio apps using Process Tap API (macOS 14.4+)
///
/// The Process Tap API can enumerate processes that are currently producing audio.
/// This requires the NSAudioCaptureUsageDescription entitlement.
fn list_apps_via_process_tap() -> Result<Vec<ApplicationInfo>, PlatformError> {
    // TODO: Implement using AudioHardwareTapDescription with:
    // - kAudioHardwareTapDescriptionKey_Processes = kAudioHardwareTapDescriptionKey_AllProcesses
    // - Create an aggregate device to enumerate available taps
    //
    // For now, return empty list. Full implementation requires:
    // 1. Create tap description with mute + mixdown behavior
    // 2. Call AudioHardwareCreateProcessTap to get list of tappable processes
    // 3. Map PIDs to application names using NSRunningApplication

    // Placeholder: Use system process enumeration to find common audio apps
    list_running_audio_apps()
}

/// List audio apps via helper daemon (older macOS)
fn list_apps_via_helper_daemon() -> Result<Vec<ApplicationInfo>, PlatformError> {
    // TODO: Query helper daemon via Unix socket
    // The helper daemon tracks apps that have been routed to Gecko virtual devices

    // Placeholder: Use system process enumeration
    list_running_audio_apps()
}

/// Apps to exclude from the list (system utilities, helpers, and non-audio processes)
const EXCLUDE_PATTERNS: &[&str] = &[
    "Finder",
    "System Preferences",
    "System Settings",
    "Activity Monitor",
    "Terminal",
    "Console",
    "Disk Utility",
    "Keychain Access",
    "Gecko Audio",
    "gecko_ui",
    "loginwindow",
    "WindowManager",
    "Dock",
    "ControlCenter",
    "SystemUIServer",
    "Spotlight",
    "CoreServicesUIAgent",
    "com.apple.",           // Apple helper processes
    "plugin-container",     // Firefox helper
    "Google Chrome Helper", // Chrome helper
    "Slack Helper",         // Slack helper
    "ViewBridgeAuxiliary",
    "UIKitSystem",
    "NotificationCenter",
    "WallpaperAgent",
    "node",      // Node.js processes (dev tools)
    "osascript", // Script runner
];

/// Refresh the GUI apps cache by running AppleScript (synchronous)
///
/// This is the SLOW operation (~400-500ms) - called only by background thread.
/// Returns the list of (pid, name) tuples for all visible GUI apps.
fn refresh_gui_apps_cache_sync() -> Vec<(u32, String)> {
    use std::process::Command;

    let mut apps = Vec::new();

    // FAST approach: Single AppleScript call that returns all data at once
    // The repeat loop in AppleScript is extremely slow - this one-liner is much faster
    let output = Command::new("osascript")
        .args([
            "-e",
            r#"tell application "System Events" to get {unix id, name} of (every application process whose visible is true)"#,
        ])
        .output();

    if let Ok(output) = output {
        if output.status.success() {
            let stdout = String::from_utf8_lossy(&output.stdout);
            let stdout = stdout.trim();

            // AppleScript returns format: "pid1, pid2, ..., name1, name2, ..."
            // All values are comma-separated. First half are PIDs (integers),
            // second half are names (strings).
            let items: Vec<&str> = stdout.split(", ").collect();

            if items.len() >= 2 {
                // Find where PIDs end and names begin by finding first non-numeric item
                let split_idx = items
                    .iter()
                    .position(|s| s.parse::<u32>().is_err())
                    .unwrap_or(items.len() / 2);

                let pids: Vec<u32> = items[..split_idx]
                    .iter()
                    .filter_map(|s| s.trim().parse().ok())
                    .collect();

                let names: Vec<&str> = items[split_idx..].iter().map(|s| s.trim()).collect();

                trace!(
                    "Parsed {} PIDs and {} names from AppleScript",
                    pids.len(),
                    names.len()
                );

                // Combine PIDs and names, filtering out excluded apps
                for i in 0..pids.len().min(names.len()) {
                    let pid = pids[i];
                    let name = names[i];

                    // Skip excluded apps
                    let should_skip = EXCLUDE_PATTERNS
                        .iter()
                        .any(|&pattern| name == pattern || name.starts_with(pattern));

                    if !should_skip && !name.is_empty() {
                        apps.push((pid, name.to_string()));
                    }
                }
            } else if !stdout.is_empty() {
                warn!("Unexpected AppleScript output format: {}", stdout);
            }
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr);
            warn!("AppleScript failed: {}", stderr);
        }
    }

    apps
}

/// Get cached GUI apps (never blocks - returns cached data immediately)
///
/// This function always returns immediately with cached data.
/// The cache is refreshed every 3 seconds by a background thread.
/// On first call, starts the background refresher and does one sync fetch.
fn get_cached_gui_apps() -> Vec<(u32, String)> {
    // Start the background refresher if not already running
    // This is idempotent - only starts once
    start_cache_refresher();

    // Return cached data (always fast - just a clone)
    let cache = GUI_APPS_CACHE.lock().unwrap();
    trace!("Returning cached GUI apps ({} apps)", cache.apps.len());
    cache.apps.clone()
}

/// List applications that are actively playing audio
///
/// Uses CoreAudio to get PIDs of processes with active audio clients,
/// then maps those PIDs to application names via cached AppleScript results.
///
/// **Permission handling**: If Screen Recording permission is not granted,
/// the active audio query fails. In that case, we fall back to showing
/// all visible GUI apps so the user can click "Capture" and trigger
/// the permission request.
///
/// **Performance**: Uses a 3-second cache for GUI apps to avoid blocking
/// AppleScript calls on every poll. Most calls return in <1ms.
fn list_running_audio_apps() -> Result<Vec<ApplicationInfo>, PlatformError> {
    use std::collections::HashSet;

    // Try to get PIDs of processes that are actively playing audio via CoreAudio
    // This requires Screen Recording permission - if we don't have it, the list
    // will be empty and we should fall back to showing all visible apps
    let active_audio_pids: HashSet<i32> = unsafe {
        super::process_tap_ffi::get_audio_active_pids()
            .into_iter()
            .collect()
    };

    // Determine if we should filter by active audio
    // - If we got PIDs, filter to only those apps
    // - If empty (no permission or no audio), show all visible apps as fallback
    //   so user can click "Capture" to trigger permission request
    let filter_by_active_audio = !active_audio_pids.is_empty();

    if filter_by_active_audio {
        trace!(
            "Found {} PIDs with active audio - filtering app list",
            active_audio_pids.len()
        );
    } else {
        trace!("No active audio PIDs found (no permission or no audio) - showing all visible apps");
    }

    // Get cached GUI apps (fast - usually returns immediately)
    let gui_apps = get_cached_gui_apps();

    // Build result list
    let mut apps = Vec::new();
    for (pid, name) in gui_apps {
        // If we have permission and active audio data, filter to only those apps
        // Otherwise, show all apps so user can click Capture to trigger permission
        if filter_by_active_audio && !active_audio_pids.contains(&(pid as i32)) {
            continue;
        }

        // Mark as active only if we know it's playing audio
        let is_active = !filter_by_active_audio || active_audio_pids.contains(&(pid as i32));

        apps.push(ApplicationInfo {
            pid,
            name,
            icon: None,
            is_active,
        });
    }

    trace!("Found {} running GUI apps", apps.len());
    Ok(apps)
}

// ============================================================================
// System Volume Control
// ============================================================================

/// Get the system output volume (0.0 - 1.0)
///
/// Returns the volume of the default output device.
/// On devices without a master volume control, this may return an error.
pub fn get_system_volume() -> Result<f32, PlatformError> {
    let device_id = get_default_output_device()?;

    unsafe {
        // Try to get the volume from master element (channel 0)
        let property_address = AudioObjectPropertyAddress {
            mSelector: kAudioDevicePropertyVolumeScalar,
            mScope: kAudioObjectPropertyScopeOutput,
            mElement: kAudioObjectPropertyElementMaster,
        };

        let mut volume: f32 = 0.0;
        let mut size = mem::size_of::<f32>() as u32;

        let status = AudioObjectGetPropertyData(
            device_id,
            &property_address,
            0,
            ptr::null(),
            &mut size,
            &mut volume as *mut f32 as *mut c_void,
        );

        if status != 0 {
            // Some devices don't have a master volume control
            // Try channel 1 (left channel) as fallback
            let property_address_ch1 = AudioObjectPropertyAddress {
                mSelector: kAudioDevicePropertyVolumeScalar,
                mScope: kAudioObjectPropertyScopeOutput,
                mElement: 1, // Left channel
            };

            let status = AudioObjectGetPropertyData(
                device_id,
                &property_address_ch1,
                0,
                ptr::null(),
                &mut size,
                &mut volume as *mut f32 as *mut c_void,
            );

            if status != 0 {
                return Err(PlatformError::Internal(format!(
                    "Failed to get system volume: OSStatus {}",
                    status
                )));
            }
        }

        Ok(volume)
    }
}

/// Set the system output volume (0.0 - 1.0)
///
/// Sets the volume of the default output device.
/// On devices without a master volume control, sets both left and right channels.
pub fn set_system_volume(volume: f32) -> Result<(), PlatformError> {
    let device_id = get_default_output_device()?;
    let volume = volume.clamp(0.0, 1.0);

    unsafe {
        // Try to set the master volume (channel 0)
        let property_address = AudioObjectPropertyAddress {
            mSelector: kAudioDevicePropertyVolumeScalar,
            mScope: kAudioObjectPropertyScopeOutput,
            mElement: kAudioObjectPropertyElementMaster,
        };

        let size = mem::size_of::<f32>() as u32;

        let status = AudioObjectSetPropertyData(
            device_id,
            &property_address,
            0,
            ptr::null(),
            size,
            &volume as *const f32 as *const c_void,
        );

        if status != 0 {
            // Some devices don't have a master volume control
            // Set left and right channels individually
            for channel in 1..=2 {
                let property_address_ch = AudioObjectPropertyAddress {
                    mSelector: kAudioDevicePropertyVolumeScalar,
                    mScope: kAudioObjectPropertyScopeOutput,
                    mElement: channel,
                };

                let ch_status = AudioObjectSetPropertyData(
                    device_id,
                    &property_address_ch,
                    0,
                    ptr::null(),
                    size,
                    &volume as *const f32 as *const c_void,
                );

                if ch_status != 0 && channel == 1 {
                    // If we can't even set channel 1, fail
                    return Err(PlatformError::Internal(format!(
                        "Failed to set system volume: OSStatus {}",
                        ch_status
                    )));
                }
            }
        }

        debug!("Set system volume to {:.2}", volume);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_list_audio_devices() {
        let devices = list_audio_devices();
        assert!(devices.is_ok());

        let devices = devices.unwrap();
        println!("Found {} audio devices:", devices.len());
        for device in &devices {
            println!("  - {} (ID: {}, Class: {})", device.name, device.id, device.media_class);
        }

        // Should have at least one device (built-in speakers usually)
        // Note: This may fail in CI without audio hardware
        if !devices.is_empty() {
            assert!(!devices[0].name.is_empty());
        }
    }

    #[test]
    fn test_get_default_output() {
        let result = get_default_output_device();
        println!("Default output device result: {:?}", result);

        // Should succeed on a normal macOS system
        if let Ok(device_id) = result {
            assert!(device_id > 0, "Device ID should be non-zero");
        }
    }

    #[test]
    fn test_list_audio_applications() {
        let apps = list_audio_applications();
        assert!(apps.is_ok());

        let apps = apps.unwrap();
        println!("Found {} audio applications:", apps.len());
        for app in &apps {
            println!("  - {} (PID: {})", app.name, app.pid);
        }
    }
}
