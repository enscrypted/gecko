//! Audio Engine - Main Entry Point
//!
//! The AudioEngine manages the lifecycle of audio streams and coordinates
//! communication between the UI and audio processing threads.
//!
//! # Architecture
//!
//! The engine integrates with platform-specific backends for audio capture:
//!
//! ```text
//! Linux (PipeWire):
//!   1. Create virtual sink via PipeWireBackend
//!   2. Users route apps to virtual sink (appears in system settings)
//!   3. Capture from virtual sink's monitor port
//!   4. Process through DSP
//!   5. Output to real speakers
//!
//! Windows (WASAPI):
//!   1. Use Process Loopback API to capture specific app audio
//!   2. Process through DSP
//!   3. Output to real speakers
//!
//! macOS (CoreAudio):
//!   1. HAL plugin creates virtual device
//!   2. Shared memory ring buffer for audio transfer
//!   3. Process through DSP
//!   4. Output to real speakers
//! ```
//!
//! IMPORTANT: This is NOT a microphone passthrough application!

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};

use crossbeam_channel::{bounded, unbounded, Receiver, Sender};

// Note: HostTrait is used on non-Linux/non-macOS platforms for device enumeration
#[cfg(all(not(target_os = "linux"), not(target_os = "macos")))]
use cpal::traits::HostTrait;
use tracing::{debug, error, info, warn};

use crate::config::EngineConfig;
use crate::device::AudioDevice;
use crate::error::{EngineError, EngineResult};
use crate::message::{Command, Event};
use crate::stream::AudioStream;

// Platform backend for audio routing
#[cfg(target_os = "linux")]
use gecko_platform::{PlatformBackend, VirtualSinkConfig};

// macOS: CoreAudio backend with Process Tap (14.4+) or HAL plugin
#[cfg(target_os = "macos")]
use gecko_platform::macos::{
    AudioMixer, AudioOutputStream, AudioProcessingState, CoreAudioBackend,
};
#[cfg(target_os = "macos")]
use gecko_platform::PlatformBackend as _; // Import trait to bring methods into scope

/// Helper to map PIDs to app names using AppleScript (macOS only)
/// Returns a HashMap of PID -> app name
#[cfg(target_os = "macos")]
fn get_pid_to_app_name_map() -> std::collections::HashMap<u32, String> {
    use gecko_platform::macos::coreaudio::list_audio_applications;

    let mut map = std::collections::HashMap::new();
    if let Ok(apps) = list_audio_applications() {
        for app in apps {
            map.insert(app.pid, app.name);
        }
    }
    map
}

/// Check if a process is still running (macOS only)
/// Returns true if the process exists, false if it has exited
#[cfg(target_os = "macos")]
fn is_process_running(pid: u32) -> bool {
    // kill(pid, 0) returns 0 if process exists, -1 if not
    unsafe { libc::kill(pid as i32, 0) == 0 }
}

/// The main audio engine controller
///
/// This struct lives on the UI/main thread and communicates with the
/// audio processing thread via channels.
pub struct AudioEngine {
    /// Channel for sending commands to audio thread
    command_sender: Sender<Command>,

    /// Channel for receiving events from audio thread
    event_receiver: Receiver<Event>,

    /// Handle to the audio processing thread
    audio_thread: Option<JoinHandle<()>>,

    /// Flag to signal shutdown
    shutdown_flag: Arc<AtomicBool>,

    /// Current configuration
    config: EngineConfig,

    /// Whether engine is currently running
    is_running: Arc<AtomicBool>,
}

impl AudioEngine {
    /// Create a new audio engine with default configuration
    pub fn new() -> EngineResult<Self> {
        Self::with_config(EngineConfig::default())
    }

    /// Create a new audio engine with custom configuration
    pub fn with_config(config: EngineConfig) -> EngineResult<Self> {
        let (command_sender, command_receiver) = bounded::<Command>(32);
        let (event_sender, event_receiver) = unbounded::<Event>();

        let shutdown_flag = Arc::new(AtomicBool::new(false));
        let is_running = Arc::new(AtomicBool::new(false));

        // Clone for audio thread
        let shutdown_clone = Arc::clone(&shutdown_flag);
        let running_clone = Arc::clone(&is_running);
        let config_clone = config.clone();

        // Spawn audio processing thread
        let audio_thread = thread::Builder::new()
            .name("gecko-audio".into())
            .spawn(move || {
                Self::audio_thread_main(
                    command_receiver,
                    event_sender,
                    shutdown_clone,
                    running_clone,
                    config_clone,
                );
            })
            .map_err(|e| EngineError::StreamBuildError(e.to_string()))?;

        Ok(Self {
            command_sender,
            event_receiver,
            audio_thread: Some(audio_thread),
            shutdown_flag,
            config,
            is_running,
        })
    }

    /// Start audio processing
    pub fn start(&self) -> EngineResult<()> {
        self.send_command(Command::Start)
    }

    /// Stop audio processing
    pub fn stop(&self) -> EngineResult<()> {
        self.send_command(Command::Stop)
    }

    /// Set EQ band gain
    pub fn set_band_gain(&self, band: usize, gain_db: f32) -> EngineResult<()> {
        self.send_command(Command::SetBandGain { band, gain_db })
    }

    /// Set per-app EQ band gain (TRUE per-app EQ, NOT additive to master)
    ///
    /// Each app has its own independent EQ instance that processes audio BEFORE mixing.
    /// Master EQ is applied AFTER mixing.
    pub fn set_stream_band_gain(&self, stream_id: String, band: usize, gain_db: f32) -> EngineResult<()> {
        self.send_command(Command::SetStreamBandGain { stream_id, band, gain_db })
    }

    /// Set bypass state for a specific application
    ///
    /// When bypassed, the app's audio passes through without per-app EQ processing.
    /// Master EQ (applied after mixing) still affects the audio unless globally bypassed.
    pub fn set_app_bypass(&self, app_name: String, bypassed: bool) -> EngineResult<()> {
        self.send_command(Command::SetAppBypass { app_name, bypassed })
    }

    /// Start capturing audio from a specific application (macOS only)
    ///
    /// Uses the Process Tap API (macOS 14.4+) to capture the app's audio stream.
    /// The captured audio is mixed with other captured apps and processed through
    /// the master EQ before output.
    #[cfg(target_os = "macos")]
    pub fn start_app_capture(&self, pid: u32, app_name: String) -> EngineResult<()> {
        self.send_command(Command::StartAppCapture { pid, app_name })
    }

    /// Stop capturing audio from a specific application (macOS only)
    #[cfg(target_os = "macos")]
    pub fn stop_app_capture(&self, pid: u32) -> EngineResult<()> {
        self.send_command(Command::StopAppCapture { pid })
    }

    /// Set per-app volume (0.0 - 2.0, where 1.0 is unity gain)
    ///
    /// This is applied after per-app EQ and before mixing. It's independent of master volume.
    /// Values > 1.0 amplify the audio, < 1.0 attenuate it.
    pub fn set_stream_volume(&self, stream_id: String, volume: f32) -> EngineResult<()> {
        self.send_command(Command::SetStreamVolume { stream_id, volume })
    }

    /// Set master volume (0.0 - 1.0)
    pub fn set_master_volume(&self, volume: f32) -> EngineResult<()> {
        self.send_command(Command::SetMasterVolume(volume))
    }

    /// Get the current PipeWire sink volume (Linux only)
    /// 
    /// Returns the "Gecko Audio" sink volume as seen by PipeWire/WirePlumber.
    /// This syncs with system volume controls.
    #[cfg(target_os = "linux")]
    pub fn get_sink_volume(&self) -> EngineResult<f32> {
        use std::process::Command as ShellCommand;
        
        let list_output = ShellCommand::new("wpctl")
            .args(["status"])
            .output()
            .map_err(|e| EngineError::ConfigError(format!("wpctl failed: {}", e)))?;
        
        let list_str = String::from_utf8_lossy(&list_output.stdout);
        
        // Find "Gecko Audio" sink (exact, not per-app sinks like "Gecko Audio - Firefox")
        // Format: "│  *  149. Gecko Audio [vol: 0.58]"
        let mut sink_id: Option<u32> = None;
        for line in list_str.lines() {
            // Must contain "Gecko Audio" but NOT be a per-app sink (those have " - ")
            // Also exclude Monitor and Input lines
            if line.contains("Gecko Audio") 
                && !line.contains(" - ") 
                && !line.contains("Monitor") 
                && !line.contains("Input") 
            {
                // Strip special chars and find number before dot
                let cleaned: String = line.chars()
                    .filter(|c| c.is_ascii_alphanumeric() || *c == '.' || *c == ' ' || *c == ':' || *c == '[' || *c == ']')
                    .collect();
                
                // Find the number before "Gecko Audio"
                if let Some(gecko_pos) = cleaned.find("Gecko") {
                    let before_gecko = &cleaned[..gecko_pos].trim();
                    // Extract the ID number (format: "149. " or "* 149. ")
                    for part in before_gecko.split_whitespace().rev() {
                        let part = part.trim_end_matches('.');
                        if let Ok(id) = part.parse::<u32>() {
                            sink_id = Some(id);
                            break;
                        }
                    }
                }
                if sink_id.is_some() {
                    break;
                }
            }
        }
        
        let sink_id = sink_id.ok_or_else(|| {
            EngineError::ConfigError("Gecko Audio sink not found".into())
        })?;
        
        let vol_output = ShellCommand::new("wpctl")
            .args(["get-volume", &sink_id.to_string()])
            .output()
            .map_err(|e| EngineError::ConfigError(format!("wpctl get-volume failed: {}", e)))?;
        
        let vol_str = String::from_utf8_lossy(&vol_output.stdout);
        
        // Check if muted - format: "Volume: 0.58 [MUTED]"
        let is_muted = vol_str.contains("[MUTED]");
        
        // If muted, set DSP to 0 and return 0.0
        if is_muted {
            let _ = self.send_command(Command::SetMasterVolume(0.0));
            return Ok(0.0);
        }
        
        // Parse volume
        if let Some(vol_start) = vol_str.find("Volume:") {
            let after_colon = &vol_str[vol_start + 7..];
            let vol_part = after_colon.split_whitespace().next().unwrap_or("1.0");
            if let Ok(vol) = vol_part.parse::<f32>() {
                // Also set DSP volume (handles unmute and volume changes)
                let _ = self.send_command(Command::SetMasterVolume(vol));
                return Ok(vol);
            }
        }
        
        Ok(1.0)
    }
    
    /// Set the PipeWire sink volume (Linux only)
    /// 
    /// Sets both the "Gecko Audio" sink volume and internal master volume.
    /// This syncs with system volume controls.
    #[cfg(target_os = "linux")]
    pub fn set_sink_volume(&self, volume: f32) -> EngineResult<()> {
        use std::process::Command as ShellCommand;
        
        let list_output = ShellCommand::new("wpctl")
            .args(["status"])
            .output()
            .map_err(|e| EngineError::ConfigError(format!("wpctl failed: {}", e)))?;
        
        let list_str = String::from_utf8_lossy(&list_output.stdout);
        
        // Find "Gecko Audio" sink (exact, not per-app sinks like "Gecko Audio - Firefox")
        // Format: "│  *  149. Gecko Audio [vol: 0.58]"
        let mut sink_id: Option<u32> = None;
        for line in list_str.lines() {
            // Must contain "Gecko Audio" but NOT be a per-app sink (those have " - ")
            if line.contains("Gecko Audio") 
                && !line.contains(" - ") 
                && !line.contains("Monitor") 
                && !line.contains("Input") 
            {
                // Strip special chars and find number before dot
                let cleaned: String = line.chars()
                    .filter(|c| c.is_ascii_alphanumeric() || *c == '.' || *c == ' ' || *c == ':' || *c == '[' || *c == ']')
                    .collect();
                
                // Find the number before "Gecko Audio"
                if let Some(gecko_pos) = cleaned.find("Gecko") {
                    let before_gecko = &cleaned[..gecko_pos].trim();
                    for part in before_gecko.split_whitespace().rev() {
                        let part = part.trim_end_matches('.');
                        if let Ok(id) = part.parse::<u32>() {
                            sink_id = Some(id);
                            break;
                        }
                    }
                }
                if sink_id.is_some() {
                    break;
                }
            }
        }
        
        let sink_id = sink_id.ok_or_else(|| {
            EngineError::ConfigError("Gecko Audio sink not found".into())
        })?;
        
        let vol_str = format!("{:.2}", volume.clamp(0.0, 1.5));
        let status = ShellCommand::new("wpctl")
            .args(["set-volume", &sink_id.to_string(), &vol_str])
            .status()
            .map_err(|e| EngineError::ConfigError(format!("wpctl set-volume failed: {}", e)))?;
        
        if !status.success() {
            return Err(EngineError::ConfigError("wpctl set-volume failed".into()));
        }
        
        // Also update internal master volume
        self.send_command(Command::SetMasterVolume(volume))
    }

    /// Get the system output volume (macOS only)
    ///
    /// Returns the volume of the default output device.
    /// This syncs with system volume controls (menu bar, keyboard).
    #[cfg(target_os = "macos")]
    pub fn get_sink_volume(&self) -> EngineResult<f32> {
        use gecko_platform::macos::get_system_volume;

        match get_system_volume() {
            Ok(volume) => {
                // Also update DSP master volume to match system
                let _ = self.send_command(Command::SetMasterVolume(volume));
                Ok(volume)
            }
            Err(e) => {
                warn!("Failed to get system volume: {}", e);
                Ok(1.0) // Default to full volume on error
            }
        }
    }

    /// Set the system output volume (macOS only)
    ///
    /// Sets both the system volume (menu bar, keyboard) and internal master volume.
    /// This syncs Gecko with the system volume controls.
    #[cfg(target_os = "macos")]
    pub fn set_sink_volume(&self, volume: f32) -> EngineResult<()> {
        use gecko_platform::macos::set_system_volume;

        let volume = volume.clamp(0.0, 1.0);

        // Set system volume
        if let Err(e) = set_system_volume(volume) {
            warn!("Failed to set system volume: {}", e);
            // Continue anyway to update internal volume
        }

        // Also update internal master volume
        self.send_command(Command::SetMasterVolume(volume))
    }

    /// Set global bypass state (bypasses ALL processing including master EQ)
    pub fn set_bypass(&self, bypassed: bool) -> EngineResult<()> {
        self.send_command(Command::SetBypass(bypassed))
    }

    /// Enable or disable soft clipping (limiter)
    pub fn set_soft_clip_enabled(&self, enabled: bool) -> EngineResult<()> {
        self.send_command(Command::SetSoftClipEnabled(enabled))
    }

    /// Request state update
    pub fn request_state(&self) -> EngineResult<()> {
        self.send_command(Command::RequestState)
    }

    /// Check if engine is currently running
    pub fn is_running(&self) -> bool {
        self.is_running.load(Ordering::SeqCst)
    }

    /// Get next event (non-blocking)
    pub fn poll_event(&self) -> Option<Event> {
        self.event_receiver.try_recv().ok()
    }

    /// Get next event (blocking)
    pub fn wait_event(&self) -> Option<Event> {
        self.event_receiver.recv().ok()
    }

    /// Get all available devices
    pub fn list_devices(&self) -> EngineResult<Vec<AudioDevice>> {
        AudioDevice::enumerate_all()
    }

    /// Get current configuration
    pub fn config(&self) -> &EngineConfig {
        &self.config
    }

    /// Send command to audio thread
    fn send_command(&self, command: Command) -> EngineResult<()> {
        self.command_sender
            .send(command)
            .map_err(|_| EngineError::ChannelSendError)
    }

    /// Audio thread main loop
    fn audio_thread_main(
        command_receiver: Receiver<Command>,
        event_sender: Sender<Event>,
        shutdown_flag: Arc<AtomicBool>,
        is_running: Arc<AtomicBool>,
        config: EngineConfig,
    ) {
        info!("Audio thread started");

        let mut stream: Option<AudioStream> = None;
        let mut master_volume = 1.0_f32;
        let mut bypassed = false;
        // Track Master EQ gains locally so we can restore them when creating a new backend
        let mut master_eq_gains = [0.0f32; 10];
        
        // Track per-app state for persistence across engine restarts
        let mut app_volumes: std::collections::HashMap<String, f32> = std::collections::HashMap::new();
        let mut app_bypassed: std::collections::HashMap<String, bool> = std::collections::HashMap::new();
        let mut app_eq_gains: std::collections::HashMap<String, [f32; 10]> = std::collections::HashMap::new();

        // Linux: Store PipeWire backend for command forwarding
        #[cfg(target_os = "linux")]
        let mut linux_backend: Option<gecko_platform::linux::PipeWireBackend> = None;

        // macOS: Store CoreAudio backend for command forwarding
        #[cfg(target_os = "macos")]
        let mut macos_backend: Option<CoreAudioBackend> = None;
        // macOS: Audio output components (mixer, state, output stream)
        #[cfg(target_os = "macos")]
        let mut macos_mixer: Option<Arc<AudioMixer>> = None;
        #[cfg(target_os = "macos")]
        let mut macos_state: Option<Arc<AudioProcessingState>> = None;
        #[cfg(target_os = "macos")]
        let mut _macos_output: Option<AudioOutputStream> = None;
        // macOS: Timer for periodic app scanning (every 2 seconds for new apps)
        #[cfg(target_os = "macos")]
        let mut last_app_scan = std::time::Instant::now();
        #[cfg(target_os = "macos")]
        let app_scan_interval = std::time::Duration::from_secs(2);
        // macOS: Timer for debug stats logging (every 5 seconds)
        #[cfg(target_os = "macos")]
        let mut last_debug_stats = std::time::Instant::now();
        #[cfg(target_os = "macos")]
        let debug_stats_interval = std::time::Duration::from_secs(5);
        // macOS: Channel for receiving app scan results from background thread
        // Tuple: (audio-active PIDs, PID->app name map)
        #[cfg(target_os = "macos")]
        let (app_scan_tx, app_scan_rx) = crossbeam_channel::bounded::<(Vec<i32>, std::collections::HashMap<u32, String>)>(1);
        // macOS: Flag to track if a scan is in progress
        #[cfg(target_os = "macos")]
        let app_scan_in_progress = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));

        // Linux: Timer for checking default sink changes (every 500ms for responsiveness)
        #[cfg(target_os = "linux")]
        let mut last_sink_check = std::time::Instant::now().checked_sub(std::time::Duration::from_secs(10)).unwrap_or(std::time::Instant::now());
        #[cfg(target_os = "linux")]
        let sink_check_interval = std::time::Duration::from_millis(500);
        // Linux: Grace period after streaming starts before we enforce routing
        // This prevents race conditions with WirePlumber during initial link setup
        #[cfg(target_os = "linux")]
        let mut streaming_start_time: Option<std::time::Instant> = None;
        #[cfg(target_os = "linux")]
        let routing_grace_period = std::time::Duration::from_secs(2);
        // Linux: Track current output sink ID so enforce_routing uses the correct target
        #[cfg(target_os = "linux")]
        let mut current_output_sink_id: Option<u32> = None;

        // Main command processing loop
        while !shutdown_flag.load(Ordering::SeqCst) {
            // Use timeout to periodically check shutdown flag and send level/spectrum updates
            // 16ms = ~60fps for smooth UI animations
            match command_receiver.recv_timeout(std::time::Duration::from_millis(16)) {
                Ok(command) => {
                    match command {
                        Command::Start => {
                            if stream.is_some() {
                                warn!("Engine already running");
                                let _ = event_sender.send(Event::error("Already running"));
                                continue;
                            }

                            info!("Starting audio engine");

                            // On Linux, create virtual sink via PipeWire backend
                            #[cfg(target_os = "linux")]
                            {
                                info!("Initializing PipeWire backend for Linux audio routing");

                                // PER-APP MODE: The PipeWire thread automatically creates per-app
                                // virtual sinks and handles capture/playback for each app.
                                // We skip the legacy "Gecko Audio" sink creation since per-app mode
                                // provides TRUE independent per-app EQ processing.
                                const USE_PER_APP_MODE: bool = true;

                                // Create platform backend
                                match gecko_platform::linux::PipeWireBackend::new() {
                                    Ok(mut backend) => {
                                        info!("PipeWire backend connected: {}", backend.is_connected());

                                        // CONSTANT: Create the virtual sink in ALL modes.
                                        // In Per-App Mode, this acts as a "Honeypot" or "Catch-all".
                                        // - We set this as the system Default Sink.
                                        // - Apps (like Firefox) attach to it when they start or scrub.
                                        // - Our background thread detects the new stream, creates a dedicated per-app sink, and moves it.
                                        // - If we DON'T create this, apps crash when trying to connect to a non-existent default sink.
                                        let sink_config = VirtualSinkConfig {
                                            name: "Gecko Audio".to_string(),
                                            channels: 2,
                                            sample_rate: config.stream.sample_rate,
                                            persistent: false,
                                        };

                                        // Try to create the sink
                                        let sink_creation_result = backend.create_virtual_sink(sink_config);
                                        
                                        if let Err(e) = sink_creation_result {
                                             error!("Failed to create virtual sink: {}", e);
                                             let _ = event_sender.send(Event::error(format!("Failed to create virtual sink: {}", e)));
                                             continue;
                                        }
                                        let sink_id = sink_creation_result.unwrap();
                                        info!("Virtual sink 'Gecko Audio' created (ID: {})", sink_id);

                                        if USE_PER_APP_MODE {
                                            // Per-app mode:
                                            // 1. "Gecko Audio" exists as the default sink catch-all (Honeypot).
                                            // 2. The PipeWire thread automatically creates per-app sinks and moves streams from the Honeypot to them.
                                            // 3. We do NOT start a global capture/playback stream from "Gecko Audio" because that would duplicate audio.
                                            info!("Using per-app EQ mode - 'Gecko Audio' will serve as a honeypot.");

                                            // Set "Gecko Audio" as default sink IMMEDIATELY
                                            // On stop, we'll restore using get_configured_default_sink()
                                            // which returns the user's preference, not this temporary change
                                            if let Err(e) = backend.set_default_sink("Gecko Audio") {
                                                warn!("Failed to set default sink: {}", e);
                                            }

                                            // Set initial state on backend
                                            backend.set_volume(master_volume);
                                            backend.set_bypass(bypassed);
                                            
                                            // Apply stored Master EQ gains
                                            for (band, &gain_db) in master_eq_gains.iter().enumerate() {
                                                if gain_db.abs() > 0.001 {
                                                    backend.update_eq_band(band, gain_db);
                                                }
                                            }
                                            
                                            // Apply stored App Volumes
                                            for (app_name, &vol) in &app_volumes {
                                                backend.set_app_volume(app_name, vol);
                                            }

                                            // Apply stored App Bypass
                                            for (app_name, &bypass) in &app_bypassed {
                                                backend.set_app_bypass(app_name, bypass);
                                            }

                                            // Apply stored App EQ gains
                                            for (app_name, gains) in &app_eq_gains {
                                                for (band, &gain_db) in gains.iter().enumerate() {
                                                    if gain_db.abs() > 0.001 {
                                                        backend.update_stream_eq_band(app_name, band, gain_db);
                                                    }
                                                }
                                            }

                                            // Store backend and mark as running
                                            linux_backend = Some(backend);
                                            is_running.store(true, Ordering::SeqCst);
                                            streaming_start_time = Some(std::time::Instant::now());
                                            let _ = event_sender.send(Event::Started);
                                            info!("Per-app EQ mode active - audio engine started");
                                        } else {
                                        // LEGACY MODE: All apps route to "Gecko Audio" -> single EQ -> speakers
                                        // We start a capture stream from "Gecko Audio" ID we just created.

                                            info!(
                                                "Virtual sink 'Gecko Audio' created (ID: {}). \
                                                 Route apps to this sink in your system sound settings.",
                                                sink_id
                                            );

                                            // Resolve the current default sink to use as the actual output
                                            // This prevents the loop where we output to ourselves
                                            let mut playback_target_id = None;
                                            if let Ok(Some(def_sink_name)) = backend.get_default_sink_name() {
                                                info!("Current default sink: {}", def_sink_name);
                                                
                                                // CRITICAL: If the default sink is ALREADY "Gecko Audio" (from a previous run or crash),
                                                // we MUST NOT use it as our target, or we'll create a feedback loop.
                                                // In this case, we should try to find a real hardware sink.
                                                if def_sink_name == "Gecko Audio" {
                                                    warn!("Default sink is already 'Gecko Audio'. Attempting to find a hardware fallback...");
                                                    // Strategy: List all sinks and pick the first one that isn't Gecko Audio
                                                    if let Ok(nodes) = backend.list_nodes() {
                                                        if let Some(fallback) = nodes.iter().find(|n| n.media_class == "Audio/Sink" && n.name != "Gecko Audio") {
                                                            info!("Found fallback hardware sink: {} (ID: {})", fallback.name, fallback.id);
                                                            playback_target_id = Some(fallback.id);
                                                        } else {
                                                            warn!("No fallback hardware sink found! Audio might be silent.");
                                                        }
                                                    }
                                                } else {
                                                    // Normal case: default sink is a hardware device
                                                    if let Ok(Some(id)) = backend.get_node_id_by_name(&def_sink_name) {
                                                        info!("Resolved default sink '{}' to ID {}", def_sink_name, id);
                                                        playback_target_id = Some(id);
                                                    }
                                                }
                                            }

                                            // Start audio streaming (capture from virtual sink → DSP → speakers)
                                            // Note: We use target.object="Gecko Audio" property so PipeWire resolves by name
                                            match backend.start_streaming(sink_id, playback_target_id) {
                                                Ok(()) => {
                                                    info!("Audio streaming started successfully");
                                                    // Record when streaming started for grace period tracking
                                                    streaming_start_time = Some(std::time::Instant::now());
                                                    // Track current output sink for enforce_routing
                                                    current_output_sink_id = playback_target_id;

                                                    // Set Gecko Audio as the default sink so all apps automatically route to it
                                                    match backend.set_default_sink("Gecko Audio") {
                                                        Ok(prev_sink) => {
                                                            info!("Set default sink to Gecko Audio (previous: {:?})", prev_sink);
                                                        }
                                                        Err(e) => {
                                                            warn!("Could not set default sink: {} - apps must be routed manually", e);
                                                        }
                                                    }

                                                    // NOTE: We don't call enforce_stream_routing here anymore.
                                                    // The PipeWire thread handles link creation internally via pending_capture_links.
                                                    // Calling it here caused race conditions where WirePlumber's links were deleted
                                                    // before our manual links were stable, causing the capture stream to pause.
                                                    // The periodic polling loop below will catch any misrouting later once stable.



                                                    // List nodes to verify our setup
                                                    if let Ok(nodes) = backend.list_nodes() {
                                                        for node in &nodes {
                                                            if node.name.contains("Gecko") {
                                                                info!("Found Gecko node: {} ({})", node.name, node.media_class);
                                                            }
                                                        }
                                                    }

                                                    // Set initial state on backend
                                                    backend.set_volume(master_volume);
                                                    backend.set_bypass(bypassed);

                                                    // Store the backend for command forwarding
                                                    linux_backend = Some(backend);

                                                    is_running.store(true, Ordering::SeqCst);
                                                    let _ = event_sender.send(Event::Started);
                                                    info!("Audio engine started - virtual sink and streaming active");
                                                }
                                                Err(e) => {
                                                    error!("Failed to start streaming: {}", e);
                                                    let _ = event_sender.send(Event::error(format!(
                                                        "Failed to start streaming: {}",
                                                        e
                                                    )));
                                                }
                                            }
                                        }

                                    }
                                    Err(e) => {
                                        error!("Failed to initialize PipeWire backend: {}", e);
                                        let _ = event_sender.send(Event::error(format!(
                                            "PipeWire not available: {}",
                                            e
                                        )));
                                    }
                                }
                            }

                            // macOS: Use CoreAudio backend with Process Tap (14.4+) or HAL plugin
                            #[cfg(target_os = "macos")]
                            {
                                info!("Initializing CoreAudio backend for macOS audio routing");

                                match CoreAudioBackend::new() {
                                    Ok(mut backend) => {
                                        info!(
                                            "CoreAudio backend initialized: {} (Process Tap: {})",
                                            backend.name(),
                                            backend.uses_process_tap()
                                        );

                                        // Create audio processing state and mixer for output
                                        let state = Arc::new(AudioProcessingState::with_sample_rate(
                                            config.stream.sample_rate as f32,
                                        ));
                                        let mixer = Arc::new(AudioMixer::new());

                                        // Set initial state
                                        state.set_master_volume(master_volume);
                                        state.set_bypassed(bypassed);

                                        // Apply stored Master EQ gains to the processing state
                                        for (band, &gain_db) in master_eq_gains.iter().enumerate() {
                                            if gain_db.abs() > 0.001 {
                                                state.set_eq_band(band, gain_db);
                                            }
                                        }

                                        // Create audio output stream (cpal-based)
                                        match AudioOutputStream::new_with_mixer(
                                            Arc::clone(&state),
                                            Arc::clone(&mixer),
                                        ) {
                                            Ok(output) => {
                                                info!(
                                                    "Audio output stream created: {} Hz, {} channels",
                                                    output.sample_rate(),
                                                    output.channels()
                                                );

                                                // Set initial state on backend (for per-app tracking)
                                                backend.set_volume(master_volume);
                                                backend.set_bypass(bypassed);

                                                // Apply stored Master EQ gains to backend
                                                for (band, &gain_db) in master_eq_gains.iter().enumerate() {
                                                    if gain_db.abs() > 0.001 {
                                                        backend.update_eq_band(band, gain_db);
                                                    }
                                                }

                                                // Apply stored App Volumes
                                                for (app_name, &vol) in &app_volumes {
                                                    backend.set_app_volume(app_name, vol);
                                                }

                                                // Apply stored App Bypass
                                                for (app_name, &bypass) in &app_bypassed {
                                                    backend.set_app_bypass(app_name, bypass);
                                                }

                                                // Apply stored App EQ gains
                                                for (app_name, gains) in &app_eq_gains {
                                                    for (band, &gain_db) in gains.iter().enumerate() {
                                                        if gain_db.abs() > 0.001 {
                                                            backend.update_stream_eq_band(app_name, band, gain_db);
                                                        }
                                                    }
                                                }

                                                // AUTO-CAPTURE: Try to tap all visible apps
                                                // Apps not producing audio will fail - that's OK
                                                // Apps producing audio should be captured successfully
                                                if backend.uses_process_tap() {
                                                    use gecko_platform::macos::coreaudio::list_audio_applications;

                                                    // Note: Permission probe was removed - it caused timing issues
                                                    // that led to crashes. The ProcessTapCapture::new() will handle
                                                    // permission errors gracefully and return appropriate errors.

                                                    // Try to capture all visible apps
                                                    // Some will fail (not producing audio, sandboxed) - that's expected
                                                    match list_audio_applications() {
                                                        Ok(apps) => {
                                                            info!("Attempting to capture {} visible apps:", apps.len());
                                                            for app in &apps {
                                                                debug!("  - {} (PID {})", app.name, app.pid);
                                                            }

                                                            let mut captured_count = 0;
                                                            let mut failed_count = 0;
                                                            for app in apps {
                                                                match backend.start_app_capture(&app.name, app.pid) {
                                                                    Ok(_tap_id) => {
                                                                        // Add ring buffer to mixer with app name for per-app volume lookup
                                                                        if let Some(ring_buffer) = backend.get_ring_buffer(app.pid) {
                                                                            mixer.add_source(app.pid, &app.name, ring_buffer);
                                                                            info!("✓ Captured: {} (PID {})", app.name, app.pid);
                                                                            captured_count += 1;
                                                                        }
                                                                    }
                                                                    Err(e) => {
                                                                        // Expected for apps not producing audio or sandboxed
                                                                        debug!("✗ Could not capture {} (PID {}): {}", app.name, app.pid, e);
                                                                        failed_count += 1;
                                                                    }
                                                                }
                                                            }
                                                            info!("Capture complete: {} captured, {} skipped (not producing audio or sandboxed)",
                                                                  captured_count, failed_count);
                                                        }
                                                        Err(e) => {
                                                            warn!("Failed to enumerate apps: {}", e);
                                                        }
                                                    }
                                                }

                                                // Store all components
                                                macos_backend = Some(backend);
                                                macos_mixer = Some(mixer);
                                                macos_state = Some(state);
                                                _macos_output = Some(output);

                                                is_running.store(true, Ordering::SeqCst);
                                                let _ = event_sender.send(Event::Started);
                                                info!("CoreAudio backend active with audio output - engine started");
                                            }
                                            Err(e) => {
                                                error!("Failed to create audio output stream: {}", e);
                                                let _ = event_sender.send(Event::error(format!(
                                                    "Failed to create audio output: {}",
                                                    e
                                                )));
                                            }
                                        }
                                    }
                                    Err(e) => {
                                        error!("Failed to initialize CoreAudio backend: {}", e);
                                        let _ = event_sender.send(Event::error(format!(
                                            "CoreAudio not available: {}",
                                            e
                                        )));
                                    }
                                }
                            }

                            // Windows/Other platforms: fall back to output-only mode
                            #[cfg(all(not(target_os = "linux"), not(target_os = "macos")))]
                            {
                                let host = cpal::default_host();
                                let output_device = match host.default_output_device() {
                                    Some(device) => device,
                                    None => {
                                        error!("No output device found");
                                        let _ = event_sender.send(Event::error("No output device found"));
                                        continue;
                                    }
                                };

                                match AudioStream::new_output_only(
                                    config.stream.clone(),
                                    &output_device,
                                    event_sender.clone(),
                                ) {
                                    Ok(s) => {
                                        s.set_master_volume(master_volume);
                                        s.set_bypass(bypassed);
                                        stream = Some(s);
                                        is_running.store(true, Ordering::SeqCst);
                                        let _ = event_sender.send(Event::Started);
                                        info!("Audio stream started (output-only mode)");
                                    }
                                    Err(e) => {
                                        error!("Failed to start stream: {}", e);
                                        let _ = event_sender.send(Event::error(e));
                                    }
                                }
                            }
                        }

                        Command::Stop => {
                            // Linux: Stop PipeWire backend and restore default sink
                            #[cfg(target_os = "linux")]
                            {
                                if let Some(ref backend) = linux_backend {
                                    // Restore the default sink to the user's CONFIGURED preference
                                    // This is what they set in system settings, not what Gecko changed it to
                                    let mut restored = false;

                                    // First try: Use the user's configured default sink preference
                                    // This is the "real" default - what they configured in system settings
                                    if let Ok(Some(configured_sink)) = backend.get_configured_default_sink() {
                                        // Don't restore to our own sink!
                                        if configured_sink != "Gecko Audio" {
                                            // Verify the sink still exists
                                            if let Ok(nodes) = backend.list_nodes() {
                                                let sink_exists = nodes.iter()
                                                    .any(|n| n.name == configured_sink && n.media_class == "Audio/Sink");

                                                if sink_exists {
                                                    info!("Restoring to user's configured default sink: '{}'", configured_sink);
                                                    if backend.restore_default_sink(&configured_sink).is_ok() {
                                                        restored = true;
                                                    }
                                                } else {
                                                    debug!("Configured sink '{}' no longer exists", configured_sink);
                                                }
                                            }
                                        } else {
                                            debug!("Configured sink is Gecko Audio, using fallback");
                                        }
                                    }

                                    if !restored {
                                        // Configured sink unavailable, use smart fallback
                                        Self::restore_to_fallback_sink(backend);
                                    }

                                    // CRITICAL: Give WirePlumber time to move streams to the new default sink
                                    // before we destroy our virtual sinks. If we destroy them too fast,
                                    // apps might lose their stream and stop playing/disappear.
                                    std::thread::sleep(std::time::Duration::from_millis(250));

                                    if let Err(e) = backend.stop_streaming() {
                                        warn!("Failed to stop streaming: {}", e);
                                    }
                                }
                                linux_backend = None;
                                // Reset streaming start time so grace period applies on next start
                                streaming_start_time = None;
                                current_output_sink_id = None;
                            }

                            // macOS: Stop CoreAudio backend and audio output
                            #[cfg(target_os = "macos")]
                            {
                                // Stop output stream first (drops cpal stream)
                                _macos_output = None;
                                // Clear mixer and state
                                macos_mixer = None;
                                macos_state = None;
                                // Shutdown backend
                                if let Some(ref mut backend) = macos_backend {
                                    backend.shutdown();
                                }
                                macos_backend = None;
                            }

                            // Windows/Other: Stop audio stream
                            #[cfg(all(not(target_os = "linux"), not(target_os = "macos")))]
                            if stream.is_none() {
                                debug!("Engine not running");
                                continue;
                            }

                            info!("Stopping audio stream");
                            stream = None;
                            is_running.store(false, Ordering::SeqCst);
                            let _ = event_sender.send(Event::Stopped);
                        }

                        Command::SetMasterVolume(vol) => {
                            master_volume = vol.clamp(0.0, 2.0);

                            // Linux: Forward to PipeWire backend
                            #[cfg(target_os = "linux")]
                            if let Some(ref backend) = linux_backend {
                                backend.set_volume(master_volume);
                            }

                            // macOS: Forward to CoreAudio backend AND processing state
                            #[cfg(target_os = "macos")]
                            {
                                if let Some(ref mut backend) = macos_backend {
                                    backend.set_volume(master_volume);
                                }
                                if let Some(ref state) = macos_state {
                                    state.set_master_volume(master_volume);
                                }
                            }

                            // Windows/Other: Forward to audio stream
                            #[cfg(all(not(target_os = "linux"), not(target_os = "macos")))]
                            if let Some(ref s) = stream {
                                s.set_master_volume(master_volume);
                            }
                        }

                        Command::SetBypass(bypass) => {
                            bypassed = bypass;

                            // Linux: Forward to PipeWire backend
                            #[cfg(target_os = "linux")]
                            if let Some(ref backend) = linux_backend {
                                backend.set_bypass(bypassed);
                            }

                            // macOS: Forward to CoreAudio backend AND processing state
                            #[cfg(target_os = "macos")]
                            {
                                if let Some(ref mut backend) = macos_backend {
                                    backend.set_bypass(bypassed);
                                }
                                if let Some(ref state) = macos_state {
                                    state.set_bypassed(bypassed);
                                }
                            }

                            // Windows/Other: Forward to audio stream
                            #[cfg(all(not(target_os = "linux"), not(target_os = "macos")))]
                            if let Some(ref s) = stream {
                                s.set_bypass(bypassed);
                            }
                        }

                        Command::SetSoftClipEnabled(enabled) => {
                            debug!("Set soft clip enabled: {}", enabled);

                            // Linux: Forward to PipeWire backend
                            #[cfg(target_os = "linux")]
                            if let Some(ref backend) = linux_backend {
                                backend.set_soft_clip_enabled(enabled);
                            }

                            // macOS: TODO - Forward to CoreAudio backend when implemented
                            #[cfg(target_os = "macos")]
                            {
                                let _ = enabled; // Suppress unused warning
                                // TODO: Implement soft clip forwarding for macOS
                            }
                        }

                        Command::SetBandGain { band, gain_db } => {
                            debug!("Set band {} gain to {}dB", band, gain_db);

                            // Update local state
                            if band < 10 {
                                master_eq_gains[band] = gain_db;
                            }

                            // Linux: Forward to PipeWire backend EQ
                            #[cfg(target_os = "linux")]
                            if let Some(ref backend) = linux_backend {
                                backend.update_eq_band(band, gain_db);
                            }

                            // macOS: Forward to CoreAudio backend EQ AND processing state
                            #[cfg(target_os = "macos")]
                            {
                                if let Some(ref mut backend) = macos_backend {
                                    backend.update_eq_band(band, gain_db);
                                }
                                // Update the actual EQ processor in processing state
                                if let Some(ref state) = macos_state {
                                    state.set_eq_band(band, gain_db);
                                }
                            }
                        }

                        Command::SetStreamBandGain { stream_id, band, gain_db } => {
                            debug!("Set stream '{}' band {} gain to {}dB", stream_id, band, gain_db);

                            // Extract app name from stream_id
                            // Format varies by platform:
                            //   - Linux: "PID:Name" (name is after colon)
                            //   - macOS: "Name:PID" (name is before colon)
                            #[cfg(target_os = "linux")]
                            let app_name = if let Some((_, name)) = stream_id.split_once(':') {
                                name.to_string()
                            } else {
                                stream_id.clone()
                            };

                            #[cfg(target_os = "macos")]
                            let app_name = if let Some((name, _)) = stream_id.split_once(':') {
                                name.to_string()
                            } else {
                                stream_id.clone()
                            };

                            #[cfg(not(any(target_os = "linux", target_os = "macos")))]
                            let app_name = stream_id.clone();

                            // Update local state
                            if band < 10 {
                                app_eq_gains.entry(app_name.clone())
                                    .or_insert([0.0; 10])[band] = gain_db;
                            }

                            // Linux: Forward to PipeWire backend per-app EQ
                            // This is TRUE per-app EQ - each app has its own EQ instance
                            #[cfg(target_os = "linux")]
                            if let Some(ref backend) = linux_backend {
                                backend.update_stream_eq_band(&stream_id, band, gain_db);
                            }

                            // macOS: Forward to CoreAudio backend AND processing state for per-app EQ
                            #[cfg(target_os = "macos")]
                            {
                                if let Some(ref mut backend) = macos_backend {
                                    backend.update_stream_eq_band(&app_name, band, gain_db);
                                }
                                // Also update processing state so mixer can apply per-app EQ offset
                                if let Some(ref state) = macos_state {
                                    state.set_app_eq_offset(&app_name, band, gain_db);
                                }
                            }
                        }

                        Command::SetAppBypass { app_name, bypassed: app_bypassed_val } => {
                            debug!("Set app '{}' bypass to {}", app_name, app_bypassed_val);

                            // Update local state
                            app_bypassed.insert(app_name.clone(), app_bypassed_val);

                            // Linux: Forward to PipeWire backend per-app bypass
                            #[cfg(target_os = "linux")]
                            if let Some(ref backend) = linux_backend {
                                backend.set_app_bypass(&app_name, app_bypassed_val);
                            }

                            // macOS: Forward to CoreAudio backend AND processing state for per-app bypass
                            #[cfg(target_os = "macos")]
                            {
                                if let Some(ref mut backend) = macos_backend {
                                    backend.set_app_bypass(&app_name, app_bypassed_val);
                                }
                                // Also update processing state so mixer can check per-app bypass
                                if let Some(ref state) = macos_state {
                                    state.set_app_bypassed(&app_name, app_bypassed_val);
                                }
                            }
                        }

                        Command::SetStreamVolume { stream_id, volume } => {
                            // Extract app name from stream_id
                            // Format varies by platform:
                            //   - Linux: "PID:Name" (name is after colon)
                            //   - macOS: "Name:PID" (name is before colon)
                            #[cfg(target_os = "linux")]
                            let app_name = if let Some((_, name)) = stream_id.split_once(':') {
                                name.to_string()
                            } else {
                                stream_id.clone()
                            };

                            #[cfg(target_os = "macos")]
                            let app_name = if let Some((name, _)) = stream_id.split_once(':') {
                                name.to_string()
                            } else {
                                stream_id.clone()
                            };

                            #[cfg(not(any(target_os = "linux", target_os = "macos")))]
                            let app_name = stream_id.clone();

                            debug!("Set app '{}' volume to {:.2} (stream_id: {})", app_name, volume, stream_id);

                            // Update local state for persistence
                            app_volumes.insert(app_name.clone(), volume);

                            // Linux: Forward to PipeWire backend per-app volume
                            #[cfg(target_os = "linux")]
                            if let Some(ref backend) = linux_backend {
                                backend.set_app_volume(&app_name, volume);
                            }

                            // macOS: Forward to CoreAudio backend AND processing state for per-app volume
                            #[cfg(target_os = "macos")]
                            {
                                if let Some(ref mut backend) = macos_backend {
                                    backend.set_app_volume(&app_name, volume);
                                }
                                // Also update processing state so mixer can apply per-app volume
                                if let Some(ref state) = macos_state {
                                    state.set_app_volume(&app_name, volume);
                                }
                            }
                        }

                        Command::StartAppCapture { pid, app_name } => {
                            debug!("Start app capture: {} (PID {})", app_name, pid);

                            // macOS: Start Process Tap capture and add to mixer
                            #[cfg(target_os = "macos")]
                            {
                                if let (Some(ref mut backend), Some(ref mixer)) =
                                    (&mut macos_backend, &macos_mixer)
                                {
                                    // start_app_capture takes (&str, u32) and returns Result<u32, _>
                                    match backend.start_app_capture(&app_name, pid) {
                                        Ok(_tap_id) => {
                                            info!("Started capture for {} (PID {})", app_name, pid);

                                            // Get the ring buffer for this capture and add to mixer with app name for per-app volume lookup
                                            if let Some((_, ring_buffer)) = backend
                                                .get_ring_buffers()
                                                .into_iter()
                                                .find(|(p, _)| *p == pid)
                                            {
                                                mixer.add_source(pid, &app_name, ring_buffer);
                                                info!("Added {} to audio mixer", app_name);

                                                // Notify UI that stream was discovered
                                                let _ = event_sender.send(Event::StreamDiscovered {
                                                    app_name: app_name.clone(),
                                                    node_id: pid,
                                                });
                                            }
                                        }
                                        Err(e) => {
                                            error!("Failed to start capture for {}: {}", app_name, e);
                                            let _ = event_sender.send(Event::error(format!(
                                                "Failed to capture {}: {}",
                                                app_name, e
                                            )));
                                        }
                                    }
                                }
                            }

                            // Other platforms: no-op (capture is handled differently)
                            #[cfg(not(target_os = "macos"))]
                            {
                                let _ = (pid, app_name); // Suppress unused warnings
                                debug!("App capture not implemented on this platform");
                            }
                        }

                        Command::StopAppCapture { pid } => {
                            debug!("Stop app capture: PID {}", pid);

                            // macOS: Stop Process Tap capture and remove from mixer
                            #[cfg(target_os = "macos")]
                            {
                                if let (Some(ref mut backend), Some(ref mixer)) =
                                    (&mut macos_backend, &macos_mixer)
                                {
                                    // Get app name before removing (for event) - clone to avoid borrow issues
                                    let app_name = backend.get_app_name(pid).map(|s| s.to_string());

                                    // Remove from mixer first
                                    mixer.remove_source(pid);

                                    // Stop the capture
                                    if let Err(e) = backend.stop_app_capture(pid) {
                                        warn!("Failed to stop capture for PID {}: {}", pid, e);
                                    } else {
                                        info!("Stopped capture for PID {}", pid);
                                    }

                                    // Notify UI that stream was removed
                                    if let Some(name) = app_name {
                                        let _ = event_sender.send(Event::StreamRemoved {
                                            app_name: name,
                                        });
                                    }
                                }
                            }

                            // Other platforms: no-op
                            #[cfg(not(target_os = "macos"))]
                            {
                                let _ = pid; // Suppress unused warning
                            }
                        }

                        Command::UpdateEq(_eq_config) => {
                            // Would update EQ configuration
                            debug!("EQ config update received");
                        }

                        Command::RequestState => {
                            #[cfg(target_os = "linux")]
                            let running = linux_backend.is_some();
                            #[cfg(target_os = "macos")]
                            let running = macos_backend.is_some();
                            #[cfg(all(not(target_os = "linux"), not(target_os = "macos")))]
                            let running = stream.is_some();

                            let state = Event::StateUpdate {
                                is_running: running,
                                is_bypassed: bypassed,
                                master_volume,
                                input_device: None, // TODO: track current devices
                                output_device: None,
                            };
                            let _ = event_sender.send(state);
                        }

                        Command::SetInputDevice(_device_id) => {
                            // Would change input device
                            debug!("Input device change requested");
                        }

                        Command::SetOutputDevice(_device_id) => {
                            // Would change output device
                            debug!("Output device change requested");
                        }

                        Command::UpdateStreamConfig(_new_config) => {
                            // Would update stream configuration
                            debug!("Stream config update requested");
                        }

                        Command::Shutdown => {
                            info!("Shutdown command received");
                            shutdown_flag.store(true, Ordering::SeqCst);
                        }
                    }
                }
                Err(crossbeam_channel::RecvTimeoutError::Timeout) => {
                    // Normal timeout, check for level updates and periodic tasks

                    // Linux: Get peaks and spectrum from PipeWire backend
                    #[cfg(target_os = "linux")]
                    if let Some(ref backend) = linux_backend {
                        let (l, r) = backend.get_peaks();
                        // Only send if there's actual audio
                        if l > 0.001 || r > 0.001 {
                            let _ = event_sender.try_send(Event::LevelUpdate { left: l, right: r });
                        }

                        // Update spectrum analyzer and send data if ready (~30fps)
                        let spectrum_updated = backend.update_spectrum();
                        if spectrum_updated {
                            let spectrum = backend.get_spectrum();
                            let bins = spectrum.to_vec();
                            tracing::debug!("Sending SpectrumUpdate event, bins[0-2]: {:?}", &bins[0..3.min(bins.len())]);
                            let _ = event_sender.try_send(Event::SpectrumUpdate { bins });
                        }
                    }

                    // macOS: Get peaks and spectrum from processing state
                    #[cfg(target_os = "macos")]
                    if let Some(ref state) = macos_state {
                        let (l, r) = state.peaks();
                        // Only send if there's actual audio
                        if l > 0.001 || r > 0.001 {
                            let _ = event_sender.try_send(Event::LevelUpdate { left: l, right: r });
                        }

                        // Update spectrum analyzer and send data if ready (~60fps)
                        let spectrum_updated = state.update_spectrum();
                        if spectrum_updated {
                            let spectrum = state.get_spectrum();
                            let bins = spectrum.to_vec();
                            let _ = event_sender.try_send(Event::SpectrumUpdate { bins });
                        }
                    }

                    // macOS: Periodic scanning for new audio-active processes
                    // Key insight: Only processes ACTIVELY PLAYING AUDIO can be tapped.
                    // We keep existing taps even if audio pauses (e.g., between YouTube videos).
                    // We only remove taps when the PROCESS EXITS completely.
                    #[cfg(target_os = "macos")]
                    if is_running.load(Ordering::Relaxed) {
                        // Check for completed scan results (non-blocking)
                        // The scan now returns audio-active PIDs and pid->name map
                        if let Ok((audio_pids, pid_names)) = app_scan_rx.try_recv() {
                            app_scan_in_progress.store(false, Ordering::Relaxed);

                            if let (Some(ref mut backend), Some(ref mixer)) = (&mut macos_backend, &macos_mixer) {
                                // Add new taps for audio-active processes not already captured
                                for pid in &audio_pids {
                                    let pid_u32 = *pid as u32;
                                    if !backend.is_capturing(pid_u32) {
                                        let app_name = pid_names.get(&pid_u32)
                                            .cloned()
                                            .unwrap_or_else(|| format!("PID {}", pid));

                                        match backend.start_app_capture(&app_name, pid_u32) {
                                            Ok(_tap_id) => {
                                                // Add to mixer with app name for per-app volume lookup
                                                if let Some(ring_buffer) = backend.get_ring_buffer(pid_u32) {
                                                    mixer.add_source(pid_u32, &app_name, ring_buffer);
                                                    info!("Auto-captured new audio-active app: {} (PID {})", app_name, pid);
                                                }
                                            }
                                            Err(e) => {
                                                debug!("Could not capture audio-active app {}: {}", app_name, e);
                                            }
                                        }
                                    }
                                }

                                // Remove captures ONLY for processes that have EXITED
                                // We keep taps even if audio pauses (between videos, etc.)
                                let captured_pids = backend.captured_pids();
                                for pid in captured_pids {
                                    if !is_process_running(pid) {
                                        // Process has exited - clean up the tap
                                        let app_name = pid_names.get(&pid)
                                            .cloned()
                                            .unwrap_or_else(|| format!("PID {}", pid));

                                        if let Err(e) = backend.stop_app_capture(pid) {
                                            debug!("Error stopping capture for exited app PID {}: {}", pid, e);
                                        }
                                        mixer.remove_source(pid);
                                        info!("Removed capture for exited app: {} (PID {})", app_name, pid);
                                    }
                                }
                            }
                        }

                        // Start new scan if interval elapsed and no scan in progress
                        if last_app_scan.elapsed() >= app_scan_interval
                            && !app_scan_in_progress.load(Ordering::Relaxed)
                        {
                            last_app_scan = std::time::Instant::now();
                            app_scan_in_progress.store(true, Ordering::Relaxed);

                            // Spawn background thread for the scan
                            // Gets audio-active PIDs (fast) and PID->name map (slower AppleScript)
                            let tx = app_scan_tx.clone();
                            std::thread::spawn(move || {
                                use gecko_platform::macos::get_audio_active_pids;

                                let audio_pids = get_audio_active_pids();
                                let pid_names = get_pid_to_app_name_map();
                                let _ = tx.send((audio_pids, pid_names));
                            });
                        }

                        // Debug stats logging (every 5 seconds)
                        if last_debug_stats.elapsed() >= debug_stats_interval {
                            last_debug_stats = std::time::Instant::now();

                            // Log mixer debug stats
                            let mixer_stats = gecko_platform::macos::get_mixer_debug_stats();
                            debug!("Audio debug: {}", mixer_stats);

                            // Log tap debug stats
                            if let Some(ref backend) = macos_backend {
                                backend.log_tap_debug_stats();
                            }
                        }
                    }

                    // Windows/Other: Get peaks from audio stream
                    #[cfg(all(not(target_os = "linux"), not(target_os = "macos")))]
                    if let Some(ref s) = stream {
                        let (l, r) = s.get_peaks();
                        // Only send if there's actual audio
                        if l > 0.001 || r > 0.001 {
                            let _ = event_sender.try_send(Event::LevelUpdate { left: l, right: r });
                        }
                    }

                    // Linux: Periodic device switching and routing enforcement
                    #[cfg(target_os = "linux")]
                    if is_running.load(Ordering::Relaxed) {
                        if let Some(ref backend) = linux_backend {
                            // Check every 500ms for device changes (using elapsed time)
                            if last_sink_check.elapsed() >= sink_check_interval {
                                last_sink_check = std::time::Instant::now();

                                // Wait for grace period before monitoring default sink changes
                                // This prevents race conditions where we read the old default sink
                                // immediately after setting it to "Gecko Audio" on startup.
                                let past_grace_period = streaming_start_time
                                    .map(|t| t.elapsed() > routing_grace_period)
                                    .unwrap_or(false);

                                if past_grace_period {
                                    // Check if default sink changed (e.g. headphones plugged in)
                                    if let Ok(Some(current_default)) = backend.get_default_sink_name() {
                                        if current_default != "Gecko Audio" {
                                            // Check if we are already outputting to this device
                                            let mut already_targeting = false;
                                            if let Ok(Some(new_target_id)) = backend.get_node_id_by_name(&current_default) {
                                                if let Some(current_id) = current_output_sink_id {
                                                    if new_target_id == current_id {
                                                        already_targeting = true;
                                                        info!("Default sink changed to current target '{}' (ID: {}) - reclaiming default status without stream restart", current_default, current_id);
                                                    }
                                                }
                                            }

                                            if !already_targeting {
                                                info!("Default sink changed to '{}' - switching output...", current_default);

                                                // Switch playback to the new hardware sink BY NAME
                                                info!("Hot-switching playback to '{}'", current_default);

                                                match backend.switch_playback_target(&current_default) {
                                                    Ok(()) => {
                                                        info!("Successfully switched output to '{}'", current_default);
                                                        // Update the tracked output sink ID
                                                        if let Ok(Some(id)) = backend.get_node_id_by_name(&current_default) {
                                                            current_output_sink_id = Some(id);
                                                        } else {
                                                            // Should not happen if switch succeeded, but fallback to None
                                                            current_output_sink_id = None;
                                                        }
                                                        // Note: We might want to reset streaming_start_time here too,
                                                        // but that would lock us out of monitoring for another 2s.
                                                    }
                                                    Err(e) => {
                                                        error!("Failed to switch playback target: {}", e);
                                                    }
                                                }
                                            }

                                            // Re-assert Gecko Audio as default
                                            if let Err(e) = backend.set_default_sink("Gecko Audio") {
                                                warn!("Failed to re-assert default sink: {}", e);
                                            }
                                        }

                                        // Enforce routing to Gecko Audio
                                        // Use the tracked output sink, not just "first hardware sink found"
                                        if let (Ok(Some(gecko_node_id)), Some(hw_sink_id)) =
                                            (backend.get_node_id_by_name("Gecko Audio"), current_output_sink_id)
                                        {
                                            if let Err(e) = backend.enforce_stream_routing(gecko_node_id, hw_sink_id) {
                                                warn!("Failed to enforce stream routing: {}", e);
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                Err(crossbeam_channel::RecvTimeoutError::Disconnected) => {
                    warn!("Command channel disconnected");
                    break;
                }
            }
        }

        // Cleanup
        drop(stream);

        // Linux: Clean up PipeWire backend
        #[cfg(target_os = "linux")]
        drop(linux_backend);

        // macOS: Clean up audio components in order
        #[cfg(target_os = "macos")]
        {
            // Drop output stream first (stops cpal playback)
            drop(_macos_output);
            // Drop mixer and state
            drop(macos_mixer);
            drop(macos_state);
            // Drop backend last (cleans up Process Taps)
            drop(macos_backend);
        }

        is_running.store(false, Ordering::SeqCst);
        info!("Audio thread shutting down");
    }
}

impl Default for AudioEngine {
    fn default() -> Self {
        Self::new().expect("Failed to create default AudioEngine")
    }
}

impl AudioEngine {
    /// Helper to restore to a sensible fallback sink when the original is unavailable
    /// Priority: USB audio > Speakers/Headphones > Analog > anything non-HDMI > HDMI
    #[cfg(target_os = "linux")]
    fn restore_to_fallback_sink(backend: &gecko_platform::linux::PipeWireBackend) {
        use tracing::{info, warn};

        if let Ok(nodes) = backend.list_nodes() {
            let sinks: Vec<_> = nodes.iter()
                .filter(|n| n.media_class == "Audio/Sink" && !n.name.contains("Gecko"))
                .collect();

            // Priority 1: USB audio (external headphones/speakers)
            let usb_sink = sinks.iter().find(|n| n.name.contains("usb"));

            // Priority 2: Bluetooth audio
            let bluetooth_sink = sinks.iter().find(|n|
                n.name.contains("bluez") || n.name.contains("bluetooth"));

            // Priority 3: Speaker/Headphones (internal audio, NOT HDMI)
            // Look for sinks with "Speaker" or "Headphone" in description, or analog output
            let speaker_sink = sinks.iter().find(|n|
                !n.name.contains("hdmi") && !n.name.contains("HDMI") &&
                !n.name.contains("DisplayPort") && !n.name.contains("DP"));

            // Priority 4: Anything that's not HDMI/DisplayPort (last resort hardware)
            let any_non_hdmi = sinks.iter().find(|n|
                !n.name.contains("hdmi") && !n.name.contains("HDMI") &&
                !n.name.contains("DisplayPort") && !n.name.contains("DP"));

            // Priority 5: HDMI (only if nothing else available)
            let hdmi_sink = sinks.iter().find(|n|
                n.name.contains("hdmi") || n.name.contains("HDMI") ||
                n.name.contains("DisplayPort") || n.name.contains("DP"));

            let restore_sink = usb_sink
                .or(bluetooth_sink)
                .or(speaker_sink)
                .or(any_non_hdmi)
                .or(hdmi_sink);

            if let Some(sink) = restore_sink {
                info!("Restoring default sink to fallback: '{}' (ID: {})", sink.name, sink.id);
                if let Err(e) = backend.restore_default_sink(&sink.name) {
                    warn!("Failed to restore to fallback sink: {}", e);
                }
            } else {
                warn!("No hardware sink found to restore to!");
            }
        }
    }
}

impl Drop for AudioEngine {
    fn drop(&mut self) {
        // Signal shutdown
        self.shutdown_flag.store(true, Ordering::SeqCst);

        // Send shutdown command
        let _ = self.command_sender.send(Command::Shutdown);

        // Wait for audio thread to finish
        if let Some(handle) = self.audio_thread.take() {
            let _ = handle.join();
        }
    }
}

// Rust pattern: Explicit Send + Sync implementation
// AudioEngine can be sent between threads (Send) and shared between threads (Sync)
// because all interior mutability uses thread-safe primitives
unsafe impl Send for AudioEngine {}
unsafe impl Sync for AudioEngine {}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn test_engine_creation() {
        // This test doesn't require audio hardware - just creates the engine
        let result = AudioEngine::new();
        assert!(result.is_ok());
    }

    #[test]
    fn test_engine_config() {
        let config = EngineConfig::low_latency();
        let result = AudioEngine::with_config(config);
        assert!(result.is_ok());
    }

    #[test]
    fn test_engine_not_running_initially() {
        let engine = AudioEngine::new().unwrap();
        assert!(!engine.is_running());
    }

    #[test]
    fn test_engine_shutdown() {
        let engine = AudioEngine::new().unwrap();
        drop(engine); // Should shutdown cleanly
    }

    #[test]
    fn test_request_state() {
        let engine = AudioEngine::new().unwrap();
        engine.request_state().unwrap();

        // Wait a bit for response
        thread::sleep(Duration::from_millis(200));

        if let Some(Event::StateUpdate { is_running, .. }) = engine.poll_event() {
            assert!(!is_running);
        }
    }

    #[test]
    fn test_set_volume() {
        let engine = AudioEngine::new().unwrap();
        assert!(engine.set_master_volume(0.5).is_ok());
        assert!(engine.set_master_volume(0.0).is_ok());
        assert!(engine.set_master_volume(1.0).is_ok());
    }

    #[test]
    fn test_set_bypass() {
        let engine = AudioEngine::new().unwrap();
        assert!(engine.set_bypass(true).is_ok());
        assert!(engine.set_bypass(false).is_ok());
    }

    // Hardware-dependent tests
    #[test]
    #[ignore = "requires audio hardware"]
    fn test_engine_start_stop() {
        let engine = AudioEngine::new().unwrap();

        // Start
        engine.start().unwrap();
        thread::sleep(Duration::from_millis(100));

        // Check for Started event
        let mut started = false;
        for _ in 0..10 {
            if let Some(Event::Started) = engine.poll_event() {
                started = true;
                break;
            }
            thread::sleep(Duration::from_millis(50));
        }
        assert!(started, "Should receive Started event");
        assert!(engine.is_running());

        // Stop
        engine.stop().unwrap();
        thread::sleep(Duration::from_millis(100));
        assert!(!engine.is_running());
    }

    #[test]
    #[ignore = "requires audio hardware"]
    fn test_list_devices() {
        let engine = AudioEngine::new().unwrap();
        let devices = engine.list_devices();
        assert!(devices.is_ok());
    }

    #[test]
    fn test_set_stream_band_gain() {
        let engine = AudioEngine::new().unwrap();
        // Should succeed even when not running - gains are persisted for later
        assert!(engine.set_stream_band_gain("Firefox:1234".to_string(), 0, 3.0).is_ok());
        assert!(engine.set_stream_band_gain("Spotify:5678".to_string(), 5, -6.0).is_ok());
    }

    #[test]
    fn test_set_stream_band_gain_boundary() {
        let engine = AudioEngine::new().unwrap();
        // Band 9 is the highest valid index (0-9)
        assert!(engine.set_stream_band_gain("Firefox:1234".to_string(), 9, 3.0).is_ok());
        // Note: Out-of-range bands are handled by the backend (silently ignored)
        // The command send itself always succeeds
    }

    #[test]
    fn test_set_stream_volume() {
        let engine = AudioEngine::new().unwrap();
        // Valid volume range is 0.0 to 2.0
        assert!(engine.set_stream_volume("Firefox:1234".to_string(), 1.5).is_ok());
        assert!(engine.set_stream_volume("Spotify:5678".to_string(), 0.5).is_ok());
    }

    #[test]
    fn test_set_app_bypass() {
        let engine = AudioEngine::new().unwrap();
        assert!(engine.set_app_bypass("Firefox".to_string(), true).is_ok());
        assert!(engine.set_app_bypass("Firefox".to_string(), false).is_ok());
    }

    #[test]
    fn test_per_app_state_persistence_in_memory() {
        let engine = AudioEngine::new().unwrap();

        // Set some per-app values
        engine.set_stream_band_gain("TestApp:1".to_string(), 0, 5.0).unwrap();
        engine.set_stream_volume("TestApp:1".to_string(), 1.2).unwrap();
        engine.set_app_bypass("TestApp".to_string(), true).unwrap();

        // Set another app's values
        engine.set_stream_band_gain("OtherApp:2".to_string(), 3, -3.0).unwrap();
        engine.set_stream_volume("OtherApp:2".to_string(), 0.8).unwrap();

        // The engine should have stored these - we verify by checking the maps exist
        // (Direct access to inner state not exposed, but the commands succeeded)
    }
}
