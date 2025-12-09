# Session: PipeWire Volume Sync Implementation

**Date**: 2025-12-09
**Status**: Completed

## Task Summary
Implemented bidirectional volume synchronization between Gecko's master volume and the system's PipeWire volume for the "Gecko Audio" sink. This allows users to control Gecko's audio processing via system volume keys (volume up/down/mute).

## Key Decisions

- **Polling approach**: Use 2-second polling interval with `wpctl get-volume` to detect system volume changes. This avoids process spam while providing reasonable responsiveness.
- **DSP volume in backend**: `get_sink_volume()` directly sets DSP master volume when polling, rather than relying on frontend to call a separate command. This ensures both mute and unmute work correctly.
- **Sink identification**: Filter out per-app sinks (those containing ` - ` in name) to find exact "Gecko Audio" sink when parsing `wpctl status` output.
- **No volume persistence**: Master volume is no longer persisted by Gecko - it syncs from system PipeWire volume on each poll.
- **Removed VolumeToast usage**: The VolumeToast component is no longer used in App.tsx since volume feedback now comes from system OSD. (Note: The component file was fully removed in a subsequent cleanup session.)

## Files Modified

| File | Change |
|------|--------|
| `crates/gecko_core/src/engine.rs` | Added `get_sink_volume()` and `set_sink_volume()` methods with wpctl integration |
| `src-tauri/src/commands.rs` | Added `get_sink_volume` command, modified `set_master_volume` to use sink volume on Linux, added `set_dsp_volume` command |
| `src-tauri/src/lib.rs` | Registered new Tauri commands |
| `src/App.tsx` | Added volume polling useEffect, removed VolumeToast logic |
| `src-tauri/tauri.conf.json` | Removed overlay window configuration |

## Current State

Volume synchronization is working:
- **Volume up/down keys**: Change audio level (within 2 second UI update delay, audio updates immediately)
- **Mute key**: Mutes audio output
- **Unmute (mute key again)**: Restores audio output to previous volume
- **Gecko slider**: Updates system volume bidirectionally

## Known Limitations

- 2-second polling delay for UI update (audio responds immediately)
- Requires PipeWire and wpctl to be installed
- Per-app sinks (Gecko-Firefox, etc.) have their own volumes separate from master

## Next Steps

1. Consider using PipeWire native events instead of wpctl polling for lower latency
2. Add error handling/user notification when PipeWire/wpctl not available
3. Test on systems with different PipeWire configurations

## How to Continue

To resume work on volume-related features:
1. Start a new conversation
2. Say: "Continue from session 2025-12-09_pipewire-volume-sync"
3. Key files: `crates/gecko_core/src/engine.rs` (get_sink_volume, set_sink_volume), `src/App.tsx` (polling logic)
