# Session Handoff: Feature Roadmap

**Date**: 2024-12-07
**Status**: Planning complete, ready for implementation
**Priority**: High - Core UX features

---

## Context

Gecko now has working audio streaming with 10-band EQ and volume control. The next phase adds persistence, presets, and a settings page.

### Current State
- Master volume slider works
- 10-band EQ works (atomic counter mechanism for real-time updates)
- Device hotplug works
- 69 tests passing, clippy clean
- KB documentation up to date

---

## Feature 1: Persist DSP & Volume Settings

### Goal
Save all settings on close, restore on open. Settings survive app restart.

### Implementation Plan

**Config Structure** (`crates/gecko_core/src/config.rs`):
```rust
#[derive(Serialize, Deserialize)]
pub struct GeckoSettings {
    pub master_volume: f32,
    pub master_eq: [f32; 10],  // 10-band gains
    pub bypassed: bool,
    pub active_preset: Option<String>,
    pub user_presets: Vec<UserPreset>,
    pub ui_settings: UiSettings,
}
```

**Storage Location**:
- Linux: `~/.config/gecko/settings.json`
- Windows: `%APPDATA%\gecko\settings.json`
- macOS: `~/Library/Application Support/gecko/settings.json`

**Implementation**:
1. Use `directories` crate for cross-platform config path
2. Load settings in `AudioEngine::new()` if file exists
3. Save settings on:
   - Every EQ/volume change (debounced, 1s delay)
   - App close (Tauri `on_window_close` hook)
4. Add Tauri commands: `load_settings`, `save_settings`

### Files to Create/Modify
- `crates/gecko_core/src/settings.rs` - New file for settings types
- `crates/gecko_core/src/config.rs` - Add settings to EngineConfig
- `src-tauri/src/commands.rs` - Add load/save commands
- `src-tauri/src/main.rs` - Add on_close hook
- `Cargo.toml` - Add `directories` dependency

---

## Feature 2: EQ Presets

### Goal
Built-in presets (Flat, Bass Boost, etc.) + user-created presets.

**Also**: Rename current "Volume" label to "Master Volume" in `Controls.tsx`.

### Implementation Plan

**Built-in Presets** (hardcoded):
```rust
pub const PRESETS: &[(&str, [f32; 10])] = &[
    ("Flat", [0.0; 10]),
    ("Bass Boost", [6.0, 5.0, 3.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0]),
    ("Treble Boost", [0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 3.0, 5.0, 6.0, 6.0]),
    ("Vocal Clarity", [-2.0, -1.0, 0.0, 2.0, 4.0, 4.0, 3.0, 2.0, 1.0, 0.0]),
    ("Bass Reduce", [-6.0, -4.0, -2.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0]),
    ("Loudness", [4.0, 3.0, 0.0, -1.0, -1.0, 0.0, 1.0, 2.0, 3.0, 4.0]),
];
```

**User Presets** (saved to settings.json):
```rust
pub struct UserPreset {
    pub name: String,
    pub gains: [f32; 10],
    pub created_at: DateTime<Utc>,
}
```

**UI Component** (`src/components/PresetSelector.tsx`):
```
┌─────────────────────────────┐
│ Preset: [Flat         ▼]   │  <- Dropdown/select
│                             │
│ [Save Current] [Delete]     │  <- Only for user presets
└─────────────────────────────┘
```

**Interaction**:
1. Clicking preset applies all 10 band gains instantly
2. "Save Current" opens modal to name preset
3. User presets appear below built-in presets in dropdown
4. Deleting only works for user presets

### Tauri Commands
- `get_presets` → Returns built-in + user presets
- `apply_preset { name: String }` → Applies preset gains
- `save_preset { name: String }` → Saves current EQ as user preset
- `delete_preset { name: String }` → Deletes user preset

### Files to Create/Modify
- `crates/gecko_dsp/src/presets.rs` - New file for preset definitions
- `src/components/PresetSelector.tsx` - New component
- `src/components/Equalizer.tsx` - Add preset selector above sliders
- `src-tauri/src/commands.rs` - Add preset commands

---

## Feature 3: Settings Page

### Goal
Basic settings page for user preferences.

### Proposed Settings

| Setting | Type | Default | Description |
|---------|------|---------|-------------|
| EQ Band Count | enum | 10 | Show 5 or 10 EQ bands |
| Auto-start | bool | false | Start Gecko on system boot |
| Start minimized | bool | false | Start in system tray |
| Default output device | string | "System Default" | Preferred output device |
| Save settings interval | number | 1000ms | Debounce for auto-save |
| Theme | enum | "Dark" | Dark/Light/System |
| Show level meters | bool | true | Show audio level visualization |
| Confirm before closing | bool | false | Ask before closing if audio is playing |

### 5-Band vs 10-Band Mode

When 5-band mode is selected:
- Combine adjacent bands visually (31+62, 125+250, 500+1k, 2k+4k, 8k+16k)
- Backend still uses 10 bands - UI just shows averaged values
- Changing one 5-band slider updates both underlying bands equally

### UI Layout

```
┌─────────────────────────────────────────┐
│ Settings                            [X] │
├─────────────────────────────────────────┤
│                                         │
│ Display                                 │
│ ├─ EQ Bands:        [5] [10]           │
│ ├─ Theme:           [Dark ▼]            │
│ └─ Show level meters: [✓]              │
│                                         │
│ Behavior                                │
│ ├─ Auto-start with system: [ ]         │
│ ├─ Start minimized:        [ ]         │
│ └─ Confirm before closing: [ ]         │
│                                         │
│ Audio                                   │
│ └─ Default output: [System Default ▼]  │
│                                         │
│            [Reset to Defaults]          │
└─────────────────────────────────────────┘
```

### Implementation
1. Add settings icon/button to main UI header
2. Settings opens as modal or slide-out panel
3. Changes apply immediately (no save button needed)
4. Settings stored in same `settings.json` as DSP settings

### Files to Create/Modify
- `src/components/Settings.tsx` - New component
- `src/components/ui/Modal.tsx` - If not exists
- `src/App.tsx` - Add settings button and modal state
- `crates/gecko_core/src/settings.rs` - Add UI settings

---

## Implementation Order

### Phase 1: Foundation (Persistence + Presets)
1. Rename "Volume" to "Master Volume" in `Controls.tsx`
2. Create `settings.rs` with types
3. Implement load/save settings
4. Add built-in presets
5. Add PresetSelector component
6. Wire up preset application

### Phase 2: Settings Page
1. Create Settings component
2. Implement 5-band mode (frontend only)
3. Add remaining settings
4. Persist UI preferences

---

## Test Coverage Needed

- [ ] Settings serialization/deserialization
- [ ] Preset application (all bands update)
- [ ] 5-band → 10-band mapping
- [ ] Settings file missing/corrupted handling
- [ ] User preset CRUD operations

---

## Dependencies to Add

```toml
# Cargo.toml
directories = "5.0"  # Cross-platform config paths
chrono = { version = "0.4", features = ["serde"] }  # For preset timestamps
```

---

## Notes

- 5-band mode is purely a UI concern - backend always processes 10 bands.
- Settings should debounce to avoid excessive disk writes during slider drags.
- Consider adding import/export for user presets (JSON file).
