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
    // Initialize tracing for logging
    tracing_subscriber::fmt()
        .with_env_filter("gecko=debug,tauri=info")
        .init();

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
}
