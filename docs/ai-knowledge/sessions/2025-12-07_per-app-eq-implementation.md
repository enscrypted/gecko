# Session: Per-App Additive EQ Implementation

**Date**: 2025-12-07
**Status**: In Progress (Blocker: UI slider reset)

## Task Summary
Implementing per-application additive EQ functionality where each app's EQ adjustments combine with a master EQ filter. This is a key MVP feature.

## Key Decisions

- **Architecture**: "Active Stream EQ Blending" model where:
  - Master EQ gains stored in `AudioProcessingState.master_eq_gains[10]`
  - Per-stream offsets in `AudioProcessingState.stream_eq_offsets: HashMap<String, [f32; 10]>` (uses `parking_lot::RwLock`)
  - Combined EQ = master + sum(all stream offsets) recalculated on any change
  - Audio callback reads `combined_eq_gains` and applies to single Equalizer instance

- **Persistence**: App EQ settings keyed by **app name** (not PID) in `GeckoSettings.app_eq: HashMap<String, Vec<f32>>` for stability across sessions

- **Frontend**: Master stream calls `set_band_gain`, other streams call `set_stream_band_gain`

## Files Modified

| File | Change |
|------|--------|
| `crates/gecko_platform/src/linux/audio_stream.rs` | Added `master_eq_gains`, `stream_eq_offsets`, `combined_eq_gains` to `AudioProcessingState`; added `recalculate_combined_eq()` |
| `crates/gecko_core/src/message.rs` | Added `SetStreamBandGain { stream_id, band, gain_db }` command |
| `crates/gecko_core/src/engine.rs` | Added `set_stream_band_gain()` method and command handler |
| `crates/gecko_platform/src/linux/mod.rs` | Added `update_stream_eq_band()` to `PipeWireBackend` |
| `crates/gecko_core/src/settings.rs` | Added `app_eq: HashMap<String, Vec<f32>>` to `GeckoSettings` |
| `src-tauri/src/commands.rs` | Added `set_stream_band_gain` Tauri command with persistence |
| `src-tauri/src/lib.rs` | Registered `set_stream_band_gain` command |
| `src/contexts/SettingsContext.tsx` | Added `app_eq` to frontend settings interface |
| `src/components/AudioStreamItem.tsx` | Call `set_stream_band_gain` for app streams (not master) |
| `src/components/StreamList.tsx` | Load persisted app EQ from `settings.app_eq` on stream init |

## Current State

**What works**:
- Backend architecture is complete and compiles
- Master EQ works correctly
- App EQ changes are sent to backend and persisted to settings

**Blocker**:
- App EQ sliders reset to 0 after ~1 second
- Likely cause: Frontend state management issue - either `useEffect` re-running with old bandGains prop, or stream list polling resetting state

## Debugging Clues (for next session)
1. `StreamList.tsx` polls `list_audio_streams` every 2 seconds
2. `fetchStreams` checks `if (!newStreamGains[stream.id])` before initializing - should prevent overwrite, but stream.id might be changing?
3. `AudioStreamItem` syncs `localGains` from `bandGains` prop via `useEffect([bandGains])` - if parent re-renders with stale/zero gains, this would reset sliders
4. The stream ID format is `"app_name:pid"` - if PID changes (app restart), gains might not carry over

## Next Steps

1. **Debug frontend state flow**:
   - Add console.log to `handleBandChange`, `fetchStreams`, and the `useEffect` that syncs `localGains`
   - Check if `fetchStreams` is resetting `streamGains` incorrectly
   - Check if parent is passing stale `bandGains` prop

2. **Verify stream ID stability**:
   - Log stream IDs when polling
   - Ensure ID format matches between frontend and backend

3. **Check useEffect dependencies**:
   - The `[bandGains]` dependency might be triggering on every render if array reference changes

## How to Continue
```
Continue from session 2025-12-07_per-app-eq-implementation
Focus: Debug why app EQ sliders reset to 0 after ~1 second
```
