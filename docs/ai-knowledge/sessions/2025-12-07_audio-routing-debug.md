# Session: Audio Routing Debugging

**Date**: 2025-12-07
**Status**: Resolved

## Task Summary
Diagnosed and fixed multiple audio processing issues:
1. EQ bands not affecting audio (only volume worked)
2. Device switching code never executing (was outside main loop)
3. Aggressive routing enforcement causing unnecessary link churn

## Bugs Fixed

### 1. EQ Band Updates Not Applied to Audio Callback
**Root Cause**: `UpdateEqBand` command was updating `local.equalizer` in `thread.rs`, but this was `None`. The actual EQ processing happened in the capture callback's `CaptureUserData.equalizer`, which was a separate instance that never received updates.

**Fix**: Added EQ band storage to `AudioProcessingState` with atomic access:
- `eq_band_gains: [AtomicU32; 10]` - stores band gains as bits
- `eq_update_counter: AtomicU32` - increments on any change
- Capture callback checks counter and applies all gains when changed

**Files Modified**:
- `crates/gecko_platform/src/linux/audio_stream.rs` - Added EQ state to `AudioProcessingState`
- `crates/gecko_platform/src/linux/thread.rs` - Updated `CaptureUserData` and callback to check for EQ updates

### 2. Device Switching Code Outside Main Loop
**Root Cause**: The periodic device switching code block (checking for headphone plug/unplug) was placed AFTER the `while` loop closing brace, meaning it only executed once on shutdown, not during normal operation.

**Fix**: Moved the entire periodic check logic into the `Timeout` branch of the command receiver match, where it now runs every 500ms while the engine is active.

**Files Modified**:
- `crates/gecko_core/src/engine.rs` - Moved periodic tasks into Timeout handler, removed dead code

### 3. Aggressive Routing Causing Link Churn
**Root Cause**: `move_stream_to_sink()` and `enforce_capture_routing()` were called every 500ms without checking if links were already correct, potentially causing audio glitches from constant reconnection.

**Fix**: Added early-out checks to both functions:
- Check if all expected links already exist before any modifications
- Only log and modify when changes are actually needed

**Files Modified**:
- `crates/gecko_platform/src/linux/mod.rs` - Added early-out checks to routing functions

## Key Patterns Established

1. **Cross-thread EQ communication**: Use atomics + counter pattern for real-time safe EQ updates from UI to audio callback
2. **Periodic tasks placement**: Must be INSIDE the main command loop (in Timeout branch), not after it
3. **Routing enforcement**: Always check current state before modifying to avoid unnecessary churn

## Verification
- All 58 tests pass (`cargo test --workspace`)
- No new clippy errors
- Code compiles successfully

## Next Steps for Testing
1. Run `pnpm tauri dev` and play audio
2. Move EQ sliders and verify audible changes
3. Plug/unplug headphones and verify audio continues
4. Check logs for "Applied EQ update" messages when sliders move
