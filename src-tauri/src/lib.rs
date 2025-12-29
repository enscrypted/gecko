//! Gecko UI Library - Tauri Commands and State
//!
//! This module exposes the audio engine to the frontend via Tauri commands.

mod commands;

use std::sync::Mutex;

use gecko_core::{AudioEngine, GeckoSettings};
use serde::{Deserialize, Serialize};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::menu::{MenuBuilder, MenuItemBuilder};
use tauri::{AppHandle, Manager, RunEvent, WindowEvent};
use tauri_plugin_autostart::MacosLauncher;
use tracing::info;

/// Initialize logging with file output for macOS debugging
/// Logs are written to ~/gecko-debug.log which can be tailed while app runs
fn init_logging() {
    use tracing_subscriber::prelude::*;
    use tracing_subscriber::fmt;
    use tracing_subscriber::EnvFilter;

    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("gecko=debug,tauri=info"));

    // On macOS, also write to a log file since running via `open` loses stdout
    #[cfg(target_os = "macos")]
    {
        use std::fs::OpenOptions;
        let log_path = std::env::var("HOME")
            .map(|h| std::path::PathBuf::from(h).join("gecko-debug.log"))
            .unwrap_or_else(|_| std::path::PathBuf::from("/tmp/gecko-debug.log"));

        // Try to create file logger
        if let Ok(file) = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)  // Clear on each run
            .open(&log_path)
        {
            let file_layer = fmt::layer()
                .with_writer(Mutex::new(file))
                .with_ansi(false)  // No color codes in file
                .with_target(true)
                .with_line_number(true);

            let stdout_layer = fmt::layer()
                .with_target(true);

            tracing_subscriber::registry()
                .with(filter)
                .with(file_layer)
                .with(stdout_layer)
                .init();

            eprintln!("Gecko: Logging to {} - use `tail -f {}` to view", log_path.display(), log_path.display());
            return;
        }
    }

    // Fallback: stdout only
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .init();
}

/// Application state managed by Tauri
pub struct AppState {
    pub engine: Mutex<Option<AudioEngine>>,
    pub settings: Mutex<GeckoSettings>,
}

impl AppState {
    pub fn new() -> Self {
        Self {
            engine: Mutex::new(None),
            settings: Mutex::new(GeckoSettings::load()),
        }
    }
}

impl Default for AppState {
    fn default() -> Self {
        Self::new()
    }
}

/// Clean up audio engine before exit
/// This ensures PipeWire links are properly removed and audio routing is restored
fn cleanup_audio_engine(app: &AppHandle) {
    info!("Cleaning up audio engine before exit...");
    let state = app.state::<AppState>();
    if let Ok(mut engine_guard) = state.engine.lock() {
        if let Some(ref engine) = *engine_guard {
            // Stop the engine to clean up PipeWire resources
            if let Err(e) = engine.stop() {
                tracing::error!("Error stopping engine during cleanup: {}", e);
            } else {
                info!("Audio engine stopped successfully");
            }
        }
        // Drop the engine to ensure full cleanup
        *engine_guard = None;
    }
    info!("Audio cleanup complete");
}

/// EQ band info for the frontend
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BandInfo {
    pub index: usize,
    pub frequency: f32,
    pub gain_db: f32,
    pub enabled: bool,
}

/// Device info for the frontend
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceInfo {
    pub id: String,
    pub name: String,
    pub is_input: bool,
    pub is_default: bool,
}

/// Audio stream (application) info for the frontend
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AudioStreamInfo {
    pub id: String,
    pub name: String,
    pub pid: u32,
    pub is_active: bool,
    pub is_routed_to_gecko: bool,
    /// Whether this app can be captured via Process Tap API (macOS only)
    /// Apps like Safari and system apps cannot be tapped due to Apple sandboxing
    #[serde(default = "default_true")]
    pub is_tappable: bool,
    /// Reason why the app cannot be tapped (if is_tappable is false)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub untappable_reason: Option<String>,
}

fn default_true() -> bool {
    true
}

/// Audio levels for metering
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AudioLevels {
    pub left: f32,
    pub right: f32,
}

/// Engine state for the frontend
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EngineState {
    pub is_running: bool,
    pub is_bypassed: bool,
    pub master_volume: f32,
}

// ============================================================================
// Tauri App Setup
// ============================================================================

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // Initialize tracing for logging (writes to ~/gecko-debug.log on macOS)
    init_logging();

    info!("Starting Gecko Audio Application");

    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_autostart::init(MacosLauncher::LaunchAgent, Some(vec!["--minimized"])))
        .manage(AppState::new())
        .setup(|app| {
            // Create tray menu items
            let show_item = MenuItemBuilder::with_id("show", "Show Gecko").build(app)?;
            let hide_item = MenuItemBuilder::with_id("hide", "Hide").build(app)?;
            let quit_item = MenuItemBuilder::with_id("quit", "Quit").build(app)?;

            // Build menu
            let menu = MenuBuilder::new(app)
                .item(&show_item)
                .item(&hide_item)
                .separator()
                .item(&quit_item)
                .build()?;

            // Create tray icon
            let _tray = TrayIconBuilder::new()
                .icon(app.default_window_icon().cloned().unwrap())
                .menu(&menu)
                .tooltip("Gecko Audio")
                .on_menu_event(|app, event| {
                    match event.id().as_ref() {
                        "show" => {
                            if let Some(window) = app.get_webview_window("main") {
                                let _ = window.show();
                                let _ = window.set_focus();
                            }
                        }
                        "hide" => {
                            if let Some(window) = app.get_webview_window("main") {
                                let _ = window.hide();
                            }
                        }
                        "quit" => {
                            // Clean up audio before exiting
                            cleanup_audio_engine(app);
                            app.exit(0);
                        }
                        _ => {}
                    }
                })
                .on_tray_icon_event(|tray, event| {
                    // Double-click to show window
                    if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = event {
                        if let Some(window) = tray.app_handle().get_webview_window("main") {
                            let _ = window.show();
                            let _ = window.set_focus();
                        }
                    }
                })
                .build(app)?;

            // Check if start_minimized is enabled and hide window
            let state = app.state::<AppState>();
            if let Ok(settings) = state.settings.lock() {
                if settings.ui_settings.start_minimized {
                    if let Some(window) = app.get_webview_window("main") {
                        info!("Start minimized enabled, hiding window");
                        let _ = window.hide();
                    }
                }
            }

            Ok(())
        })
        .on_window_event(|window, event| {
            // Handle close request -> minimize to tray instead of quitting
            if let WindowEvent::CloseRequested { api, .. } = event {
                // Hide the window instead of closing
                let _ = window.hide();
                // Prevent the default close behavior
                api.prevent_close();
            }
        })
        .invoke_handler(tauri::generate_handler![
            commands::init_engine,
            commands::start_engine,
            commands::stop_engine,
            commands::is_engine_running,
            commands::set_band_gain,
            commands::set_stream_band_gain,
            commands::set_app_bypass,
            commands::set_stream_volume,
            commands::set_master_volume,
            commands::set_dsp_volume,
            commands::get_sink_volume,
            commands::set_bypass,
            commands::list_devices,
            commands::list_audio_streams,
            commands::get_eq_bands,
            commands::poll_events,
            commands::get_platform_info,
            commands::get_settings,
            commands::save_settings,
            commands::get_presets,
            commands::save_preset,
            commands::delete_preset,
            commands::apply_preset,
            commands::get_autostart,
            commands::set_autostart,
            commands::set_soft_clip,
            // macOS-specific commands
            commands::get_macos_audio_info,
            commands::check_screen_recording_permission,
            commands::request_screen_recording_permission,
            commands::start_app_capture,
            commands::stop_app_capture,
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app_handle, event| {
            match event {
                // Handle app exit events - clean up audio engine
                RunEvent::Exit => {
                    info!("Application exit event received");
                    cleanup_audio_engine(app_handle);
                }
                // Handle exit request (from window close, system shutdown, etc.)
                RunEvent::ExitRequested { api, code, .. } => {
                    info!("Exit requested (code: {:?})", code);
                    // Don't prevent exit - let cleanup happen
                    let _ = api;
                    cleanup_audio_engine(app_handle);
                }
                _ => {}
            }
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_app_state_creation() {
        let state = AppState::new();
        let guard = state.engine.lock().unwrap();
        assert!(guard.is_none());
    }

    #[test]
    fn test_eq_bands_info() {
        let bands = commands::get_eq_bands();
        assert_eq!(bands.len(), 10);
        assert_eq!(bands[0].frequency, 31.0);
        assert_eq!(bands[9].frequency, 16000.0);
    }

    #[test]
    fn test_platform_info() {
        let info = commands::get_platform_info();
        assert!(info.get("platform").is_some());
    }

    #[test]
    fn test_eq_bands_frequencies_complete() {
        let bands = commands::get_eq_bands();
        // Verify all 10-band EQ frequencies match spec
        let expected: Vec<f32> = vec![31.0, 62.0, 125.0, 250.0, 500.0, 1000.0, 2000.0, 4000.0, 8000.0, 16000.0];
        for (i, band) in bands.iter().enumerate() {
            assert_eq!(band.index, i);
            assert_eq!(band.frequency, expected[i]);
            assert_eq!(band.gain_db, 0.0); // Default flat
            assert!(band.enabled);
        }
    }

    #[test]
    fn test_app_state_settings_load() {
        let state = AppState::new();
        let settings = state.settings.lock().unwrap();
        // Settings should load with defaults
        assert!(settings.master_volume >= 0.0);
        assert!(settings.master_volume <= 1.0);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn test_macos_audio_info() {
        let info = commands::get_macos_audio_info();
        // Should always have macos_version and process_tap_available fields
        assert!(info.get("macos_version").is_some());
        assert!(info.get("process_tap_available").is_some());
        assert!(info.get("per_app_audio_available").is_some());
        assert!(info.get("limitations").is_some());
    }

    #[test]
    fn test_platform_info_contains_required_fields() {
        let info = commands::get_platform_info();
        // Platform info should contain platform and capability flags
        assert!(info.get("platform").is_some());
        assert!(info.get("supports_virtual_devices").is_some());
        assert!(info.get("supports_per_app_capture").is_some());
    }

    #[test]
    fn test_band_info_serialization() {
        let band = BandInfo {
            index: 0,
            frequency: 31.0,
            gain_db: 3.5,
            enabled: true,
        };
        let json = serde_json::to_string(&band).unwrap();
        assert!(json.contains("31.0"));
        assert!(json.contains("3.5"));
    }

    #[test]
    fn test_device_info_serialization() {
        let device = DeviceInfo {
            id: "test-device".to_string(),
            name: "Test Device".to_string(),
            is_input: false,
            is_default: true,
        };
        let json = serde_json::to_string(&device).unwrap();
        assert!(json.contains("test-device"));
        assert!(json.contains("Test Device"));
    }

    #[test]
    fn test_audio_stream_info_serialization() {
        let stream = AudioStreamInfo {
            id: "stream-1".to_string(),
            name: "Firefox".to_string(),
            pid: 1234,
            is_active: true,
            is_routed_to_gecko: true,
            is_tappable: true,
            untappable_reason: None,
        };
        let json = serde_json::to_string(&stream).unwrap();
        assert!(json.contains("Firefox"));
        assert!(json.contains("1234"));
        assert!(json.contains("is_tappable"));
    }

    #[test]
    fn test_audio_stream_info_untappable() {
        let stream = AudioStreamInfo {
            id: "Safari:123".to_string(),
            name: "Safari".to_string(),
            pid: 123,
            is_active: true,
            is_routed_to_gecko: false,
            is_tappable: false,
            untappable_reason: Some("macOS security prevents per-app capture. Only Master EQ affects this app.".to_string()),
        };
        let json = serde_json::to_string(&stream).unwrap();
        assert!(json.contains("Safari"));
        assert!(json.contains("untappable_reason"));
        assert!(json.contains("macOS security"));
        assert!(!stream.is_tappable);
    }

    #[test]
    fn test_engine_state_serialization() {
        let state = EngineState {
            is_running: true,
            is_bypassed: false,
            master_volume: 0.8,
        };
        let json = serde_json::to_string(&state).unwrap();
        assert!(json.contains("0.8"));
    }

    #[test]
    fn test_audio_levels_serialization() {
        let levels = AudioLevels {
            left: 0.5,
            right: 0.7,
        };
        let json = serde_json::to_string(&levels).unwrap();
        let parsed: AudioLevels = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.left, 0.5);
        assert_eq!(parsed.right, 0.7);
    }
}
