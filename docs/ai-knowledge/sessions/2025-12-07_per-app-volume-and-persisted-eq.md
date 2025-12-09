# Session: Per-App Volume & Persisted EQ Fixes

**Date**: 2025-12-07
**Status**: Complete

## Task Summary

Two issues were being addressed after implementing per-app volume sliders:
1. **Per-app volume sliders not affecting audio output** - FIXED
2. **Persisted per-app EQ not applied on startup** - FIXED

## Context

This continues TRUE per-app EQ architecture work. Each app now has:
- Its own PipeWire virtual sink
- Independent capture stream with separate EQ instance
- Per-app volume control (0.0-2.0 range)

## Key Decisions

- **Stream ID format**: Frontend uses "name:pid" format (e.g., "Firefox:1234")
- **Backend lookup**: Uses just app name (e.g., "Firefox") for HashMap keys
- **Volume range**: 0.0-2.0 where 1.0 is unity gain (100%)
- **Persistence**: Both EQ and volume are persisted by app name in settings.json

## Files Modified

| File | Change |
|------|--------|
| `crates/gecko_core/src/engine.rs:572-583` | Fixed: Extract app name from stream_id before calling backend |
| `src/components/StreamList.tsx:82-119` | Added: Apply persisted EQ and volume to backend when new streams discovered |
| `src/components/StreamList.tsx:30` | Added: `streamVolumes` state for tracking per-app volumes |

## Current State

### Fixed (needs testing):
1. **Volume slider fix** in `engine.rs`:
   - Changed from `backend.set_app_volume(&stream_id, volume)`
   - To extracting app name first: `let app_name = stream_id.split(':').next().unwrap_or(&stream_id)`
   - Then `backend.set_app_volume(app_name, volume)`

2. **Persisted EQ on startup** in `StreamList.tsx`:
   - When a new stream is discovered with saved EQ values, now sends them to backend immediately
   - Also sends persisted volume if different from default 1.0

3. **Volume state wiring** in `StreamList.tsx` *(completed 2025-12-07)*:
   - Added separate `setStreamVolumes` initialization when streams discovered
   - Added `handleStreamVolumeChange` callback with settings persistence
   - Passed `volume` and `onVolumeChange` props to `AudioStreamItem`

4. **TypeScript types** in `SettingsContext.tsx` *(completed 2025-12-07)*:
   - Added `app_volumes: { [key: string]: number }` to `GeckoSettings` interface
   - Added `app_volumes: {}` to `defaultSettings`

## Code Changes Detail

### engine.rs fix (line 572-583):
```rust
Command::SetStreamVolume { stream_id, volume } => {
    // Extract app name from stream_id (format: "name:pid")
    // Backend lookup uses just the app name
    let app_name = stream_id.split(':').next().unwrap_or(&stream_id);
    debug!("Set app '{}' volume to {:.2} (stream_id: {})", app_name, volume, stream_id);

    #[cfg(target_os = "linux")]
    if let Some(ref backend) = linux_backend {
        backend.set_app_volume(app_name, volume);
    }
}
```

### StreamList.tsx - Volume initialization (added 2025-12-07):
```typescript
// Initialize volumes for new streams - separate from streamGains to keep logic clear
setStreamVolumes((prev) => {
  const newStreamVolumes = { ...prev };
  let hasChanges = false;
  result.forEach((stream) => {
    if (newStreamVolumes[stream.id] === undefined) {
      hasChanges = true;
      const appName = stream.name;
      const persistedVolume = settings?.app_volumes?.[appName] ?? 1.0;
      newStreamVolumes[stream.id] = persistedVolume;

      // Apply persisted volume to backend if different from default
      if (persistedVolume !== 1.0) {
        invoke("set_stream_volume", { streamId: stream.id, volume: persistedVolume }).catch(
          (e) => console.error("Failed to apply persisted volume:", e)
        );
      }
    }
  });
  return hasChanges ? newStreamVolumes : prev;
});
```

### StreamList.tsx - Volume change handler (added 2025-12-07):
```typescript
const handleStreamVolumeChange = useCallback(
  (streamId: string, volume: number) => {
    setStreamVolumes((prev) => ({
      ...prev,
      [streamId]: volume,
    }));

    // Persist volume to settings by app name
    const stream = streams.find((s) => s.id === streamId);
    if (stream && settings) {
      const appName = getAppName(stream);
      const newAppVolumes = { ...settings.app_volumes, [appName]: volume };
      updateSettings({ ...settings, app_volumes: newAppVolumes });
    }
  },
  [streams, settings, updateSettings]
);
```

## Verification

- **TypeScript build**: Passed (`pnpm build`)
- **Rust tests**: 58 tests passed (`cargo test --workspace`)
- **Manual testing**: Run `pnpm tauri dev` to verify:
  - Volume slider affects audio output
  - Persisted EQ/volume applied on startup
  - Check `~/.config/gecko/gecko/settings.json` for `app_volumes` entries

## Related Files

- Settings struct: `crates/gecko_core/src/settings.rs` (has `app_volumes: HashMap<String, f32>`)
- Tauri command: `src-tauri/src/commands.rs` (`set_stream_volume`)
- Backend: `crates/gecko_platform/src/linux/thread.rs` (has `PwCommand::SetAppVolume`)
- TypeScript types: `src/contexts/SettingsContext.tsx`

