# Windows Platform Implementation (WASAPI)

**Last Updated**: December 2024
**Context**: Read when working on Windows audio support, WASAPI, or per-process capture

## Overview

Windows uses WASAPI (Windows Audio Session API) for audio. Per-app capture requires Windows 10 Build 20348+ and uses the Process Loopback API.

## Per-Process Capture

### Requirements
- Windows 10 Build 20348 or later
- Uses `AUDIOCLIENT_PROCESS_LOOPBACK_PARAMS`

### Implementation Steps

1. **Process Enumeration**
```rust
use windows::Win32::System::ProcessStatus::*;

// Enumerate running processes
let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0)?;
// Filter out system processes (PID 0, 4) and Gecko itself
```

2. **Interface Activation**
```rust
// Cannot use IMMDevice::Activate - must use async activation
ActivateAudioInterfaceAsync(
    device_id,
    &IAudioClient::IID,
    &activation_params,
    completion_handler,
)?;
```

3. **Parameter Structure**
```rust
pub struct AUDIOCLIENT_PROCESS_LOOPBACK_PARAMS {
    pub TargetProcessId: u32,
    pub ProcessLoopbackMode: PROCESS_LOOPBACK_MODE,
}

// Modes:
// INCLUDE_TARGET_PROCESS_TREE - Capture app and its children
// EXCLUDE_TARGET_PROCESS_TREE - Capture everything EXCEPT the app
```

4. **Completion Handler**
```rust
// Implement IActivateAudioInterfaceCompletionHandler
impl IActivateAudioInterfaceCompletionHandler {
    fn ActivateCompleted(&self, result: &IActivateAudioInterfaceAsyncOperation) {
        // Get IAudioClient from result
        // Initialize with AUDCLNT_STREAMFLAGS_LOOPBACK
    }
}
```

## Fallback Strategy

For Windows versions < 10.0.20348:

```rust
pub fn supports_per_app_capture() -> bool {
    // Check Windows version
    let version = get_windows_version();
    version.build >= 20348
}

// If unsupported, fall back to system-wide loopback
if !supports_per_app_capture() {
    warn!("Per-app capture unavailable, using system loopback");
    // Standard WASAPI loopback captures all system audio
}
```

## Virtual Sink Strategy

### Challenge
Creating virtual audio devices on Windows requires **kernel drivers** - a significant undertaking.

### Recommended Approach (v1)

**Don't install a custom driver.** Instead:

1. Detect existing virtual audio drivers:
   - VB-Cable
   - Virtual Audio Cable
   - Voicemeeter

2. If found, offer as output targets in UI

3. If not found, prompt user to install one

### Future Approach (v2+)

If a custom driver is needed:
- Base on Microsoft SYSVAD sample
- Requires EV Code Signing Certificate
- Requires Hardware Developer Center account
- Use `pnputil` or `devcon` for installation

## Platform Capabilities

```rust
pub fn supports_virtual_devices() -> bool {
    false  // Requires kernel driver
}

pub fn supports_per_app_capture() -> bool {
    true  // AUDIOCLIENT_PROCESS_LOOPBACK_PARAMS (Win10 20348+)
}
```

## WASAPI Quirks

### Shared vs Exclusive Mode
- **Shared Mode**: Multiple apps share the device (use this)
- **Exclusive Mode**: Gecko takes over the device (avoid - blocks other apps)

### Silence Handling
WASAPI loopback stops delivering packets when source is silent. Gecko must:
- Detect silence condition
- Generate silence samples to prevent output underrun
- Or use a "keep-alive" pattern

```rust
if packets_available == 0 && is_loopback {
    // Source is silent - generate silence
    buffer.fill(0.0);
}
```

## Error Handling

```rust
pub enum PlatformError {
    #[error("WASAPI initialization failed: {0}")]
    WasapiInitFailed(String),

    #[error("Process not found: {0}")]
    ProcessNotFound(u32),

    #[error("Per-app capture not supported on this Windows version")]
    PerAppNotSupported,

    #[error("Device activation failed: {0}")]
    ActivationFailed(String),
}
```

## Dependencies

```toml
[target.'cfg(target_os = "windows")'.dependencies]
windows = { version = "0.58", features = [
    "Win32_Media_Audio",
    "Win32_System_Com",
    "Win32_System_Threading",
    "Win32_Foundation",
]}
```

## Related Files

- `crates/gecko_platform/src/windows/` - WASAPI implementation
- `crates/gecko_platform/src/lib.rs` - Platform trait and detection
- `docs/02_Windows_Strategy.md` - Original strategy document
