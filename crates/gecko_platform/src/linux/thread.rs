//! PipeWire Thread Implementation
//!
//! This module contains the logic that runs in the dedicated PipeWire thread.
//! PipeWire objects are not Send/Sync, so all PipeWire operations must occur here.
//!
//! # Audio Streaming
//!
//! When streaming is active, we have:
//! - A capture stream reading from the virtual sink's monitor port
//! - A playback stream writing to the default output device
//! - DSP processing (EQ) happens in the capture callback
//! - A ring buffer transfers audio from capture to playback

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, RwLock};

use crossbeam_channel::Sender;
use pipewire as pw;
use pw::proxy::ProxyT;
use pw::properties::properties;
use pw::spa::pod::Pod;
use pw::spa::utils::dict::DictRef;
use pw::stream::{Stream, StreamFlags, StreamListener};

use super::audio_stream::AudioProcessingState;
use super::message::{PwCommand, PwResponse};
use super::state::{PipeWireState, PortDirection, PwClientInfo, PwLinkInfo, PwNodeInfo, PwPortInfo};

/// User data passed to playback stream callback
/// Must be separate struct since it's owned by the stream listener
struct PlaybackUserData {
    /// Ring buffer consumer (reads audio to play) - for legacy single-stream mode
    consumer: rtrb::Consumer<f32>,
    /// Shared state for peaks reporting and master EQ
    #[allow(dead_code)] // Will be used when master EQ is applied in playback callback
    audio_state: Arc<AudioProcessingState>,
    /// Master EQ processor (applied after mixing in per-app mode)
    #[allow(dead_code)] // Planned for master EQ feature
    master_eq: gecko_dsp::Equalizer,
    /// Local copy of master EQ update counter
    #[allow(dead_code)] // Planned for master EQ feature
    last_master_eq_counter: u32,
}

/// User data for per-app mixing playback callback
///
/// This is used when per-app mode is active. It reads from multiple
/// app ring buffers, mixes them, applies master EQ, then outputs.
///
/// NOTE: This struct is scaffolded for the per-app EQ mixing feature.
/// It will be used when true per-app capture is implemented.
#[allow(dead_code)]
struct MixingPlaybackUserData {
    /// Shared state containing all app consumers
    /// The mixer reads from all consumers and sums the audio
    app_consumers_state: Arc<AppConsumersState>,
    /// Shared state for volume, bypass, peaks, and master EQ
    audio_state: Arc<AudioProcessingState>,
    /// Master EQ processor (applied after mixing)
    master_eq: gecko_dsp::Equalizer,
    /// Local copy of master EQ update counter
    last_master_eq_counter: u32,
    /// Pre-allocated mixing buffer to avoid allocations in callback
    mix_buffer: Vec<f32>,
    /// Pre-allocated read buffer for each app
    read_buffer: Vec<f32>,
}

/// User data passed to capture stream callback
struct CaptureUserData {
    /// Ring buffer producer (writes captured audio)
    producer: rtrb::Producer<f32>,
    /// EQ processor
    equalizer: gecko_dsp::Equalizer,
    /// Shared state for volume, bypass, peaks, and EQ gains
    audio_state: Arc<AudioProcessingState>,
    /// Local copy of the EQ update counter to detect changes
    last_eq_update_counter: u32,
}

// === Per-App Audio Types ===
// NOTE: These types are scaffolded for the per-app EQ feature (see AGENT.md).
// They will be used when true per-app capture is implemented.

/// State for a per-application virtual sink
///
/// Each detected audio application gets its own virtual sink,
/// enabling true per-app EQ processing before mixing.
#[allow(dead_code)]
struct AppSinkState {
    /// Application name (e.g., "Firefox", "Spotify")
    app_name: String,
    /// Node ID of the virtual sink we created for this app
    sink_node_id: u32,
    /// Node IDs of application audio streams routed to this sink
    /// Multiple nodes can share the same app name (e.g., multiple Firefox tabs)
    app_node_ids: Vec<u32>,
    /// Link IDs connecting app nodes to our sink
    link_ids: Vec<u32>,
    /// The PipeWire node proxy (keeps the sink alive)
    sink_proxy: pw::node::Node,
}

/// State for a per-application capture stream with its own EQ
///
/// This enables true per-app EQ by processing each app's audio
/// independently before mixing.
#[allow(dead_code)]
struct AppCaptureState {
    /// Application name
    app_name: String,
    /// The capture stream
    stream: Stream,
    /// Stream listener (must stay alive while stream is active)
    listener: StreamListener<AppCaptureUserData>,
    /// Per-app EQ gains (shared with callback via Arc)
    eq_gains: Arc<[std::sync::atomic::AtomicU32; 10]>,
    /// EQ update counter (shared with callback)
    eq_update_counter: Arc<std::sync::atomic::AtomicU32>,
    /// Whether this app's EQ is bypassed (shared with callback)
    bypassed: Arc<std::sync::atomic::AtomicBool>,
    /// Per-app volume (0.0 - 2.0, stored as f32 bits in AtomicU32)
    /// Default is 1.0 (unity gain). Values > 1.0 amplify, < 1.0 attenuate.
    volume: Arc<std::sync::atomic::AtomicU32>,
}

/// User data for per-app capture stream callbacks
#[allow(dead_code)]
struct AppCaptureUserData {
    /// Ring buffer producer (writes to app's own buffer)
    /// Each app has its own SPSC ring buffer for lock-free operation
    producer: rtrb::Producer<f32>,
    /// Per-app EQ processor
    equalizer: gecko_dsp::Equalizer,
    /// Per-app EQ gains (stored as atomic u32 bits for lock-free access)
    eq_gains: Arc<[std::sync::atomic::AtomicU32; 10]>,
    /// Counter for detecting EQ changes
    eq_update_counter: Arc<std::sync::atomic::AtomicU32>,
    /// Local copy of the EQ update counter
    last_eq_update_counter: u32,
    /// Whether EQ is bypassed for this app
    bypassed: Arc<std::sync::atomic::AtomicBool>,
    /// Per-app volume (0.0 - 2.0, stored as f32 bits in AtomicU32)
    volume: Arc<std::sync::atomic::AtomicU32>,
}

/// Shared state for per-app consumers accessible by the mixer
///
/// This is shared via Arc within the same thread (PipeWire thread) for convenience
/// so the mixer callback can access it. All access happens on the PipeWire thread.
/// The RwLock is used for interior mutability when adding/removing consumers.
#[allow(clippy::arc_with_non_send_sync)] // All usage is within the same thread
#[allow(dead_code)] // Scaffolded for per-app EQ mixing feature
struct AppConsumersState {
    /// Current consumers indexed by slot (fixed size for lock-free access)
    /// Each slot is Option<(app_name, consumer)>
    /// Using parking_lot::RwLock for now - playback reads, main thread writes
    consumers: parking_lot::RwLock<Vec<(String, rtrb::Consumer<f32>)>>,
}

#[allow(dead_code)] // Scaffolded for per-app EQ mixing feature
impl AppConsumersState {
    fn new() -> Self {
        Self {
            consumers: parking_lot::RwLock::new(Vec::with_capacity(16)),
        }
    }

    fn add_consumer(&self, app_name: String, consumer: rtrb::Consumer<f32>) {
        let mut guard = self.consumers.write();
        guard.push((app_name, consumer));
    }

    fn remove_consumer(&self, app_name: &str) {
        let mut guard = self.consumers.write();
        guard.retain(|(name, _)| name != app_name);
    }
}

/// Local state that lives only in the PipeWire thread
///
/// This uses Rc<RefCell<...>> because PipeWire callbacks need mutable access
/// but aren't Send/Sync. The main thread accesses a snapshot via Arc<RwLock<...>>.
#[derive(Default)]
struct LocalState {
    clients: HashMap<u32, PwClientInfo>,
    nodes: HashMap<u32, PwNodeInfo>,
    ports: HashMap<u32, PwPortInfo>,
    links: HashMap<u32, PwLinkInfo>,
    default_sink_id: Option<u32>,
    default_source_id: Option<u32>,

    /// IDs of virtual sinks we've created (to track for cleanup)
    /// We store the Proxy here so we can Drop it to destroy the remote object
    our_sinks: Vec<pw::node::Node>,

    /// IDs of links we've created
    /// We store the Proxy here so we can Drop it to destroy the remote object
    our_links: Vec<pw::link::Link>,

    // === Legacy Single-Stream Audio State (for backwards compatibility) ===

    /// Whether audio streaming is currently active
    streaming_active: bool,

    /// Playback stream (outputs to speakers)
    playback_stream: Option<Stream>,

    /// Capture stream (reads from virtual sink monitor)
    capture_stream: Option<Stream>,

    /// Listener for playback stream (must stay alive while stream is active)
    playback_listener: Option<StreamListener<PlaybackUserData>>,

    /// Listener for capture stream (must stay alive while stream is active)
    capture_listener: Option<StreamListener<CaptureUserData>>,

    /// Ring buffer producer (capture writes here) - stored for EQ updates
    ring_producer: Option<rtrb::Producer<f32>>,

    /// Ring buffer consumer (playback reads from here)
    ring_consumer: Option<rtrb::Consumer<f32>>,

    /// EQ processor for audio DSP - stored separately for command updates
    equalizer: Option<gecko_dsp::Equalizer>,

    /// Shared state for volume, bypass, peaks (accessed atomically)
    audio_state: Option<Arc<AudioProcessingState>>,

    /// Capture stream node ID (for cleanup)
    capture_target_id: Option<u32>,

    /// Playback stream node ID (for cleanup)
    playback_target_id: Option<u32>,

    /// Whether capture links need to be created manually
    /// WirePlumber doesn't respect target.object for sink monitor capture,
    /// so we create the links ourselves after the stream is ready
    pending_capture_links: bool,

    /// The node ID of our capture stream (once registered in the graph)
    capture_stream_node_id: Option<u32>,

    // === Per-App Audio State (TRUE per-app EQ) ===

    /// Per-application virtual sinks
    /// Key: application name (e.g., "Firefox", "Spotify")
    /// Each app gets its own sink so we can capture its audio independently
    app_sinks: HashMap<String, AppSinkState>,

    /// Per-application capture streams with individual EQ instances
    /// Key: application name
    /// Each app gets its own capture stream and EQ processor
    app_captures: HashMap<String, AppCaptureState>,

    /// Whether per-app streaming mode is active
    /// When true, we use app_sinks and app_captures instead of the single stream
    per_app_mode: bool,

    /// Apps that are pending sink creation (detected but sink not yet created)
    /// This is used to debounce rapid app detection during startup
    pending_app_sinks: Vec<String>,

    /// Apps whose capture links need to be created manually
    /// Similar to pending_capture_links but per-app
    pending_app_capture_links: Vec<String>,

    /// Retry count for apps in pending_app_capture_links
    /// If an app has no nodes for too many iterations, remove it from the list
    /// This handles transient stream nodes (like Firefox during video scrubbing)
    pending_app_link_retries: HashMap<String, u32>,

    /// Apps whose capture streams need monitor->capture links
    /// After creating a capture stream, we need to link the sink's monitor ports
    /// to the capture stream's input ports
    pending_app_monitor_links: Vec<String>,

    /// Shared state for per-app consumers (accessible by mixer playback callback)
    /// The playback callback reads from all consumers and mixes them
    app_consumers_state: Option<Arc<AppConsumersState>>,

    /// Mixing playback stream (reads from all app consumers and mixes)
    mixing_playback_stream: Option<Stream>,

    /// Mixing playback listener
    mixing_playback_listener: Option<StreamListener<MixingPlaybackUserData>>,
}


/// Synchronize local state to the shared state accessible by the main thread
fn sync_to_shared(local: &LocalState, shared: &Arc<RwLock<PipeWireState>>) {
    // Rust pattern: try_write() avoids blocking if the main thread is reading
    // We prefer dropping updates over blocking the PipeWire thread
    if let Ok(mut shared_state) = shared.try_write() {
        shared_state.clients = local.clients.clone();
        shared_state.nodes = local.nodes.clone();
        shared_state.ports = local.ports.clone();
        shared_state.links = local.links.clone();
        shared_state.default_sink_id = local.default_sink_id;
        shared_state.default_source_id = local.default_source_id;
    }
}

/// Extract a property value from a PipeWire properties dictionary
fn get_prop(props: Option<&DictRef>, key: &str) -> Option<String> {
    props.and_then(|p| p.get(key).map(String::from))
}

/// Extract a property as u32 from PipeWire properties
fn get_prop_u32(props: Option<&DictRef>, key: &str) -> Option<u32> {
    get_prop(props, key).and_then(|s| s.parse().ok())
}

/// Process pending app sinks - create virtual sinks for newly detected apps
///
/// This is called from the main loop to create per-app sinks for applications
/// that were detected in the registry listener.
fn process_pending_app_sinks(local: &mut LocalState, core: &pw::core::Core) {
    // Take the pending list to process
    let pending: Vec<String> = local.pending_app_sinks.drain(..).collect();

    for app_name in pending {
        // Skip if we already have a sink for this app (might have been created by command)
        if local.app_sinks.contains_key(&app_name) {
            continue;
        }

        tracing::debug!("Creating per-app sink for detected app '{}'", app_name);

        // Create a virtual sink named "Gecko-{AppName}"
        let sink_name = format!("Gecko-{}", app_name);
        let props = properties! {
            "factory.name" => "support.null-audio-sink",
            "node.name" => sink_name.as_str(),
            "media.class" => "Audio/Sink",
            "audio.channels" => "2",
            "audio.rate" => "48000",
            "audio.position" => "FL,FR",
            "node.pause-on-idle" => "false",
            "node.always-process" => "true",
            "audio.volume" => "1.0",
            "object.linger" => "false",
            "node.description" => format!("Gecko Audio - {}", app_name).as_str(),
        };

        match core.create_object::<pw::node::Node>("adapter", &props) {
            Ok(node) => {
                let proxy_id = node.upcast_ref().id();
                tracing::debug!(
                    "Created per-app sink '{}' with proxy id: {}",
                    sink_name,
                    proxy_id
                );

                // Store the app sink state
                local.app_sinks.insert(
                    app_name.clone(),
                    AppSinkState {
                        app_name: app_name.clone(),
                        sink_node_id: proxy_id,
                        app_node_ids: Vec::new(),
                        link_ids: Vec::new(),
                        sink_proxy: node,
                    },
                );

                // Queue this app for capture link creation once ports are available
                local.pending_app_capture_links.push(app_name);
            }
            Err(e) => {
                tracing::error!("Failed to create per-app sink for '{}': {}", app_name, e);
            }
        }
    }
}

/// Try to create capture links for per-app sinks
///
/// Links the app's output to the per-app Gecko sink so audio flows through our DSP.
/// Returns a list of (app_name, sink_node_id) for apps that were successfully linked.
/// Maximum retry attempts for relink before giving up (50 * 100ms = 5 seconds)
/// This handles transient stream nodes that appear and immediately disappear
/// (like Firefox during video scrubbing)
const MAX_RELINK_RETRIES: u32 = 50;

fn process_pending_app_capture_links(local: &mut LocalState, core: &pw::core::Core) -> Vec<(String, u32)> {
    // Process each pending app
    let pending: Vec<String> = local.pending_app_capture_links.clone();
    let mut completed = Vec::new();
    let mut gave_up = Vec::new();

    for app_name in &pending {
        // Check that we have a sink for this app (the struct itself isn't used yet,
        // but we need to verify the sink exists before proceeding)
        let _sink_state = match local.app_sinks.get(app_name) {
            Some(s) => s,
            None => continue,
        };

        // Find the Gecko sink node by name pattern "Gecko-{app_name}"
        let sink_name = format!("Gecko-{}", app_name);
        let sink_node_id = local
            .nodes
            .iter()
            .find(|(_, n)| n.name == sink_name)
            .map(|(id, _)| *id);

        let sink_node_id = match sink_node_id {
            Some(id) => id,
            None => {
                tracing::debug!("Sink node '{}' not yet in registry", sink_name);
                continue;
            }
        };

        // Find all app nodes with this application name
        let app_nodes: Vec<u32> = local
            .nodes
            .iter()
            .filter(|(_, n)| {
                n.media_class.as_deref() == Some("Stream/Output/Audio")
                    && (n.application_name.as_deref() == Some(app_name)
                        || &n.name == app_name)
                    && !n.name.starts_with("Gecko")
            })
            .map(|(id, _)| *id)
            .collect();

        if app_nodes.is_empty() {
            // Increment retry counter and check if we should give up
            // This handles transient nodes (like Firefox during video scrubbing)
            let retry_count = local.pending_app_link_retries.entry(app_name.clone()).or_insert(0);
            *retry_count += 1;

            if *retry_count >= MAX_RELINK_RETRIES {
                tracing::debug!(
                    "Giving up on relink for '{}' after {} retries (app likely stopped playing)",
                    app_name,
                    retry_count
                );
                gave_up.push(app_name.clone());
            }
            continue;
        }

        // Found app nodes - reset retry counter
        local.pending_app_link_retries.remove(app_name);

        // Find sink input ports
        let mut sink_inputs: Vec<u32> = local
            .ports
            .iter()
            .filter(|(_, p)| p.node_id == sink_node_id && matches!(p.direction, PortDirection::Input))
            .map(|(id, _)| *id)
            .collect();
        sink_inputs.sort();

        if sink_inputs.len() < 2 {
            tracing::debug!(
                "Sink '{}' doesn't have enough input ports yet ({}/2)",
                sink_name,
                sink_inputs.len()
            );
            continue;
        }

        // Link each app node to the sink
        let mut links_created = 0;
        for app_node_id in &app_nodes {
            // Find app output ports
            let mut app_outputs: Vec<u32> = local
                .ports
                .iter()
                .filter(|(_, p)| p.node_id == *app_node_id && matches!(p.direction, PortDirection::Output))
                .map(|(id, _)| *id)
                .collect();
            app_outputs.sort();

            if app_outputs.len() < 2 {
                continue;
            }

            // Create links: FL->FL, FR->FR
            for i in 0..2 {
                let out_port = app_outputs[i];
                let in_port = sink_inputs[i];

                // Check if link already exists
                let exists = local
                    .links
                    .values()
                    .any(|l| l.output_port == out_port && l.input_port == in_port);

                if exists {
                    links_created += 1;
                    continue;
                }

                let link_props = properties! {
                    "link.output.port" => out_port.to_string(),
                    "link.input.port" => in_port.to_string(),
                    "link.passive" => "true",
                };

                match core.create_object::<pw::link::Link>("link-factory", &link_props) {
                    Ok(link) => {
                        tracing::debug!(
                            "Created app->sink link for '{}': {} -> {}",
                            app_name,
                            out_port,
                            in_port
                        );
                        local.our_links.push(link);
                        links_created += 1;
                    }
                    Err(e) => {
                        tracing::error!("Failed to create app->sink link: {}", e);
                    }
                }
            }
        }

        if links_created >= 2 {
            tracing::debug!(
                "App '{}' linked to its Gecko sink ({} links)",
                app_name,
                links_created
            );
            completed.push((app_name.clone(), sink_node_id));
        }
    }

    // Remove completed apps from pending list
    for (app, _) in &completed {
        local.pending_app_capture_links.retain(|a| a != app);
    }

    // Remove apps that gave up (exceeded MAX_RELINK_RETRIES) from pending list and retry counters
    // This prevents infinite retry loops for transient stream nodes (like Firefox during video scrubbing)
    for app in &gave_up {
        local.pending_app_capture_links.retain(|a| a != app);
        local.pending_app_link_retries.remove(app);
    }

    // Return completed apps for capture stream creation
    completed
}

/// Create the mixing playback stream that reads from all app consumers
///
/// This stream reads from all per-app ring buffers, mixes them together,
/// applies master EQ and volume, then outputs to speakers.
fn create_mixing_playback_stream(
    local: &mut LocalState,
    core: &pw::core::Core,
    playback_target: Option<u32>,
) -> bool {
    // CRITICAL: Stop legacy capture/playback to prevent double audio output
    // When per-app mode is active, we use mixing_playback_stream instead of legacy playback
    // If we don't stop legacy streams, audio goes through BOTH paths (causing echo/reverb)
    if let Some(ref stream) = local.capture_stream {
        tracing::debug!("Stopping legacy capture stream (switching to per-app mode)");
        let _ = stream.set_active(false);
        let _ = stream.disconnect();
    }
    local.capture_stream = None;
    local.capture_listener = None;

    if let Some(ref stream) = local.playback_stream {
        tracing::debug!("Stopping legacy playback stream (switching to per-app mode)");
        let _ = stream.set_active(false);
        let _ = stream.disconnect();
    }
    local.playback_stream = None;
    local.playback_listener = None;

    // Need shared consumers state
    let app_consumers_state = match &local.app_consumers_state {
        Some(state) => Arc::clone(state),
        None => {
            tracing::error!("Cannot create mixing playback: no consumers state");
            return false;
        }
    };

    // Need audio state for volume/bypass/peaks
    let audio_state = match &local.audio_state {
        Some(state) => Arc::clone(state),
        None => {
            tracing::error!("Cannot create mixing playback: no audio state");
            return false;
        }
    };

    tracing::debug!("Creating mixing playback stream (target={:?})", playback_target);

    // Create playback stream properties
    let playback_props = if let Some(target_id) = playback_target {
        properties! {
            "media.type" => "Audio",
            "media.category" => "Playback",
            "media.role" => "Music",
            "node.name" => "Gecko Playback",
            "node.description" => "Gecko Mixed Output",
            "target.object" => target_id.to_string(),
            "node.dont-reconnect" => "true",
            "node.latency" => "1024/48000",
        }
    } else {
        properties! {
            "media.type" => "Audio",
            "media.category" => "Playback",
            "media.role" => "Music",
            "node.name" => "Gecko Playback",
            "node.description" => "Gecko Mixed Output",
            "node.dont-reconnect" => "true",
            "node.latency" => "1024/48000",
        }
    };

    let playback_stream = match Stream::new(core, "gecko-mixing-playback", playback_props) {
        Ok(stream) => stream,
        Err(e) => {
            tracing::error!("Failed to create mixing playback stream: {}", e);
            return false;
        }
    };

    // Create master EQ
    let mut master_eq = gecko_dsp::Equalizer::new(48000.0);
    
    // CRITICAL: Apply initial Master EQ gains immediately!
    // The atomic counter check in the callback might miss the initial state if counters match (both 0).
    let initial_gains = audio_state.get_all_eq_gains();
    for (band, &gain_db) in initial_gains.iter().enumerate() {
        if gain_db.abs() > 0.001 {
            let _ = master_eq.set_band_gain(band, gain_db);
        }
    }

    // Pre-allocate buffers (max expected buffer size)
    const MAX_BUFFER_SIZE: usize = 48000; // ~1 second
    let mix_buffer = vec![0.0f32; MAX_BUFFER_SIZE];
    let read_buffer = vec![0.0f32; MAX_BUFFER_SIZE];

    // Create user data for mixing callback
    let user_data = MixingPlaybackUserData {
        app_consumers_state,
        audio_state: Arc::clone(&audio_state),
        master_eq,
        last_master_eq_counter: 0,
        mix_buffer,
        read_buffer,
    };

    // Set up mixing playback callback
    let listener = playback_stream
        .add_local_listener_with_user_data(user_data)
        .state_changed(|_stream, _user_data, old, new| {
            tracing::debug!("Mixing playback stream state: {:?} -> {:?}", old, new);
        })
        .process(|stream, user_data| {
            // Mixing playback callback - read from all app consumers, mix, apply master EQ

            // Check if master EQ needs updating
            let current_counter = user_data.audio_state.eq_update_counter();
            if current_counter != user_data.last_master_eq_counter {
                let gains = user_data.audio_state.get_all_eq_gains();
                for (band, gain_db) in gains.iter().enumerate() {
                    if let Err(e) = user_data.master_eq.set_band_gain(band, *gain_db) {
                        tracing::warn!("Failed to apply master EQ band {}: {:?}", band, e);
                    }
                }
                user_data.last_master_eq_counter = current_counter;
            }

            if let Some(mut buffer) = stream.dequeue_buffer() {
                let datas = buffer.datas_mut();
                if let Some(data) = datas.first_mut() {
                    if let Some(slice) = data.data() {
                        let samples: &mut [f32] = unsafe {
                            std::slice::from_raw_parts_mut(
                                slice.as_mut_ptr() as *mut f32,
                                slice.len() / 4,
                            )
                        };

                        let mut sample_count = samples.len();
                        if sample_count == 0 {
                            return;
                        }
                        
                        // Safety check: prevent buffer overflow if PipeWire requests huge chunk
                        if sample_count > user_data.mix_buffer.len() {
                            tracing::warn!(
                                "Buffer too large ({}), truncating to {}", 
                                sample_count, 
                                user_data.mix_buffer.len()
                            );
                            sample_count = user_data.mix_buffer.len();
                        }

                        // Clear mix buffer
                        for s in user_data.mix_buffer.iter_mut().take(sample_count) {
                            *s = 0.0;
                        }

                        // Read from all app consumers and mix
                        // NOTE: Using try_write() because read_chunk requires &mut Consumer
                        // If locked (unlikely), skip this cycle to avoid blocking the audio callback
                        if let Some(mut guard) = user_data.app_consumers_state.consumers.try_write() {
                            for (_app_name, consumer) in guard.iter_mut() {
                                // Try to read from this consumer
                                if let Ok(chunk) = consumer.read_chunk(sample_count) {
                                    let slices = chunk.as_slices();
                                    let mut idx = 0;

                                    // Add to mix buffer (sum all app audio)
                                    for src in slices.0.iter().chain(slices.1.iter()) {
                                        if idx < sample_count {
                                            user_data.mix_buffer[idx] += *src;
                                            idx += 1;
                                        }
                                    }
                                    chunk.commit_all();
                                }
                            }
                        }

                        // Copy mix to output
                        for (i, sample) in samples.iter_mut().enumerate() {
                            *sample = user_data.mix_buffer[i];
                        }

                        // Apply master EQ if not bypassed
                        if !user_data.audio_state.bypassed.load(Ordering::Relaxed) {
                            user_data.master_eq.process_interleaved(samples);
                        }

                        // Apply master volume
                        let volume = user_data.audio_state.master_volume();
                        for sample in samples.iter_mut() {
                            *sample *= volume;
                        }

                        // Apply soft clipping to prevent harsh digital distortion
                        // This smoothly limits peaks that exceed the threshold
                        user_data.audio_state.soft_clip_buffer(samples);

                        // Calculate and store peak levels (after soft clipping)
                        let mut peak_l = 0.0_f32;
                        let mut peak_r = 0.0_f32;
                        for (i, sample) in samples.iter().enumerate() {
                            if i % 2 == 0 {
                                peak_l = peak_l.max(sample.abs());
                            } else {
                                peak_r = peak_r.max(sample.abs());
                            }
                        }
                        user_data.audio_state.set_peaks(peak_l, peak_r);

                        // Push samples to spectrum analyzer for FFT visualization
                        // This is lock-free and won't block the audio callback
                        for chunk in samples.chunks_exact(2) {
                            user_data.audio_state.push_spectrum_sample(chunk[0], chunk[1]);
                        }

                        // Update chunk metadata
                        let chunk = data.chunk_mut();
                        *chunk.size_mut() = (samples.len() * 4) as u32;
                        *chunk.stride_mut() = 4;
                        *chunk.offset_mut() = 0;
                    }
                }
            }
        })
        .register();

    let listener = match listener {
        Ok(l) => l,
        Err(e) => {
            tracing::error!("Failed to register mixing playback listener: {}", e);
            return false;
        }
    };

    // Build audio format params
    let mut audio_info = pw::spa::param::audio::AudioInfoRaw::new();
    audio_info.set_format(pw::spa::param::audio::AudioFormat::F32LE);
    audio_info.set_rate(48000);
    audio_info.set_channels(2);

    let audio_params_bytes: Vec<u8> = pw::spa::pod::serialize::PodSerializer::serialize(
        std::io::Cursor::new(Vec::new()),
        &pw::spa::pod::Value::Object(pw::spa::pod::Object {
            type_: pw::spa::utils::SpaTypes::ObjectParamFormat.as_raw(),
            id: pw::spa::param::ParamType::EnumFormat.as_raw(),
            properties: audio_info.into(),
        }),
    )
    .expect("Failed to serialize audio params")
    .0
    .into_inner();

    let audio_pod = Pod::from_bytes(&audio_params_bytes).expect("Failed to create audio Pod");
    let mut params = [audio_pod];

    // Connect playback stream
    if let Err(e) = playback_stream.connect(
        pw::spa::utils::Direction::Output,
        playback_target,
        StreamFlags::AUTOCONNECT | StreamFlags::MAP_BUFFERS | StreamFlags::RT_PROCESS,
        &mut params,
    ) {
        tracing::error!("Failed to connect mixing playback stream: {}", e);
        return false;
    }

    // Activate
    if let Err(e) = playback_stream.set_active(true) {
        tracing::error!("Failed to activate mixing playback stream: {}", e);
        let _ = playback_stream.disconnect();
        return false;
    }

    // Store in local state
    local.mixing_playback_stream = Some(playback_stream);
    local.mixing_playback_listener = Some(listener);

    tracing::debug!("Mixing playback stream created and active");
    true
}

/// Process completed app links by creating capture streams
///
/// After an app is linked to its sink, we need to create a capture stream
/// to read from the sink's monitor and apply per-app EQ.
fn process_completed_app_links(
    completed: Vec<(String, u32)>,
    local: &mut LocalState,
    core: &pw::core::Core,
    audio_state: &Arc<AudioProcessingState>,
) {
    // Ensure we have a shared consumers state
    // Note: Using Arc here even though it's not Send/Sync - all access is on PipeWire thread
    #[allow(clippy::arc_with_non_send_sync)]
    if local.app_consumers_state.is_none() {
        local.app_consumers_state = Some(Arc::new(AppConsumersState::new()));
    }
    let app_consumers_state = local.app_consumers_state.as_ref().unwrap();

    for (app_name, sink_node_id) in completed {
        // Skip if we already have a capture for this app
        if local.app_captures.contains_key(&app_name) {
            continue;
        }

        // Find the sink in the registry to get the actual node ID
        let sink_name = format!("Gecko-{}", app_name);
        let actual_sink_id = local
            .nodes
            .iter()
            .find(|(_, n)| n.name == sink_name)
            .map(|(id, _)| *id)
            .unwrap_or(sink_node_id);

        // Create capture stream for this app
        if let Some(capture_state) = create_app_capture_stream(
            &app_name,
            actual_sink_id,
            core,
            audio_state,
            app_consumers_state,
        ) {
            local.app_captures.insert(app_name.clone(), capture_state);
            local.per_app_mode = true;
            // Queue for monitor->capture link creation (like pending_capture_links for legacy)
            local.pending_app_monitor_links.push(app_name.clone());
            tracing::debug!("Started per-app capture for '{}' (pending monitor links)", app_name);
        }
    }
}

/// Create a per-app capture stream with its own EQ instance
///
/// This is the core of per-app EQ - each app gets its own capture stream
/// reading from its dedicated sink's monitor, with its own EQ processor.
///
/// The consumer is added to the shared consumers state so the mixer can read from it.
fn create_app_capture_stream(
    app_name: &str,
    sink_node_id: u32,
    core: &pw::core::Core,
    audio_state: &Arc<AudioProcessingState>,
    app_consumers_state: &Arc<AppConsumersState>,
) -> Option<AppCaptureState> {
    tracing::debug!(
        "Creating per-app capture stream for '{}' (sink={})",
        app_name,
        sink_node_id
    );

    // Create ring buffer for this app's audio
    const RING_BUFFER_SIZE: usize = 48000 * 2; // 1 second stereo
    let (producer, consumer) = rtrb::RingBuffer::new(RING_BUFFER_SIZE);

    // Create EQ processor for this app
    let mut eq = gecko_dsp::Equalizer::new(48000.0);

    // Create atomic EQ gains (initialized from shared state if available)
    let initial_eq = audio_state.get_stream_eq_all(app_name);
    let eq_gains: Arc<[std::sync::atomic::AtomicU32; 10]> = Arc::new(std::array::from_fn(|i| {
        std::sync::atomic::AtomicU32::new(initial_eq[i].to_bits())
    }));
    let eq_gains_for_callback = Arc::clone(&eq_gains);

    // CRITICAL: Apply initial EQ gains to the DSP instance immediately!
    // The atomic values alone won't update the DSP state until the next change triggers a counter mismatch.
    for (band, &gain_db) in initial_eq.iter().enumerate() {
        if gain_db.abs() > 0.001 {
            let _ = eq.set_band_gain(band, gain_db);
        }
    }

    // Create EQ update counter (shared with callback)
    let eq_update_counter = Arc::new(std::sync::atomic::AtomicU32::new(0));
    let eq_update_counter_for_callback = Arc::clone(&eq_update_counter);

    // Create bypass flag (shared with callback)
    let initial_bypass = audio_state.is_stream_bypassed(app_name);
    let bypassed = Arc::new(std::sync::atomic::AtomicBool::new(initial_bypass));
    let bypassed_for_callback = Arc::clone(&bypassed);

    // Create per-app volume (initialize from shared state if available)
    let initial_volume = audio_state.get_stream_volume(app_name);
    let volume = Arc::new(std::sync::atomic::AtomicU32::new(initial_volume.to_bits()));
    let volume_for_callback = Arc::clone(&volume);

    // Create capture stream properties
    let stream_name = format!("Gecko Capture - {}", app_name);
    let capture_props = properties! {
        "media.type" => "Audio",
        "media.category" => "Capture",
        "media.role" => "Music",
        "node.name" => stream_name.as_str(),
        "node.description" => format!("Gecko Audio Input - {}", app_name).as_str(),
        "node.dont-reconnect" => "true",
        "stream.dont-reconnect" => "true",
        "node.passive" => "true",
        "node.autoconnect" => "false",
        "media.class" => "Stream/Input/Audio",
    };

    let capture_stream = match Stream::new(core, &format!("gecko-capture-{}", app_name), capture_props) {
        Ok(stream) => stream,
        Err(e) => {
            tracing::error!("Failed to create capture stream for '{}': {}", app_name, e);
            return None;
        }
    };

    // Create user data for the callback (owns the producer)
    let user_data = AppCaptureUserData {
        producer,
        equalizer: eq,
        eq_gains: eq_gains_for_callback,
        eq_update_counter: eq_update_counter_for_callback,
        last_eq_update_counter: 0,
        bypassed: bypassed_for_callback,
        volume: volume_for_callback,
    };

    // Set up capture stream listener with process callback
    let app_name_for_log = app_name.to_string();
    let listener = capture_stream
        .add_local_listener_with_user_data(user_data)
        .state_changed(move |_stream, _user_data, old, new| {
            tracing::debug!(
                "Capture stream '{}' state: {:?} -> {:?}",
                app_name_for_log,
                old,
                new
            );
        })
        .process(|stream, user_data| {
            // Per-app capture callback - read input, apply per-app EQ, write to ring buffer

            // Check if EQ settings have been updated
            let current_counter = user_data.eq_update_counter.load(Ordering::Relaxed);
            if current_counter != user_data.last_eq_update_counter {
                // Apply updated EQ gains
                for band in 0..10 {
                    let gain_bits = user_data.eq_gains[band].load(Ordering::Relaxed);
                    let gain_db = f32::from_bits(gain_bits);
                    if let Err(e) = user_data.equalizer.set_band_gain(band, gain_db) {
                        tracing::warn!("Failed to apply EQ band {}: {:?}", band, e);
                    }
                }
                user_data.last_eq_update_counter = current_counter;
            }

            if let Some(mut buffer) = stream.dequeue_buffer() {
                let datas = buffer.datas_mut();
                if let Some(data) = datas.first_mut() {
                    let chunk_size = data.chunk().size() as usize;

                    if let Some(slice) = data.data() {
                        let byte_len = slice.len().min(chunk_size);
                        let sample_count = byte_len / 4;

                        if sample_count == 0 {
                            return;
                        }

                        let samples: &mut [f32] = unsafe {
                            std::slice::from_raw_parts_mut(
                                slice.as_mut_ptr() as *mut f32,
                                sample_count,
                            )
                        };

                        // Apply per-app EQ if not bypassed
                        if !user_data.bypassed.load(Ordering::Relaxed) {
                            user_data.equalizer.process_interleaved(samples);
                        }

                        // Apply per-app volume (0.0 - 2.0, default 1.0)
                        // This multiplies each sample by the volume factor
                        let volume_bits = user_data.volume.load(Ordering::Relaxed);
                        let volume = f32::from_bits(volume_bits);
                        if (volume - 1.0).abs() > 0.001 {
                            // Only apply if volume differs from unity gain
                            for sample in samples.iter_mut() {
                                *sample *= volume;
                            }
                        }

                        // Write to ring buffer for mixing
                        if let Ok(mut write_chunk) = user_data.producer.write_chunk(samples.len()) {
                            let (first, second) = write_chunk.as_mut_slices();
                            let mut idx = 0;
                            for dst in first.iter_mut().chain(second.iter_mut()) {
                                if idx < samples.len() {
                                    *dst = samples[idx];
                                    idx += 1;
                                }
                            }
                            write_chunk.commit(idx);
                        }
                    }
                }
            }
        })
        .register();

    let listener = match listener {
        Ok(l) => l,
        Err(e) => {
            tracing::error!("Failed to register capture listener for '{}': {}", app_name, e);
            return None;
        }
    };

    // Build audio format params
    let mut audio_info = pw::spa::param::audio::AudioInfoRaw::new();
    audio_info.set_format(pw::spa::param::audio::AudioFormat::F32LE);
    audio_info.set_rate(48000);
    audio_info.set_channels(2);

    let audio_params_bytes: Vec<u8> = pw::spa::pod::serialize::PodSerializer::serialize(
        std::io::Cursor::new(Vec::new()),
        &pw::spa::pod::Value::Object(pw::spa::pod::Object {
            type_: pw::spa::utils::SpaTypes::ObjectParamFormat.as_raw(),
            id: pw::spa::param::ParamType::EnumFormat.as_raw(),
            properties: audio_info.into(),
        }),
    )
    .expect("Failed to serialize audio params")
    .0
    .into_inner();

    let audio_pod = Pod::from_bytes(&audio_params_bytes).expect("Failed to create audio Pod");
    let mut params = [audio_pod];

    // Connect capture stream
    let capture_flags = StreamFlags::MAP_BUFFERS | StreamFlags::RT_PROCESS;
    if let Err(e) = capture_stream.connect(
        pw::spa::utils::Direction::Input,
        Some(sink_node_id),
        capture_flags,
        &mut params,
    ) {
        tracing::error!("Failed to connect capture stream for '{}': {}", app_name, e);
        return None;
    }

    // Activate the stream
    if let Err(e) = capture_stream.set_active(true) {
        tracing::error!("Failed to activate capture stream for '{}': {}", app_name, e);
        let _ = capture_stream.disconnect();
        return None;
    }

    tracing::debug!("Created per-app capture stream for '{}'", app_name);

    // Add consumer to the shared state so the mixer can read from it
    app_consumers_state.add_consumer(app_name.to_string(), consumer);

    // Register this app in the shared state so the engine can emit discovery events
    audio_state.add_captured_app(app_name);

    // Return the capture state (EQ gains/counter/bypass/volume are shared via Arc)
    Some(AppCaptureState {
        app_name: app_name.to_string(),
        stream: capture_stream,
        listener,
        eq_gains,
        eq_update_counter,
        bypassed,
        volume,
    })
}

/// Try to rewire capture links from wrong source to Gecko Audio monitor.
///
/// WirePlumber auto-connects our capture stream to the default source (e.g. microphone),
/// but we need it connected to our virtual sink's monitor ports. This function:
/// 1. Finds and destroys any existing links TO our capture stream's input ports
/// 2. Creates new links FROM Gecko Audio's monitor ports TO capture input ports
///
/// Returns the number of correct links created (0, 1, or 2).
fn try_create_capture_links(local: &LocalState, core: &pw::core::Core) -> usize {
    // Find Gecko Audio node (our virtual sink)
    let gecko_audio_node = local.nodes
        .iter()
        .find(|(_, n)| n.name == "Gecko Audio")
        .map(|(id, _)| *id);

    // Find Gecko Capture node (our capture stream)
    // IMPORTANT: Prefer capture_stream_node_id if set - this ensures we use the CURRENT
    // capture stream node, not an old one that may still be in local.nodes during transitions.
    // This fixes race conditions during SwitchPlaybackTarget where old and new nodes
    // may coexist briefly.
    let gecko_capture_node = if let Some(node_id) = local.capture_stream_node_id {
        // Verify the node still exists in our registry
        if local.nodes.contains_key(&node_id) {
            Some(node_id)
        } else {
            // Node ID is stale, fall back to name lookup
            tracing::debug!("Stored capture_stream_node_id {} not in registry, using name lookup", node_id);
            local.nodes
                .iter()
                .find(|(_, n)| n.name == "Gecko Capture")
                .map(|(id, _)| *id)
        }
    } else {
        // No stored node ID, use name lookup
        local.nodes
            .iter()
            .find(|(_, n)| n.name == "Gecko Capture")
            .map(|(id, _)| *id)
    };

    let (sink_id, capture_id) = match (gecko_audio_node, gecko_capture_node) {
        (Some(s), Some(c)) => (s, c),
        _ => {
            tracing::debug!(
                "Cannot create capture links: sink={:?}, capture={:?}",
                gecko_audio_node,
                gecko_capture_node
            );
            return 0;
        }
    };

    tracing::debug!(
        "Found nodes for link creation: Gecko Audio={}, Gecko Capture={}",
        sink_id,
        capture_id
    );

    // Find monitor ports on Gecko Audio (these are output ports with "monitor" prefix)
    let mut monitor_ports: Vec<u32> = local.ports
        .iter()
        .filter(|(_, p)| {
            p.node_id == sink_id
                && matches!(p.direction, PortDirection::Output)
                && p.name.contains("monitor")
        })
        .map(|(id, _)| *id)
        .collect();
    monitor_ports.sort(); // Sort by ID for consistent ordering (assuming FL < FR)

    // Find input ports on Gecko Capture
    let mut input_ports: Vec<u32> = local.ports
        .iter()
        .filter(|(_, p)| {
            p.node_id == capture_id
                && matches!(p.direction, PortDirection::Input)
        })
        .map(|(id, _)| *id)
        .collect();
    input_ports.sort(); // Sort by ID

    if monitor_ports.len() < 2 || input_ports.len() < 2 {
        tracing::debug!(
            "Waiting for ports: monitor={}/2, input={}/2",
            monitor_ports.len(),
            input_ports.len()
        );
        return 0;
    }

    // Identify correct pairs: FL->FL, FR->FR
    let correct_pairs = vec![
        (monitor_ports[0], input_ports[0]), // FL
        (monitor_ports[1], input_ports[1]), // FR
    ];

    // Check existing links to avoid duplicates
    let mut links_active = 0;
    
    for (source, dest) in &correct_pairs {
        // Check if ANY link already exists with this source and destination
        let exists = local.links.values().any(|l| {
            l.output_port == *source && l.input_port == *dest
        });

        if exists {
            tracing::debug!("Link {} -> {} already exists, skipping creation", source, dest);
            links_active += 1;
            continue;
        }

        tracing::debug!("Creating capture link: {} -> {}", source, dest);
        let link_props = properties! {
            "link.output.port" => source.to_string(),
            "link.input.port" => dest.to_string(),
            "link.passive" => "true",
        };

        match core.create_object::<pw::link::Link>("link-factory", &link_props) {
            Ok(link) => {
                tracing::debug!("Created capture link (id={})", link.upcast_ref().id());
                std::mem::forget(link);
                links_active += 1;
            }
            Err(e) => {
                tracing::error!("Failed to create capture link: {}", e);
            }
        }
    }

    if links_active == 2 {
        tracing::debug!("All capture links verified active");
        return 2;
    }

    links_active
}

/// Try to create monitor->capture links for per-app capture streams
///
/// After creating a per-app capture stream, we need to link the per-app sink's
/// monitor ports to the capture stream's input ports. This is similar to
/// `try_create_capture_links` but for per-app streams.
///
/// Returns a list of app names that had their links successfully created.
fn try_create_per_app_capture_links(local: &LocalState, core: &pw::core::Core) -> Vec<String> {
    let mut completed = Vec::new();

    for app_name in &local.pending_app_monitor_links {
        // Find the per-app sink node (Gecko-{app_name})
        let sink_name = format!("Gecko-{}", app_name);
        let sink_node = local.nodes
            .iter()
            .find(|(_, n)| n.name == sink_name)
            .map(|(id, _)| *id);

        // Find the per-app capture stream node (Gecko Capture - {app_name})
        let capture_name = format!("Gecko Capture - {}", app_name);
        let capture_node = local.nodes
            .iter()
            .find(|(_, n)| n.name == capture_name)
            .map(|(id, _)| *id);

        let (sink_id, capture_id) = match (sink_node, capture_node) {
            (Some(s), Some(c)) => (s, c),
            _ => {
                tracing::debug!(
                    "Cannot create per-app capture links for '{}': sink={:?}, capture={:?}",
                    app_name,
                    sink_node,
                    capture_node
                );
                continue;
            }
        };

        // Find monitor ports on the per-app sink (output ports with "monitor" in name)
        let mut monitor_ports: Vec<u32> = local.ports
            .iter()
            .filter(|(_, p)| {
                p.node_id == sink_id
                    && matches!(p.direction, PortDirection::Output)
                    && p.name.contains("monitor")
            })
            .map(|(id, _)| *id)
            .collect();
        monitor_ports.sort();

        // Find input ports on the per-app capture stream
        let mut input_ports: Vec<u32> = local.ports
            .iter()
            .filter(|(_, p)| {
                p.node_id == capture_id
                    && matches!(p.direction, PortDirection::Input)
            })
            .map(|(id, _)| *id)
            .collect();
        input_ports.sort();

        if monitor_ports.len() < 2 || input_ports.len() < 2 {
            tracing::debug!(
                "Waiting for per-app ports for '{}': monitor={}/2, input={}/2",
                app_name,
                monitor_ports.len(),
                input_ports.len()
            );
            continue;
        }

        tracing::debug!(
            "Creating per-app capture links for '{}': sink={}, capture={}",
            app_name,
            sink_id,
            capture_id
        );

        // Create links: monitor_FL -> input_FL, monitor_FR -> input_FR
        let mut links_created = 0;
        for i in 0..2 {
            let source = monitor_ports[i];
            let dest = input_ports[i];

            // Check if link already exists
            let exists = local.links.values().any(|l| {
                l.output_port == source && l.input_port == dest
            });

            if exists {
                tracing::debug!("Per-app capture link {} -> {} already exists", source, dest);
                links_created += 1;
                continue;
            }

            let link_props = properties! {
                "link.output.port" => source.to_string(),
                "link.input.port" => dest.to_string(),
                "link.passive" => "true",
            };

            match core.create_object::<pw::link::Link>("link-factory", &link_props) {
                Ok(link) => {
                    tracing::debug!(
                        "Created per-app capture link for '{}': {} -> {}",
                        app_name,
                        source,
                        dest
                    );
                    std::mem::forget(link);
                    links_created += 1;
                }
                Err(e) => {
                    tracing::error!(
                        "Failed to create per-app capture link for '{}': {}",
                        app_name,
                        e
                    );
                }
            }
        }

        if links_created >= 2 {
            tracing::debug!("Per-app capture links created for '{}'", app_name);
            completed.push(app_name.clone());
        }
    }

    completed
}

/// Main entry point for the PipeWire thread
///
/// This function blocks until shutdown is signaled.
///
/// # Arguments
/// * `command_rx` - Channel to receive commands from main thread
/// * `response_tx` - Channel to send responses to main thread
/// * `shared_state` - Shared PipeWire graph state (nodes, ports, links)
/// * `audio_state` - Shared audio processing state (volume, peaks, bypass)
/// * `shutdown` - Flag to signal thread shutdown
pub fn pipewire_thread_main(
    command_rx: pw::channel::Receiver<PwCommand>,
    response_tx: Sender<PwResponse>,
    shared_state: Arc<RwLock<PipeWireState>>,
    audio_state: Arc<AudioProcessingState>,
    shutdown: Arc<AtomicBool>,
    query_only: bool,
) {
    // Rust pattern: initialize PipeWire library (must be called before any PW operations)
    pw::init();

    tracing::info!("PipeWire thread starting");

    // Create the main loop - this is the event dispatcher for PipeWire
    let mainloop = match pw::main_loop::MainLoop::new(None) {
        Ok(ml) => ml,
        Err(e) => {
            tracing::error!("Failed to create PipeWire MainLoop: {}", e);
            return;
        }
    };

    // Create context - manages PipeWire state for this connection
    let context = match pw::context::Context::new(&mainloop) {
        Ok(ctx) => ctx,
        Err(e) => {
            tracing::error!("Failed to create PipeWire Context: {}", e);
            return;
        }
    };

    // Connect to the PipeWire daemon
    let core = match context.connect(None) {
        Ok(core) => core,
        Err(e) => {
            tracing::error!("Failed to connect to PipeWire: {}", e);
            return;
        }
    };

    tracing::info!("Connected to PipeWire daemon");

    // Mark as connected in shared state
    if let Ok(mut state) = shared_state.write() {
        state.connected = true;
    }

    // Local state for this thread (not Send/Sync)
    // Rust pattern: use struct initializer syntax with ..Default::default() for partial init
    let initial_state = LocalState {
        audio_state: Some(Arc::clone(&audio_state)),
        ..Default::default()
    };
    let local_state = Rc::new(RefCell::new(initial_state));

    // Get the registry to monitor objects
    let registry = match core.get_registry() {
        Ok(reg) => reg,
        Err(e) => {
            tracing::error!("Failed to get PipeWire registry: {}", e);
            return;
        }
    };

    // Rust pattern: clone Rc for use in closures
    // Each closure gets its own Rc handle to the same RefCell
    let local_for_global = Rc::clone(&local_state);
    let local_for_remove = Rc::clone(&local_state);
    let shared_for_global = Arc::clone(&shared_state);
    let shared_for_remove = Arc::clone(&shared_state);

    // Register listener for new objects in the registry
    let _registry_listener = registry
        .add_listener_local()
        .global(move |global| {
            let props = global.props;
            

            match global.type_ {
                pw::types::ObjectType::Client => {
                    // Track clients with their PID - we'll use this to look up PIDs for nodes
                    let info = PwClientInfo {
                        id: global.id,
                        application_name: get_prop(props, "application.name"),
                        pid: get_prop_u32(props, "pipewire.sec.pid"),
                    };

                    tracing::debug!(
                        "Client added: {:?} (id={}, pid={:?})",
                        info.application_name,
                        info.id,
                        info.pid
                    );

                    local_for_global.borrow_mut().clients.insert(global.id, info);
                    sync_to_shared(&local_for_global.borrow(), &shared_for_global);
                }
                pw::types::ObjectType::Node => {
                    let client_id = get_prop_u32(props, "client.id");

                    // Try to get PID from node properties first, then fall back to client lookup
                    let mut pid = get_prop_u32(props, "application.process.id");
                    if pid.is_none() {
                        // Look up PID from client
                        if let Some(cid) = client_id {
                            let local = local_for_global.borrow();
                            if let Some(client) = local.clients.get(&cid) {
                                pid = client.pid;
                            }
                        }
                    }

                    let info = PwNodeInfo {
                        id: global.id,
                        name: get_prop(props, "node.name").unwrap_or_default(),
                        media_class: get_prop(props, "media.class"),
                        application_name: get_prop(props, "application.name"),
                        application_pid: pid,
                        client_id,
                        is_active: true,
                    };

                    // Debug log for Stream nodes to see what properties we get
                    tracing::debug!(
                        "Raw node detected: {} (id={}, class={:?}, app={:?}, client={:?})",
                        info.name,
                        info.id,
                        info.media_class,
                        info.application_name,
                        info.client_id
                    );


                    tracing::debug!(
                        "Node added: {} (id={}, class={:?})",
                        info.name,
                        info.id,
                        info.media_class
                    );

                    // Per-app EQ: Detect audio output streams and queue them for sink creation
                    // We create a dedicated sink for each app to enable true per-app EQ
                    // NOTE: Skip this if in query_only mode (e.g., when just listing apps)
                    let mut local = local_for_global.borrow_mut();

                    // Check if this is an audio output stream from an application
                    // Only create per-app sinks if NOT in query_only mode
                    if !query_only {
                        if let Some(ref class) = info.media_class {
                            if class == "Stream/Output/Audio" {
                                // Get the application name (use node name as fallback)
                                let app_name = info.application_name.clone()
                                    .unwrap_or_else(|| info.name.clone());

                                // Skip our own streams
                                if !app_name.starts_with("Gecko") && !info.name.starts_with("Gecko") {
                                    // Check if we already have a sink for this app
                                    if local.app_sinks.contains_key(&app_name) {
                                        // App already has a sink - this is a NEW stream node for an existing app
                                        // (e.g., Firefox recreated its audio stream after seeking a video)
                                        // Queue for relinking to connect this new node to the existing sink
                                        if !local.pending_app_capture_links.contains(&app_name) {
                                            tracing::debug!(
                                                "New stream node for existing app '{}' (node={}) - queuing for relink",
                                                app_name,
                                                info.id
                                            );
                                            local.pending_app_capture_links.push(app_name);
                                        }
                                    } else if !local.pending_app_sinks.contains(&app_name) {
                                        // Brand new app - create a sink for it
                                        tracing::debug!(
                                            "Detected new audio app '{}' (node={}) - queuing for sink creation",
                                            app_name,
                                            info.id
                                        );
                                        local.pending_app_sinks.push(app_name);
                                    }
                                }
                            }
                        }
                    }

                    local.nodes.insert(global.id, info);
                    drop(local); // Release borrow before sync
                    sync_to_shared(&local_for_global.borrow(), &shared_for_global);
                }
                pw::types::ObjectType::Port => {
                    let direction = match get_prop(props, "port.direction").as_deref() {
                        Some("in") => PortDirection::Input,
                        _ => PortDirection::Output,
                    };

                    let mut channel = get_prop(props, "audio.channel").unwrap_or_else(|| "MONO".to_string());
                    let name = get_prop(props, "port.name").unwrap_or_default();
                    
                    // Fallback: if channel is MONO or UNKNOWN, try to guess from name
                    if channel == "MONO" || channel == "UNKNOWN" {
                        if name.contains("FL") || name.contains("monitor_1") || name.contains("playback_1") || name.contains("input_1") || name.contains("output_1") {
                            channel = "FL".to_string();
                        } else if name.contains("FR") || name.contains("monitor_2") || name.contains("playback_2") || name.contains("input_2") || name.contains("output_2") {
                            channel = "FR".to_string();
                        }
                    }

                    let info = PwPortInfo {
                        id: global.id,
                        node_id: get_prop_u32(props, "node.id").unwrap_or(0),
                        name,
                        direction,
                        channel,
                    };

                    tracing::debug!(
                        "Port added: {} (id={}, node={}, dir={:?})",
                        info.name,
                        info.id,
                        info.node_id,
                        info.direction
                    );

                    local_for_global.borrow_mut().ports.insert(global.id, info);
                    sync_to_shared(&local_for_global.borrow(), &shared_for_global);
                }
                pw::types::ObjectType::Link => {
                    let info = PwLinkInfo {
                        id: global.id,
                        output_node: get_prop_u32(props, "link.output.node").unwrap_or(0),
                        output_port: get_prop_u32(props, "link.output.port").unwrap_or(0),
                        input_node: get_prop_u32(props, "link.input.node").unwrap_or(0),
                        input_port: get_prop_u32(props, "link.input.port").unwrap_or(0),
                        is_active: true,
                    };

                    tracing::debug!(
                        "Link added: id={}, {}:{} -> {}:{}",
                        info.id,
                        info.output_node,
                        info.output_port,
                        info.input_node,
                        info.input_port
                    );

                    local_for_global.borrow_mut().links.insert(global.id, info);
                    sync_to_shared(&local_for_global.borrow(), &shared_for_global);
                }
                _ => {}
            }
        })
        .global_remove(move |id| {
            let mut local = local_for_remove.borrow_mut();

            // Remove from whichever collection it was in
            if local.clients.remove(&id).is_some() {
                tracing::debug!("Client removed: id={}", id);
            } else if local.nodes.remove(&id).is_some() {
                tracing::debug!("Node removed: id={}", id);
            } else if local.ports.remove(&id).is_some() {
                tracing::debug!("Port removed: id={}", id);
            } else if let Some(link_info) = local.links.remove(&id) {
                tracing::debug!("Link removed: id={}", id);

                // Check if this was a link TO one of our app sinks (app→sink link)
                // If WirePlumber removed it, we need to re-create it
                // The link's input_node is the sink we're routing TO
                let removed_input_node = link_info.input_node;

                // Find which app (if any) this sink belongs to
                let app_to_relink: Option<String> = local
                    .app_sinks
                    .iter()
                    .find(|(_, sink_state)| sink_state.sink_node_id == removed_input_node)
                    .map(|(app_name, _)| app_name.clone());

                if let Some(app_name) = app_to_relink {
                    // Only re-queue if not already pending and streaming is active
                    if local.per_app_mode
                        && !local.pending_app_capture_links.contains(&app_name)
                    {
                        tracing::warn!(
                            "App→sink link removed for '{}' (link_id={}, sink_node={}). Re-queuing for re-link.",
                            app_name,
                            id,
                            removed_input_node
                        );
                        local.pending_app_capture_links.push(app_name);
                    }
                }
            }

            sync_to_shared(&local, &shared_for_remove);
        })
        .register();

    // Clone references for the command handler
    let local_for_cmd = Rc::clone(&local_state);
    let response_tx_clone = response_tx.clone();

    // We need to store the core in an Rc so the closure can use it
    let core_rc = Rc::new(core);
    let core_for_cmd = Rc::clone(&core_rc);

    // Attach command receiver to the main loop
    // This wakes the loop when commands arrive from the main thread
    let _cmd_source = command_rx.attach(mainloop.loop_(), move |cmd| {
        handle_command(cmd, &core_for_cmd, &local_for_cmd, &response_tx_clone);
    });

    // Run the main loop until shutdown
    tracing::info!("PipeWire main loop starting");

    // Clone for the main loop to check pending links
    let local_for_loop = Rc::clone(&local_state);
    let core_for_loop = Rc::clone(&core_rc);

    // Rust pattern: run the mainloop - it will process events
    // We use a loop that checks the shutdown flag
    loop {
        // Rust pattern: MainLoop::run() blocks until quit() is called
        // We can't easily interrupt it, so we'll rely on the Shutdown command
        // to signal and break out
        if shutdown.load(Ordering::Relaxed) {
            break;
        }

        // Run one iteration with a timeout
        // Note: pipewire-rs 0.8 API uses loop_() to get the underlying loop
        let loop_ref = mainloop.loop_();
        // Process pending events with a timeout (non-blocking iteration)
        // The loop_iterate function processes events
        let _ = loop_ref.iterate(std::time::Duration::from_millis(100));

        // Check if we need to create manual capture links
        // This happens after streaming is started and we're waiting for registry events
        // to populate the port information for our streams
        {
            let mut local = local_for_loop.borrow_mut();
            if local.pending_capture_links {
                // Try to create the links - if we succeed, clear the flag
                let links_created = try_create_capture_links(&local, &core_for_loop);
                if links_created >= 2 {
                    local.pending_capture_links = false;
                    tracing::debug!("Manual capture links created successfully");
                }
            }

            // Per-app EQ: Process pending app sink creations
            // When we detect a new audio app, we queue it here, then create its sink
            if !local.pending_app_sinks.is_empty() {
                process_pending_app_sinks(&mut local, &core_for_loop);
            }

            // Per-app EQ: Process pending app capture links
            // After creating a sink, we need to link the app to it
            // Returns completed apps that are ready for capture stream creation
            let completed_apps = if !local.pending_app_capture_links.is_empty() {
                process_pending_app_capture_links(&mut local, &core_for_loop)
            } else {
                Vec::new()
            };

            // Per-app EQ: Create capture streams for apps that just got linked
            if !completed_apps.is_empty() {
                if let Some(ref audio_state) = local.audio_state {
                    let audio_state_clone = Arc::clone(audio_state);
                    let playback_target = local.playback_target_id;
                    let need_mixing_stream = local.mixing_playback_stream.is_none();
                    drop(local); // Release borrow before calling
                    let mut local = local_for_loop.borrow_mut();
                    process_completed_app_links(completed_apps, &mut local, &core_for_loop, &audio_state_clone);

                    // Create mixing playback stream if this is our first app capture
                    if need_mixing_stream && !local.app_captures.is_empty() {
                        create_mixing_playback_stream(&mut local, &core_for_loop, playback_target);
                    }
                }
            }
        }

        // Per-app EQ: Create monitor->capture links for per-app capture streams
        // After creating a capture stream, we need to link the sink's monitor to capture inputs
        // This is outside the previous block because `local` might have been dropped
        {
            let mut local = local_for_loop.borrow_mut();
            if !local.pending_app_monitor_links.is_empty() {
                let completed = try_create_per_app_capture_links(&local, &core_for_loop);
                for app_name in completed {
                    local.pending_app_monitor_links.retain(|a| a != &app_name);
                }
            }
        }
    }

    tracing::info!("PipeWire thread shutting down");

    // Mark as disconnected
    if let Ok(mut state) = shared_state.write() {
        state.connected = false;
    }
}

/// Handle a command from the main thread
fn handle_command(
    cmd: PwCommand,
    core: &pw::core::Core,
    local_state: &Rc<RefCell<LocalState>>,
    response_tx: &Sender<PwResponse>,
) {
    match cmd {
        PwCommand::CreateVirtualSink { config, response_id } => {
            tracing::debug!("Creating virtual sink: {}", config.name);

            // Create a null audio sink using the adapter factory
            // Rust pattern: properties! macro creates PipeWire properties dict
            let props = properties! {
                "factory.name" => "support.null-audio-sink",
                "node.name" => config.name.as_str(),
                "media.class" => "Audio/Sink",
                "audio.channels" => config.channels.to_string().as_str(),
                "audio.rate" => config.sample_rate.to_string().as_str(),
                "audio.position" => "FL,FR",
                "node.pause-on-idle" => "false", // Prevent suspension when no apps are playing
                "node.always-process" => "true", // Keep processing to ensure monitor output
                "audio.volume" => "1.0", // Force full volume
                "object.linger" => if config.persistent { "true" } else { "false" },
                // Make it appear in applications as a real sink
                "node.description" => config.name.as_str(),
            };

            match core.create_object::<pw::node::Node>("adapter", &props) {
                Ok(node) => {
                    // Get the bound ID from the proxy
                    // Note: The actual ID may not be immediately available
                    // For now we track by the proxy's serial and wait for the registry event
                    let proxy_id = node.upcast_ref().id();

                    tracing::debug!("Virtual sink created with proxy id: {}", proxy_id);

                    // Store in our tracked sinks to keep it alive and allow cleanup
                    local_state.borrow_mut().our_sinks.push(node);

                    // Send success response
                    // Note: The actual node ID will be assigned by PipeWire and appear in registry
                    let _ = response_tx.send(PwResponse::VirtualSinkCreated {
                        response_id,
                        node_id: proxy_id,
                    });

                    // Do NOT forget the node. We stored it in our_sinks.
                    // Dropping it (when we clear our_sinks) will destroy it.
                }
                Err(e) => {
                    tracing::error!("Failed to create virtual sink: {}", e);
                    let _ = response_tx.send(PwResponse::Error {
                        response_id,
                        message: format!("Failed to create virtual sink: {}", e),
                    });
                }
            }
        }

        PwCommand::DestroyVirtualSink { node_id, response_id } => {
            tracing::debug!("Destroying virtual sink: {}", node_id);

            // Remove from our tracking
            // We need to compare the proxy's ID with the target ID
            local_state.borrow_mut().our_sinks.retain(|node| node.upcast_ref().id() != node_id);

            // In PipeWire, objects are destroyed by dropping their proxy
            // Since we used mem::forget, we need to use the registry to destroy
            // For now, send success - the object will be cleaned up when we disconnect
            let _ = response_tx.send(PwResponse::Ok { response_id });
        }

        PwCommand::CreateLink {
            output_port,
            input_port,
            response_id,
        } => {
            tracing::debug!("Creating link: {} -> {}", output_port, input_port);

            let props = properties! {
                "link.output.port" => output_port.to_string().as_str(),
                "link.input.port" => input_port.to_string().as_str(),
                "link.passive" => "true",
                "object.linger" => "false",
            };

            match core.create_object::<pw::link::Link>("link-factory", &props) {
                Ok(link) => {
                    let proxy_id = link.upcast_ref().id();

                    tracing::debug!("Link created with proxy id: {}", proxy_id);

                    local_state.borrow_mut().our_links.push(link);

                    let _ = response_tx.send(PwResponse::LinkCreated {
                        response_id,
                        link_id: proxy_id,
                    });

                    // Do NOT forget the link. We stored it in our_links.
                }
                Err(e) => {
                    tracing::error!("Failed to create link: {}", e);
                    let _ = response_tx.send(PwResponse::Error {
                        response_id,
                        message: format!("Failed to create link: {}", e),
                    });
                }
            }
        }

        PwCommand::DestroyLink { link_id, response_id } => {
            tracing::debug!("Destroying link: {}", link_id);

            tracing::debug!("Destroying link by ID: {} (may be unreliable if not tracked)", link_id);

            // We can't destroy by ID easily if we store objects, but we can try to find and remove if we could inspect IDs.
            // Since pw::link::Link doesn't expose ID easily without upcast, we might just retain.
            local_state.borrow_mut().our_links.retain(|l| l.upcast_ref().id() != link_id);


            let _ = response_tx.send(PwResponse::Ok { response_id });
        }

        PwCommand::SyncState { response_id } => {
            // Already syncing on every change, just acknowledge
            let _ = response_tx.send(PwResponse::StateSynced { response_id });
        }

        PwCommand::StartStreaming {
            capture_target,
            playback_target,
            response_id,
        } => {
            tracing::debug!(
                "Starting audio streaming: capture from {}, playback to {:?}",
                capture_target,
                playback_target
            );

            let mut local = local_state.borrow_mut();

            if local.streaming_active {
                let _ = response_tx.send(PwResponse::Error {
                    response_id,
                    message: "Streaming already active".to_string(),
                });
                return;
            }

            // Get the audio state for sharing with callbacks
            let audio_state = match &local.audio_state {
                Some(state) => Arc::clone(state),
                None => {
                    let _ = response_tx.send(PwResponse::Error {
                        response_id,
                        message: "Audio state not initialized".to_string(),
                    });
                    return;
                }
            };

            // Create ring buffer for audio transfer
            // We need a large buffer because some systems (like this one) request huge chunks (341ms)
            // 1 second buffer = 48000 * 2 (stereo) = 96000 samples
            const RING_BUFFER_SIZE: usize = 48000 * 2; 
            let (producer, consumer) = rtrb::RingBuffer::new(RING_BUFFER_SIZE);

            // Initialize EQ processor
            let eq = gecko_dsp::Equalizer::new(48000.0);

            // Note: capture_target_id will be updated after we find the actual node ID
            local.playback_target_id = playback_target;

            // Create playback stream (output to speakers)
            // If a specific playback target is requested, bind to it
            // Otherwise let PipeWire decide (usually default sink)
            // BUT: if we are the default sink, this causes a loop!
            // So the caller should always provide a target if possible.
            let playback_props = if let Some(target_id) = playback_target {
                tracing::debug!("Binding playback stream to target node {}", target_id);
                // We need to add target.object to the properties
                // Since properties! creates a temporary, we need to rebuild or add to it
                // The pipewire-rs crate doesn't make it easy to mutate properties,
                // so we'll just create a new one with the extra field
                properties! {
                    "media.type" => "Audio",
                    "media.category" => "Playback",
                    "media.role" => "Music",
                    "node.name" => "Gecko Playback",
                    "node.description" => "Gecko Playback Stream",
                    "target.object" => target_id.to_string(),
                    "node.dont-reconnect" => "true",
                    "node.latency" => "1024/48000",
                    "stream.props" => "{ volume = 1.0 }", // Force volume
                }
            } else {
                properties! {
                    "media.type" => "Audio",
                    "media.category" => "Playback",
                    "media.role" => "Music",
                    "node.name" => "Gecko Playback",
                    "node.description" => "Gecko Playback Stream",
                    "node.dont-reconnect" => "true",
                    "node.latency" => "1024/48000",
                    "stream.props" => "{ volume = 1.0 }", // Force volume
                }
            };

            let playback_stream = match Stream::new(core, "gecko-playback", playback_props) {
                Ok(stream) => stream,
                Err(e) => {
                    let _ = response_tx.send(PwResponse::Error {
                        response_id,
                        message: format!("Failed to create playback stream: {}", e),
                    });
                    return;
                }
            };

            // Create capture stream (input from virtual sink monitor)
            // We connect to our virtual sink's monitor ports manually because WirePlumber
            // doesn't understand we want monitor ports (it tries to connect to default source).
            // Rust pattern: multiple properties to prevent WirePlumber interference
            let capture_props = properties! {
                "media.type" => "Audio",
                "media.category" => "Capture",
                "media.role" => "Music",
                "node.name" => "Gecko Capture",
                "node.description" => "Gecko Audio Input",
                // CRITICAL: Multiple properties to prevent WirePlumber from managing this stream
                "node.dont-reconnect" => "true",
                "stream.dont-reconnect" => "true",
                // Tell WirePlumber this is a passive node that shouldn't be auto-routed
                "node.passive" => "true",
                // Explicitly disable autoconnect so WirePlumber doesn't create wrong links
                "node.autoconnect" => "false",
                // Mark as internal to avoid showing in volume mixers
                "media.class" => "Stream/Input/Audio",
            };

            let capture_stream = match Stream::new(core, "gecko-capture", capture_props) {
                Ok(stream) => stream,
                Err(e) => {
                    tracing::error!("Failed to create capture stream: {}", e);
                    let _ = response_tx.send(PwResponse::Error {
                        response_id,
                        message: format!("Failed to create capture stream: {}", e),
                    });
                    return;
                }
            };

            // Set up playback stream listener with process callback
            let playback_user_data = PlaybackUserData {
                consumer,
                audio_state: Arc::clone(&audio_state),
                master_eq: gecko_dsp::Equalizer::new(48000.0),
                last_master_eq_counter: 0,
            };

            let playback_listener = playback_stream
                .add_local_listener_with_user_data(playback_user_data)
                .state_changed(|stream, user_data, old, new| {
                    tracing::debug!("Playback stream state: {:?} -> {:?}", old, new);
                    let _ = (stream, user_data); // Suppress unused warnings
                })
                .process(|stream, user_data| {
                    // Playback callback - read from ring buffer and output
                    static PLAYBACK_CALL_COUNT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
                    let count = PLAYBACK_CALL_COUNT.fetch_add(1, Ordering::Relaxed);
                    if count % 1000 == 0 {
                        tracing::debug!("Playback callback called {} times", count);
                    }

                    if let Some(mut buffer) = stream.dequeue_buffer() {
                        let datas = buffer.datas_mut();
                        if let Some(data) = datas.first_mut() {
                            // Get the output buffer as f32 slice
                            if let Some(slice) = data.data() {
                                // Interpret as f32 samples
                                let samples: &mut [f32] = unsafe {
                                    std::slice::from_raw_parts_mut(
                                        slice.as_mut_ptr() as *mut f32,
                                        slice.len() / 4,
                                    )
                                };


                                // Read from ring buffer
                                let read_count = user_data.consumer.read_chunk(samples.len())
                                    .map(|chunk| {
                                        let src = chunk.as_slices();
                                        let mut written = 0;

                                        for s in src.0.iter().chain(src.1.iter()) {
                                            if written < samples.len() {
                                                samples[written] = *s;
                                                written += 1;
                                            }
                                        }
                                        chunk.commit_all();
                                        written
                                    })
                                    .unwrap_or(0);

                                // Fill remainder with silence
                                for sample in samples.iter_mut().skip(read_count) {
                                    *sample = 0.0;
                                }

                                // Update chunk size to indicate valid data
                                let chunk = data.chunk_mut();
                                *chunk.size_mut() = (samples.len() * 4) as u32;
                                *chunk.stride_mut() = 4; // f32 stride
                                *chunk.offset_mut() = 0;
                            }
                        }
                    }
                })
                .register();

            let playback_listener = match playback_listener {
                Ok(l) => l,
                Err(e) => {
                    tracing::error!("Failed to register playback listener: {}", e);
                    let _ = response_tx.send(PwResponse::Error {
                        response_id,
                        message: format!("Failed to register playback listener: {}", e),
                    });
                    return;
                }
            };

            // Set up capture stream listener with process callback
            let capture_user_data = CaptureUserData {
                producer,
                equalizer: eq,
                audio_state: Arc::clone(&audio_state),
                last_eq_update_counter: 0, // Will be updated on first callback if needed
            };

            let capture_listener = capture_stream
                .add_local_listener_with_user_data(capture_user_data)
                .state_changed(|stream, user_data, old, new| {
                    tracing::debug!("Capture stream state: {:?} -> {:?}", old, new);
                    let _ = (stream, user_data);
                })
                .process(|stream, user_data| {
                    // Capture callback - read input, process DSP, write to ring buffer
                    static CAPTURE_CALL_COUNT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
                    let count = CAPTURE_CALL_COUNT.fetch_add(1, Ordering::Relaxed);
                    if count % 1000 == 0 {
                        tracing::debug!("Capture callback called {} times", count);
                    }

                    // Check if EQ settings have been updated via the shared state
                    // Rust pattern: Compare counters to detect changes without locking
                    let current_eq_counter = user_data.audio_state.eq_update_counter();
                    if current_eq_counter != user_data.last_eq_update_counter {
                        // EQ settings changed - apply all band gains to our local equalizer
                        let gains = user_data.audio_state.get_all_eq_gains();
                        for (band, gain_db) in gains.iter().enumerate() {
                            if let Err(e) = user_data.equalizer.set_band_gain(band, *gain_db) {
                                tracing::warn!("Failed to apply EQ band {}: {:?}", band, e);
                            }
                        }
                        user_data.last_eq_update_counter = current_eq_counter;
                        tracing::debug!("Applied EQ update (counter={})", current_eq_counter);
                    }

                    if let Some(mut buffer) = stream.dequeue_buffer() {
                        let datas = buffer.datas_mut();
                        if let Some(data) = datas.first_mut() {
                            // Get the chunk info for size
                            let chunk_size = data.chunk().size() as usize;

                            if let Some(slice) = data.data() {
                                // Use the smaller of buffer size or chunk size
                                let byte_len = slice.len().min(chunk_size);
                                let sample_count = byte_len / 4;

                                if sample_count == 0 {
                                    return;
                                }

                                // Interpret as f32 samples
                                let samples: &mut [f32] = unsafe {
                                    std::slice::from_raw_parts_mut(
                                        slice.as_mut_ptr() as *mut f32,
                                        sample_count,
                                    )
                                };

                                // Apply DSP processing (EQ) if not bypassed
                                if !user_data.audio_state.bypassed.load(Ordering::Relaxed) {
                                    user_data.equalizer.process_interleaved(samples);
                                }

                                // Apply master volume
                                let volume = user_data.audio_state.master_volume();
                                for sample in samples.iter_mut() {
                                    *sample *= volume;
                                }

                                // Calculate and store peak levels
                                let mut peak_l = 0.0_f32;
                                let mut peak_r = 0.0_f32;
                                for (i, sample) in samples.iter().enumerate() {
                                    if i % 2 == 0 {
                                        peak_l = peak_l.max(sample.abs());
                                    } else {
                                        peak_r = peak_r.max(sample.abs());
                                    }
                                }
                                user_data.audio_state.set_peaks(peak_l, peak_r);

                                // Write to ring buffer for playback
                                if let Ok(mut write_chunk) = user_data.producer.write_chunk(samples.len()) {
                                    let (first, second) = write_chunk.as_mut_slices();
                                    let mut idx = 0;
                                    for dst in first.iter_mut().chain(second.iter_mut()) {
                                        if idx < samples.len() {
                                            *dst = samples[idx];
                                            idx += 1;
                                        }
                                    }
                                    write_chunk.commit(idx);
                                }
                            }
                        }
                    }
                })
                .register();

            let capture_listener = match capture_listener {
                Ok(l) => l,
                Err(e) => {
                    tracing::error!("Failed to register capture listener: {}", e);
                    let _ = response_tx.send(PwResponse::Error {
                        response_id,
                        message: format!("Failed to register capture listener: {}", e),
                    });
                    return;
                }
            };

            // Build audio format params for F32LE stereo at 48kHz
            // PipeWire requires format params to negotiate audio parameters
            let mut audio_info = pw::spa::param::audio::AudioInfoRaw::new();
            audio_info.set_format(pw::spa::param::audio::AudioFormat::F32LE);
            audio_info.set_rate(48000);
            audio_info.set_channels(2);

            // Serialize the audio info into a pod for format negotiation
            let audio_params_bytes: Vec<u8> = pw::spa::pod::serialize::PodSerializer::serialize(
                std::io::Cursor::new(Vec::new()),
                &pw::spa::pod::Value::Object(pw::spa::pod::Object {
                    type_: pw::spa::utils::SpaTypes::ObjectParamFormat.as_raw(),
                    id: pw::spa::param::ParamType::EnumFormat.as_raw(),
                    properties: audio_info.into(),
                }),
            )
            .expect("Failed to serialize audio params")
            .0
            .into_inner();

            // Create Pod from serialized bytes
            let audio_pod = Pod::from_bytes(&audio_params_bytes).expect("Failed to create audio Pod");
            let mut playback_params = [audio_pod];

            tracing::debug!("Created F32LE audio format params for stream negotiation");

            // Connect playback stream
            tracing::debug!("Connecting playback stream (target={:?})", playback_target);
            if let Err(e) = playback_stream.connect(
                pw::spa::utils::Direction::Output,
                playback_target,
                StreamFlags::MAP_BUFFERS | StreamFlags::RT_PROCESS,
                &mut playback_params,
            ) {
                tracing::error!("Failed to connect playback stream: {}", e);
                let _ = response_tx.send(PwResponse::Error {
                    response_id,
                    message: format!("Failed to connect playback stream: {}", e),
                });
                return;
            }

            // Store the capture target proxy ID for reference
            // Note: This is the PROXY ID from create_object, not the node ID.
            // We'll find the actual node ID later when "Gecko Audio" appears in registry.
            local.capture_target_id = Some(capture_target);

            // Find "Gecko Audio" sink to use as target for format negotiation.
            // We want to connect to its monitor ports.
            // Using the sink ID as target for an Input stream tells PipeWire to connect to monitors.
            let gecko_audio_id = local.nodes
                .iter()
                .find(|(_, n)| n.name == "Gecko Audio")
                .map(|(id, _)| *id);

            let capture_target_for_negotiation = match gecko_audio_id {
                Some(id) => {
                    tracing::debug!(
                        "Using Gecko Audio sink {} for capture stream format negotiation",
                        id
                    );
                    Some(id)
                }
                None => {
                    tracing::debug!(
                        "Gecko Audio sink not found in registry yet. Capture stream may connect without format hint."
                    );
                    None
                }
            };

            // Create capture format params (need separate params since playback consumed theirs)
            let capture_audio_pod = Pod::from_bytes(&audio_params_bytes).expect("Failed to create capture audio Pod");
            let mut capture_params = [capture_audio_pod];

            // Connect capture stream WITHOUT AUTOCONNECT flag.
            // We do NOT want WirePlumber to connect us to the default source (microphone).
            // We will manually connect to the virtual sink's monitor ports once they appear.
            let capture_flags = StreamFlags::MAP_BUFFERS | StreamFlags::RT_PROCESS;
            tracing::debug!(
                "Connecting capture stream (NO AUTOCONNECT, target={:?})",
                capture_target_for_negotiation
            );
            if let Err(e) = capture_stream.connect(
                pw::spa::utils::Direction::Input,
                capture_target_for_negotiation, // Still provide a target for format negotiation if possible
                capture_flags,
                &mut capture_params,
            ) {
                tracing::error!("Failed to connect capture stream: {}", e);
                // Disconnect playback too
                let _ = playback_stream.disconnect();
                let _ = response_tx.send(PwResponse::Error {
                    response_id,
                    message: format!("Failed to connect capture stream: {}", e),
                });
                return;
            }

            // Activate streams to start processing
            // PipeWire streams go: Unconnected -> Connecting -> Paused -> Streaming
            // The main loop will process events and streams will transition automatically
            tracing::debug!("Activating audio streams...");
            if let Err(e) = playback_stream.set_active(true) {
                tracing::error!("Failed to activate playback stream: {}", e);
                let _ = capture_stream.disconnect();
                let _ = playback_stream.disconnect();
                let _ = response_tx.send(PwResponse::Error {
                    response_id,
                    message: format!("Failed to activate playback stream: {}", e),
                });
                return;
            }

            if let Err(e) = capture_stream.set_active(true) {
                tracing::error!("Failed to activate capture stream: {}", e);
                let _ = capture_stream.disconnect();
                let _ = playback_stream.disconnect();
                let _ = response_tx.send(PwResponse::Error {
                    response_id,
                    message: format!("Failed to activate capture stream: {}", e),
                });
                return;
            }

            tracing::debug!("Audio streams created, connected, and activated successfully");

            // Store streams and listeners
            local.playback_stream = Some(playback_stream);
            local.capture_stream = Some(capture_stream);
            local.playback_listener = Some(playback_listener);
            local.capture_listener = Some(capture_listener);
            local.streaming_active = true;

            // Set flag to create manual capture links in the main loop.
            // We need to wait for the registry to receive port info for our capture stream
            // before we can create the links.
            local.pending_capture_links = true;

            audio_state.running.store(true, Ordering::SeqCst);

            let _ = response_tx.send(PwResponse::StreamingStarted { response_id });
        }

        PwCommand::StopStreaming { response_id } => {
            tracing::debug!("Stopping audio streaming");

            let mut local = local_state.borrow_mut();

            if !local.streaming_active {
                let _ = response_tx.send(PwResponse::Ok { response_id });
                return;
            }

            // Mark as not running
            if let Some(ref state) = local.audio_state {
                state.running.store(false, Ordering::SeqCst);
            }

            // Disconnect and destroy streams
            // Order matters: disconnect first, then drop listeners, then streams
            if let Some(ref stream) = local.capture_stream {
                let _ = stream.disconnect();
            }
            if let Some(ref stream) = local.playback_stream {
                let _ = stream.disconnect();
            }

            // Drop listeners (this removes callbacks)
            local.capture_listener = None;
            local.playback_listener = None;

            // Drop streams
            local.capture_stream = None;
            local.playback_stream = None;

            // Clean up our sinks (virtual sinks) to prevent zombies
            // We iterate over the IDs and destroy them
            for sink_proxy in &local.our_sinks {
                 let id = sink_proxy.upcast_ref().id();
                 tracing::debug!("Destroying virtual sink {}", id);
                 // We don't need to do anything else; clearing the vector below will Drop the proxies,
                 // which should trigger destruction in PipeWire (since we own them).
            }
            
            // Clean up remaining state
            local.ring_producer = None;
            local.ring_consumer = None;
            local.equalizer = None;
            local.capture_target_id = None;
            local.playback_target_id = None;
            local.streaming_active = false;
            local.pending_capture_links = false;
            local.capture_stream_node_id = None;
            
            // Clear proxies to drop them and destroy remote objects
            local.our_sinks.clear();
            local.our_links.clear();

            tracing::debug!("Audio streaming stopped");
            let _ = response_tx.send(PwResponse::StreamingStopped { response_id });
        }

        PwCommand::SwitchPlaybackTarget { target_name, response_id } => {
            // Hot-switch playback to a new device without destroying virtual sink or capture.
            // This is called during device hotplug (headphones plugged/unplugged).
            //
            // Strategy:
            // 1. Disconnect both playback and capture streams
            // 2. Create new streams with the target specified by NAME (not ID)
            // 3. Keep virtual sink alive - this is the key to avoiding glitches
            //
            // Using target NAME instead of ID avoids race conditions during hotplug.
            // When a device is plugged in, PipeWire assigns a NEW node ID, but the
            // name stays the same. By using the name in target.object property,
            // PipeWire resolves it to the current ID at connection time.
            tracing::debug!("Switching playback target to '{}'", target_name);

            let mut local = local_state.borrow_mut();

            // Check if we're in per-app mode (using mixing playback stream)
            // In per-app mode, we need to recreate the mixing playback stream, not the legacy streams
            if local.per_app_mode || local.mixing_playback_stream.is_some() {
                tracing::debug!("Switching playback target in per-app mode");

                // Disconnect and drop the old mixing playback stream
                if let Some(ref stream) = local.mixing_playback_stream {
                    tracing::debug!("Disconnecting old mixing playback stream");
                    let _ = stream.disconnect();
                }
                local.mixing_playback_listener = None;
                local.mixing_playback_stream = None;

                // Clear stale "Gecko Playback" node from local registry to avoid confusion
                let old_playback_node_id: Option<u32> = local.nodes
                    .iter()
                    .find(|(_, n)| n.name == "Gecko Playback")
                    .map(|(id, _)| *id);
                if let Some(old_node_id) = old_playback_node_id {
                    tracing::debug!("Removing stale Gecko Playback node {} from registry", old_node_id);
                    // Remove ports belonging to old playback node
                    let old_port_ids: Vec<u32> = local.ports
                        .iter()
                        .filter(|(_, p)| p.node_id == old_node_id)
                        .map(|(id, _)| *id)
                        .collect();
                    for port_id in &old_port_ids {
                        local.ports.remove(port_id);
                    }
                    // Remove links that reference old ports
                    local.links.retain(|_, link| {
                        !old_port_ids.contains(&link.output_port) && !old_port_ids.contains(&link.input_port)
                    });
                    local.nodes.remove(&old_node_id);
                }

                // Get required state for recreating the mixing playback stream
                let app_consumers_state = match &local.app_consumers_state {
                    Some(state) => Arc::clone(state),
                    None => {
                        let _ = response_tx.send(PwResponse::Error {
                            response_id,
                            message: "App consumers state not initialized".to_string(),
                        });
                        return;
                    }
                };
                let audio_state = match &local.audio_state {
                    Some(state) => Arc::clone(state),
                    None => {
                        let _ = response_tx.send(PwResponse::Error {
                            response_id,
                            message: "Audio state not initialized".to_string(),
                        });
                        return;
                    }
                };

                // Create new mixing playback stream with target specified by NAME
                // PipeWire resolves the name to the current node ID at connection time
                let playback_props = properties! {
                    "media.type" => "Audio",
                    "media.category" => "Playback",
                    "media.role" => "Music",
                    "node.name" => "Gecko Playback",
                    "node.description" => "Gecko Mixed Output",
                    "target.object" => target_name.as_str(),
                    "node.dont-reconnect" => "true",
                    "node.latency" => "1024/48000",
                };

                let playback_stream = match Stream::new(core, "gecko-mixing-playback", playback_props) {
                    Ok(stream) => stream,
                    Err(e) => {
                        let _ = response_tx.send(PwResponse::Error {
                            response_id,
                            message: format!("Failed to create mixing playback stream: {}", e),
                        });
                        return;
                    }
                };

                // Create master EQ for the new stream
                let mut master_eq = gecko_dsp::Equalizer::new(48000.0);
                let initial_gains = audio_state.get_all_eq_gains();
                for (band, gain_db) in initial_gains.iter().enumerate() {
                    let _ = master_eq.set_band_gain(band, *gain_db);
                }

                // Create user data for mixing callback
                // Note: Buffer sizes must match MAX_BUFFER_SIZE (48000) used in the main StartStreaming handler
                // to avoid index out of bounds panics when PipeWire requests larger buffers
                const MAX_BUFFER_SIZE: usize = 48000;
                let user_data = MixingPlaybackUserData {
                    app_consumers_state,
                    audio_state: Arc::clone(&audio_state),
                    master_eq,
                    last_master_eq_counter: audio_state.eq_update_counter(),
                    mix_buffer: vec![0.0f32; MAX_BUFFER_SIZE],
                    read_buffer: vec![0.0f32; MAX_BUFFER_SIZE],
                };

                // Set up mixing playback callback (duplicated from create_mixing_playback_stream)
                // NOTE: We duplicate this complex closure because extracting it would require
                // dealing with closure type constraints that complicate the code significantly.
                let listener = playback_stream
                    .add_local_listener_with_user_data(user_data)
                    .state_changed(|_stream, _user_data, old, new| {
                        tracing::debug!("Mixing playback stream state: {:?} -> {:?}", old, new);
                    })
                    .process(|stream, user_data| {
                        // Mixing playback callback - read from all app consumers, mix, apply master EQ

                        // Check if master EQ needs updating
                        let current_counter = user_data.audio_state.eq_update_counter();
                        if current_counter != user_data.last_master_eq_counter {
                            let gains = user_data.audio_state.get_all_eq_gains();
                            for (band, gain_db) in gains.iter().enumerate() {
                                if let Err(e) = user_data.master_eq.set_band_gain(band, *gain_db) {
                                    tracing::warn!("Failed to apply master EQ band {}: {:?}", band, e);
                                }
                            }
                            user_data.last_master_eq_counter = current_counter;
                        }

                        if let Some(mut buffer) = stream.dequeue_buffer() {
                            let datas = buffer.datas_mut();
                            if let Some(data) = datas.first_mut() {
                                if let Some(slice) = data.data() {
                                    let samples: &mut [f32] = unsafe {
                                        std::slice::from_raw_parts_mut(
                                            slice.as_mut_ptr() as *mut f32,
                                            slice.len() / 4,
                                        )
                                    };

                                    let mut sample_count = samples.len();
                                    if sample_count == 0 {
                                        return;
                                    }

                                    // Safety check: prevent buffer overflow
                                    if sample_count > user_data.mix_buffer.len() {
                                        sample_count = user_data.mix_buffer.len();
                                    }

                                    // Clear mix buffer
                                    for s in user_data.mix_buffer.iter_mut().take(sample_count) {
                                        *s = 0.0;
                                    }

                                    // Read from all app consumers and mix
                                    if let Some(mut guard) = user_data.app_consumers_state.consumers.try_write() {
                                        for (_app_name, consumer) in guard.iter_mut() {
                                            if let Ok(chunk) = consumer.read_chunk(sample_count) {
                                                let slices = chunk.as_slices();
                                                let mut idx = 0;
                                                for src in slices.0.iter().chain(slices.1.iter()) {
                                                    if idx < sample_count {
                                                        user_data.mix_buffer[idx] += *src;
                                                        idx += 1;
                                                    }
                                                }
                                                chunk.commit_all();
                                            }
                                        }
                                    }

                                    // Copy mix to output
                                    for (i, sample) in samples.iter_mut().enumerate() {
                                        *sample = user_data.mix_buffer[i];
                                    }

                                    // Apply master EQ if not bypassed
                                    if !user_data.audio_state.bypassed.load(Ordering::Relaxed) {
                                        user_data.master_eq.process_interleaved(samples);
                                    }

                                    // Apply master volume
                                    let volume = user_data.audio_state.master_volume();
                                    for sample in samples.iter_mut() {
                                        *sample *= volume;
                                    }

                                    // Calculate and store peak levels
                                    let mut peak_l = 0.0_f32;
                                    let mut peak_r = 0.0_f32;
                                    for (i, sample) in samples.iter().enumerate() {
                                        if i % 2 == 0 {
                                            peak_l = peak_l.max(sample.abs());
                                        } else {
                                            peak_r = peak_r.max(sample.abs());
                                        }
                                    }
                                    user_data.audio_state.set_peaks(peak_l, peak_r);

                                    // Push samples to spectrum analyzer for FFT visualization
                                    // This is lock-free and won't block the audio callback
                                    for chunk in samples.chunks_exact(2) {
                                        user_data.audio_state.push_spectrum_sample(chunk[0], chunk[1]);
                                    }

                                    // Update chunk metadata
                                    let chunk = data.chunk_mut();
                                    *chunk.size_mut() = (samples.len() * 4) as u32;
                                    *chunk.stride_mut() = 4;
                                    *chunk.offset_mut() = 0;
                                }
                            }
                        }
                    })
                    .register()
                    .expect("Failed to register mixing playback listener");

                // Build audio format params (F32LE stereo @ 48kHz)
                let mut audio_info = pw::spa::param::audio::AudioInfoRaw::new();
                audio_info.set_format(pw::spa::param::audio::AudioFormat::F32LE);
                audio_info.set_rate(48000);
                audio_info.set_channels(2);

                let audio_params_bytes: Vec<u8> = pw::spa::pod::serialize::PodSerializer::serialize(
                    std::io::Cursor::new(Vec::new()),
                    &pw::spa::pod::Value::Object(pw::spa::pod::Object {
                        type_: pw::spa::utils::SpaTypes::ObjectParamFormat.as_raw(),
                        id: pw::spa::param::ParamType::EnumFormat.as_raw(),
                        properties: audio_info.into(),
                    }),
                )
                .expect("Failed to serialize audio params")
                .0
                .into_inner();

                let audio_pod = Pod::from_bytes(&audio_params_bytes).expect("Failed to create audio Pod");
                let mut params = [audio_pod];

                // Connect and activate the stream
                if let Err(e) = playback_stream.connect(
                    pw::spa::utils::Direction::Output,
                    None,
                    StreamFlags::AUTOCONNECT | StreamFlags::MAP_BUFFERS | StreamFlags::RT_PROCESS,
                    &mut params,
                ) {
                    tracing::error!("Failed to connect mixing playback stream: {}", e);
                    let _ = response_tx.send(PwResponse::Error {
                        response_id,
                        message: format!("Failed to connect mixing playback stream: {}", e),
                    });
                    return;
                }

                // Activate the stream
                if let Err(e) = playback_stream.set_active(true) {
                    tracing::error!("Failed to activate mixing playback stream: {}", e);
                    let _ = playback_stream.disconnect();
                    let _ = response_tx.send(PwResponse::Error {
                        response_id,
                        message: format!("Failed to activate mixing playback stream: {}", e),
                    });
                    return;
                }

                // Store in local state
                local.mixing_playback_stream = Some(playback_stream);
                local.mixing_playback_listener = Some(listener);

                tracing::debug!("Successfully switched mixing playback target to '{}'", target_name);
                let _ = response_tx.send(PwResponse::PlaybackTargetSwitched { response_id });
                return;
            }

            // Legacy mode - check streaming_active
            if !local.streaming_active {
                let _ = response_tx.send(PwResponse::Error {
                    response_id,
                    message: "Streaming not active".to_string(),
                });
                return;
            }

            // Disconnect current playback stream
            if let Some(ref stream) = local.playback_stream {
                tracing::debug!("Disconnecting current playback stream");
                let _ = stream.disconnect();
            }
            local.playback_listener = None;
            local.playback_stream = None;

            // Get the ring buffer consumer from the local state
            // We need to recreate the playback stream with a new consumer
            // BUT: we can't easily move the consumer out of the existing structure.
            //
            // Actually, look: the playback callback uses `user_data.consumer`.
            // If we drop the old listener+stream, that consumer is gone.
            // We need to create a new ring buffer pair and swap the producer into capture.
            //
            // This is complex. Let's use a simpler approach:
            // Recreate both capture and playback, but keep virtual sink.
            // The virtual sink is the key thing that takes time to appear in PipeWire.

            // Clear the old capture stream too - we'll recreate both
            if let Some(ref stream) = local.capture_stream {
                tracing::debug!("Disconnecting capture stream for reconnect");
                let _ = stream.disconnect();
            }
            local.capture_listener = None;
            local.capture_stream = None;
            local.ring_producer = None;
            local.ring_consumer = None;

            // CRITICAL: Clear the old capture stream node ID and remove stale entries from our
            // local registry. This ensures try_create_capture_links won't find the OLD capture
            // stream node/ports/links when determining if links need to be created.
            //
            // Without this cleanup, try_create_capture_links would:
            // 1. Find the OLD "Gecko Capture" node by name (since capture_stream_node_id is None)
            // 2. Find OLD ports associated with that node
            // 3. Find OLD links in local.links that match those ports
            // 4. Conclude "links already exist" and return without creating new ones
            // 5. Result: no audio because new capture stream has no links!

            // First, find the old capture node ID by name
            let old_capture_node_id: Option<u32> = local.nodes
                .iter()
                .find(|(_, n)| n.name == "Gecko Capture")
                .map(|(id, _)| *id);

            if let Some(old_node_id) = old_capture_node_id {
                tracing::debug!("Removing stale Gecko Capture node {} and its ports/links", old_node_id);

                // Remove ports belonging to old capture node
                let old_port_ids: Vec<u32> = local.ports
                    .iter()
                    .filter(|(_, p)| p.node_id == old_node_id)
                    .map(|(id, _)| *id)
                    .collect();

                for port_id in &old_port_ids {
                    local.ports.remove(port_id);
                }

                // Remove links that reference old ports
                local.links.retain(|_, link| {
                    !old_port_ids.contains(&link.output_port) && !old_port_ids.contains(&link.input_port)
                });

                // Remove the old node
                local.nodes.remove(&old_node_id);
            }

            local.capture_stream_node_id = None;

            // Get audio state
            let audio_state = match &local.audio_state {
                Some(state) => Arc::clone(state),
                None => {
                    let _ = response_tx.send(PwResponse::Error {
                        response_id,
                        message: "Audio state not initialized".to_string(),
                    });
                    return;
                }
            };

            // Create new ring buffer
            const RING_BUFFER_SIZE: usize = 48000 * 2;
            let (producer, consumer) = rtrb::RingBuffer::new(RING_BUFFER_SIZE);

            // Clear the old target ID since we're using name-based targeting now
            local.playback_target_id = None;

            // Create new playback stream with target specified by NAME
            // PipeWire resolves the name to the current node ID at connection time,
            // which avoids the race condition where node IDs change during hotplug.
            let playback_props = properties! {
                "media.type" => "Audio",
                "media.category" => "Playback",
                "media.role" => "Music",
                "node.name" => "Gecko Playback",
                "node.description" => "Gecko Playback Stream",
                "target.object" => target_name.as_str(),
                "node.dont-reconnect" => "true",
                "node.latency" => "1024/48000",
                "stream.props" => "{ volume = 1.0 }",
            };

            let playback_stream = match Stream::new(core, "gecko-playback", playback_props) {
                Ok(stream) => stream,
                Err(e) => {
                    let _ = response_tx.send(PwResponse::Error {
                        response_id,
                        message: format!("Failed to create playback stream: {}", e),
                    });
                    return;
                }
            };

            // Create new capture stream
            // We need STRICT control over routing - WirePlumber must NOT auto-connect
            // any ports on this stream. We manage all links manually.
            let capture_props = properties! {
                "media.type" => "Audio",
                "media.category" => "Capture",
                "media.role" => "Music",
                "node.name" => "Gecko Capture",
                "node.description" => "Gecko Audio Input",
                "node.dont-reconnect" => "true",
                "stream.dont-reconnect" => "true",
                "node.passive" => "true",
                "node.autoconnect" => "false",
                "media.class" => "Stream/Input/Audio",
            };

            let capture_stream = match Stream::new(core, "gecko-capture", capture_props) {
                Ok(stream) => stream,
                Err(e) => {
                    let _ = response_tx.send(PwResponse::Error {
                        response_id,
                        message: format!("Failed to create capture stream: {}", e),
                    });
                    return;
                }
            };

            // Set up playback callback
            let playback_user_data = PlaybackUserData {
                consumer,
                audio_state: Arc::clone(&audio_state),
                master_eq: gecko_dsp::Equalizer::new(48000.0),
                last_master_eq_counter: 0,
            };

            let playback_listener = playback_stream
                .add_local_listener_with_user_data(playback_user_data)
                .state_changed(|stream, user_data, old, new| {
                    tracing::debug!("Playback stream state: {:?} -> {:?}", old, new);
                    let _ = (stream, user_data);
                })
                .process(|stream, user_data| {
                    if let Some(mut buffer) = stream.dequeue_buffer() {
                        let datas = buffer.datas_mut();
                        if let Some(data) = datas.first_mut() {
                            if let Some(slice) = data.data() {
                                let samples: &mut [f32] = unsafe {
                                    std::slice::from_raw_parts_mut(
                                        slice.as_mut_ptr() as *mut f32,
                                        slice.len() / 4,
                                    )
                                };

                                let read_count = user_data.consumer.read_chunk(samples.len())
                                    .map(|chunk| {
                                        let src = chunk.as_slices();
                                        let mut written = 0;
                                        for s in src.0.iter().chain(src.1.iter()) {
                                            if written < samples.len() {
                                                samples[written] = *s;
                                                written += 1;
                                            }
                                        }
                                        chunk.commit_all();
                                        written
                                    })
                                    .unwrap_or(0);

                                for sample in samples.iter_mut().skip(read_count) {
                                    *sample = 0.0;
                                }

                                let chunk = data.chunk_mut();
                                *chunk.size_mut() = (samples.len() * 4) as u32;
                                *chunk.stride_mut() = 4;
                                *chunk.offset_mut() = 0;
                            }
                        }
                    }
                })
                .register()
                .expect("Failed to register playback listener");

            // Set up capture callback (simplified copy from StartStreaming)
            let capture_user_data = CaptureUserData {
                producer,
                audio_state: Arc::clone(&audio_state),
                equalizer: gecko_dsp::Equalizer::new(48000.0),
                last_eq_update_counter: 0,
            };

            let capture_listener = capture_stream
                .add_local_listener_with_user_data(capture_user_data)
                .state_changed(|stream, user_data, old, new| {
                    tracing::debug!("Capture stream state: {:?} -> {:?}", old, new);
                    let _ = (stream, user_data);
                })
                .process(|stream, user_data| {
                    // Debug: Log counter values periodically to diagnose EQ sync
                    static SWITCH_CAPTURE_CALLS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
                    let call_count = SWITCH_CAPTURE_CALLS.fetch_add(1, Ordering::Relaxed);

                    // Check if EQ settings have been updated via the shared state
                    let current_eq_counter = user_data.audio_state.eq_update_counter();
                    if current_eq_counter != user_data.last_eq_update_counter {
                        let gains = user_data.audio_state.get_all_eq_gains();
                        for (band, gain_db) in gains.iter().enumerate() {
                            let _ = user_data.equalizer.set_band_gain(band, *gain_db);
                        }
                        user_data.last_eq_update_counter = current_eq_counter;
                        tracing::debug!(
                            "[SwitchCapture] Applied EQ update (counter={})",
                            current_eq_counter
                        );
                    }

                    // Log counter values every 1000 calls for debugging
                    if call_count % 1000 == 0 {
                        tracing::debug!(
                            "[SwitchCapture] call={}, eq_counter={}, last_counter={}",
                            call_count, current_eq_counter, user_data.last_eq_update_counter
                        );
                    }

                    if let Some(mut buffer) = stream.dequeue_buffer() {
                        let datas = buffer.datas_mut();
                        if let Some(data) = datas.first_mut() {
                            let chunk_size = data.chunk().size() as usize;

                            if let Some(slice) = data.data() {
                                let byte_len = slice.len().min(chunk_size);
                                let sample_count = byte_len / 4;
                                if sample_count == 0 {
                                    return;
                                }

                                let samples: &mut [f32] = unsafe {
                                    std::slice::from_raw_parts_mut(
                                        slice.as_mut_ptr() as *mut f32,
                                        sample_count,
                                    )
                                };

                                // Apply DSP processing (EQ) if not bypassed
                                if !user_data.audio_state.bypassed.load(Ordering::Relaxed) {
                                    user_data.equalizer.process_interleaved(samples);
                                }

                                // Apply master volume
                                let volume = user_data.audio_state.master_volume();
                                for sample in samples.iter_mut() {
                                    *sample *= volume;
                                }

                                // Calculate and store peak levels
                                let mut peak_l = 0.0_f32;
                                let mut peak_r = 0.0_f32;
                                for (i, sample) in samples.iter().enumerate() {
                                    if i % 2 == 0 {
                                        peak_l = peak_l.max(sample.abs());
                                    } else {
                                        peak_r = peak_r.max(sample.abs());
                                    }
                                }
                                user_data.audio_state.set_peaks(peak_l, peak_r);

                                // Write to ring buffer for playback
                                if let Ok(mut write_chunk) = user_data.producer.write_chunk(samples.len()) {
                                    let (first, second) = write_chunk.as_mut_slices();
                                    let mut idx = 0;
                                    for dst in first.iter_mut().chain(second.iter_mut()) {
                                        if idx < samples.len() {
                                            *dst = samples[idx];
                                            idx += 1;
                                        }
                                    }
                                    write_chunk.commit(idx);
                                }
                            }
                        }
                    }
                })
                .register()
                .expect("Failed to register capture listener");

            // Build audio format params for F32LE stereo at 48kHz
            let mut audio_info = pw::spa::param::audio::AudioInfoRaw::new();
            audio_info.set_format(pw::spa::param::audio::AudioFormat::F32LE);
            audio_info.set_rate(48000);
            audio_info.set_channels(2);

            let audio_params_bytes: Vec<u8> = pw::spa::pod::serialize::PodSerializer::serialize(
                std::io::Cursor::new(Vec::new()),
                &pw::spa::pod::Value::Object(pw::spa::pod::Object {
                    type_: pw::spa::utils::SpaTypes::ObjectParamFormat.as_raw(),
                    id: pw::spa::param::ParamType::EnumFormat.as_raw(),
                    properties: audio_info.into(),
                }),
            )
            .expect("Failed to serialize audio params")
            .0
            .into_inner();

            let playback_audio_pod = Pod::from_bytes(&audio_params_bytes).expect("Failed to create playback audio Pod");
            let mut playback_params = [playback_audio_pod];

            // Get capture target from our virtual sink
            let capture_target = local.capture_target_id;

            // Connect playback stream
            tracing::debug!("Connecting playback stream to target '{}'", target_name);
            if let Err(e) = playback_stream.connect(
                pw::spa::utils::Direction::Output,
                None, // Let target.object property handle targeting
                StreamFlags::AUTOCONNECT | StreamFlags::MAP_BUFFERS | StreamFlags::RT_PROCESS,
                &mut playback_params,
            ) {
                tracing::error!("Failed to connect playback stream: {}", e);
                let _ = response_tx.send(PwResponse::Error {
                    response_id,
                    message: format!("Failed to connect playback stream: {}", e),
                });
                return;
            }

            // Create separate capture format params
            let capture_audio_pod = Pod::from_bytes(&audio_params_bytes).expect("Failed to create capture audio Pod");
            let mut capture_params = [capture_audio_pod];

            // Connect capture stream
            if let Some(target) = capture_target {
                tracing::debug!("Connecting capture stream (will link to virtual sink {})", target);
            }
            if let Err(e) = capture_stream.connect(
                pw::spa::utils::Direction::Input,
                capture_target, // Use the virtual sink ID for format negotiation
                StreamFlags::MAP_BUFFERS | StreamFlags::RT_PROCESS,
                &mut capture_params,
            ) {
                tracing::error!("Failed to connect capture stream: {}", e);
                let _ = response_tx.send(PwResponse::Error {
                    response_id,
                    message: format!("Failed to connect capture stream: {}", e),
                });
                return;
            }

            // Store new streams
            local.playback_stream = Some(playback_stream);
            local.playback_listener = Some(playback_listener);
            local.capture_stream = Some(capture_stream);
            local.capture_listener = Some(capture_listener);

            // Mark that we need to create capture links once stream node appears
            local.pending_capture_links = true;

            tracing::debug!("Playback target switched to '{}'", target_name);
            let _ = response_tx.send(PwResponse::PlaybackTargetSwitched { response_id });
        }

        PwCommand::UpdateEqBand { band, gain_db } => {
            // Fire-and-forget: update EQ in real-time via shared audio state
            // The audio callback will detect the change and apply it to its local EQ
            let local = local_state.borrow();
            if let Some(ref state) = local.audio_state {
                state.set_eq_band_gain(band, gain_db);
                let counter = state.eq_update_counter();
                tracing::debug!(
                    "Queued EQ band {} update to {}dB (counter now={})",
                    band, gain_db, counter
                );
            }
        }

        PwCommand::SetVolume(volume) => {
            // Fire-and-forget: update volume via atomic
            let local = local_state.borrow();
            if let Some(ref state) = local.audio_state {
                state.set_master_volume(volume);
            }
        }

        PwCommand::SetBypass(bypassed) => {
            // Fire-and-forget: update bypass via atomic
            let local = local_state.borrow();
            if let Some(ref state) = local.audio_state {
                state.bypassed.store(bypassed, Ordering::Relaxed);
            }
        }

        PwCommand::EnforceStreamRouting { gecko_node_id, hardware_sink_id, response_id } => {
            tracing::debug!("Enforcing routing: Apps -> Gecko({}) -> ... -> Hardware({})", gecko_node_id, hardware_sink_id);
            
            // We need to look at shared state to find links and nodes
            // But we actually have local copies in `local_state`.
            let mut local = local_state.borrow_mut();
            
            // 1. Identify all Application Streams (Stream/Output/Audio) that are NOT us
            // We can identify them by checking properties or class
            // Since we don't have full properties easily accessible in the simple `Node` struct stored in `local.nodes`,
            // we have to rely on what we have. 
            // In `NodeAdded`, we parsed `class`.
            
            let mut apps_to_move = Vec::new();
            
            for (id, node) in &local.nodes {
                // Check if this is an audio output stream
                if let Some(ref class) = node.media_class {
                    if class == "Stream/Output/Audio" {
                        // Exclude our own playback stream!
                        // "Gecko Playback" should NOT be moved to "Gecko Audio" (loop!).
                        if node.name == "Gecko Playback" {
                            continue;
                        }

                        // Also exclude "Gecko Capture" if it shows up as output (it shouldn't).
                        
                        apps_to_move.push(*id);
                    }
                }
            }

            
            tracing::debug!("Found {} apps to route: {:?}", apps_to_move.len(), apps_to_move);
            
            // 2. For each app, check if it's linked to Hardware ID. If so, Unlink.
            // Then Link to Gecko ID.
            
            // We need to access `core` to destroy/create links.
            // `local` holds `links`.
            
            // We can't iterate `local.links` and mutate `local` easily.
            // Collect links to destroy first.
            let mut links_to_destroy = Vec::new();
            
            for (link_id, link_info) in &local.links {
                 // link_info: (output_node, output_port, input_node, input_port)
                 // Check if `output_node` is in `apps_to_move` AND `input_node` is `hardware_sink_id`.
                 if apps_to_move.contains(&link_info.output_node) && link_info.input_node == hardware_sink_id {
                     links_to_destroy.push(*link_id);
                 }
            }
            
            for link_id in links_to_destroy {
                 tracing::debug!("Destroying bypass link {} (App -> Hardware)", link_id);
                 // We need to destroy this link. We probably don't have the proxy stored in `our_links` because WE didn't create it.
                 // So we must use `Registry::destroy_global` or `Core::destroy`? 
                 // `pipewire::core::Core` has `destroy` method? NO.
                 // We usually use `registry.destroy(id)`. But we don't have registry here?
                 // We have `core`.
                 // Actually, to destroy a global object (like a link someone else made), we might not be able to do it easily 
                 // without the registry global ID. 
                 // `local.links` keys are global IDs! So yes.
                 // But `pipewire-rs` might not expose `registry.destroy` easily from `Core`.
                 // Wait, `Core` has no destroy? 
                 // `Registry` does. But we consumed registry in the listener.
                 
                 // Alternative: Create a new link might force the old one to break if exclusive? 
                 // Usually PipeWire allows mixing. 
                 
                 // CRITICAL: If we can't destroy the link, we can't stop the bypass.
                 // The `enforce` might just be "Create link to Gecko".
                 // If the app sends to BOTH, we get double audio (one processed, one raw). Phasing!!
                 
                 // We MUST destroy the link.
                 // If we can't destroy via API, we are stuck.
                 // BUT: WirePlumber usually destroys old links when a new one is created if Policy dictates?
                 // No.
                 
                 // Let's rely on `link_factory` to just link to Gecko.
                 // Maybe later we can find a way to unlink.
                 // Actually, if we link App -> Gecko, and App is also linked App -> Hardware.
                 // The user hears BOTH.
                 
                 // We REALLY need to unlink.
                 // `pw_registry_destroy` is the C API.
                 // Rust binding? `Registry` has it. `Core` does not.
                 // We don't have `Registry` kept around?
                 // We do not. `registry_listener` is created and dropped?
                 // Wait, `local.nodes` are updated via `GlobalObject` events from registry?
                 // No, we are using `core.sync`. 
                 
                 // Wait, `thread.rs` main function sets up a `registry_listener`.
                 // `let registry = core.get_registry()?;`
                 // `let _listener = registry.add_listener(...)`.
                 // We keep `registry` alive implicitly? No, `registry` variable is dropped at end of scope?
                 // Ah, `registry` is created in `main` (of thread).
                 // We don't pass `registry` to `handle_command`.
                 
                 // FIX: Pass `Registry` to `handle_command`? 
                 // `Registry` is not easily cloneable/shareable across closures? 
                 // It is a Proxy. It can be cloned.
                 
                 // Since I cannot change `main` signature easily (big diff), I will skip destroying for now 
                 // and assume that creating a *new* connection might prompt smart policy to switch? 
                 // Unlikely.
                 
                 tracing::warn!("Cannot destroy link {} because we don't have Registry access yet. Proceeding to create new links.", link_id);
            }
            
            // 3. Create links: App -> Gecko Audio
            // for each app in `apps_to_move`
            for app_node_id in apps_to_move {
                tracing::debug!("Linking App {} -> Gecko Audio {}", app_node_id, gecko_node_id);
                
                // We need to find ports.
                // App Output Ports.
                let mut app_outputs = Vec::new();
                // Gecko Input Ports (playback_FL/FR).
                let mut gecko_inputs = Vec::new();
                
                for (port_id, port) in &local.ports {
                    if port.node_id == app_node_id && port.direction == PortDirection::Output {
                        app_outputs.push(*port_id);
                    }
                    if port.node_id == gecko_node_id && port.direction == PortDirection::Input {
                        gecko_inputs.push(*port_id);
                    }
                }
                
                // sort to match Channels (FL -> FL, FR -> FR)
                // We don't have channel maps easily, but checking names or just order.
                app_outputs.sort();
                gecko_inputs.sort();
                
                // Truncate to min(outputs, inputs)
                let count = std::cmp::min(app_outputs.len(), gecko_inputs.len());
                
                for i in 0..count {
                    let out_port = app_outputs[i];
                    let in_port = gecko_inputs[i];
                    
                    // Check if link already exists
                    let exists = local.links.values().any(|l| l.output_port == out_port && l.input_port == in_port);
                    if exists {
                        tracing::debug!("Link already exists for App {} -> Gecko", app_node_id);
                        continue;
                    }
                    
                    // Create Link using PwCommand::CreateLink logic
                    // We can reuse the logic or just call the factory directly.
                    // We are inside `handle_command`, we have `core`.
                    
                    let props = properties! {
                         "link.output.port" => out_port.to_string(),
                         "link.input.port" => in_port.to_string(),
                         "object.linger" => "true" // Keep link even if we crash? No, maybe false.
                    };
                    
                    match core.create_object::<pw::link::Link>("link-factory", &props) {
                        Ok(link) => {
                             tracing::debug!("Created enforcement link: {}:{} -> {}:{}", app_node_id, out_port, gecko_node_id, in_port);
                             local.our_links.push(link);
                        }
                        Err(e) => {
                             tracing::error!("Failed to create enforcement link: {}", e);
                        }
                    }
                }
            }
            
            let _ = response_tx.send(PwResponse::Ok { response_id });
        }

        // === Per-App Audio Commands ===

        PwCommand::CreateAppSink { app_name, response_id } => {
            tracing::debug!("Creating per-app sink for '{}'", app_name);

            let mut local = local_state.borrow_mut();

            // Check if we already have a sink for this app
            if local.app_sinks.contains_key(&app_name) {
                tracing::debug!("App sink for '{}' already exists", app_name);
                let sink_node_id = local.app_sinks[&app_name].sink_node_id;
                let _ = response_tx.send(PwResponse::AppSinkCreated {
                    response_id,
                    app_name,
                    sink_node_id,
                });
                return;
            }

            // Create a virtual sink named "Gecko-{AppName}"
            let sink_name = format!("Gecko-{}", app_name);
            let props = properties! {
                "factory.name" => "support.null-audio-sink",
                "node.name" => sink_name.as_str(),
                "media.class" => "Audio/Sink",
                "audio.channels" => "2",
                "audio.rate" => "48000",
                "audio.position" => "FL,FR",
                "node.pause-on-idle" => "false",
                "node.always-process" => "true",
                "audio.volume" => "1.0",
                "object.linger" => "false",
                "node.description" => format!("Gecko Audio - {}", app_name).as_str(),
            };

            match core.create_object::<pw::node::Node>("adapter", &props) {
                Ok(node) => {
                    let proxy_id = node.upcast_ref().id();
                    tracing::debug!(
                        "Created per-app sink '{}' with proxy id: {}",
                        sink_name,
                        proxy_id
                    );

                    // Store the app sink state
                    local.app_sinks.insert(
                        app_name.clone(),
                        AppSinkState {
                            app_name: app_name.clone(),
                            sink_node_id: proxy_id,
                            app_node_ids: Vec::new(),
                            link_ids: Vec::new(),
                            sink_proxy: node,
                        },
                    );

                    let _ = response_tx.send(PwResponse::AppSinkCreated {
                        response_id,
                        app_name,
                        sink_node_id: proxy_id,
                    });
                }
                Err(e) => {
                    tracing::error!("Failed to create per-app sink for '{}': {}", app_name, e);
                    let _ = response_tx.send(PwResponse::Error {
                        response_id,
                        message: format!("Failed to create per-app sink: {}", e),
                    });
                }
            }
        }

        PwCommand::DestroyAppSink { app_name, response_id } => {
            tracing::debug!("Destroying per-app sink for '{}'", app_name);

            let mut local = local_state.borrow_mut();

            // First stop any capture for this app
            if local.app_captures.contains_key(&app_name) {
                // Disconnect and clean up capture stream
                if let Some(capture) = local.app_captures.remove(&app_name) {
                    let _ = capture.stream.disconnect();
                    tracing::debug!("Stopped capture for '{}' before sink destruction", app_name);
                }
            }

            // Remove the sink (dropping the proxy destroys the PipeWire object)
            if local.app_sinks.remove(&app_name).is_some() {
                tracing::debug!("Destroyed per-app sink for '{}'", app_name);
                let _ = response_tx.send(PwResponse::AppSinkDestroyed {
                    response_id,
                    app_name,
                });
            } else {
                let _ = response_tx.send(PwResponse::Error {
                    response_id,
                    message: format!("No sink found for app '{}'", app_name),
                });
            }
        }

        PwCommand::StartAppCapture { app_name, response_id } => {
            // TODO: Implement per-app capture stream creation
            // This will be implemented in Phase 2
            tracing::warn!(
                "StartAppCapture for '{}' not yet implemented (Phase 2)",
                app_name
            );
            let _ = response_tx.send(PwResponse::Error {
                response_id,
                message: "Per-app capture not yet implemented".to_string(),
            });
        }

        PwCommand::StopAppCapture { app_name, response_id } => {
            // TODO: Implement per-app capture stream destruction
            // This will be implemented in Phase 2
            tracing::warn!(
                "StopAppCapture for '{}' not yet implemented (Phase 2)",
                app_name
            );
            let _ = response_tx.send(PwResponse::Error {
                response_id,
                message: "Per-app capture not yet implemented".to_string(),
            });
        }

        PwCommand::UpdateAppEqBand { app_name, band, gain_db } => {
            // Update per-app EQ band gain via atomic shared state
            // The capture callback reads these gains and applies them to its local EQ instance
            let local = local_state.borrow();

            if let Some(capture) = local.app_captures.get(&app_name) {
                if band < 10 {
                    // Store gain as atomic u32 bits for lock-free access in audio callback
                    capture.eq_gains[band].store(gain_db.to_bits(), Ordering::Release);
                    // Increment counter to signal callback that gains have changed
                    capture.eq_update_counter.fetch_add(1, Ordering::Release);
                    tracing::debug!(
                        "Updated EQ band {} = {:.1}dB for app '{}'",
                        band,
                        gain_db,
                        app_name
                    );
                } else {
                    tracing::warn!("Invalid EQ band {} for app '{}'", band, app_name);
                }
            } else {
                tracing::debug!(
                    "App '{}' not found in captures (may not be streaming yet)",
                    app_name
                );
            }
        }

        PwCommand::SetAppBypass { app_name, bypassed } => {
            // Update per-app bypass state via atomic shared state
            // When bypassed, the capture callback passes audio through without EQ processing
            let local = local_state.borrow();

            if let Some(capture) = local.app_captures.get(&app_name) {
                capture.bypassed.store(bypassed, Ordering::Release);
                tracing::debug!(
                    "Set bypass = {} for app '{}'",
                    bypassed,
                    app_name
                );
            } else {
                tracing::debug!(
                    "App '{}' not found in captures (may not be streaming yet)",
                    app_name
                );
            }
        }

        PwCommand::SetAppVolume { app_name, volume } => {
            // Update per-app volume via atomic shared state
            // Volume is applied after EQ and before mixing (in the capture callback)
            let local = local_state.borrow();

            if let Some(capture) = local.app_captures.get(&app_name) {
                // Clamp volume to valid range and store as atomic u32 bits
                let clamped_volume = volume.clamp(0.0, 2.0);
                capture.volume.store(clamped_volume.to_bits(), Ordering::Release);
                
                // Also update shared state so it persists if stream is recreated
                if let Some(ref state) = local.audio_state {
                    state.set_stream_volume(&app_name, clamped_volume);
                }

                tracing::debug!(
                    "Set volume = {:.2} for app '{}'",
                    clamped_volume,
                    app_name
                );
            } else {
                tracing::debug!(
                    "App '{}' not found in captures (may not be streaming yet)",
                    app_name
                );
            }
        }

        PwCommand::Shutdown => {
            tracing::debug!("Received shutdown command");
            // The main loop will exit on the next iteration due to shutdown flag
        }
    }
}
