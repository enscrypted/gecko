# Session: Windows WASAPI Implementation Fixes

**Date**: 2025-12-30 (Updated: 2025-12-31)
**Status**: Integration Complete - Ready for Testing

## Task Summary
Continued from previous session where Windows WASAPI implementation was created. Fixed compilation errors when building on actual Windows system. The previous implementation was built on WSL/Linux where Windows-specific code wasn't compiled, so many API incompatibilities weren't caught.

## Key Decisions
- Made `windows` crate non-optional on Windows (always included) to avoid feature flag issues with target-specific dependencies
- Used `PropVariantToStringAlloc` helper instead of raw PROPVARIANT field access (windows 0.58 API change)
- Simplified rtrb ring buffer to use `push()`/`pop()` instead of chunk APIs for cleaner code
- GetBuffer/GetNextPacketSize APIs use out-parameters, not return tuples

## Files Modified
| File | Change |
|------|--------|
| `crates/gecko_platform/Cargo.toml` | Made windows dependency non-optional on Windows |
| `crates/gecko_platform/src/error.rs` | Added `From<windows::core::Error>` for PlatformError |
| `crates/gecko_platform/src/windows/session.rs` | Added Interface import, fixed type annotations for BOOL/f32 |
| `crates/gecko_platform/src/windows/device.rs` | Added Interface import, fixed PROPVARIANT access using PropVariantToStringAlloc |
| `crates/gecko_platform/src/windows/thread.rs` | Fixed ProcessContext fields, AudioProcessor import, rtrb API, GetBuffer/GetNextPacketSize APIs |
| `crates/gecko_platform/src/windows/message.rs` | Removed unused Arc import |

## Current State
- App compiles and runs on Windows with `pnpm tauri dev`
- Vite frontend starts at localhost:5173
- Tauri window opens successfully
- **Issue**: User seeing "Nahimic mirroring device" instead of real speakers
  - This is likely a system config issue (Nahimic audio software on gaming laptops)
  - User restarting laptop to reset audio subsystem

## Windows API Fixes Applied

### 1. Interface trait for .cast()
```rust
#[cfg(target_os = "windows")]
use windows::core::Interface;
```

### 2. PROPVARIANT access (windows 0.58)
```rust
// Old (broken):
if prop.Anonymous.Anonymous.vt.0 == 31 { ... }

// New (working):
use windows::Win32::System::Com::StructuredStorage::PropVariantToStringAlloc;
match PropVariantToStringAlloc(&prop) {
    Ok(pwstr) => pwstr.to_string()...
}
```

### 3. GetBuffer API (takes out-params)
```rust
let mut data_ptr: *mut u8 = std::ptr::null_mut();
let mut frames_available: u32 = 0;
let mut flags: u32 = 0;

self.capture_client.GetBuffer(
    &mut data_ptr,
    &mut frames_available,
    &mut flags,
    None,
    None,
)
```

### 4. ProcessContext fields
```rust
// Correct fields (no is_offline):
gecko_dsp::ProcessContext {
    sample_rate: 48000.0,
    channels: 2,
    buffer_size: samples_read,
}
```

## Next Steps
1. User restart to fix Nahimic audio device issue
2. Test actual audio capture/output with real speakers
3. Verify per-app audio capture works
4. Test EQ processing
5. If Nahimic persists, may need device filtering or selection UI

## How to Continue
1. Start a new conversation
2. Say: "Continue from session 2025-12-30_windows-wasapi-fixes"
3. After restart, report:
   - Do real audio devices show up?
   - Does audio output work?
   - Any errors in console?

## Development Setup (Windows)
```powershell
# Prerequisites installed:
# - Rust via rustup.rs
# - Node.js + pnpm
# - Visual Studio Build Tools with C++ workload

# Run the app:
cd C:\work\gecko
pnpm install
pnpm tauri dev
```

---

## Update 2025-12-31: Full gecko_core Integration

### Problem
The Windows WASAPI backend was built but gecko_core::engine was using a fallback "output-only mode" with cpal instead of our new WasapiBackend.

### Solution
Integrated WasapiBackend into gecko_core::engine.rs with full feature parity:

### Changes Made to `crates/gecko_core/src/engine.rs`

1. **Added Windows imports**:
   ```rust
   #[cfg(target_os = "windows")]
   use gecko_platform::windows::{AudioProcessingState as WasapiProcessingState, WasapiBackend};
   ```

2. **Added Windows state variables**:
   ```rust
   #[cfg(target_os = "windows")]
   let mut windows_backend: Option<WasapiBackend> = None;
   #[cfg(target_os = "windows")]
   let mut windows_state: Option<Arc<WasapiProcessingState>> = None;
   #[cfg(target_os = "windows")]
   let mut captured_pids: std::collections::HashSet<u32> = std::collections::HashSet::new();
   ```

3. **Command::Start for Windows**:
   - Initializes WasapiBackend
   - Sets initial volume/bypass/EQ
   - Starts audio output
   - Auto-enumerates and captures active audio apps
   - Sends StreamDiscovered events

4. **Command::Stop for Windows**:
   - Stops all captures
   - Stops output
   - Cleans up backend state

5. **EQ/Volume/Bypass commands**:
   - SetMasterVolume → backend.set_master_volume()
   - SetBypass → backend.set_master_bypass()
   - SetBandGain → backend.set_master_eq_gains()

6. **Periodic app scanning (every 2 seconds)**:
   - Scans for new audio apps
   - Auto-captures newly active apps
   - Sends StreamDiscovered events

7. **Level updates**:
   - Gets peak levels from WasapiProcessingState
   - Sends LevelUpdate events

### Expected Behavior After Integration

When you run `pnpm tauri dev` on Windows:
1. WASAPI backend initializes
2. Audio output starts
3. Active audio apps are detected and captured
4. Apps appear in the UI app list
5. EQ processing applies to captured audio
6. Level meters show audio activity
7. New apps starting audio are auto-captured every 2 seconds

### Testing Commands
```powershell
# Test just the platform crate
cargo test -p gecko_platform

# Run the full app
pnpm tauri dev
```
