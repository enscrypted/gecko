//! Persistent Settings Management
//!
//! Handles saving/loading application state to disk.
//!
//! # Storage Locations
//! - Linux: `~/.config/gecko/settings.json`
//! - Windows: `%APPDATA%\gecko\settings.json`
//! - macOS: `~/Library/Application Support/gecko/settings.json`

use std::fs;
use std::path::PathBuf;

use chrono::{DateTime, Utc};
use directories::ProjectDirs;
use serde::{Deserialize, Serialize};
use tracing::{error, info};

/// User-defined EQ preset
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserPreset {
    pub name: String,
    pub gains: [f32; 10],
    pub created_at: DateTime<Utc>,
}

/// UI-specific settings
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UiSettings {
    pub theme: String, // "Dark", "Light", "System"
    pub show_level_meters: bool,
    pub start_minimized: bool,
    pub eq_bands_ui: u32, // 5 or 10
    /// Enable soft clipping (limiter) to prevent harsh digital distortion
    #[serde(default = "default_soft_clip")]
    pub soft_clip_enabled: bool,
}

fn default_soft_clip() -> bool {
    true // Enabled by default for better audio quality
}

impl Default for UiSettings {
    fn default() -> Self {
        Self {
            theme: "Dark".to_string(),
            show_level_meters: true,
            start_minimized: false,
            eq_bands_ui: 10,
            soft_clip_enabled: true,
        }
    }
}

/// Root settings structure
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GeckoSettings {
    pub master_volume: f32,
    pub master_eq: [f32; 10],
    /// Per-app EQ settings (keyed by app name for stability across sessions)
    #[serde(default)]
    pub app_eq: std::collections::HashMap<String, Vec<f32>>,
    /// Global bypass state (bypasses ALL processing)
    pub bypassed: bool,
    /// Set of apps that have per-app bypass enabled (EQ bypassed for these apps only)
    #[serde(default)]
    pub bypassed_apps: std::collections::HashSet<String>,
    /// Set of apps hidden from the UI (still processed, just not shown)
    #[serde(default)]
    pub hidden_apps: std::collections::HashSet<String>,
    /// Per-app volume settings (keyed by app name, 0.0-2.0, default 1.0)
    #[serde(default)]
    pub app_volumes: std::collections::HashMap<String, f32>,
    pub active_preset: Option<String>,
    pub user_presets: Vec<UserPreset>,
    pub ui_settings: UiSettings,
}

impl Default for GeckoSettings {
    fn default() -> Self {
        Self {
            master_volume: 1.0,
            master_eq: [0.0; 10],
            app_eq: std::collections::HashMap::new(),
            bypassed: false,
            bypassed_apps: std::collections::HashSet::new(),
            hidden_apps: std::collections::HashSet::new(),
            app_volumes: std::collections::HashMap::new(),
            active_preset: Some("Flat".to_string()),
            user_presets: Vec::new(),
            ui_settings: UiSettings::default(),
        }
    }
}

impl GeckoSettings {
    /// Load settings from disk, or return default if missing/corrupt
    pub fn load() -> Self {
        let path = Self::get_config_path();
        
        if let Some(path) = path {
            if path.exists() {
                match fs::File::open(&path) {
                    Ok(file) => {
                        match serde_json::from_reader(file) {
                            Ok(settings) => {
                                info!("Settings loaded from {:?}", path);
                                return settings;
                            }
                            Err(e) => {
                                error!("Failed to parse settings file: {}", e);
                            }
                        }
                    }
                    Err(e) => {
                        error!("Failed to open settings file: {}", e);
                    }
                }
            }
        }
        
        info!("Using default settings");
        Self::default()
    }

    /// Save settings to disk
    pub fn save(&self) -> Result<(), String> {
        let path = Self::get_config_path().ok_or("Could not determine config path")?;
        
        // Ensure directory exists
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }

        let file = fs::File::create(&path).map_err(|e| e.to_string())?;
        serde_json::to_writer_pretty(file, self).map_err(|e| e.to_string())?;
        
        info!("Settings saved to {:?}", path);
        Ok(())
    }

    /// Get the platform-specific configuration file path
    fn get_config_path() -> Option<PathBuf> {
        ProjectDirs::from("com", "gecko", "gecko")
            .map(|proj| proj.config_dir().join("settings.json"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_settings() {
        let settings = GeckoSettings::default();
        assert_eq!(settings.master_volume, 1.0);
        assert_eq!(settings.master_eq, [0.0; 10]);
        assert!(!settings.bypassed);
        assert!(settings.app_eq.is_empty());
        assert!(settings.app_volumes.is_empty());
        assert!(settings.bypassed_apps.is_empty());
        assert!(settings.hidden_apps.is_empty());
    }

    #[test]
    fn test_settings_serialization_roundtrip() {
        let mut settings = GeckoSettings::default();
        settings.master_volume = 0.75;
        settings.master_eq[0] = 3.5;
        settings.master_eq[9] = -2.0;
        settings.bypassed = true;

        // Add per-app settings
        settings.app_eq.insert("Firefox".to_string(), vec![1.0, 2.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, -1.0]);
        settings.app_volumes.insert("Firefox".to_string(), 1.5);
        settings.bypassed_apps.insert("Spotify".to_string());
        settings.hidden_apps.insert("systemsounds".to_string());

        // Serialize to JSON
        let json = serde_json::to_string_pretty(&settings).unwrap();

        // Deserialize back
        let deserialized: GeckoSettings = serde_json::from_str(&json).unwrap();

        assert_eq!(deserialized.master_volume, 0.75);
        assert_eq!(deserialized.master_eq[0], 3.5);
        assert_eq!(deserialized.master_eq[9], -2.0);
        assert!(deserialized.bypassed);
        assert_eq!(deserialized.app_eq.get("Firefox").unwrap()[0], 1.0);
        assert_eq!(deserialized.app_volumes.get("Firefox").unwrap(), &1.5);
        assert!(deserialized.bypassed_apps.contains("Spotify"));
        assert!(deserialized.hidden_apps.contains("systemsounds"));
    }

    #[test]
    fn test_settings_backward_compat_missing_fields() {
        // Simulate loading old settings that don't have new fields
        let old_json = r#"{
            "master_volume": 0.8,
            "master_eq": [0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
            "bypassed": false,
            "active_preset": "Flat",
            "user_presets": [],
            "ui_settings": {
                "theme": "Dark",
                "show_level_meters": true,
                "start_minimized": false,
                "eq_bands_ui": 10
            }
        }"#;

        let settings: GeckoSettings = serde_json::from_str(old_json).unwrap();

        // New fields should default properly
        assert!(settings.app_eq.is_empty());
        assert!(settings.app_volumes.is_empty());
        assert!(settings.bypassed_apps.is_empty());
        assert!(settings.hidden_apps.is_empty());
    }

    #[test]
    fn test_user_preset_serialization() {
        let preset = UserPreset {
            name: "Bass Boost".to_string(),
            gains: [6.0, 4.0, 2.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
            created_at: Utc::now(),
        };

        let json = serde_json::to_string(&preset).unwrap();
        let deserialized: UserPreset = serde_json::from_str(&json).unwrap();

        assert_eq!(deserialized.name, "Bass Boost");
        assert_eq!(deserialized.gains[0], 6.0);
    }

    #[test]
    fn test_ui_settings_default() {
        let ui = UiSettings::default();
        assert_eq!(ui.theme, "Dark");
        assert!(ui.show_level_meters);
        assert!(!ui.start_minimized);
        assert_eq!(ui.eq_bands_ui, 10);
        assert!(ui.soft_clip_enabled);
    }

    #[test]
    fn test_per_app_eq_multiple_apps() {
        let mut settings = GeckoSettings::default();

        // Add EQ settings for multiple apps
        settings.app_eq.insert("Firefox".to_string(), vec![3.0; 10]);
        settings.app_eq.insert("Spotify".to_string(), vec![-3.0; 10]);
        settings.app_eq.insert("Discord".to_string(), vec![0.0; 10]);

        assert_eq!(settings.app_eq.len(), 3);
        assert_eq!(settings.app_eq.get("Firefox").unwrap()[0], 3.0);
        assert_eq!(settings.app_eq.get("Spotify").unwrap()[0], -3.0);
    }

    #[test]
    fn test_per_app_volume_range() {
        let mut settings = GeckoSettings::default();

        // Test various volume levels
        settings.app_volumes.insert("quiet".to_string(), 0.25);
        settings.app_volumes.insert("normal".to_string(), 1.0);
        settings.app_volumes.insert("loud".to_string(), 2.0);
        settings.app_volumes.insert("muted".to_string(), 0.0);

        assert_eq!(settings.app_volumes.get("muted").unwrap(), &0.0);
        assert_eq!(settings.app_volumes.get("loud").unwrap(), &2.0);
    }
}
