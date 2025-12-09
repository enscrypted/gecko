import { useEffect, useState, useCallback, useRef } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Badge, Button } from "./components/ui";
import { Controls } from "./components/Controls";
import { LevelMeter, LevelMeterHandle } from "./components/LevelMeter";
import { CompactSpectrumAnalyzer, SpectrumAnalyzerHandle } from "./components/SpectrumAnalyzer";
import { StreamList } from "./components/StreamList";
import { useSettings } from "./contexts/SettingsContext";
import { Settings } from "./components/Settings";
// Debounce delay for volume backend calls (ms)
const VOLUME_DEBOUNCE_MS = 16; // ~60fps update rate to backend

interface PlatformInfo {
  platform: string;
  supports_virtual_devices: boolean;
  supports_per_app_capture: boolean;
}

// Display mode for audio visualization
type VisualizationMode = "levels" | "spectrum";

function App() {
  const { settings, updateSettings } = useSettings();
  const [isRunning, setIsRunning] = useState(false);
  const [isBypassed, setIsBypassed] = useState(false);
  // Initialize from settings, fallback to 1.0
  const [masterVolume, setMasterVolume] = useState(() => settings?.master_volume ?? 1.0);
  const [platformInfo, setPlatformInfo] = useState<PlatformInfo | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [visualizationMode, setVisualizationMode] = useState<VisualizationMode>("levels");
  const [settingsOpen, setSettingsOpen] = useState(false);

  // Refs for visualization components - update imperatively to avoid re-renders
  const levelMeterRef = useRef<LevelMeterHandle>(null);
  const spectrumRef = useRef<SpectrumAnalyzerHandle>(null);

  // Refs for debouncing volume backend calls
  const volumeDebounceTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const pendingVolume = useRef<number | null>(null);

  // Cleanup volume debounce timer on unmount
  useEffect(() => {
    return () => {
      if (volumeDebounceTimer.current) {
        clearTimeout(volumeDebounceTimer.current);
      }
    };
  }, []);

  // Sync master volume from settings when they load
  useEffect(() => {
    if (settings?.master_volume !== undefined) {
      setMasterVolume(settings.master_volume);
    }
  }, [settings?.master_volume]);

  // Initialize engine on mount
  useEffect(() => {
    const init = async () => {
      try {
        await invoke("init_engine");
        const info = await invoke<PlatformInfo>("get_platform_info");
        setPlatformInfo(info);
      } catch (e) {
        setError(String(e));
      }
    };
    init();
  }, []);

  // Ref to prevent overlapping poll requests
  const pollInProgress = useRef<boolean>(false);

  // Poll for events using setInterval - decoupled from rendering
  // This runs the IPC call independently so it doesn't block the render cycle
  useEffect(() => {
    if (!isRunning) return;

    const pollEvents = async () => {
      if (pollInProgress.current) return;
      pollInProgress.current = true;

      try {
        const events = await invoke<string[]>("poll_events");
        for (const eventJson of events) {
          const event = JSON.parse(eventJson);
          if (event.type === "LevelUpdate") {
            // Update LevelMeter imperatively - no React re-render!
            levelMeterRef.current?.updateLevels(event.payload.left, event.payload.right);
          } else if (event.type === "SpectrumUpdate") {
            // Update SpectrumAnalyzer imperatively - no React re-render!
            spectrumRef.current?.updateBins(event.payload.bins);
          } else if (event.type === "Error") {
            setError(event.payload.message);
          }
        }
      } catch {
        // Ignore polling errors
      }
      pollInProgress.current = false;
    };

    // Poll at ~60fps (16ms) to get events as fast as possible
    // The LevelMeter and SpectrumAnalyzer components handle their own smoothing
    const intervalId = setInterval(pollEvents, 16);

    return () => {
      clearInterval(intervalId);
    };
  }, [isRunning]);

  const handleStart = useCallback(async () => {
    try {
      await invoke("start_engine");
      setIsRunning(true);
      setError(null);
    } catch (e) {
      setError(String(e));
    }
  }, []);

  const handleStop = useCallback(async () => {
    try {
      await invoke("stop_engine");
      setIsRunning(false);
      // Reset visualization via refs
      levelMeterRef.current?.updateLevels(0, 0);
      spectrumRef.current?.updateBins(new Array(32).fill(0));
    } catch (e) {
      setError(String(e));
    }
  }, []);

  const handleBypassChange = useCallback(async (bypassed: boolean) => {
    try {
      await invoke("set_bypass", { bypassed });
      setIsBypassed(bypassed);
    } catch (e) {
      setError(String(e));
    }
  }, []);

  const handleVolumeChange = useCallback((volume: number) => {
    // Update UI immediately for responsiveness - no lag
    setMasterVolume(volume);

    // Store the pending value
    pendingVolume.current = volume;

    // Clear existing timer
    if (volumeDebounceTimer.current) {
      clearTimeout(volumeDebounceTimer.current);
    }

    // Set new debounced timer to send to backend
    volumeDebounceTimer.current = setTimeout(() => {
      const value = pendingVolume.current;
      if (value !== null) {
        invoke("set_master_volume", { volume: value }).catch((e) => {
          setError(String(e));
        });
        // Persist to settings (updateSettings has its own debounce)
        updateSettings({ master_volume: value });
        pendingVolume.current = null;
      }
      volumeDebounceTimer.current = null;
    }, VOLUME_DEBOUNCE_MS);
  }, [updateSettings]);

  const dismissError = useCallback(() => setError(null), []);

  // Show level meters based on settings
  const showLevelMeters = settings?.ui_settings?.show_level_meters ?? true;
  const eqBandCount = settings?.ui_settings?.eq_bands_ui ?? 10;

  // Poll for PipeWire sink volume changes (system volume sync)
  // This detects when user changes volume via system controls
  useEffect(() => {
    if (!isRunning) return;

    // Use ref to track last volume to avoid useEffect dependency issues
    let lastSyncedVolume = masterVolume;

    const pollSinkVolume = async () => {
      try {
        const sinkVol = await invoke<number>("get_sink_volume");
        // Only update UI if significantly different (avoid floating point noise)
        // DSP is already updated in the backend get_sink_volume call
        if (Math.abs(sinkVol - lastSyncedVolume) > 0.005) {
          lastSyncedVolume = sinkVol;
          setMasterVolume(sinkVol);
          updateSettings({ master_volume: sinkVol });
        }
      } catch {
        // Ignore errors - sink might not exist yet
      }
    };

    // Poll at 0.5Hz (2 seconds) - responsive enough for volume sync, avoids process spam
    const intervalId = setInterval(pollSinkVolume, 2000);

    // Initial poll after short delay to let sink be created
    const timeoutId = setTimeout(pollSinkVolume, 500);

    return () => {
      clearInterval(intervalId);
      clearTimeout(timeoutId);
    };
  }, [isRunning, updateSettings]); // Removed masterVolume from deps - use local var instead

  // Main App Mode
  return (
    <div className="flex flex-col min-h-screen p-4">
      {/* No Toast in Main App (User Request) */}

      {/* Header */}
      <header className="flex items-center justify-between pb-4 border-b border-gecko-border mb-4">
        <div className="flex items-center gap-3">
          <h1 className="text-xl font-semibold text-gecko-text-primary">
            Gecko Audio
          </h1>
          {platformInfo && (
            <Badge variant="default">{platformInfo.platform}</Badge>
          )}
        </div>
        <Button variant="default" size="sm" onClick={() => setSettingsOpen(true)}>
          ⚙️ Settings
        </Button>
      </header>

      {/* Error Banner */}
      {error && (
        <div
          onClick={dismissError}
          className="bg-gecko-danger/90 text-white px-4 py-3 rounded mb-4 cursor-pointer hover:bg-gecko-danger transition-colors"
          role="alert"
        >
          <span className="font-medium">Error:</span> {error}
        </div>
      )}

      {/* Main Content */}
      <main className="flex-1 flex flex-col gap-4">
        {/* Controls Row */}
        <div className="flex items-center gap-4 flex-wrap">
          <Controls
            isRunning={isRunning}
            isBypassed={isBypassed}
            masterVolume={masterVolume}
            onStart={handleStart}
            onStop={handleStop}
            onBypassChange={handleBypassChange}
            onVolumeChange={handleVolumeChange}
          />
          {/* Audio visualization with toggle */}
          {showLevelMeters && (
            <div className="flex items-center gap-2">
              {visualizationMode === "levels" ? (
                <LevelMeter ref={levelMeterRef} />
              ) : (
                <CompactSpectrumAnalyzer ref={spectrumRef} />
              )}
              <button
                onClick={() => setVisualizationMode(m => m === "levels" ? "spectrum" : "levels")}
                className="text-xs px-2 py-1 rounded bg-gecko-bg-tertiary text-gecko-text-secondary hover:bg-gecko-bg-elevated transition-colors"
                title={visualizationMode === "levels" ? "Show spectrum analyzer" : "Show level meters"}
              >
                {visualizationMode === "levels" ? "FFT" : "L/R"}
              </button>
            </div>
          )}
        </div>

        {/* Audio Streams with per-app EQ */}
        <StreamList disabled={isBypassed} isRunning={isRunning} eqBandCount={eqBandCount} />
      </main>

      {/* Footer */}
      <footer className="pt-4 border-t border-gecko-border mt-4 text-center">
        <span className="text-xs text-gecko-text-muted">
          Gecko Audio v0.1.0
        </span>
      </footer>

      {/* Settings Modal */}
      <Settings isOpen={settingsOpen} onClose={() => setSettingsOpen(false)} />
    </div>
  );
}

export default App;
