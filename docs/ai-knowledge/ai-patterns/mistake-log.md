# Agent Mistake Log

## Purpose

This file tracks patterns where AI agents make mistakes that get corrected. When a pattern appears 3+ times, it should be added to AGENT.md "Things to Avoid" section.

## Entry Format

- **Date**: When it occurred
- **Pattern**: What the agent did wrong
- **Correction**: What it should have done
- **Count**: How many times this has occurred

---

## Logged Patterns

### 2024-12-06 - Test Expectations

**Mistake**: Expected BiQuad filters to pass through audio unchanged at 0dB gain.

**Correction**: BiQuad filters have transient response. Need to "warm up" the filter with ~1000 samples before testing steady-state behavior. Even at 0dB, cascaded filters slightly affect the signal.

**Prevention**: For audio filter tests, process warmup samples first, then verify output is stable and reasonable rather than exactly equal to input.

**Count**: 1

---

### 2024-12-06 - Microphone Input Instead of Application Audio Capture

**Mistake**: Initial `gecko_core/src/stream.rs` implementation used `host.default_input_device()` to capture audio, which grabs the microphone. This caused a feedback loop (high-pitched shrieking sound) when the user ran the application.

**Correction**: Gecko is designed to capture **application audio** (Firefox, Spotify, etc.), NOT microphone input. The correct architecture is:
- **Linux**: Create a PipeWire virtual sink → Apps route to it → Capture from sink's monitor port
- **Windows**: Use WASAPI Process Loopback API to capture specific app audio
- **macOS**: Use HAL plugin with shared memory ring buffer

The key insight: This is a **system audio processor**, not a voice application. There should be **zero microphone involvement**.

**Prevention**:
1. Always check project documentation (PDF design docs, AGENT.md) before implementing audio capture
2. The term "passthrough" in audio processing does NOT imply microphone input
3. "Capture" in this context means capturing application audio routed through virtual devices

**Count**: 1

---

---

### 2024-12-07 - PipeWire Capture Stream Port Creation Attempts

**Mistake**: Multiple failed attempts to get capture stream input ports to be created by PipeWire.

**Attempts tried (all failed to create input ports)**:

1. **No AUTOCONNECT, manual links**: Removed AUTOCONNECT flag and tried to create links manually after stream connected.
   - Result: Stream entered "Paused" state but NO input ports were ever registered in PipeWire registry.
   - Why it failed: PipeWire needs to negotiate format with a target to create ports.

2. **AUTOCONNECT flag + property, but `None` target**: Added `StreamFlags::AUTOCONNECT` and `"node.autoconnect" => "true"` property, but passed `None` as target to `connect()`.
   - Result: Same - stream reached Paused but no ports created.
   - Why it failed: Even with AUTOCONNECT, without a target ID PipeWire cannot negotiate format.

3. **AUTOCONNECT + `Some(capture_target)` (sink proxy ID 3)**: Passed the virtual sink's proxy ID as target.
   - Result: FAILED - still 0 input ports. "Found 2 monitor ports on sink 91: [103, 106]" but "Found 0 input ports on capture 111: []"
   - Why it failed: Proxy ID (3) != Node ID (91). PipeWire cannot find node with ID 3 to negotiate format.

4. **FIX APPROACH**: Wait for virtual sink to appear in registry, use its NODE ID not proxy ID.
   - Need to wait for "Gecko Audio" node to appear in registry with a valid node ID
   - Then pass that node ID to capture stream connect() for format negotiation

**Root cause understanding**: PipeWire capture streams need a valid target (node ID or object ID) to negotiate sample format, channel count, etc. Without format negotiation, the stream cannot create ports. The virtual sink's proxy ID (from `create_object`) may not be the same as its node ID (from registry).

**SOLUTION FOUND (2024-12-07)**:
The fix was to use ANY existing `Audio/Source` node (e.g., a microphone) as the target for format negotiation when connecting the capture stream. This forces PipeWire to negotiate format and create input ports. Then in the main loop, we wait for ports to appear in registry and create manual links from Gecko Audio's monitor ports to Gecko Capture's input ports.

Key code pattern:
```rust
// Find any audio source for format negotiation
let any_source_id = local.nodes
    .iter()
    .find(|(_, n)| n.media_class.as_deref() == Some("Audio/Source"))
    .map(|(id, _)| *id);

// Connect with AUTOCONNECT and the source target
capture_stream.connect(
    Direction::Input,
    any_source_id, // Forces format negotiation
    StreamFlags::AUTOCONNECT | StreamFlags::MAP_BUFFERS | StreamFlags::RT_PROCESS,
    &mut capture_params,
);

// Set flag to rewire in main loop after ports appear
local.pending_capture_links = true;
```

The main loop then polls for port availability and creates manual links using `core.create_object::<Link>()`.

**Count**: 3+ (RESOLVED)

---

### 2025-12-07 - Autoconnect Causing Feedback Loop

**Mistake**: Used `StreamFlags::AUTOCONNECT` for the capture stream to force port creation, targeting "any audio source" (often the microphone).

**Correction**: This caused WirePlumber to automatically link the hardware microphone to the capture stream, creating a feedback loop or "dirty" input. The correct approach is to disable `AUTOCONNECT` and manually connect the capture stream *only* to the virtual sink's monitor ports once they appear.

**Prevention**: Never use `AUTOCONNECT` for streams that require specific, exclusive routing (like internal DSP pipelines). Always manage links manually for these cases.

**Count**: 1

---

### 2025-12-07 - Inadequate Ring Buffer Size Causing Silence

**Mistake**: Spent significant time debugging "routing issues" and "PipeWire graph connections" when the application produced no audio. The actual root cause was the ring buffer being too small (4096 frames), causing immediate underruns and silence.

**Correction**: Check for buffer underruns/overflows early when debugging silence. Increased ring buffer size to 32768 frames (approx 680ms at 48kHz) resolved the silence immediately.

**Prevention**:
1. When diagnosing "no audio", first verify that data is actually flowing (logs, buffer stats) before assuming it's a routing/graph issue.
2. Use safe defaults for ring buffers (e.g., 500ms+) in development to rule out starvation.

**Count**: 1

---

### 2025-12-07 - Aggressive Routing Enforcement vs Session Managers

**Mistake**: Attempted to "fight" WirePlumber by implementing an aggressive loop (every 200ms) that manually breaks and recreates links to force a specific routing topology. This leads to race conditions, "phasing" (double audio), and "zombie sinks" when the enforcement logic fights the session manager's restoration logic.

**Correction**: Cooperate with the session manager where possible. Use "passive" links or policy configuration if available. If manual routing is required, ensure it handles existing links gracefully rather than blindly creating new ones.

**Prevention**:
Avoid polling loops that mutate graph state. Use event-driven responses to graph changes.

**Count**: 1

---

### 2025-12-07 - EQ Updates Not Reaching Audio Callback

**Mistake**: EQ band gain updates were sent to `local.equalizer` in the PipeWire thread, but this was `None`. The actual EQ processing happened in the capture callback's own `CaptureUserData.equalizer`, a separate instance that never received the updates. Result: only volume control worked, EQ sliders had no effect.

**Correction**: The EQ state must be shared between the command handler and the audio callback. Added EQ band storage to `AudioProcessingState` using atomics (same pattern as volume). The capture callback now checks an update counter and applies all band gains when changes are detected.

**Prevention**:
1. When DSP state needs to be updated from outside the audio callback, use shared atomic state (not a local variable in the command handler)
2. The "counter + check" pattern allows the callback to detect changes without locking
3. Always trace data flow: Command → Shared State → Audio Callback

**Count**: 1

---

### 2025-12-07 - Periodic Code Placed Outside Main Loop

**Mistake**: Device switching/monitoring code was placed AFTER the closing brace of the main `while` loop in `engine.rs`, so it only executed once on shutdown instead of periodically during operation.

**Correction**: Moved periodic task logic INSIDE the main command loop, specifically in the `Timeout` branch of the channel receive.

**Prevention**:
1. Periodic tasks in event loops must be inside the loop, typically in a timeout/idle handler
2. When reviewing code structure, verify that periodic checks are actually reachable during normal operation
3. Watch for orphaned code blocks that appear related to looping behavior but are outside the loop

**Count**: 1

---

### 2025-12-07 - Stale Registry Data After Device Hotplug

**Mistake**: After calling `SwitchPlaybackTarget` during device hotplug, audio worked for a split second then went silent. The `try_create_capture_links` function found OLD "Gecko Capture" node, ports, and links in the local registry (HashMap) and concluded links already existed.

**Correction**: Before creating new streams in `SwitchPlaybackTarget`, explicitly remove stale registry entries:
1. Find old "Gecko Capture" node by name
2. Remove all ports belonging to that node from `local.ports`
3. Remove all links referencing those port IDs from `local.links`
4. Remove the old node from `local.nodes`
5. Clear `capture_stream_node_id = None`

**Prevention**:
1. When recreating PipeWire streams, always clean up stale local state first
2. Registry events (Node/Port/Link removed) may arrive asynchronously - don't rely on them for immediate cleanup
3. Use explicit cleanup before creating replacement objects

**Count**: 1

---

### 2025-12-07 - Device Targeting by ID vs NAME During Hotplug

**Mistake**: Used node ID to target playback device during hotplug. But node IDs change when devices are unplugged and replugged - the old ID is invalid by the time the switch command executes.

**Correction**: Use device NAME (e.g., `alsa_output.usb-Apple__Inc._EarPods_H9M6QJ666V-00.analog-stereo`) instead of node ID. PipeWire resolves the name to the current ID at connection time via `target.object` property.

**Prevention**:
1. For hotplug scenarios, always use device NAME, never ID
2. Names are stable across unplug/replug cycles; IDs are not
3. The `SwitchPlaybackTarget` command takes a `target_name: String` for this reason

**Count**: 1

---

### 2025-12-07 - Frontend State Reset Due to Parent Re-render / Polling

**Mistake**: Implemented per-app EQ sliders that reset to 0 after ~1 second. The backend persistence and command chain was built correctly, but slider UI keeps resetting.

**Root Cause Found**:
- `StreamList.tsx` polls `list_audio_streams` every 2 seconds
- On each poll, `fetchStreams` was recreating the `streamGains` state object
- The initial guard `if (!newStreamGains[stream.id])` was checking the wrong variable
- useEffect dependency on `bandGains` array caused re-sync from parent props

**Solution Applied**:
1. Changed to functional state updates (`setStreamGains(prev => {...})`) to avoid stale closure issues
2. Added `initialSyncComplete` tracking to apply persisted EQ to backend only once per stream
3. Applied persisted EQ/volume to backend immediately when streams first appear
4. Separated volume and EQ state management for clarity

**Prevention**:
1. Use functional state updates when inside polling callbacks to avoid stale closures
2. Track "first sync" state to avoid repeatedly applying persisted settings
3. Load persisted settings in the poll callback where new streams are detected, not in separate useEffects

**Count**: 1 (RESOLVED)

---

### 2025-12-26 - Attempted Global Tap Workaround Instead of Per-Process

**Mistake**: When debugging macOS Process Tap API "!obj" error, attempted to use `initStereoGlobalTapButExcludeProcesses:` as a "workaround" to test if CATapDescription works. This directly violates the core principle: **Per-app EQ is the CORE MVP FEATURE**.

**Correction**: Per-app EQ is non-negotiable. A global tap defeats the entire purpose of the application. The correct approach is to fix the actual per-process tap (`initStereoMixdownOfProcesses:`) rather than implementing a shortcut that compromises the product.

**Prevention**:
1. ALWAYS re-read AGENT.md before implementing workarounds - it explicitly says: "DO NOT implement shortcuts or approximations"
2. If a core feature isn't working, fix it - don't ship a crippled version
3. When user says "no global tap" multiple times, STOP and listen
4. Per-app EQ differentiates Gecko from competitors - shortcuts destroy market positioning

**Count**: 1

---

### 2025-12-27 - Attempted to Remove Permission Reset from Dev Script

**Mistake**: When the app crashed during permission granting, attempted to "fix" it by removing the permission reset from `dev-macos.sh`, claiming permissions would persist between runs.

**Why This Was Wrong**:
1. Ad-hoc code signing (`codesign --sign -`) creates a NEW identity each build
2. macOS permissions are tied to code signing identity
3. Therefore permissions DO NOT persist between builds - they MUST be reset
4. The script resets permissions for a critical technical reason, not convenience

**Correction**: The real issue is that granting "System Audio Recording Only" permission mid-session causes macOS to invalidate running audio devices, crashing the app. The fix should be in the permission handling flow, NOT in the build script.

**Prevention**:
1. Understand WHY something exists before removing it
2. Don't remove infrastructure without understanding the technical constraints
3. When debugging permission issues, the fix should be in permission handling code, not build scripts
4. Ad-hoc signing = new identity = permissions don't persist (fundamental macOS behavior)

**Count**: 1

---
