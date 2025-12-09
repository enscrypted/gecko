# Windows Implementation Strategy: Process Loopback & Virtualization

## 1. Capture Strategy: The Process Loopback API

Standard WASAPI Loopback captures the entire system mix. Gecko utilizes the **Application Loopback API** (Windows 10 Build 20348+) to capture specific process trees.

### 1.1 Technical Implementation

The capture logic is implemented in the `gecko_platform::windows` module using the `windows` crate.

#### Process Enumeration

```rust
// Use CreateToolhelp32Snapshot to list running processes
// Filter out system processes (pid 0, 4) and Gecko itself
```

#### Interface Activation

We cannot use `IMMDevice::Activate`. We must use `ActivateAudioInterfaceAsync`.

```rust
pub struct AUDIOCLIENT_PROCESS_LOOPBACK_PARAMS {
    pub TargetProcessId: u32,
    pub ProcessLoopbackMode: PROCESS_LOOPBACK_MODE,
}
```

The `ProcessLoopbackMode` can be set to:
- `PROCESS_LOOPBACK_MODE_INCLUDE_TARGET_PROCESS_TREE` (capture the app and its children)
- `PROCESS_LOOPBACK_MODE_EXCLUDE_TARGET_PROCESS_TREE` (capture everything except the app)

#### Async Handler

Rust must implement the COM interface `IActivateAudioInterfaceCompletionHandler`. Upon completion, we receive the `IAudioClient`.

### 1.2 Fallback Strategy

For Windows versions < 10.0.20348:
- The system degrades to **System Wide Loopback**
- The UI must inform the user: "Per-application capture is unavailable on this version of Windows."

## 2. Virtual Sink Strategy (Output)

Gecko requires an output endpoint to route processed audio to other applications.

### 2.1 The "No-Driver" Approach (v1.0)

- Gecko does not install a custom driver initially
- It detects if **VB-Cable** or **Virtual Audio Cable** is installed
- If detected, it offers them as output targets in the UI

### 2.2 The Custom Driver Approach (Roadmap)

- **Source**: Microsoft SYSVAD Virtual Audio Device sample
- **Modification**: Create a "Simple Audio Sample" driver that exposes a Render Endpoint and a Capture Endpoint
- **Routing**: The driver internally copies the Render buffer to the Capture buffer
- **Deployment**: Requires a Hardware Developer Center account and EV Code Signing Certificate

## 3. WASAPI Quirks and Mitigations

### Exclusive Mode
Gecko avoids Exclusive Mode for loopback, as it blocks other apps from hearing the stream. **Shared Mode is strictly used.**

### Silence Handling
WASAPI Loopback stops delivering packets if the source app is silent. Gecko implements a "Silence Generator" in the ring buffer to ensure the output stream doesn't underrun.

## 4. Implementation Checklist

- [ ] Process enumeration via `CreateToolhelp32Snapshot`
- [ ] `ActivateAudioInterfaceAsync` implementation
- [ ] `IActivateAudioInterfaceCompletionHandler` COM interface
- [ ] Fallback to system loopback on older Windows
- [ ] VB-Cable/VAC detection
- [ ] Silence handling for loopback streams
