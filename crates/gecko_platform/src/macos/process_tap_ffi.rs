//! Process Tap API FFI Bindings (macOS 14.4+)
//!
//! These are raw FFI bindings for the `AudioHardwareCreateProcessTap` API
//! introduced in macOS 14.4 Sonoma. This API enables per-application audio
//! capture without requiring a HAL plugin installation.
//!
//! # References
//!
//! - Apple docs: https://developer.apple.com/documentation/coreaudio/capturing-system-audio-with-core-audio-taps
//! - AudioServerPlugIn.h in IOKit
//!
//! # Safety
//!
//! These are raw C bindings. Use the safe wrappers in `process_tap.rs` instead.

#![allow(non_upper_case_globals)]
#![allow(non_camel_case_types)]
#![allow(non_snake_case)]
#![allow(dead_code)]

use std::ffi::c_void;

// CoreAudio types from coreaudio-sys
pub use coreaudio_sys::{
    AudioDeviceID, AudioObjectID, AudioObjectPropertyAddress, OSStatus,
    kAudioObjectPropertyElementMain, kAudioObjectPropertyScopeGlobal,
    kAudioObjectPropertyScopeOutput, kAudioObjectSystemObject,
    AudioObjectGetPropertyData, AudioObjectGetPropertyDataSize,
    AudioObjectSetPropertyData,
};

// CoreFoundation types
pub type CFStringRef = *const c_void;
pub type CFDictionaryRef = *const c_void;
pub type CFMutableDictionaryRef = *mut c_void;
pub type CFArrayRef = *const c_void;
pub type CFMutableArrayRef = *mut c_void;
pub type CFNumberRef = *const c_void;
pub type CFTypeRef = *const c_void;
pub type CFIndex = isize;
pub type CFAllocatorRef = *const c_void;

// Audio Tap types
pub type AudioHardwareTapID = AudioObjectID;

/// Tap description dictionary keys (CFString constants)
/// These are the keys used in the tap description dictionary
pub mod tap_keys {
    // Rust pattern: Link to CoreAudio framework for these constants
    // Note: These are CFString constants that need to be looked up at runtime
    // or declared as extern statics if available in the SDK

    /// Key for the array of process IDs to tap
    /// Value: CFArray of CFNumbers (pids)
    pub const PROCESSES_KEY: &str = "Processes";

    /// Key for whether to mute the tapped processes' audio
    /// Value: CFBoolean (true = mute original output)
    pub const MUTE_KEY: &str = "Mute";

    /// Key for UUID string identifying this tap
    /// Value: CFString
    pub const UUID_KEY: &str = "UUID";

    /// Key for mixdown behavior
    /// Value: CFNumber (0 = stereo mixdown, 1 = mono)
    pub const MIXDOWN_KEY: &str = "MixdownBehavior";

    /// Key to request all processes (instead of specific PIDs)
    /// Value: CFBoolean
    pub const ALL_PROCESSES_KEY: &str = "AllProcesses";

    /// Key for private tap (not visible to other apps)
    /// Value: CFBoolean
    pub const PRIVATE_KEY: &str = "Private";
}

/// Process Tap property selectors
pub mod tap_properties {
    /// Property to get the audio format of a tap
    pub const kAudioTapPropertyFormat: u32 = 0x74617066; // 'tapf'

    /// Property to check if tap is active
    pub const kAudioTapPropertyIsActive: u32 = 0x74617061; // 'tapa'

    /// Property to get the UID string of a tap
    /// This is the UID to use in the aggregate device tap list!
    /// SoundPusher reads this instead of setting UUID on CATapDescription
    pub const kAudioTapPropertyUID: u32 = 0x74756964; // 'tuid'
}

/// Hardware property selectors for process enumeration
pub mod hardware_properties {
    /// Get list of AudioObjectIDs for processes currently using audio
    /// Returns an array of AudioObjectIDs (not PIDs!)
    pub const kAudioHardwarePropertyProcessObjectList: u32 = 0x706F626A; // 'pobj'

    /// Translate a PID to an AudioObjectID
    /// Input: pid_t, Output: AudioObjectID
    /// Only succeeds if the process is currently using audio
    pub const kAudioHardwarePropertyTranslatePIDToProcessObject: u32 = 0x70326F62; // 'p2ob'
}

/// Process object property selectors
pub mod process_properties {
    /// Get the PID from an AudioObjectID (process object)
    /// macOS 14.0+
    pub const kAudioProcessPropertyPID: u32 = 0x70706964; // 'ppid'

    /// Get the bundle ID from a process object
    pub const kAudioProcessPropertyBundleID: u32 = 0x70626964; // 'pbid'
}

/// Aggregate device dictionary keys
pub mod aggregate_keys {
    /// Key for the aggregate device UID
    pub const UID_KEY: &str = "uid";

    /// Key for the aggregate device name
    pub const NAME_KEY: &str = "name";

    /// Key for the main subdevice UID
    pub const MAIN_SUBDEVICE_KEY: &str = "master";

    /// Key for whether the device is private
    pub const IS_PRIVATE_KEY: &str = "private";

    /// Key for whether the device is stacked
    pub const IS_STACKED_KEY: &str = "stacked";

    /// Key for the sub-device list
    pub const SUB_DEVICE_LIST_KEY: &str = "subdevices";

    /// Key for the tap list (array of tap dictionaries)
    pub const TAP_LIST_KEY: &str = "taps";

    /// Key for tap auto-start behavior
    pub const TAP_AUTO_START_KEY: &str = "tapautostart";
}

/// Sub-device dictionary keys (used within SUB_DEVICE_LIST_KEY arrays)
pub mod sub_device_keys {
    /// UID of the sub-device
    pub const UID_KEY: &str = "uid";
}

/// Sub-tap dictionary keys (used within TAP_LIST_KEY arrays)
pub mod sub_tap_keys {
    /// UID of the tap (matches CATapDescription UUID)
    pub const UID_KEY: &str = "uid";

    /// Enable drift compensation for the tap
    pub const DRIFT_COMPENSATION_KEY: &str = "drift";
}

// CoreFoundation extern functions for dictionary creation
#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    pub static kCFAllocatorDefault: CFAllocatorRef;
    pub static kCFBooleanTrue: CFTypeRef;
    pub static kCFBooleanFalse: CFTypeRef;

    pub fn CFDictionaryCreateMutable(
        allocator: CFAllocatorRef,
        capacity: CFIndex,
        keyCallBacks: *const c_void,
        valueCallBacks: *const c_void,
    ) -> CFMutableDictionaryRef;

    pub fn CFDictionarySetValue(
        dict: CFMutableDictionaryRef,
        key: CFTypeRef,
        value: CFTypeRef,
    );

    pub fn CFArrayCreateMutable(
        allocator: CFAllocatorRef,
        capacity: CFIndex,
        callBacks: *const c_void,
    ) -> CFMutableArrayRef;

    pub fn CFArrayAppendValue(
        array: CFMutableArrayRef,
        value: CFTypeRef,
    );

    pub fn CFNumberCreate(
        allocator: CFAllocatorRef,
        theType: CFIndex,
        valuePtr: *const c_void,
    ) -> CFNumberRef;

    pub fn CFStringCreateWithCString(
        alloc: CFAllocatorRef,
        cStr: *const i8,
        encoding: u32,
    ) -> CFStringRef;

    pub fn CFRelease(cf: CFTypeRef);
    pub fn CFRetain(cf: CFTypeRef) -> CFTypeRef;

    pub static kCFTypeDictionaryKeyCallBacks: c_void;
    pub static kCFTypeDictionaryValueCallBacks: c_void;
    pub static kCFTypeArrayCallBacks: c_void;
}

// CFNumber types
pub const kCFNumberSInt32Type: CFIndex = 3;
pub const kCFNumberSInt64Type: CFIndex = 4;

// CFString encoding
pub const kCFStringEncodingUTF8: u32 = 0x08000100;

// CoreAudio extern functions for Process Tap
// Note: These functions are available starting macOS 14.4
#[link(name = "CoreAudio", kind = "framework")]
extern "C" {
    /// Create a process tap
    ///
    /// # Parameters
    /// - `inDescription`: A CATapDescription object OR CFDictionary describing the tap.
    ///   CATapDescription is preferred as it's the official API.
    /// - `outTapID`: Receives the created tap ID
    ///
    /// # Returns
    /// - noErr (0) on success
    /// - Error code on failure
    ///
    /// # Availability
    /// macOS 14.4+
    ///
    /// # Note
    /// The API accepts both CFDictionary and CATapDescription objects as CFTypeRef.
    /// CATapDescription (NSObject subclass) is the intended way to configure taps.
    pub fn AudioHardwareCreateProcessTap(
        inDescription: CFTypeRef,  // CATapDescription* or CFDictionaryRef
        outTapID: *mut AudioHardwareTapID,
    ) -> OSStatus;

    /// Destroy a process tap
    ///
    /// # Parameters
    /// - `inTapID`: The tap to destroy
    ///
    /// # Returns
    /// - noErr (0) on success
    pub fn AudioHardwareDestroyProcessTap(
        inTapID: AudioHardwareTapID,
    ) -> OSStatus;

    /// Create an aggregate audio device
    ///
    /// # Parameters
    /// - `inDescription`: A CFDictionary describing the aggregate device
    /// - `outDeviceID`: Receives the created device ID
    ///
    /// # Returns
    /// - noErr (0) on success
    pub fn AudioHardwareCreateAggregateDevice(
        inDescription: CFDictionaryRef,
        outDeviceID: *mut AudioDeviceID,
    ) -> OSStatus;

    /// Destroy an aggregate audio device
    pub fn AudioHardwareDestroyAggregateDevice(
        inDeviceID: AudioDeviceID,
    ) -> OSStatus;

    /// Create an IO proc for an audio device
    ///
    /// # Parameters
    /// - `inDevice`: The device to create the IO proc for
    /// - `inProc`: The callback function
    /// - `inClientData`: User data passed to callback
    /// - `outIOProcID`: Receives the IO proc ID
    pub fn AudioDeviceCreateIOProcID(
        inDevice: AudioDeviceID,
        inProc: AudioDeviceIOProc,
        inClientData: *mut c_void,
        outIOProcID: *mut AudioDeviceIOProcID,
    ) -> OSStatus;

    /// Destroy an IO proc
    pub fn AudioDeviceDestroyIOProcID(
        inDevice: AudioDeviceID,
        inIOProcID: AudioDeviceIOProcID,
    ) -> OSStatus;

    /// Start audio IO for a device
    pub fn AudioDeviceStart(
        inDevice: AudioDeviceID,
        inProcID: AudioDeviceIOProcID,
    ) -> OSStatus;

    /// Stop audio IO for a device
    pub fn AudioDeviceStop(
        inDevice: AudioDeviceID,
        inProcID: AudioDeviceIOProcID,
    ) -> OSStatus;
}

// IO Proc types
pub type AudioDeviceIOProcID = *mut c_void;

// AudioUnit types for alternative input approach
pub type AudioUnit = *mut c_void;
pub type AudioComponent = *mut c_void;
pub type AudioComponentInstance = AudioUnit;

/// AudioComponentDescription for finding audio units
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct AudioComponentDescription {
    pub componentType: u32,
    pub componentSubType: u32,
    pub componentManufacturer: u32,
    pub componentFlags: u32,
    pub componentFlagsMask: u32,
}

/// Audio unit types
pub mod audio_unit_types {
    /// Output unit type
    pub const kAudioUnitType_Output: u32 = 0x61756F75; // 'auou'
    /// HAL output subtype (default for device I/O)
    pub const kAudioUnitSubType_HALOutput: u32 = 0x6168616C; // 'ahal'
    /// Apple manufacturer
    pub const kAudioUnitManufacturer_Apple: u32 = 0x6170706C; // 'appl'
}

/// Audio unit properties
pub mod audio_unit_properties {
    /// Enable/disable input on HAL output unit
    pub const kAudioOutputUnitProperty_EnableIO: u32 = 2003;
    /// Set the current device for a HAL output unit
    pub const kAudioOutputUnitProperty_CurrentDevice: u32 = 2000;
    /// Set input callback
    pub const kAudioOutputUnitProperty_SetInputCallback: u32 = 2005;
}

/// Audio unit scopes
pub mod audio_unit_scopes {
    pub const kAudioUnitScope_Input: u32 = 1;
    pub const kAudioUnitScope_Output: u32 = 2;
    pub const kAudioUnitScope_Global: u32 = 0;
}

/// Audio unit elements
pub mod audio_unit_elements {
    /// Input element (for input from device)
    pub const kInputBus: u32 = 1;
    /// Output element (for output to device)
    pub const kOutputBus: u32 = 0;
}

/// Render callback struct
#[repr(C)]
pub struct AURenderCallbackStruct {
    pub inputProc: AURenderCallback,
    pub inputProcRefCon: *mut c_void,
}

/// Render callback type
pub type AURenderCallback = extern "C" fn(
    inRefCon: *mut c_void,
    ioActionFlags: *mut u32,
    inTimeStamp: *const AudioTimeStamp,
    inBusNumber: u32,
    inNumberFrames: u32,
    ioData: *mut AudioBufferList,
) -> OSStatus;

#[link(name = "AudioToolbox", kind = "framework")]
extern "C" {
    pub fn AudioComponentFindNext(
        inComponent: AudioComponent,
        inDesc: *const AudioComponentDescription,
    ) -> AudioComponent;

    pub fn AudioComponentInstanceNew(
        inComponent: AudioComponent,
        outInstance: *mut AudioComponentInstance,
    ) -> OSStatus;

    pub fn AudioComponentInstanceDispose(inInstance: AudioComponentInstance) -> OSStatus;

    pub fn AudioUnitInitialize(inUnit: AudioUnit) -> OSStatus;

    pub fn AudioUnitUninitialize(inUnit: AudioUnit) -> OSStatus;

    pub fn AudioOutputUnitStart(ci: AudioUnit) -> OSStatus;

    pub fn AudioOutputUnitStop(ci: AudioUnit) -> OSStatus;

    pub fn AudioUnitSetProperty(
        inUnit: AudioUnit,
        inID: u32,
        inScope: u32,
        inElement: u32,
        inData: *const c_void,
        inDataSize: u32,
    ) -> OSStatus;

    pub fn AudioUnitGetProperty(
        inUnit: AudioUnit,
        inID: u32,
        inScope: u32,
        inElement: u32,
        outData: *mut c_void,
        ioDataSize: *mut u32,
    ) -> OSStatus;

    pub fn AudioUnitRender(
        inUnit: AudioUnit,
        ioActionFlags: *mut u32,
        inTimeStamp: *const AudioTimeStamp,
        inOutputBusNumber: u32,
        inNumberFrames: u32,
        ioData: *mut AudioBufferList,
    ) -> OSStatus;
}

/// Audio IO proc callback type
///
/// Called by CoreAudio when audio data is available.
pub type AudioDeviceIOProc = extern "C" fn(
    inDevice: AudioDeviceID,
    inNow: *const AudioTimeStamp,
    inInputData: *const AudioBufferList,
    inInputTime: *const AudioTimeStamp,
    outOutputData: *mut AudioBufferList,
    inOutputTime: *const AudioTimeStamp,
    inClientData: *mut c_void,
) -> OSStatus;

/// Audio time stamp (simplified)
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct AudioTimeStamp {
    pub mSampleTime: f64,
    pub mHostTime: u64,
    pub mRateScalar: f64,
    pub mWordClockTime: u64,
    pub mSMPTETime: SMPTETime,
    pub mFlags: u32,
    pub mReserved: u32,
}

/// SMPTE time (simplified)
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct SMPTETime {
    pub mSubframes: i16,
    pub mSubframeDivisor: i16,
    pub mCounter: u32,
    pub mType: u32,
    pub mFlags: u32,
    pub mHours: i16,
    pub mMinutes: i16,
    pub mSeconds: i16,
    pub mFrames: i16,
}

/// Audio buffer list
#[repr(C)]
pub struct AudioBufferList {
    pub mNumberBuffers: u32,
    // Variable length array follows
    // mBuffers: [AudioBuffer; N]
}

impl AudioBufferList {
    /// Get a pointer to the buffers array
    ///
    /// # Safety
    /// Caller must ensure the AudioBufferList has at least `index` buffers
    pub unsafe fn buffer(&self, index: u32) -> *const AudioBuffer {
        let base = (self as *const AudioBufferList).add(1) as *const AudioBuffer;
        base.add(index as usize)
    }

    /// Get a mutable pointer to the buffers array
    ///
    /// # Safety
    ///
    /// - Caller must ensure `index < mNumberBuffers`
    /// - The AudioBufferList must have been properly allocated with space for `index + 1` buffers
    /// - The returned pointer is only valid for the lifetime of `self`
    pub unsafe fn buffer_mut(&mut self, index: u32) -> *mut AudioBuffer {
        let base = (self as *mut AudioBufferList).add(1) as *mut AudioBuffer;
        base.add(index as usize)
    }
}

/// Single audio buffer
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct AudioBuffer {
    pub mNumberChannels: u32,
    pub mDataByteSize: u32,
    pub mData: *mut c_void,
}

/// Helper to create a CFString from a Rust string
///
/// # Safety
///
/// Caller must CFRelease the returned string when done.
pub unsafe fn create_cf_string(s: &str) -> CFStringRef {
    let c_str = std::ffi::CString::new(s).unwrap();
    CFStringCreateWithCString(
        kCFAllocatorDefault,
        c_str.as_ptr(),
        kCFStringEncodingUTF8,
    )
}

/// Convert a CFString to a Rust String
///
/// # Safety
///
/// The cf_string must be a valid CFString reference. This function does NOT
/// release the CFString - caller is responsible for releasing it.
pub unsafe fn cfstring_to_string(cf_string: CFStringRef) -> Option<String> {
    use core_foundation::base::TCFType;
    use core_foundation::string::CFString;

    if cf_string.is_null() {
        return None;
    }

    // Wrap in CFString for safe conversion
    let cf_str = CFString::wrap_under_get_rule(cf_string as *const _);
    Some(cf_str.to_string())
}

/// Helper to create a CFNumber from a u32
///
/// # Safety
///
/// Caller must CFRelease the returned number when done.
pub unsafe fn create_cf_number_u32(value: u32) -> CFNumberRef {
    CFNumberCreate(
        kCFAllocatorDefault,
        kCFNumberSInt32Type,
        &value as *const u32 as *const c_void,
    )
}

/// Helper to create a CFNumber from a pid_t (i32)
///
/// # Safety
///
/// Caller must CFRelease the returned number when done.
pub unsafe fn create_cf_number_pid(pid: i32) -> CFNumberRef {
    CFNumberCreate(
        kCFAllocatorDefault,
        kCFNumberSInt32Type,
        &pid as *const i32 as *const c_void,
    )
}

/// Property selector for device UID
const K_AUDIO_DEVICE_PROPERTY_DEVICE_UID: u32 = 0x75696420; // 'uid '

/// Get the UID string for an audio device
///
/// # Safety
///
/// Uses CoreAudio FFI calls.
pub unsafe fn get_device_uid(device_id: AudioDeviceID) -> Option<String> {
    use core_foundation::base::TCFType;
    use core_foundation::string::CFString;
    use std::mem;

    let address = AudioObjectPropertyAddress {
        mSelector: K_AUDIO_DEVICE_PROPERTY_DEVICE_UID,
        mScope: kAudioObjectPropertyScopeGlobal,
        mElement: kAudioObjectPropertyElementMain,
    };

    let mut uid_ref: CFStringRef = std::ptr::null();
    let mut data_size = mem::size_of::<CFStringRef>() as u32;

    let status = AudioObjectGetPropertyData(
        device_id,
        &address,
        0,
        std::ptr::null(),
        &mut data_size,
        &mut uid_ref as *mut CFStringRef as *mut _,
    );

    if status != 0 || uid_ref.is_null() {
        tracing::warn!("Failed to get device UID for device {}: OSStatus {}", device_id, status);
        return None;
    }

    // Convert CFStringRef to Rust String using core_foundation
    let cf_string = CFString::wrap_under_get_rule(uid_ref as *const _);
    Some(cf_string.to_string())
}

/// Create a tap description dictionary for a specific process
///
/// # Safety
///
/// Caller must CFRelease the returned dictionary when done.
pub unsafe fn create_tap_description(
    pid: u32,
    mute: bool,
    private: bool,
) -> CFMutableDictionaryRef {
    // Create the dictionary
    let dict = CFDictionaryCreateMutable(
        kCFAllocatorDefault,
        0,
        &kCFTypeDictionaryKeyCallBacks,
        &kCFTypeDictionaryValueCallBacks,
    );

    // Create processes array with single PID
    let processes = CFArrayCreateMutable(
        kCFAllocatorDefault,
        1,
        &kCFTypeArrayCallBacks,
    );
    let pid_num = create_cf_number_pid(pid as i32);
    CFArrayAppendValue(processes, pid_num as CFTypeRef);
    CFRelease(pid_num as CFTypeRef);

    // Set Processes key
    let processes_key = create_cf_string(tap_keys::PROCESSES_KEY);
    CFDictionarySetValue(dict, processes_key as CFTypeRef, processes as CFTypeRef);
    CFRelease(processes_key as CFTypeRef);
    CFRelease(processes as CFTypeRef);

    // Set Mute key
    let mute_key = create_cf_string(tap_keys::MUTE_KEY);
    let mute_value = if mute { kCFBooleanTrue } else { kCFBooleanFalse };
    CFDictionarySetValue(dict, mute_key as CFTypeRef, mute_value);
    CFRelease(mute_key as CFTypeRef);

    // Set Private key
    let private_key = create_cf_string(tap_keys::PRIVATE_KEY);
    let private_value = if private { kCFBooleanTrue } else { kCFBooleanFalse };
    CFDictionarySetValue(dict, private_key as CFTypeRef, private_value);
    CFRelease(private_key as CFTypeRef);

    // Generate UUID for this tap
    let uuid = uuid::Uuid::new_v4().to_string();
    let uuid_key = create_cf_string(tap_keys::UUID_KEY);
    let uuid_value = create_cf_string(&uuid);
    CFDictionarySetValue(dict, uuid_key as CFTypeRef, uuid_value as CFTypeRef);
    CFRelease(uuid_key as CFTypeRef);
    CFRelease(uuid_value as CFTypeRef);

    // Set stereo mixdown
    let mixdown_key = create_cf_string(tap_keys::MIXDOWN_KEY);
    let mixdown_value = create_cf_number_u32(0); // 0 = stereo
    CFDictionarySetValue(dict, mixdown_key as CFTypeRef, mixdown_value as CFTypeRef);
    CFRelease(mixdown_key as CFTypeRef);
    CFRelease(mixdown_value as CFTypeRef);

    dict
}

/// Create an aggregate device description that includes a tap
///
/// The aggregate device combines the tap with the default output device,
/// allowing us to receive audio from the tapped process.
///
/// This follows the AudioCap reference implementation pattern:
/// - Includes the system output device as main sub-device
/// - Sets tap auto-start to true
/// - Uses tap UUID with drift compensation
///
/// # Arguments
/// * `tap_uid` - The UID string read from the tap using kAudioTapPropertyUID
/// * `name` - Name for the aggregate device
///
/// # SoundPusher Approach
///
/// According to SoundPusher (https://github.com/q-p/SoundPusher), the aggregate
/// device should ONLY contain the tap - NOT the output device!
///
/// > "it seems we only need the tap, not the actual device in there"
///
/// Also, the tap UID should be READ from the tap after creation using
/// `kAudioTapPropertyUID`, not set by us on CATapDescription.
///
/// # Safety
///
/// Caller must CFRelease the returned dictionary when done.
pub unsafe fn create_aggregate_device_description(
    tap_uid: &str,
    name: &str,
) -> CFMutableDictionaryRef {
    use tracing::debug;

    let dict = CFDictionaryCreateMutable(
        kCFAllocatorDefault,
        0,
        &kCFTypeDictionaryKeyCallBacks,
        &kCFTypeDictionaryValueCallBacks,
    );

    // Generate unique UID for the aggregate device
    let agg_uid = format!("com.gecko.aggregate.{}", uuid::Uuid::new_v4());
    let uid_key = create_cf_string(aggregate_keys::UID_KEY);
    let uid_value = create_cf_string(&agg_uid);
    CFDictionarySetValue(dict, uid_key as CFTypeRef, uid_value as CFTypeRef);
    CFRelease(uid_key as CFTypeRef);
    CFRelease(uid_value as CFTypeRef);

    // Set device name
    let name_key = create_cf_string(aggregate_keys::NAME_KEY);
    let name_value = create_cf_string(name);
    CFDictionarySetValue(dict, name_key as CFTypeRef, name_value as CFTypeRef);
    CFRelease(name_key as CFTypeRef);
    CFRelease(name_value as CFTypeRef);

    // NOTE: SoundPusher DOES NOT include main subdevice or subdevice list!
    // The aggregate device ONLY contains the tap.

    // Make it private (not visible in system preferences)
    let private_key = create_cf_string(aggregate_keys::IS_PRIVATE_KEY);
    CFDictionarySetValue(dict, private_key as CFTypeRef, kCFBooleanTrue);
    CFRelease(private_key as CFTypeRef);

    // NOTE: SoundPusher does NOT set stacked or tapautostart keys

    // Create tap list with tap UID and drift compensation
    // Structure: [ { "uid": "<tap_uid>", "drift": true } ]
    let taps = CFArrayCreateMutable(
        kCFAllocatorDefault,
        1,
        &kCFTypeArrayCallBacks,
    );
    let tap_dict = CFDictionaryCreateMutable(
        kCFAllocatorDefault,
        0,
        &kCFTypeDictionaryKeyCallBacks,
        &kCFTypeDictionaryValueCallBacks,
    );
    let tap_uid_key = create_cf_string(sub_tap_keys::UID_KEY);
    let tap_uid_value = create_cf_string(tap_uid);
    CFDictionarySetValue(tap_dict, tap_uid_key as CFTypeRef, tap_uid_value as CFTypeRef);
    CFRelease(tap_uid_key as CFTypeRef);
    CFRelease(tap_uid_value as CFTypeRef);

    let drift_key = create_cf_string(sub_tap_keys::DRIFT_COMPENSATION_KEY);
    CFDictionarySetValue(tap_dict, drift_key as CFTypeRef, kCFBooleanTrue);
    CFRelease(drift_key as CFTypeRef);

    CFArrayAppendValue(taps, tap_dict as CFTypeRef);
    CFRelease(tap_dict as CFTypeRef);

    let taps_key = create_cf_string(aggregate_keys::TAP_LIST_KEY);
    CFDictionarySetValue(dict, taps_key as CFTypeRef, taps as CFTypeRef);
    CFRelease(taps_key as CFTypeRef);
    CFRelease(taps as CFTypeRef);

    debug!(
        "Created aggregate device description (SoundPusher style): name='{}', tap_uid='{}'",
        name, tap_uid
    );

    dict
}

/// Get list of AudioObjectIDs for processes currently using audio
///
/// Returns a vector of AudioObjectIDs. These are NOT PIDs - use
/// `get_pid_from_process_object` to convert them.
///
/// # Safety
/// Uses CoreAudio FFI calls.
pub unsafe fn get_audio_process_objects() -> Vec<AudioObjectID> {
    use hardware_properties::kAudioHardwarePropertyProcessObjectList;
    use tracing::trace;

    let property_address = AudioObjectPropertyAddress {
        mSelector: kAudioHardwarePropertyProcessObjectList,
        mScope: kAudioObjectPropertyScopeGlobal,
        mElement: kAudioObjectPropertyElementMain,
    };

    // Get size first
    let mut size: u32 = 0;
    let status = AudioObjectGetPropertyDataSize(
        kAudioObjectSystemObject,
        &property_address,
        0,
        std::ptr::null(),
        &mut size,
    );

    if status != 0 {
        // This often fails without Screen Recording permission - expected, not a warning
        trace!("kAudioHardwarePropertyProcessObjectList GetPropertyDataSize failed: OSStatus {}", status);
        return Vec::new();
    }

    if size == 0 {
        trace!("kAudioHardwarePropertyProcessObjectList returned size 0 (no audio processes)");
        return Vec::new();
    }

    // Allocate buffer
    let count = size as usize / std::mem::size_of::<AudioObjectID>();
    trace!("kAudioHardwarePropertyProcessObjectList: {} bytes, {} objects", size, count);
    let mut objects: Vec<AudioObjectID> = vec![0; count];

    let status = AudioObjectGetPropertyData(
        kAudioObjectSystemObject,
        &property_address,
        0,
        std::ptr::null(),
        &mut size,
        objects.as_mut_ptr() as *mut c_void,
    );

    if status != 0 {
        trace!("kAudioHardwarePropertyProcessObjectList GetPropertyData failed: OSStatus {}", status);
        return Vec::new();
    }

    trace!("Found {} audio process objects: {:?}", objects.len(), objects);
    objects
}

/// Get PID from an AudioObjectID (process object)
///
/// Returns None if the object is not a valid process object.
///
/// # Safety
/// Uses CoreAudio FFI calls.
pub unsafe fn get_pid_from_process_object(object_id: AudioObjectID) -> Option<i32> {
    use process_properties::kAudioProcessPropertyPID;

    let property_address = AudioObjectPropertyAddress {
        mSelector: kAudioProcessPropertyPID,
        mScope: kAudioObjectPropertyScopeGlobal,
        mElement: kAudioObjectPropertyElementMain,
    };

    let mut pid: i32 = 0;
    let mut size = std::mem::size_of::<i32>() as u32;

    let status = AudioObjectGetPropertyData(
        object_id,
        &property_address,
        0,
        std::ptr::null(),
        &mut size,
        &mut pid as *mut i32 as *mut c_void,
    );

    if status == 0 && pid > 0 {
        Some(pid)
    } else {
        None
    }
}

/// Translate a PID to an AudioObjectID
///
/// Returns None if the process is not currently using audio.
/// This is the key filter - only processes actively playing audio will succeed.
///
/// # Safety
/// Uses CoreAudio FFI calls.
pub unsafe fn translate_pid_to_audio_object(pid: i32) -> Option<AudioObjectID> {
    use hardware_properties::kAudioHardwarePropertyTranslatePIDToProcessObject;

    let property_address = AudioObjectPropertyAddress {
        mSelector: kAudioHardwarePropertyTranslatePIDToProcessObject,
        mScope: kAudioObjectPropertyScopeGlobal,
        mElement: kAudioObjectPropertyElementMain,
    };

    let mut object_id: AudioObjectID = 0;
    let mut size = std::mem::size_of::<AudioObjectID>() as u32;

    let status = AudioObjectGetPropertyData(
        kAudioObjectSystemObject,
        &property_address,
        std::mem::size_of::<i32>() as u32,
        &pid as *const i32 as *const c_void,
        &mut size,
        &mut object_id as *mut AudioObjectID as *mut c_void,
    );

    if status == 0 && object_id != 0 {
        Some(object_id)
    } else {
        None
    }
}

/// Get list of PIDs currently playing audio
///
/// This is the main function to determine which processes can be tapped.
/// Only processes in this list can have Process Taps created for them.
///
/// # Safety
/// Uses CoreAudio FFI calls.
pub unsafe fn get_audio_active_pids() -> Vec<i32> {
    let objects = get_audio_process_objects();
    let mut pids = Vec::new();

    for obj in objects {
        if let Some(pid) = get_pid_from_process_object(obj) {
            pids.push(pid);
        }
    }

    pids
}

/// Device property selectors for stream queries
pub mod device_properties {
    /// Get the list of stream IDs for a device
    /// Scope determines input vs output streams
    pub const kAudioDevicePropertyStreams: u32 = 0x73746D23; // 'stm#'

    /// Get the stream's physical format
    pub const kAudioStreamPropertyPhysicalFormat: u32 = 0x70667420; // 'pft '
}

/// Query the number of input streams on a device
///
/// Returns the count of input streams, or 0 if query fails.
/// This is useful for debugging aggregate device configuration.
///
/// # Safety
/// Uses CoreAudio FFI calls.
pub unsafe fn get_device_input_stream_count(device_id: AudioDeviceID) -> u32 {
    use coreaudio_sys::kAudioObjectPropertyScopeInput;
    use device_properties::kAudioDevicePropertyStreams;
    use tracing::debug;

    let property_address = AudioObjectPropertyAddress {
        mSelector: kAudioDevicePropertyStreams,
        mScope: kAudioObjectPropertyScopeInput,
        mElement: kAudioObjectPropertyElementMain,
    };

    let mut size: u32 = 0;
    let status = AudioObjectGetPropertyDataSize(
        device_id,
        &property_address,
        0,
        std::ptr::null(),
        &mut size,
    );

    if status != 0 {
        debug!("Failed to get input stream count for device {}: OSStatus {}", device_id, status);
        return 0;
    }

    let count = size / std::mem::size_of::<AudioObjectID>() as u32;
    debug!("Device {} has {} input stream(s)", device_id, count);
    count
}

/// Query the number of output streams on a device
///
/// Returns the count of output streams, or 0 if query fails.
///
/// # Safety
/// Uses CoreAudio FFI calls.
pub unsafe fn get_device_output_stream_count(device_id: AudioDeviceID) -> u32 {
    use device_properties::kAudioDevicePropertyStreams;
    use tracing::debug;

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
        std::ptr::null(),
        &mut size,
    );

    if status != 0 {
        debug!("Failed to get output stream count for device {}: OSStatus {}", device_id, status);
        return 0;
    }

    let count = size / std::mem::size_of::<AudioObjectID>() as u32;
    debug!("Device {} has {} output stream(s)", device_id, count);
    count
}

/// Log all stream information for a device (for debugging)
///
/// # Safety
/// Uses CoreAudio FFI calls.
pub unsafe fn log_device_stream_info(device_id: AudioDeviceID, device_name: &str) {
    use tracing::trace;

    let input_count = get_device_input_stream_count(device_id);
    let output_count = get_device_output_stream_count(device_id);

    trace!(
        "Device '{}' (ID {}): {} input stream(s), {} output stream(s)",
        device_name, device_id, input_count, output_count
    );
}

/// Audio stream basic description (format info)
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct AudioStreamBasicDescription {
    pub mSampleRate: f64,
    pub mFormatID: u32,
    pub mFormatFlags: u32,
    pub mBytesPerPacket: u32,
    pub mFramesPerPacket: u32,
    pub mBytesPerFrame: u32,
    pub mChannelsPerFrame: u32,
    pub mBitsPerChannel: u32,
    pub mReserved: u32,
}

/// Query the audio format of a tap
///
/// This reads `kAudioTapPropertyFormat` from the tap to get its audio stream format.
/// IMPORTANT: AudioCap calls this BEFORE creating the aggregate device.
/// This may be required to "activate" the tap's audio streams.
///
/// # Arguments
/// * `tap_id` - The tap ID returned by AudioHardwareCreateProcessTap
///
/// # Returns
/// The audio stream format if successful, None on error.
///
/// # Safety
/// Uses CoreAudio FFI calls.
pub unsafe fn get_tap_stream_format(tap_id: AudioObjectID) -> Option<AudioStreamBasicDescription> {
    use tap_properties::kAudioTapPropertyFormat;
    use tracing::{debug, warn};

    let property_address = AudioObjectPropertyAddress {
        mSelector: kAudioTapPropertyFormat,
        mScope: kAudioObjectPropertyScopeGlobal,
        mElement: kAudioObjectPropertyElementMain,
    };

    let mut format = AudioStreamBasicDescription::default();
    let mut size = std::mem::size_of::<AudioStreamBasicDescription>() as u32;

    let status = AudioObjectGetPropertyData(
        tap_id,
        &property_address,
        0,
        std::ptr::null(),
        &mut size,
        &mut format as *mut AudioStreamBasicDescription as *mut c_void,
    );

    if status != 0 {
        warn!(
            "Failed to get tap format for tap {}: OSStatus {} (0x{:08x})",
            tap_id, status, status as u32
        );
        return None;
    }

    debug!(
        "Tap {} format: {:.0}Hz, {} channels, {} bits, format_id=0x{:08x}",
        tap_id, format.mSampleRate, format.mChannelsPerFrame, format.mBitsPerChannel, format.mFormatID
    );

    Some(format)
}

/// Get the UID string from a created tap
///
/// This is the UID that must be used in the aggregate device tap list!
/// According to SoundPusher, reading this from the tap (rather than setting
/// a UUID ourselves on CATapDescription) is the correct approach.
///
/// # Arguments
/// * `tap_id` - The AudioObjectID of the tap (from AudioHardwareCreateProcessTap)
///
/// # Returns
/// The tap UID string if successful, None on error.
///
/// # Safety
/// Uses CoreAudio FFI calls.
pub unsafe fn get_tap_uid(tap_id: AudioObjectID) -> Option<String> {
    use tap_properties::kAudioTapPropertyUID;
    use tracing::{debug, warn};

    let property_address = AudioObjectPropertyAddress {
        mSelector: kAudioTapPropertyUID,
        mScope: kAudioObjectPropertyScopeGlobal,
        mElement: kAudioObjectPropertyElementMain,
    };

    // First get the size
    let mut size: u32 = 0;
    let status = AudioObjectGetPropertyDataSize(
        tap_id,
        &property_address,
        0,
        std::ptr::null(),
        &mut size,
    );

    if status != 0 {
        warn!(
            "Failed to get tap UID size for tap {}: OSStatus {} (0x{:08x})",
            tap_id, status, status as u32
        );
        return None;
    }

    // Read the CFString
    let mut cf_string: CFStringRef = std::ptr::null();
    let status = AudioObjectGetPropertyData(
        tap_id,
        &property_address,
        0,
        std::ptr::null(),
        &mut size,
        &mut cf_string as *mut CFStringRef as *mut c_void,
    );

    if status != 0 || cf_string.is_null() {
        warn!(
            "Failed to get tap UID for tap {}: OSStatus {} (0x{:08x})",
            tap_id, status, status as u32
        );
        return None;
    }

    // Convert CFString to Rust String
    let uid = cfstring_to_string(cf_string);
    CFRelease(cf_string as CFTypeRef);

    if let Some(ref uid_str) = uid {
        debug!("Tap {} has UID: {}", tap_id, uid_str);
    }

    uid
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cf_string_creation() {
        unsafe {
            let s = create_cf_string("test");
            assert!(!s.is_null());
            CFRelease(s as CFTypeRef);
        }
    }

    #[test]
    fn test_cf_number_creation() {
        unsafe {
            let n = create_cf_number_u32(42);
            assert!(!n.is_null());
            CFRelease(n as CFTypeRef);
        }
    }

    #[test]
    #[ignore = "requires macOS 14.4+ to run"]
    fn test_tap_description_creation() {
        unsafe {
            let desc = create_tap_description(1234, false, true);
            assert!(!desc.is_null());
            CFRelease(desc as CFTypeRef);
        }
    }

    #[test]
    fn test_get_audio_active_pids() {
        unsafe {
            let pids = get_audio_active_pids();
            println!("Audio-active PIDs: {:?}", pids);
            // May be empty if nothing is playing audio
        }
    }
}
