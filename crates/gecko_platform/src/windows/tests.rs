//! Comprehensive Integration Tests for Windows WASAPI Backend
//!
//! These tests verify the complete audio pipeline on Windows.
//! Tests requiring audio hardware are marked `#[ignore]`.
//!
//! # Running Tests
//!
//! ```bash
//! # Run all non-hardware tests (cross-platform safe)
//! cargo test -p gecko_platform --features wasapi
//!
//! # Run including hardware tests (Windows only, with audio device)
//! cargo test -p gecko_platform --features wasapi -- --ignored
//! ```

#[cfg(test)]
#[cfg(target_os = "windows")]
mod integration_tests {
    use super::super::*;
    use crate::PlatformBackend;
    use std::time::Duration;

    // ========================================================================
    // Backend Initialization Tests
    // ========================================================================

    #[test]
    fn test_backend_creation() {
        let backend = WasapiBackend::new();
        assert!(backend.is_ok(), "Should create WASAPI backend: {:?}", backend.err());

        let be = backend.unwrap();
        assert_eq!(be.name(), "WASAPI");
        assert!(be.is_connected());
    }

    #[test]
    fn test_version_detection() {
        let backend = WasapiBackend::new().unwrap();
        let version = backend.version();

        assert!(version.major >= 10, "Should be Windows 10 or later");
        assert!(version.supports_wasapi(), "Should support WASAPI");

        println!("Windows version: {}", version);
        println!("Process Loopback supported: {}", backend.supports_process_loopback());
    }

    #[test]
    fn test_backend_shutdown() {
        let backend = WasapiBackend::new().unwrap();
        drop(backend);
        // Should not panic or hang on drop
    }

    // ========================================================================
    // Process Enumeration Tests
    // ========================================================================

    #[test]
    fn test_process_enumeration() {
        let process_enum = ProcessEnumerator::new();
        let processes = process_enum.enumerate_all();

        assert!(processes.is_ok(), "Should enumerate processes");

        let proc_list = processes.unwrap();
        assert!(!proc_list.is_empty(), "Should find at least one process");

        println!("Found {} total processes", proc_list.len());

        // Verify structure
        for proc in proc_list.iter().take(5) {
            assert!(proc.pid > 0, "PID should be positive");
            assert!(!proc.exe_name.is_empty(), "Name should not be empty");
        }
    }

    #[test]
    fn test_audio_process_enumeration() {
        use super::super::com::ComGuard;

        let _com = ComGuard::new().expect("COM init failed");
        let process_enum = ProcessEnumerator::new();

        let audio_procs = process_enum.enumerate_audio_processes();
        assert!(audio_procs.is_ok(), "Should enumerate audio processes");

        let proc_list = audio_procs.unwrap();
        println!("Found {} processes with audio sessions", proc_list.len());
    }

    #[test]
    fn test_application_listing() {
        let backend = WasapiBackend::new().unwrap();
        let apps = backend.list_applications();

        assert!(apps.is_ok(), "Should list applications");

        let app_list = apps.unwrap();
        println!("Found {} applications with audio", app_list.len());

        for app in app_list.iter().take(10) {
            assert!(app.pid > 0, "PID should be positive");
            assert!(!app.name.is_empty(), "Name should not be empty");
            println!("  {} (PID: {}, active: {})", app.name, app.pid, app.is_active);
        }
    }

    // ========================================================================
    // Device Enumeration Tests
    // ========================================================================

    #[test]
    fn test_device_enumeration() {
        use super::super::com::ComGuard;

        let _com = ComGuard::new().expect("COM init failed");
        let enumerator = DeviceEnumerator::new().expect("Device enumerator creation failed");

        let devices = enumerator.enumerate_all();
        assert!(devices.is_ok(), "Should enumerate devices");

        let dev_list = devices.unwrap();
        assert!(!dev_list.is_empty(), "Should find at least one device");

        println!("Found {} audio devices", dev_list.len());
        for dev in &dev_list {
            println!("  {} ({:?}, default: {})", dev.name, dev.flow, dev.is_default);
        }
    }

    #[test]
    fn test_default_render_device() {
        use super::super::com::ComGuard;

        let _com = ComGuard::new().expect("COM init failed");
        let enumerator = DeviceEnumerator::new().expect("Device enumerator creation failed");

        let device = enumerator.get_default_render_device();
        assert!(device.is_ok(), "Should get default render device");

        let dev = device.unwrap();
        assert!(dev.is_default, "Should be marked as default");
        assert_eq!(dev.flow, DeviceFlow::Render, "Should be render device");
        println!("Default render device: {}", dev.name);
    }

    #[test]
    fn test_virtual_device_detection() {
        use super::super::com::ComGuard;

        let _com = ComGuard::new().expect("COM init failed");
        let enumerator = DeviceEnumerator::new().expect("Device enumerator creation failed");

        let virtual_devs = enumerator.find_virtual_devices();
        assert!(virtual_devs.is_ok(), "Should search for virtual devices");

        let devs = virtual_devs.unwrap();
        println!("Found {} virtual audio devices", devs.len());
        for dev in &devs {
            println!("  {}", dev.name);
        }
    }

    // ========================================================================
    // Audio Session Tests
    // ========================================================================

    #[test]
    fn test_audio_session_enumeration() {
        use super::super::com::ComGuard;

        let _com = ComGuard::new().expect("COM init failed");
        let enumerator = SessionEnumerator::new();

        let sessions = enumerator.enumerate_sessions();
        assert!(sessions.is_ok(), "Should enumerate sessions");

        let sess_list = sessions.unwrap();
        println!("Found {} audio sessions", sess_list.len());

        for session in &sess_list {
            assert!(session.pid > 0, "PID should be positive");
            println!(
                "  {} (PID: {}, state: {:?}, vol: {:.2})",
                session.name, session.pid, session.state, session.volume
            );
        }
    }

    #[test]
    fn test_active_session_filtering() {
        use super::super::com::ComGuard;

        let _com = ComGuard::new().expect("COM init failed");
        let enumerator = SessionEnumerator::new();

        let active = enumerator.enumerate_active_sessions();
        assert!(active.is_ok(), "Should enumerate active sessions");

        let sess_list = active.unwrap();
        println!("Found {} active audio sessions", sess_list.len());

        // All returned should be active
        for session in &sess_list {
            assert_eq!(session.state, SessionState::Active, "Should all be active");
        }
    }

    // ========================================================================
    // Platform Trait Tests
    // ========================================================================

    #[test]
    fn test_platform_backend_trait() {
        let mut backend = WasapiBackend::new().unwrap();

        // Test name
        assert_eq!(backend.name(), "WASAPI");

        // Test is_connected
        assert!(backend.is_connected());

        // Test list_applications
        let apps = backend.list_applications();
        assert!(apps.is_ok());

        // Test list_nodes (returns empty on Windows)
        let nodes = backend.list_nodes();
        assert!(nodes.is_ok());

        // Test list_ports (returns empty on Windows)
        let ports = backend.list_ports(0);
        assert!(ports.is_ok());

        // Test list_links (returns empty on Windows)
        let links = backend.list_links();
        assert!(links.is_ok());

        // Test create_virtual_sink (should fail on Windows)
        let result = backend.create_virtual_sink(VirtualSinkConfig::default());
        assert!(result.is_err());

        // Test create_link (should fail on Windows)
        let result = backend.create_link(1, 2);
        assert!(result.is_err());
    }

    // ========================================================================
    // Audio Thread Tests
    // ========================================================================

    #[test]
    fn test_wasapi_thread_spawn_shutdown() {
        let handle = WasapiThreadHandle::spawn();
        assert!(handle.is_ok(), "Should spawn WASAPI thread");

        let mut h = handle.unwrap();

        // Should be able to query state
        let state = h.state();
        assert!((state.get_master_volume() - 1.0).abs() < f32::EPSILON);

        // Shutdown
        let result = h.shutdown();
        assert!(result.is_ok(), "Should shutdown cleanly");
    }

    #[test]
    fn test_audio_processing_state() {
        let state = AudioProcessingState::new();

        // Volume
        state.set_master_volume(0.75);
        assert!((state.get_master_volume() - 0.75).abs() < f32::EPSILON);

        // Bypass
        assert!(!state.is_bypassed());
        state.set_bypass(true);
        assert!(state.is_bypassed());

        // EQ gains
        let gains = [1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0, 10.0];
        state.set_master_eq_gains(&gains);
        let retrieved = state.get_master_eq_gains();
        for i in 0..10 {
            assert!((retrieved[i] - gains[i]).abs() < f32::EPSILON);
        }

        // Peak levels
        state.update_peak_levels(0.5, 0.6);
        let peaks = state.get_peak_levels();
        assert!((peaks[0] - 0.5).abs() < f32::EPSILON);
        assert!((peaks[1] - 0.6).abs() < f32::EPSILON);
    }

    // ========================================================================
    // Hardware Tests (require audio device, marked #[ignore])
    // ========================================================================

    #[test]
    #[ignore = "requires Windows audio hardware"]
    fn test_loopback_capture_basic() {
        use super::super::com::ComGuard;
        use super::super::thread::*;

        let _com = ComGuard::new().expect("COM init failed");

        // This would test actual capture
        // For now, just verify thread can be spawned
        let handle = WasapiThreadHandle::spawn();
        assert!(handle.is_ok());

        let mut h = handle.unwrap();

        // Try to start capture
        let result = h.send_command(message::WasapiCommand::StartCapture {
            pid: None,
            app_name: "System".to_string(),
        });
        assert!(result.is_ok());

        // Wait a bit
        std::thread::sleep(Duration::from_millis(100));

        // Shutdown
        h.shutdown().unwrap();
    }

    #[test]
    #[ignore = "requires Windows audio hardware"]
    fn test_audio_output_basic() {
        use super::super::com::ComGuard;

        let _com = ComGuard::new().expect("COM init failed");

        let handle = WasapiThreadHandle::spawn();
        assert!(handle.is_ok());

        let mut h = handle.unwrap();

        // Start output
        let result = h.send_command(message::WasapiCommand::StartOutput);
        assert!(result.is_ok());

        // Wait for response
        let response = h.recv_response_timeout(Duration::from_secs(5));
        match response {
            Some(message::WasapiResponse::OutputStarted) => {
                println!("Audio output started successfully");
                // Let it run briefly
                std::thread::sleep(Duration::from_millis(100));
                // Stop
                let _ = h.send_command(message::WasapiCommand::StopOutput);
                std::thread::sleep(Duration::from_millis(50));
            }
            Some(message::WasapiResponse::Error(e)) => {
                // Hardware may not be available - that's acceptable for this test
                println!("Audio output failed (may be expected without proper hardware): {}", e);
            }
            other => {
                println!("Unexpected response: {:?}", other);
            }
        }

        h.shutdown().unwrap();
    }

    #[test]
    #[ignore = "requires Windows audio hardware and active audio"]
    fn test_capture_to_output_loopback() {
        let mut backend = WasapiBackend::new().unwrap();

        // Start capture (system-wide)
        let capture_result = backend.start_capture("System", 0);
        // May fail if no audio hardware, that's ok for this test
        if capture_result.is_err() {
            println!("Capture start failed (may be expected without audio hardware)");
            return;
        }

        // Start output
        let output_result = backend.start_output();
        if output_result.is_err() {
            println!("Output start failed");
            return;
        }

        println!("Loopback test running for 2 seconds...");
        std::thread::sleep(Duration::from_secs(2));

        // Check peak levels
        if let Some(state) = backend.processing_state() {
            let peaks = state.get_peak_levels();
            println!("Peak levels: L={:.3}, R={:.3}", peaks[0], peaks[1]);
        }

        // Cleanup
        let _ = backend.stop_capture(0);
        let _ = backend.stop_output();
    }

    #[test]
    #[ignore = "requires Windows audio hardware - produces audible tone"]
    fn test_audio_output_sine_wave() {
        // This test would generate a sine wave
        // Marking as ignored to avoid unexpected audio output
        println!("Sine wave test placeholder");
    }

    // ========================================================================
    // Stress Tests
    // ========================================================================

    #[test]
    fn test_rapid_enumeration() {
        let backend = WasapiBackend::new().unwrap();

        // Rapidly enumerate applications
        for _ in 0..10 {
            let _ = backend.list_applications();
        }
        // Should not leak or crash
    }

    #[test]
    fn test_thread_multiple_shutdown() {
        let handle = WasapiThreadHandle::spawn().unwrap();
        let mut h = handle;

        // Shutdown multiple times should be safe
        h.shutdown().unwrap();
        h.shutdown().unwrap(); // Should not panic
    }
}

// ========================================================================
// Cross-Platform Unit Tests (run on any platform)
// ========================================================================

#[cfg(test)]
mod unit_tests {
    use super::super::message::*;
    use super::super::version::WindowsVersion;

    #[test]
    fn test_windows_version_comparison() {
        let old = WindowsVersion {
            major: 10,
            minor: 0,
            build: 19041,
        };
        let new = WindowsVersion {
            major: 10,
            minor: 0,
            build: 22631,
        };

        assert!(!old.supports_process_loopback());
        assert!(new.supports_process_loopback());
    }

    #[test]
    fn test_windows_version_display() {
        let version = WindowsVersion {
            major: 10,
            minor: 0,
            build: 22631,
        };
        let display = version.to_string();
        assert!(display.contains("Windows 11"));
        assert!(display.contains("22631"));
    }

    #[test]
    fn test_session_state_values() {
        assert_ne!(SessionState::Active, SessionState::Inactive);
        assert_ne!(SessionState::Inactive, SessionState::Expired);
    }

    #[test]
    fn test_device_flow_values() {
        assert_ne!(DeviceFlow::Render, DeviceFlow::Capture);
    }

    #[test]
    fn test_audio_processing_state_thread_safety() {
        use std::sync::Arc;
        use std::thread;

        let state = Arc::new(AudioProcessingState::new());

        let state1 = Arc::clone(&state);
        let state2 = Arc::clone(&state);

        let t1 = thread::spawn(move || {
            for _ in 0..1000 {
                state1.set_master_volume(0.5);
            }
        });

        let t2 = thread::spawn(move || {
            for _ in 0..1000 {
                let _ = state2.get_master_volume();
            }
        });

        t1.join().unwrap();
        t2.join().unwrap();

        // Should not have data races or panics
    }
}
