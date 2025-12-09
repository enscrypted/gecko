//! PipeWire State Types
//!
//! Internal state representations for PipeWire objects discovered via the registry.

use std::collections::HashMap;

/// Information about a PipeWire client (discovered from registry)
///
/// Clients represent applications connected to PipeWire.
#[derive(Debug, Clone)]
pub struct PwClientInfo {
    /// Unique client identifier assigned by PipeWire
    pub id: u32,

    /// Application name (from application.name property)
    pub application_name: Option<String>,

    /// Process ID from pipewire.sec.pid
    pub pid: Option<u32>,
}

/// Information about a PipeWire node (discovered from registry)
///
/// Nodes represent audio sinks, sources, and application streams in the PipeWire graph.
#[derive(Debug, Clone)]
pub struct PwNodeInfo {
    /// Unique node identifier assigned by PipeWire
    pub id: u32,

    /// Human-readable node name (from node.name property)
    pub name: String,

    /// Media class (e.g., "Audio/Sink", "Audio/Source", "Stream/Output/Audio")
    pub media_class: Option<String>,

    /// Application name if this is an application stream
    pub application_name: Option<String>,

    /// Application process ID if available (from node or looked up via client)
    pub application_pid: Option<u32>,

    /// Client ID that owns this node (from client.id property)
    pub client_id: Option<u32>,

    /// Whether the node is currently active
    pub is_active: bool,
}

/// Information about a PipeWire port
///
/// Ports are connection points on nodes - audio flows between linked ports.
#[derive(Debug, Clone)]
pub struct PwPortInfo {
    /// Unique port identifier
    pub id: u32,

    /// Parent node ID
    pub node_id: u32,

    /// Port name (e.g., "output_FL", "input_FR")
    pub name: String,

    /// Port direction
    pub direction: PortDirection,

    /// Audio channel (e.g., "FL", "FR", "MONO")
    pub channel: String,
}

/// Port direction in the audio graph
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PortDirection {
    /// Input port - receives audio
    Input,
    /// Output port - sends audio
    Output,
}

/// Information about a PipeWire link
///
/// Links connect output ports to input ports, defining the audio routing graph.
#[derive(Debug, Clone)]
pub struct PwLinkInfo {
    /// Unique link identifier
    pub id: u32,

    /// Source node ID (for convenience, derived from output_port)
    pub output_node: u32,

    /// Source port ID
    pub output_port: u32,

    /// Destination node ID (for convenience, derived from input_port)
    pub input_node: u32,

    /// Destination port ID
    pub input_port: u32,

    /// Whether the link is currently active (negotiated and passing audio)
    pub is_active: bool,
}

/// Snapshot of the PipeWire graph state
///
/// This structure is shared between the PipeWire thread and the main thread
/// via Arc<RwLock<...>>. The PipeWire thread updates it periodically, and
/// the main thread reads from it for list_* operations.
#[derive(Debug, Clone, Default)]
pub struct PipeWireState {
    /// All discovered clients indexed by ID
    pub clients: HashMap<u32, PwClientInfo>,

    /// All discovered nodes indexed by ID
    pub nodes: HashMap<u32, PwNodeInfo>,

    /// All discovered ports indexed by ID
    pub ports: HashMap<u32, PwPortInfo>,

    /// All discovered links indexed by ID
    pub links: HashMap<u32, PwLinkInfo>,

    /// Default audio sink node ID (from metadata)
    pub default_sink_id: Option<u32>,

    /// Default audio source node ID (from metadata)
    pub default_source_id: Option<u32>,

    /// Whether we're connected to the PipeWire daemon
    pub connected: bool,
}

impl PipeWireState {
    /// Create a new empty state
    pub fn new() -> Self {
        Self::default()
    }

    /// Mark the connection as established
    pub fn set_connected(&mut self, connected: bool) {
        self.connected = connected;
    }

    /// Get ports belonging to a specific node
    pub fn ports_for_node(&self, node_id: u32) -> Vec<&PwPortInfo> {
        self.ports
            .values()
            .filter(|p| p.node_id == node_id)
            .collect()
    }

    /// Get output ports for a node
    pub fn output_ports_for_node(&self, node_id: u32) -> Vec<&PwPortInfo> {
        self.ports
            .values()
            .filter(|p| p.node_id == node_id && p.direction == PortDirection::Output)
            .collect()
    }

    /// Get input ports for a node
    pub fn input_ports_for_node(&self, node_id: u32) -> Vec<&PwPortInfo> {
        self.ports
            .values()
            .filter(|p| p.node_id == node_id && p.direction == PortDirection::Input)
            .collect()
    }

    /// Find nodes by application name
    pub fn nodes_for_application(&self, app_name: &str) -> Vec<&PwNodeInfo> {
        self.nodes
            .values()
            .filter(|n| n.application_name.as_deref() == Some(app_name))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_state_default() {
        let state = PipeWireState::new();
        assert!(!state.connected);
        assert!(state.nodes.is_empty());
        assert!(state.ports.is_empty());
        assert!(state.links.is_empty());
    }

    #[test]
    fn test_ports_for_node() {
        let mut state = PipeWireState::new();

        // Add some ports
        state.ports.insert(
            1,
            PwPortInfo {
                id: 1,
                node_id: 100,
                name: "output_FL".to_string(),
                direction: PortDirection::Output,
                channel: "FL".to_string(),
            },
        );
        state.ports.insert(
            2,
            PwPortInfo {
                id: 2,
                node_id: 100,
                name: "output_FR".to_string(),
                direction: PortDirection::Output,
                channel: "FR".to_string(),
            },
        );
        state.ports.insert(
            3,
            PwPortInfo {
                id: 3,
                node_id: 200,
                name: "input_FL".to_string(),
                direction: PortDirection::Input,
                channel: "FL".to_string(),
            },
        );

        let ports_100 = state.ports_for_node(100);
        assert_eq!(ports_100.len(), 2);

        let output_ports = state.output_ports_for_node(100);
        assert_eq!(output_ports.len(), 2);

        let input_ports = state.input_ports_for_node(200);
        assert_eq!(input_ports.len(), 1);
    }

    #[test]
    fn test_nodes_for_application() {
        let mut state = PipeWireState::new();

        state.nodes.insert(
            1,
            PwNodeInfo {
                id: 1,
                name: "Firefox".to_string(),
                media_class: Some("Stream/Output/Audio".to_string()),
                application_name: Some("Firefox".to_string()),
                application_pid: Some(1234),
                client_id: Some(10),
                is_active: true,
            },
        );
        state.nodes.insert(
            2,
            PwNodeInfo {
                id: 2,
                name: "Chrome".to_string(),
                media_class: Some("Stream/Output/Audio".to_string()),
                application_name: Some("Chrome".to_string()),
                application_pid: Some(5678),
                client_id: Some(11),
                is_active: true,
            },
        );

        let firefox_nodes = state.nodes_for_application("Firefox");
        assert_eq!(firefox_nodes.len(), 1);
        assert_eq!(firefox_nodes[0].id, 1);
    }
}
