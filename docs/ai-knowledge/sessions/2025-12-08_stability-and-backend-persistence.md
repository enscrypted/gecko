# Session: Stability Checks & Backend Persistence
**Date**: 2025-12-08
**Status**: Complete

## Task Summary

Addressed critical instability and state loss issues discovered during extensive testing:
1.  **App Disappearance/Crash**: Firefox (and other apps) disappearing from the UI loop after a Stop/Start cycle.
2.  **State Loss**: Application-specific settings (Volume, EQ, Bypass) resetting to defaults on engine restart.
3.  **UI UX Bug**: Accidental accordion expansion when interacting with per-app controls.

## Key Decisions & Fixes

### 1. Backend Race Condition Fixes (Stability)
**Context**: "Firefox dropped off... right after a stop/start".
-   **Diagnosis**: Two race conditions were identified.
    1.  **Stop Race**: Virtual sinks were destroyed *immediately* on stop, severing the app's audio stream before WirePlumber could migrate it to the hardware sink.
    2.  **Start Race**: The "Default Sink Monitor" ran immediately on start, saw the OLD default sink (before the system updated), and triggered a panic/switch routine that conflicted with startup.
-   **Fixes**:
    -   Added `std::thread::sleep(Duration::from_millis(250))` in `Command::Stop` to allow stream migration.
    -   Extended `routing_grace_period` (2s) to cover the Default Sink Monitor check in `engine.rs`.

### 2. Backend State Persistence
**Context**: "It applied the EQ effects but NOT the app-level volume".
-   **Diagnosis**: `AudioProcessingState` is tied to the backend instance. When the backend is recreated on `Command::Start`, all previous state was lost.
-   **Fix**: Implemented local state caching in `AudioEngine::audio_thread_main`:
    ```rust
    // crates/gecko_core/src/engine.rs
    let mut app_volumes: HashMap<String, f32> = HashMap::new();
    let mut app_bypassed: HashMap<String, bool> = HashMap::new();
    let mut app_eq_gains: HashMap<String, [f32; 10]> = HashMap::new();
    // ... updated on commands, applied on Start ...
    ```

### 3. UI Event Propagation
-   **Fix**: Removed `onClick` handler from the per-app control row in `AudioStreamItem.tsx` to prevent accidental expansion when adjusting volume.

## Files Modified

| File | Change |
|------|--------|
| `crates/gecko_core/src/engine.rs` | Added persistence maps, Grace Period check, Stop Delay. |
| `crates/gecko_platform/src/linux/thread.rs` | Added bounds check for stability, Enhanced logging. |
| `src/components/AudioStreamItem.tsx` | Removed click handler from controls row. |

## Future Work (TODO)

1.  **Themes & Styling**: Add themes/colorways to the styling library and settings page.
2.  **Documentation Cleanup**: Remove "Platform Capabilities" section (deprecated).
3.  **Codebase Review**: Conduct a thorough review of state and architecture, update documentation, ensure full test coverage, and enforce coding standards.
