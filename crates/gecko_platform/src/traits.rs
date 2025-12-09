//! Platform Backend Traits
//!
//! Defines the interface that all platform implementations must provide.

use serde::{Deserialize, Serialize};

use crate::error::PlatformError;

/// Information about a running application that produces audio
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApplicationInfo {
    /// Process ID (platform-specific meaning)
    pub pid: u32,

    /// Application name
    pub name: String,

    /// Optional icon path or identifier
    pub icon: Option<String>,

    /// Whether this application is currently producing audio
    pub is_active: bool,
}

/// Represents a node in the audio graph (PipeWire concept, adapted for other platforms)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AudioNode {
    /// Unique node identifier
    pub id: u32,

    /// Human-readable name
    pub name: String,

    /// Node type (e.g., "Audio/Sink", "Audio/Source", "Stream/Output/Audio")
    pub media_class: String,

    /// Associated application info (if this is an application node)
    pub application: Option<ApplicationInfo>,
}

/// Represents a port on an audio node
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AudioPort {
    /// Unique port identifier
    pub id: u32,

    /// Parent node ID
    pub node_id: u32,

    /// Port name (e.g., "output_FL", "input_FR")
    pub name: String,

    /// Direction: "input" or "output"
    pub direction: String,

    /// Channel name (e.g., "FL", "FR", "MONO")
    pub channel: String,
}

/// Represents a link between two ports
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LinkInfo {
    /// Link identifier
    pub id: u32,

    /// Source port ID
    pub output_port: u32,

    /// Destination port ID
    pub input_port: u32,

    /// Whether this link is currently active
    pub active: bool,
}

/// Configuration for creating a virtual audio sink
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VirtualSinkConfig {
    /// Name for the virtual device
    pub name: String,

    /// Number of channels (usually 2 for stereo)
    pub channels: u32,

    /// Sample rate
    pub sample_rate: u32,

    /// Whether the device should persist after the app closes
    pub persistent: bool,
}

impl Default for VirtualSinkConfig {
    fn default() -> Self {
        Self {
            name: "Gecko Virtual Sink".to_string(),
            channels: 2,
            sample_rate: 48000,
            persistent: false,
        }
    }
}

/// Trait for platform-specific audio routing backends
///
/// Each platform (Linux/Windows/macOS) implements this trait to provide
/// unified access to OS-specific audio capabilities.
pub trait PlatformBackend: Send + Sync {
    /// Get the name of this backend (e.g., "PipeWire", "WASAPI", "CoreAudio")
    fn name(&self) -> &'static str;

    /// Check if the backend is connected and ready
    fn is_connected(&self) -> bool;

    /// List all audio-producing applications
    fn list_applications(&self) -> Result<Vec<ApplicationInfo>, PlatformError>;

    /// List all audio nodes in the graph
    fn list_nodes(&self) -> Result<Vec<AudioNode>, PlatformError>;

    /// List all ports for a given node
    fn list_ports(&self, node_id: u32) -> Result<Vec<AudioPort>, PlatformError>;

    /// List all active links
    fn list_links(&self) -> Result<Vec<LinkInfo>, PlatformError>;

    /// Create a virtual audio sink
    ///
    /// Returns the node ID of the created sink
    fn create_virtual_sink(&mut self, config: VirtualSinkConfig) -> Result<u32, PlatformError>;

    /// Destroy a virtual sink
    fn destroy_virtual_sink(&mut self, node_id: u32) -> Result<(), PlatformError>;

    /// Create a link between two ports
    ///
    /// Returns the link ID
    fn create_link(&mut self, output_port: u32, input_port: u32) -> Result<u32, PlatformError>;

    /// Destroy a link
    fn destroy_link(&mut self, link_id: u32) -> Result<(), PlatformError>;

    /// Route an application's audio to a specific sink
    ///
    /// This is a convenience method that finds the application's output ports
    /// and creates links to the sink's input ports.
    fn route_application_to_sink(
        &mut self,
        app_name: &str,
        sink_node_id: u32,
    ) -> Result<Vec<u32>, PlatformError>;

    /// Get the default output device node ID
    fn default_output_node(&self) -> Result<u32, PlatformError>;

    /// Get the default input device node ID
    fn default_input_node(&self) -> Result<u32, PlatformError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_virtual_sink_config_default() {
        let config = VirtualSinkConfig::default();
        assert_eq!(config.channels, 2);
        assert_eq!(config.sample_rate, 48000);
        assert!(!config.persistent);
    }

    #[test]
    fn test_application_info_serialization() {
        let app = ApplicationInfo {
            pid: 1234,
            name: "Firefox".to_string(),
            icon: Some("/usr/share/icons/firefox.png".to_string()),
            is_active: true,
        };

        let json = serde_json::to_string(&app).unwrap();
        let deserialized: ApplicationInfo = serde_json::from_str(&json).unwrap();

        assert_eq!(app.pid, deserialized.pid);
        assert_eq!(app.name, deserialized.name);
    }

    #[test]
    fn test_audio_node_serialization() {
        let node = AudioNode {
            id: 42,
            name: "Gecko Virtual Sink".to_string(),
            media_class: "Audio/Sink".to_string(),
            application: None,
        };

        let json = serde_json::to_string(&node).unwrap();
        assert!(json.contains("Gecko Virtual Sink"));
    }
}
