# Session: UI Polish, Bug Fixes & Device Hotplug

**Date**: 2025-12-08
**Status**: Completed (ready for next phase)

## Task Summary

This session focused on polishing the Gecko Audio UI, fixing critical bugs, and improving user experience. Work continued from a previous session that implemented accessibility improvements (ThemeDropdown, EditableValue components).

## Key Decisions

- **Box-shadow over borders**: Use `box-shadow: inset 0 0 0 1px` instead of CSS borders to avoid subpixel rendering artifacts that cause white lines on dark themes
- **Theme-aware accent colors**: Master EQ border uses `var(--gecko-accent)` CSS variable instead of hardcoded green, so it adapts to all themes including colorblind-safe blue
- **Master volume persistence**: Added to settings context with debounced save, synced on app load

## Files Modified

| File | Change |
|------|--------|
| `crates/gecko_platform/src/linux/thread.rs` | Fixed audio device switching in per-app mode (recreates mixing_playback_stream with new target); Fixed buffer size crash (8192→48000) |
| `src/styles.css` | Darkened border colors (#262626→#1a1a1a) to reduce subpixel artifacts; Added ring-offset-color fix |
| `src/components/ui/card.tsx` | Changed from `border` to `shadow-[inset_0_0_0_1px]` for card outlines |
| `src/components/AudioStreamItem.tsx` | Changed borders to box-shadows; Fixed master border to use theme accent color; Removed shadow on expanded content that was hiding bottom border |
| `src/App.tsx` | Added `updateSettings` integration for master volume; Added useEffect to sync volume from settings on load |

## Bugs Fixed

1. **App crash on Start** - Index out of bounds panic at `thread.rs:2613` when PipeWire requested >8192 samples. Fixed by using `MAX_BUFFER_SIZE` (48000) in SwitchPlaybackTarget handler.

2. **Audio goes silent on device hotplug** - When unplugging/replugging headphones, `SwitchPlaybackTarget` only handled legacy mode. Fixed by adding per-app mode detection that recreates `mixing_playback_stream` targeting new device by NAME.

3. **White border lines on dark themes** - Subpixel rendering artifacts from 1px CSS borders. Fixed by using box-shadow instead of border.

4. **Master volume not persisting** - Volume was only stored in local state, not saved to settings. Fixed by calling `updateSettings({ master_volume })` and syncing on load.

5. **Master border hardcoded green** - Border color was `rgba(74,222,128,0.5)` regardless of theme. Fixed to use `var(--gecko-accent)`.

## Current State

Linux platform is fully functional with all Phase 1-2 features and Phase 4 polish items complete:
- Per-app EQ, volume, bypass all working
- Settings persistence (including master volume now)
- 7 themes with proper accent colors
- UI borders look clean on all themes
- Audio device hotplug works correctly

## What's Left (from VISION.md)

### Phase 3: Cross-Platform (Major Effort)
1. **Windows: WASAPI Process Loopback** - Capture per-process audio
   - Requires Windows 10 2004+ Process Loopback API
   - `AUDIOCLIENT_PROCESS_LOOPBACK_PARAMS` with target PID
   - IAudioClient3 activation for low-latency

2. **macOS: CoreAudio HAL Plugin** - Significant effort
   - AudioServerPlugIn system extension
   - Shared memory ring buffer IPC
   - Virtual device routing via aggregate device
   - Codesigning/notarization requirements

### Phase 4: Polish (Remaining Items)
1. **FFT Visualization** - Real-time spectrum analyzer
   - Need analysis thread to compute FFT
   - Send results to UI for rendering
   - Consider Web Audio API for frontend or rust FFT crate

2. **System Tray Integration** - Background operation
   - Tauri provides system tray API
   - Minimize to tray, show/hide window
   - Quick access to bypass toggle

3. **Auto-start Option** - Launch on system boot
   - Linux: .desktop file in autostart
   - Windows: Registry or startup folder
   - macOS: LaunchAgent plist

4. **Soft Clipping** - Prevent hard distortion
   - Implement in DSP pipeline after volume/EQ
   - Simple tanh() or polynomial soft clipper

## Implementation Plan for Remaining Work

### FFT Visualization (Estimated: Medium complexity)
1. Add `rustfft` or similar crate to `gecko_dsp`
2. Create FFT analyzer struct with windowing (Hann)
3. In audio callback, copy samples to analysis ring buffer
4. Analysis thread: compute FFT at ~30fps
5. Send magnitude spectrum to frontend via events
6. Create `SpectrumAnalyzer.tsx` component with canvas

### System Tray (Estimated: Low complexity)
1. Add tray icon assets (16x16, 32x32 PNG)
2. Configure Tauri tray in `tauri.conf.json`
3. Add tray menu: Show/Hide, Bypass, Quit
4. Handle window close to minimize to tray
5. Add setting for "minimize to tray on close"

### Auto-start (Estimated: Low complexity)
1. Add setting in UI: "Start with system"
2. Linux: Write .desktop file to `~/.config/autostart/`
3. Use Tauri's `autostart` plugin if available
4. Test on each platform

### Soft Clipping (Estimated: Low complexity)
1. Add `soft_clip()` function to `gecko_dsp`
2. Use tanh-based curve: `x / (1 + |x|)` or similar
3. Apply after master volume, before output
4. Add threshold/knee parameters if desired

## How to Continue

1. Start a new conversation
2. Say: "Continue from session 2025-12-08_ui-polish-and-bugfixes"
3. Reference this document for context and remaining work
4. Pick from remaining items: FFT, System Tray, Auto-start, Soft Clipping, or Cross-Platform

## Commands for Testing

```bash
# Run the app
pnpm tauri dev

# Run tests
cargo test --workspace

# Lint
cargo clippy --workspace
```

---

*Session completed with all immediate bugs fixed and UI polished. Ready for next phase of development.*
