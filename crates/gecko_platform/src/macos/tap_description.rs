//! CATapDescription Objective-C bindings for macOS 14.4+ Process Tap API
//!
//! CATapDescription is an Objective-C class used with AudioHardwareCreateProcessTap.
//! It inherits from NSObject (NOT toll-free bridged to CFDictionary).
//!
//! The API accepts CATapDescription as a CFTypeRef - CoreAudio handles the
//! conversion internally.
//!
//! Reference: https://developer.apple.com/documentation/coreaudio/catapdescription
//!
//! ## Critical Discovery (2025-12-26)
//!
//! `initStereoMixdownOfProcesses:` requires **AudioObjectIDs**, not PIDs or NSRunningApplication!
//!
//! The correct workflow (from AudioCap reference implementation):
//! 1. Convert PID → AudioObjectID using `kAudioHardwarePropertyTranslatePIDToProcessObject`
//! 2. Pass array of AudioObjectIDs (as NSNumbers) to `initStereoMixdownOfProcesses:`
//!
//! Using NSRunningApplication or PIDs directly fails with 'what' error (0x77686174).

use objc2::rc::Retained;
use objc2::runtime::{AnyClass, NSObject};
use objc2::msg_send;
use objc2_foundation::{NSArray, NSNumber};
use tracing::{debug, error, trace, warn};

use coreaudio_sys::{
    kAudioObjectPropertyElementMain, kAudioObjectPropertyScopeGlobal,
    kAudioObjectSystemObject, AudioObjectID, AudioObjectGetPropertyData,
    AudioObjectPropertyAddress,
};
use std::mem;

/// kAudioHardwarePropertyTranslatePIDToProcessObject = 'id2p'
/// Converts a PID (pid_t) to an AudioObjectID for the process.
/// This is required for CATapDescription.initStereoMixdownOfProcesses:.
const K_AUDIO_HARDWARE_PROPERTY_TRANSLATE_PID_TO_PROCESS_OBJECT: u32 = 0x69643270; // 'id2p'

/// Opaque pointer type for passing to CoreAudio
/// CATapDescription is passed as CFTypeRef (void*) to AudioHardwareCreateProcessTap
pub type CATapDescriptionRef = *const std::ffi::c_void;

/// Wrapper around CATapDescription Objective-C class
///
/// CATapDescription creates the proper tap configuration that
/// AudioHardwareCreateProcessTap expects.
pub struct TapDescription {
    /// The underlying Objective-C object (CATapDescription inherits from NSObject)
    inner: Retained<NSObject>,
    /// UUID assigned to this tap (used for aggregate device configuration)
    uuid: String,
}

// Safety: CATapDescription is thread-safe for our use case
// We only create it, configure it, and pass it to CoreAudio
unsafe impl Send for TapDescription {}
unsafe impl Sync for TapDescription {}

/// Translate a PID to an AudioObjectID using CoreAudio
///
/// This uses `kAudioHardwarePropertyTranslatePIDToProcessObject` to convert
/// a process ID to the AudioObjectID that CoreAudio uses internally.
///
/// Returns None if the process doesn't exist or isn't producing audio.
fn translate_pid_to_audio_object_id(pid: i32) -> Option<AudioObjectID> {
    unsafe {
        let address = AudioObjectPropertyAddress {
            mSelector: K_AUDIO_HARDWARE_PROPERTY_TRANSLATE_PID_TO_PROCESS_OBJECT,
            mScope: kAudioObjectPropertyScopeGlobal,
            mElement: kAudioObjectPropertyElementMain,
        };

        let mut process_object_id: AudioObjectID = 0;
        let mut data_size = mem::size_of::<AudioObjectID>() as u32;
        let qualifier_size = mem::size_of::<i32>() as u32;

        let status = AudioObjectGetPropertyData(
            kAudioObjectSystemObject,
            &address,
            qualifier_size,
            &pid as *const i32 as *const _,
            &mut data_size,
            &mut process_object_id as *mut AudioObjectID as *mut _,
        );

        if status != 0 {
            warn!(
                "Failed to translate PID {} to AudioObjectID: OSStatus {} (process may not have audio)",
                pid, status
            );
            return None;
        }

        if process_object_id == 0 {
            warn!("PID {} translated to invalid AudioObjectID 0", pid);
            return None;
        }

        debug!("Translated PID {} to AudioObjectID {}", pid, process_object_id);
        Some(process_object_id)
    }
}

impl TapDescription {
    /// Create a tap description for specific processes by PID
    ///
    /// Uses `initStereoMixdownOfProcesses:` with AudioObjectIDs (as NSNumbers).
    /// This is the correct method discovered from the AudioCap reference implementation.
    ///
    /// The workflow:
    /// 1. Convert each PID → AudioObjectID using `kAudioHardwarePropertyTranslatePIDToProcessObject`
    /// 2. Create NSArray of AudioObjectIDs (as NSNumbers)
    /// 3. Pass to `initStereoMixdownOfProcesses:`
    ///
    /// Note: Using NSRunningApplication or PIDs directly fails with 'what' error.
    ///
    /// # Arguments
    /// * `pids` - Process IDs to capture audio from
    ///
    /// # Returns
    /// A TapDescription or None if creation failed
    pub fn with_process_ids(pids: &[i32]) -> Option<Self> {
        if pids.is_empty() {
            error!("Cannot create tap with empty PID list");
            return None;
        }

        trace!("TapDescription: Creating per-process tap for PIDs: {:?}", pids);

        unsafe {
            // Get CATapDescription class
            trace!("TapDescription: Looking up CATapDescription class...");
            let tap_class = match AnyClass::get(c"CATapDescription") {
                Some(c) => {
                    trace!("TapDescription: CATapDescription class found");
                    c
                }
                None => {
                    error!("CATapDescription class not found - macOS 14.4+ required");
                    return None;
                }
            };

            // Convert PIDs to AudioObjectIDs
            // AudioObjectID is what CATapDescription.initStereoMixdownOfProcesses: actually expects!
            let mut audio_object_ids: Vec<Retained<NSNumber>> = Vec::with_capacity(pids.len());

            for &pid in pids {
                if let Some(object_id) = translate_pid_to_audio_object_id(pid) {
                    // Wrap AudioObjectID (u32) in NSNumber
                    let ns_object_id = NSNumber::new_u32(object_id);
                    debug!("PID {} → AudioObjectID {} (NSNumber)", pid, object_id);
                    audio_object_ids.push(ns_object_id);
                } else {
                    debug!("Skipping PID {} - no AudioObjectID (process may not be producing audio)", pid);
                }
            }

            if audio_object_ids.is_empty() {
                error!(
                    "No valid AudioObjectIDs found for PIDs {:?}. \
                     Processes may not be producing audio or may have exited.",
                    pids
                );
                return None;
            }

            // Create NSArray of AudioObjectIDs (as NSNumbers)
            trace!("TapDescription: Creating NSArray with {} AudioObjectID(s)...", audio_object_ids.len());
            let object_ids_array = NSArray::from_retained_slice(&audio_object_ids);
            trace!("TapDescription: NSArray created");

            trace!("TapDescription: Allocating CATapDescription...");
            let alloc: *mut NSObject = msg_send![tap_class, alloc];
            if alloc.is_null() {
                error!("Failed to allocate CATapDescription");
                return None;
            }
            trace!("TapDescription: CATapDescription allocated at {:?}", alloc);

            // Use initStereoMixdownOfProcesses: with AudioObjectIDs (as NSNumbers)
            // This is the correct method - AudioCap does the same thing!
            trace!("TapDescription: Calling initStereoMixdownOfProcesses:...");
            let obj: *mut NSObject = msg_send![alloc, initStereoMixdownOfProcesses: &*object_ids_array];
            trace!("TapDescription: initStereoMixdownOfProcesses: returned {:?}", obj);

            if obj.is_null() {
                error!(
                    "initStereoMixdownOfProcesses: returned nil for AudioObjectIDs (PIDs: {:?})",
                    pids
                );
                return None;
            }

            // Generate and set UUID like AudioCap does (REQUIRED for aggregate device tap list)
            let tap_uuid = uuid::Uuid::new_v4().to_string();
            let uuid_class = AnyClass::get(c"NSUUID");
            if let Some(uuid_class) = uuid_class {
                // Create NSUUID from our UUID string
                let uuid_string_class = AnyClass::get(c"NSString")?;
                let uuid_cstr = std::ffi::CString::new(tap_uuid.as_str()).ok()?;

                // Create NSString from our UUID
                let ns_string: *mut NSObject = msg_send![uuid_string_class, stringWithUTF8String: uuid_cstr.as_ptr()];
                if ns_string.is_null() {
                    warn!("Failed to create NSString for UUID");
                } else {
                    // Create NSUUID from string
                    let uuid_alloc: *mut NSObject = msg_send![uuid_class, alloc];
                    let ns_uuid: *mut NSObject = msg_send![uuid_alloc, initWithUUIDString: ns_string];

                    if !ns_uuid.is_null() {
                        let _: () = msg_send![obj, setUUID: ns_uuid];
                        debug!("Set UUID {} on CATapDescription", tap_uuid);
                    } else {
                        // Fallback: use random NSUUID (but we won't know the string)
                        let random_uuid: *mut NSObject = msg_send![uuid_class, UUID];
                        if !random_uuid.is_null() {
                            let _: () = msg_send![obj, setUUID: random_uuid];
                            warn!("Using random UUID on CATapDescription (string creation failed)");
                        }
                    }
                }
            }

            let inner = Retained::from_raw(obj)?;
            debug!(
                "Created CATapDescription for {} process(es) (UUID: {})",
                audio_object_ids.len(),
                tap_uuid
            );
            Some(Self { inner, uuid: tap_uuid })
        }
    }

    /// Create a tap description for specific processes (legacy method)
    ///
    /// This calls `with_process_ids` internally.
    #[inline]
    pub fn with_processes(pids: &[i32]) -> Option<Self> {
        Self::with_process_ids(pids)
    }

    /// Create a global tap that captures all system audio except specified processes
    ///
    /// # Arguments
    /// * `exclude_pids` - Process IDs to exclude (empty = capture everything)
    ///
    /// # Returns
    /// A TapDescription or None if creation failed
    #[allow(dead_code)]
    pub fn stereo_global_tap_excluding(exclude_pids: &[i32]) -> Option<Self> {
        unsafe {
            let class = match AnyClass::get(c"CATapDescription") {
                Some(c) => c,
                None => {
                    error!("CATapDescription class not found - macOS 14.4+ required");
                    return None;
                }
            };

            // Create NSArray of excluded PIDs
            let ns_pids: Vec<Retained<NSNumber>> = exclude_pids
                .iter()
                .map(|&pid| NSNumber::new_i32(pid))
                .collect();
            let exclude_array = NSArray::from_retained_slice(&ns_pids);

            let alloc: *mut NSObject = msg_send![class, alloc];
            if alloc.is_null() {
                error!("Failed to allocate CATapDescription");
                return None;
            }

            // Initialize with global tap excluding processes
            let obj: *mut NSObject = msg_send![alloc, initStereoGlobalTapButExcludeProcesses: &*exclude_array];

            if obj.is_null() {
                error!("initStereoGlobalTapButExcludeProcesses: returned nil");
                return None;
            }

            // Generate UUID for global tap too
            let tap_uuid = uuid::Uuid::new_v4().to_string();

            let inner = Retained::from_raw(obj)?;
            debug!("Created global CATapDescription excluding {} processes (UUID: {})", exclude_pids.len(), tap_uuid);
            Some(Self { inner, uuid: tap_uuid })
        }
    }

    /// Get the UUID assigned to this tap
    ///
    /// This UUID must match the one used in the aggregate device tap list.
    pub fn uuid(&self) -> &str {
        &self.uuid
    }

    /// Set whether to mute the original audio output
    ///
    /// muteBehavior values:
    /// - 0 = no mute (audio still plays to speakers)
    /// - 1 = mute when tapped (only Gecko hears the audio)
    pub fn set_mute(&self, mute: bool) {
        unsafe {
            // Use i64 as the type signature shows 'q' (quad/long long)
            let behavior: i64 = if mute { 1 } else { 0 };
            let _: () = msg_send![&*self.inner, setMuteBehavior: behavior];
            debug!("Set mute behavior to {}", behavior);
        }
    }

    /// Set whether this tap is private (not visible to other apps)
    pub fn set_private(&self, private: bool) {
        unsafe {
            let _: () = msg_send![&*self.inner, setPrivate: private];
            debug!("Set private to {}", private);
        }
    }

    /// Set whether to use exclusive mode
    pub fn set_exclusive(&self, exclusive: bool) {
        unsafe {
            let _: () = msg_send![&*self.inner, setExclusive: exclusive];
            debug!("Set exclusive to {}", exclusive);
        }
    }

    /// Get the raw pointer to pass to AudioHardwareCreateProcessTap
    ///
    /// CATapDescription inherits from NSObject, NOT CFDictionary.
    /// However, AudioHardwareCreateProcessTap accepts it as a CFTypeRef.
    /// We pass the raw NSObject pointer which CoreAudio interprets correctly.
    pub fn as_ptr(&self) -> CATapDescriptionRef {
        Retained::as_ptr(&self.inner) as CATapDescriptionRef
    }

    /// Get the underlying NSObject pointer (for debugging)
    #[allow(dead_code)]
    pub fn inner_ptr(&self) -> *const NSObject {
        Retained::as_ptr(&self.inner)
    }
}

impl Drop for TapDescription {
    fn drop(&mut self) {
        // Retained handles release automatically via ARC
        debug!("Dropping CATapDescription");
    }
}

/// Create a tap description for a single process using CATapDescription class
///
/// Converts the PID to an AudioObjectID and uses `initStereoMixdownOfProcesses:`
/// for per-app audio capture.
///
/// Note: The process must be actively producing audio for this to succeed.
/// Use `list_audio_applications()` to get PIDs of processes with audio.
pub fn create_tap_for_process(pid: u32) -> Option<TapDescription> {
    TapDescription::with_process_ids(&[pid as i32])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tap_description_class_exists() {
        unsafe {
            let class = AnyClass::get(c"CATapDescription");
            // This will fail on macOS < 14.4, which is expected
            if class.is_some() {
                println!("CATapDescription class found");
            } else {
                println!("CATapDescription class not found (expected on macOS < 14.4)");
            }
        }
    }

    #[test]
    fn test_create_per_process_tap() {
        // Use a known PID (our own process) for testing
        let our_pid = std::process::id() as i32;
        if let Some(tap) = TapDescription::with_process_ids(&[our_pid]) {
            println!("Created per-process tap for PID {}", our_pid);
            let ptr = tap.as_ptr();
            println!("Tap pointer: {:?}", ptr);
            assert!(!ptr.is_null());
        } else {
            println!("Failed to create per-process tap (expected on macOS < 14.4)");
        }
    }
}
