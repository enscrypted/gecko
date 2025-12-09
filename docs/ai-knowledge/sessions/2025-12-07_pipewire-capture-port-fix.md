# Session: PipeWire Capture Stream Port Creation Fix

**Date**: 2025-12-07
**Status**: In Progress (Major Breakthrough)

## Task Summary
Fixed the long-standing issue where PipeWire capture stream input ports were not being created, which blocked the entire audio pipeline. The capture stream is critical for reading audio from the Gecko Audio virtual sink's monitor ports.

## Key Decisions
- **Use any Audio/Source for format negotiation**: Instead of trying to connect to the virtual sink directly (which failed because proxy ID != node ID), we connect to any existing Audio/Source in the registry to force PipeWire to complete format negotiation and create input ports.
- **Manual link rewiring in main loop**: After ports are created, we poll in the main loop until both Gecko Audio monitor ports and Gecko Capture input ports appear, then manually create links between them using `core.create_object::<Link>()`.
- **Removed broken sync_state call**: The sync_state() method was never implemented and wasn't needed with the new approach.

## Files Modified
| File | Change |
|------|--------|
| `crates/gecko_platform/src/linux/thread.rs` | Find any Audio/Source for capture target; port creation now works |
| `crates/gecko_core/src/engine.rs` | Removed non-existent sync_state() call |
| `docs/ai-knowledge/ai-patterns/mistake-log.md` | Documented the solution |

## Current State
- **Capture stream ports ARE being created** (verified in logs: `Found 2 input ports on capture 109: [94, 100]`)
- **Manual links created successfully**: `Gecko Audio:monitor_1 -> Gecko Capture:input_FL`, `Gecko Audio:monitor_2 -> Gecko Capture:input_FR`
- **Both streams entering Streaming state**: Logs show `Capture stream state: Paused -> Streaming` and `Playback stream state: Paused -> Streaming`
- **PipeWire graph verified**: `pw-link -l` shows correct connections

## What's Working
1. Virtual sink creation ("Gecko Audio")
2. Capture stream format negotiation and port creation
3. Playback stream format negotiation and port creation
4. Manual link creation from monitor ports to capture ports
5. Both streams entering "Streaming" state

## What's NOT Yet Tested
1. Actual audio flow end-to-end (route Firefox to Gecko Audio, verify sound)
2. Level meters animating with audio
3. EQ adjustments affecting audio
4. Output device changes

## Next Steps
1. Test actual audio playback end-to-end:
   - Run app: `pnpm tauri dev`
   - Click Start button
   - In system settings, route Firefox audio to "Gecko Audio"
   - Play audio in Firefox
   - Verify sound comes through speakers
   - Verify level meters animate
2. Test EQ adjustments affect audio output
3. Address security concern about name-based registry lookups (deferred, noted by user)
4. Clean up excessive polling in main loop (optimize retry interval)

## Technical Details

### The Root Cause
PipeWire capture streams need a valid target node to negotiate sample format, channel count, buffer size etc. Without format negotiation, the stream cannot create ports. Our previous attempts failed because:
- Proxy ID from `create_object()` is NOT the same as node ID in registry
- `None` target doesn't trigger format negotiation even with AUTOCONNECT
- Virtual sink doesn't appear in registry immediately after creation

### The Solution Pattern
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

## How to Continue
1. Start a new conversation
2. Say: "Continue from session 2025-12-07_pipewire-capture-port-fix"
3. Run `pnpm tauri dev` and test actual audio flow
4. Check if level meters animate when audio plays through Gecko Audio

## Commands Reference
```bash
pnpm tauri dev          # Run the app
pw-link -l              # Check PipeWire links
pw-cli ls Node          # List all nodes
cargo test --workspace  # Run tests (69 tests)
```
