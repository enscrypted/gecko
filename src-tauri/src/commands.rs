//! Tauri Commands - Called from the frontend via invoke()

use crate::{AppState, AudioStreamInfo, BandInfo, DeviceInfo};
use gecko_core::{DeviceType, GeckoSettings, UserPreset, EQ_BANDS};
use gecko_dsp::PRESETS;
use tauri::{AppHandle, State};
use tauri_plugin_autostart::ManagerExt;

/// Initialize the audio engine
#[tauri::command]
pub fn init_engine(state: State<AppState>) -> Result<(), String> {
    use gecko_core::AudioEngine;
    use tracing::{error, info};

    let mut engine_guard = state.engine.lock().map_err(|e| e.to_string())?;

    if engine_guard.is_some() {
        return Ok(()); // Already initialized
    }

    info!("Initializing audio engine");

    match AudioEngine::new() {
        Ok(engine) => {
            // Apply settings from persistence
            if let Ok(settings) = state.settings.lock() {
                info!("Applying persisted settings to engine");
                // NOTE: Don't apply saved master_volume - it syncs from PipeWire sink volume
                // The system retains volume state across app restarts
                let _ = engine.set_bypass(settings.bypassed);
                for (i, gain) in settings.master_eq.iter().enumerate() {
                    let _ = engine.set_band_gain(i, *gain);
                }

                // Apply per-app EQ settings
                for (app_name, gains) in &settings.app_eq {
                    for (i, gain) in gains.iter().enumerate() {
                        // Persisted as app_name, needs to be applied to engine
                        // Engine will cache this in AudioProcessingState for when stream is created
                        let _ = engine.set_stream_band_gain(app_name.clone(), i, *gain);
                    }
                }

                // Apply per-app volume settings
                for (app_name, volume) in &settings.app_volumes {
                    let _ = engine.set_stream_volume(app_name.clone(), *volume);
                }

                // Apply per-app bypass settings
                for app_name in &settings.bypassed_apps {
                    let _ = engine.set_app_bypass(app_name.clone(), true);
                }
            }
            
            *engine_guard = Some(engine);
            Ok(())
        }
        Err(e) => {
            error!("Failed to initialize engine: {}", e);
            Err(e.to_string())
        }
    }
}

/// Start audio processing
#[tauri::command]
pub fn start_engine(state: State<AppState>) -> Result<(), String> {
    let engine_guard = state.engine.lock().map_err(|e| e.to_string())?;

    if let Some(ref engine) = *engine_guard {
        engine.start().map_err(|e| e.to_string())
    } else {
        Err("Engine not initialized".into())
    }
}

/// Stop audio processing
#[tauri::command]
pub fn stop_engine(state: State<AppState>) -> Result<(), String> {
    let engine_guard = state.engine.lock().map_err(|e| e.to_string())?;

    if let Some(ref engine) = *engine_guard {
        engine.stop().map_err(|e| e.to_string())
    } else {
        Err("Engine not initialized".into())
    }
}

/// Get engine running state
#[tauri::command]
pub fn is_engine_running(state: State<AppState>) -> Result<bool, String> {
    let engine_guard = state.engine.lock().map_err(|e| e.to_string())?;

    if let Some(ref engine) = *engine_guard {
        Ok(engine.is_running())
    } else {
        Ok(false)
    }
}

/// Set EQ band gain
#[tauri::command]
pub fn set_band_gain(state: State<AppState>, band: usize, gain_db: f32) -> Result<(), String> {
    let engine_guard = state.engine.lock().map_err(|e| e.to_string())?;

    if let Some(ref engine) = *engine_guard {
        engine.set_band_gain(band, gain_db).map_err(|e| e.to_string())?;
        
        // Persist to settings
        if let Ok(mut settings) = state.settings.lock() {
            if band < settings.master_eq.len() {
                settings.master_eq[band] = gain_db;
                let _ = settings.save();
            }
        }
        Ok(())
    } else {
        Err("Engine not initialized".into())
    }
}

/// Set per-stream EQ band offset (additive to master EQ)
/// stream_id format is "name:pid" - we persist by name for stability
#[tauri::command]
pub fn set_stream_band_gain(state: State<AppState>, stream_id: String, band: usize, gain_db: f32) -> Result<(), String> {
    let engine_guard = state.engine.lock().map_err(|e| e.to_string())?;

    if let Some(ref engine) = *engine_guard {
        engine.set_stream_band_gain(stream_id.clone(), band, gain_db).map_err(|e| e.to_string())?;
        
        // Extract app name from stream_id (format: "name:pid")
        // Persist by app name so settings survive across sessions
        let app_name = stream_id.split(':').next().unwrap_or(&stream_id).to_string();
        
        // Persist to settings
        if let Ok(mut settings) = state.settings.lock() {
            let eq = settings.app_eq.entry(app_name).or_insert_with(|| vec![0.0; 10]);
            if band < eq.len() {
                eq[band] = gain_db;
            } else if band < 10 {
                eq.resize(10, 0.0);
                eq[band] = gain_db;
            }
            // Optimization: Don't save to disk here to avoid I/O blocking during slider moves
            // Frontend will call save_settings() debounced
            // let _ = settings.save();
        }
        Ok(())
    } else {
        Err("Engine not initialized".into())
    }
}

/// Set bypass state for a specific application
///
/// When bypassed, the app's audio passes through without per-app EQ processing.
/// Master EQ (applied after mixing) still affects the audio unless globally bypassed.
#[tauri::command]
pub fn set_app_bypass(state: State<AppState>, app_name: String, bypassed: bool) -> Result<(), String> {
    let engine_guard = state.engine.lock().map_err(|e| e.to_string())?;

    if let Some(ref engine) = *engine_guard {
        engine.set_app_bypass(app_name.clone(), bypassed).map_err(|e| e.to_string())?;

        // Persist to settings
        if let Ok(mut settings) = state.settings.lock() {
            if bypassed {
                settings.bypassed_apps.insert(app_name);
            } else {
                settings.bypassed_apps.remove(&app_name);
            }
            let _ = settings.save();
        }
        Ok(())
    } else {
        Err("Engine not initialized".into())
    }
}

/// Set per-app volume (0.0 - 2.0, where 1.0 is unity gain)
///
/// This volume is applied after per-app EQ and before mixing.
/// It's independent of master volume.
/// stream_id format is "name:pid" - we persist by app name for stability
#[tauri::command]
pub fn set_stream_volume(state: State<AppState>, stream_id: String, volume: f32) -> Result<(), String> {
    let engine_guard = state.engine.lock().map_err(|e| e.to_string())?;

    if let Some(ref engine) = *engine_guard {
        engine.set_stream_volume(stream_id.clone(), volume).map_err(|e| e.to_string())?;

        // Extract app name from stream_id (format: "name:pid")
        // Persist by app name so settings survive across sessions
        let app_name = stream_id.split(':').next().unwrap_or(&stream_id).to_string();

        // Persist to settings
        if let Ok(mut settings) = state.settings.lock() {
            settings.app_volumes.insert(app_name, volume);
            // Optimization: Don't save to disk here
            // let _ = settings.save();
        }
        Ok(())
    } else {
        Err("Engine not initialized".into())
    }
}

/// Set master volume (0.0 - 1.0+)
/// 
/// On Linux, this sets both PipeWire sink volume and internal DSP volume.
/// This syncs with system volume controls bidirectionally.
#[tauri::command]
pub fn set_master_volume(state: State<AppState>, volume: f32) -> Result<(), String> {
    let engine_guard = state.engine.lock().map_err(|e| e.to_string())?;

    if let Some(ref engine) = *engine_guard {
        // On Linux, use sink volume for bidirectional sync with system
        #[cfg(target_os = "linux")]
        {
            engine.set_sink_volume(volume).map_err(|e| e.to_string())?;
        }
        
        // On other platforms, use internal volume only
        #[cfg(not(target_os = "linux"))]
        {
            engine.set_master_volume(volume).map_err(|e| e.to_string())?;
        }
        
        // Update settings (but don't persist master_volume since it's synced with system)
        if let Ok(mut settings) = state.settings.lock() {
            settings.master_volume = volume;
            // Note: We don't save to disk since volume syncs with system
        }
        
        Ok(())
    } else {
        Err("Engine not initialized".into())
    }
}

/// Set internal DSP master volume only (no PipeWire interaction)
/// 
/// Used when syncing from system volume changes to avoid
/// redundant PipeWire calls.
#[tauri::command]
pub fn set_dsp_volume(state: State<AppState>, volume: f32) -> Result<(), String> {
    let engine_guard = state.engine.lock().map_err(|e| e.to_string())?;

    if let Some(ref engine) = *engine_guard {
        engine.set_master_volume(volume).map_err(|e| e.to_string())?;
        
        // Update settings state
        if let Ok(mut settings) = state.settings.lock() {
            settings.master_volume = volume;
        }
        
        Ok(())
    } else {
        Err("Engine not initialized".into())
    }
}

/// Get current PipeWire sink volume (Linux only)
/// 
/// Returns the "Gecko Audio" sink volume as seen by PipeWire/WirePlumber.
/// This syncs with system volume controls.
#[tauri::command]
pub fn get_sink_volume(state: State<AppState>) -> Result<f32, String> {
    let engine_guard = state.engine.lock().map_err(|e| e.to_string())?;

    if let Some(ref engine) = *engine_guard {
        #[cfg(target_os = "linux")]
        {
            engine.get_sink_volume().map_err(|e| e.to_string())
        }
        
        #[cfg(not(target_os = "linux"))]
        {
            // Return settings value on non-Linux
            if let Ok(settings) = state.settings.lock() {
                Ok(settings.master_volume)
            } else {
                Ok(1.0)
            }
        }
    } else {
        // Return settings value if engine not running
        if let Ok(settings) = state.settings.lock() {
            Ok(settings.master_volume)
        } else {
            Ok(1.0)
        }
    }
}

/// Set bypass state
#[tauri::command]
pub fn set_bypass(state: State<AppState>, bypassed: bool) -> Result<(), String> {
    let engine_guard = state.engine.lock().map_err(|e| e.to_string())?;

    if let Some(ref engine) = *engine_guard {
        engine.set_bypass(bypassed).map_err(|e| e.to_string())
    } else {
        Err("Engine not initialized".into())
    }
}

/// Get list of available audio devices
#[tauri::command]
pub fn list_devices(state: State<AppState>) -> Result<Vec<DeviceInfo>, String> {
    let engine_guard = state.engine.lock().map_err(|e| e.to_string())?;

    if let Some(ref engine) = *engine_guard {
        let devices = engine.list_devices().map_err(|e| e.to_string())?;

        Ok(devices
            .into_iter()
            .map(|d| DeviceInfo {
                id: d.id,
                name: d.name,
                is_input: d.device_type == DeviceType::Input,
                is_default: d.is_default,
            })
            .collect())
    } else {
        Err("Engine not initialized".into())
    }
}

/// Get list of audio-producing applications (streams)
///
/// On Linux, this queries PipeWire for Stream/Output/Audio nodes
#[tauri::command]
pub fn list_audio_streams() -> Result<Vec<AudioStreamInfo>, String> {
    #[cfg(target_os = "linux")]
    {
        use gecko_platform::linux::PipeWireBackend;
        use gecko_platform::PlatformBackend;

        // Create a query-only backend connection to query apps
        // This backend will NOT create per-app sinks, preserving the main engine's state
        match PipeWireBackend::new_query_only() {
            Ok(backend) => {
                // Wait for PipeWire to populate state (registry events are async)
                // Poll a few times with delays to ensure we get data
                let mut last_count = 0;
                for _ in 0..5 {
                    std::thread::sleep(std::time::Duration::from_millis(100));
                    if let Ok(apps) = backend.list_applications() {
                        let count = apps.len();
                        // If count stabilized, we probably have all apps
                        if count > 0 && count == last_count {
                            break;
                        }
                        last_count = count;
                    }
                }

                match backend.list_applications() {
                    Ok(apps) => Ok(apps
                        .into_iter()
                        .map(|app| AudioStreamInfo {
                            id: format!("{}:{}", app.pid, app.name),
                            name: app.name,
                            pid: app.pid,
                            is_active: app.is_active,
                            is_routed_to_gecko: false, // TODO: check actual routing
                        })
                        .collect()),
                    Err(e) => Err(format!("Failed to list applications: {}", e)),
                }
            }
            Err(e) => Err(format!("Failed to connect to PipeWire: {}", e)),
        }
    }

    #[cfg(not(target_os = "linux"))]
    {
        // Return empty list on non-Linux platforms for now
        Ok(Vec::new())
    }
}

/// Get EQ band information
#[tauri::command]
pub fn get_eq_bands() -> Vec<BandInfo> {
    EQ_BANDS
        .iter()
        .enumerate()
        .map(|(i, &freq)| BandInfo {
            index: i,
            frequency: freq,
            gain_db: 0.0,
            enabled: true,
        })
        .collect()
}

/// Poll for audio events (levels, errors)
#[tauri::command]
pub fn poll_events(state: State<AppState>) -> Result<Vec<String>, String> {
    let engine_guard = state.engine.lock().map_err(|e| e.to_string())?;

    if let Some(ref engine) = *engine_guard {
        let mut events = Vec::new();

        while let Some(event) = engine.poll_event() {
            events.push(serde_json::to_string(&event).unwrap_or_default());
        }

        Ok(events)
    } else {
        Ok(Vec::new())
    }
}

/// Get current settings
#[tauri::command]
pub fn get_settings(state: State<AppState>) -> Result<GeckoSettings, String> {
    let settings = state.settings.lock().map_err(|e| e.to_string())?;
    Ok(settings.clone())
}

/// Save settings
#[tauri::command]
pub fn save_settings(state: State<AppState>, settings: GeckoSettings) -> Result<(), String> {
    let mut current_settings = state.settings.lock().map_err(|e| e.to_string())?;
    *current_settings = settings.clone();
    
    // Persist to disk
    current_settings.save().map_err(|e| e.to_string())?;
    
    // Apply to engine if running
    let engine_guard = state.engine.lock().map_err(|e| e.to_string())?;
    if let Some(ref engine) = *engine_guard {
        // NOTE: Don't apply master_volume here - it syncs bidirectionally with PipeWire
        
        // Apply bypass
        let _ = engine.set_bypass(settings.bypassed);
        
        // Apply EQ
        for (i, gain) in settings.master_eq.iter().enumerate() {
            let _ = engine.set_band_gain(i, *gain);
        }
    }
    
    Ok(())
}

/// Get available presets (built-in + user)
#[tauri::command]
pub fn get_presets(state: State<AppState>) -> Result<Vec<(String, Vec<f32>, bool)>, String> { // (Name, Gains, IsUser)
    let settings = state.settings.lock().map_err(|e| e.to_string())?;
    
    let mut result = Vec::new();
    
    // Add built-in presets
    for (name, gains) in PRESETS {
        result.push((name.to_string(), gains.to_vec(), false));
    }
    
    // Add user presets
    for preset in &settings.user_presets {
        result.push((preset.name.clone(), preset.gains.to_vec(), true));
    }
    
    Ok(result)
}

/// Save current EQ as user preset
#[tauri::command]
pub fn save_preset(state: State<AppState>, name: String, gains: Vec<f32>) -> Result<(), String> {
    if gains.len() != 10 {
        return Err("Invalid gain count".into());
    }
    
    let mut settings = state.settings.lock().map_err(|e| e.to_string())?;
    
    // Check if name conflicts with built-in
    if PRESETS.iter().any(|(n, _)| *n == name) {
        return Err("Cannot overwrite built-in preset".into());
    }
    
    let mut gains_arr = [0.0; 10];
    gains_arr.copy_from_slice(&gains);
    
    let preset = UserPreset {
        name: name.clone(),
        gains: gains_arr,
        created_at: chrono::Utc::now(),
    };
    
    // Update or append
    if let Some(existing) = settings.user_presets.iter_mut().find(|p| p.name == name) {
        *existing = preset;
    } else {
        settings.user_presets.push(preset);
    }
    
    settings.save().map_err(|e| e.to_string())?;
    Ok(())
}

/// Delete user preset
#[tauri::command]
pub fn delete_preset(state: State<AppState>, name: String) -> Result<(), String> {
    let mut settings = state.settings.lock().map_err(|e| e.to_string())?;
    
    let len_before = settings.user_presets.len();
    settings.user_presets.retain(|p| p.name != name);
    
    if settings.user_presets.len() == len_before {
        return Err("Preset not found or is built-in".into());
    }
    
    settings.save().map_err(|e| e.to_string())?;
    Ok(())
}

/// Apply a preset (convenience to set all bands and update settings)
#[tauri::command]
pub fn apply_preset(state: State<AppState>, name: String, gains: Vec<f32>) -> Result<(), String> {
    if gains.len() != 10 {
        return Err("Invalid gain count".into());
    }
    
    let mut settings = state.settings.lock().map_err(|e| e.to_string())?;
    
    // Update active preset tracking
    settings.active_preset = Some(name);
    
    // Update EQ values in settings
    for (i, &gain) in gains.iter().enumerate() {
        if i < 10 {
            settings.master_eq[i] = gain;
        }
    }
    
    // Save settings
    settings.save().map_err(|e| e.to_string())?;
    
    // Apply to engine
    let engine_guard = state.engine.lock().map_err(|e| e.to_string())?;
    if let Some(ref engine) = *engine_guard {
        for (i, &gain) in gains.iter().enumerate() {
            let _ = engine.set_band_gain(i, gain);
        }
    }
    
    Ok(())
}

/// Get platform capabilities
#[tauri::command]
pub fn get_platform_info() -> serde_json::Value {
    serde_json::json!({
        "platform": std::env::consts::OS,
        "supports_virtual_devices": gecko_platform::supports_virtual_devices(),
        "supports_per_app_capture": gecko_platform::supports_per_app_capture(),
    })
}

/// Check if auto-start is enabled
#[tauri::command]
pub fn get_autostart(app: AppHandle) -> Result<bool, String> {
    app.autolaunch()
        .is_enabled()
        .map_err(|e| e.to_string())
}

/// Enable or disable auto-start
#[tauri::command]
pub fn set_autostart(app: AppHandle, enabled: bool) -> Result<(), String> {
    let autolaunch = app.autolaunch();
    if enabled {
        autolaunch.enable().map_err(|e| e.to_string())
    } else {
        autolaunch.disable().map_err(|e| e.to_string())
    }
}

/// Enable or disable soft clipping (limiter)
#[tauri::command]
pub fn set_soft_clip(state: State<AppState>, enabled: bool) -> Result<(), String> {
    let engine_guard = state.engine.lock().map_err(|e| e.to_string())?;

    if let Some(ref engine) = *engine_guard {
        engine.set_soft_clip_enabled(enabled).map_err(|e| e.to_string())?;
    }

    // Persist to settings
    if let Ok(mut settings) = state.settings.lock() {
        settings.ui_settings.soft_clip_enabled = enabled;
        let _ = settings.save();
    }

    Ok(())
}
