import { useEffect, useState, useCallback, useRef } from "react";
import { invoke } from "@tauri-apps/api/core";
import { AudioStreamItem } from "./AudioStreamItem";
import { useSettings } from "../contexts/SettingsContext";

interface AudioStream {
  id: string;
  name: string;
  pid: number;
  is_active: boolean;
  is_routed_to_gecko: boolean;
  /** macOS only: Whether this app can be captured via Process Tap API */
  is_tappable: boolean;
  /** macOS only: Reason why the app cannot be tapped (if is_tappable is false) */
  untappable_reason?: string;
}

interface MacOSAudioInfo {
  macos_version: string;
  process_tap_available: boolean;
}

/** Extract app name from stream (used for settings persistence) */
function getAppName(stream: AudioStream): string {
  return stream.name;
}

interface StreamListProps {
  disabled?: boolean;
  isRunning: boolean;
  eqBandCount?: number;
}

export function StreamList({ disabled = false, isRunning, eqBandCount = 10 }: StreamListProps) {
  const { settings, updateSettings, loading: settingsLoading } = useSettings(); // Fix destructuring
  const [streams, setStreams] = useState<AudioStream[]>([]);
  const [streamVolumes, setStreamVolumes] = useState<Record<string, number>>({});
  const [streamGains, setStreamGains] = useState<Record<string, number[]>>({});

  // Track if we've done the initial sync for streams to avoid repeated applications
  const [initialSyncComplete, setInitialSyncComplete] = useState<Record<string, boolean>>({});

  const [masterGains, setMasterGains] = useState<number[]>(Array(10).fill(0));
  const [isLoading, setIsLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [showHidden, setShowHidden] = useState(false);

  // macOS-specific state: detect if Process Tap is available
  const [isMacOS, setIsMacOS] = useState(false);
  const [usesProcessTap, setUsesProcessTap] = useState(false);

  // Get bypass/hidden state from settings
  // State for streams and volumes
  // hiddenApps and bypassedApps are accessed directly from settings to ensure single source of truth

  // Load persisted master EQ gains from settings on mount
  useEffect(() => {
    if (settings?.master_eq) {
      setMasterGains([...settings.master_eq]);
    }
  }, [settings?.master_eq]);

  // Detect macOS platform and Process Tap availability
  useEffect(() => {
    const checkMacOS = async () => {
      try {
        const info = await invoke<MacOSAudioInfo>("get_macos_audio_info");
        setIsMacOS(true);
        setUsesProcessTap(info.process_tap_available);
      } catch {
        // Not on macOS or command not available
        setIsMacOS(false);
        setUsesProcessTap(false);
      }
    };
    checkMacOS();
  }, []);

  // Track order of streams by when they were first seen (stable ordering)
  const streamOrderRef = useRef<string[]>([]);

  // Fetch audio streams
  // Note: We use functional updates for setStreamGains to avoid stale closure issues
  // with the polling interval - this ensures we always read the latest state
  const fetchStreams = useCallback(async () => {
    if (!isRunning) {
      setStreams([]);
      streamOrderRef.current = [];
      return;
    }

    setIsLoading(true);
    setError(null);

    // blocked by settings loading to prevent race conditions
    if (settingsLoading) return;

    try {
      const result = await invoke<AudioStream[]>("list_audio_streams");

      const newStreamMap = new Map(result.map(s => [s.id, s]));

      // STICKY STREAMS: Apps stay visible until user hides them or app closes
      // 1. Keep all existing streams (mark inactive if not in new result)
      // 2. Add any new streams from result
      // 3. Only remove if PID no longer exists (process closed)

      const mergedStreams: AudioStream[] = [];
      const seenIds = new Set<string>();

      // First, update existing streams
      for (const existingStream of streams) {
        const newData = newStreamMap.get(existingStream.id);
        if (newData) {
          // Stream still active - use new data
          mergedStreams.push(newData);
        } else {
          // Stream no longer in result - mark as inactive but keep it
          // User can hide it manually if they don't want to see it
          mergedStreams.push({
            ...existingStream,
            is_active: false,
          });
        }
        seenIds.add(existingStream.id);
      }

      // Then, add any new streams not already tracked
      for (const newStream of result) {
        if (!seenIds.has(newStream.id)) {
          mergedStreams.push(newStream);
        }
      }

      setStreams(mergedStreams);

      // Initialize gains for new streams only - use functional update to avoid stale closures
      setStreamGains((prev) => {
        const newStreamGains = { ...prev };
        let hasChanges = false;

        // Helper to track which streams we sync in this cycle
        const newInitialSyncHelper = { ...initialSyncComplete };
        let syncChanged = false;

        result.forEach((stream) => {
          if (!newStreamGains[stream.id]) {
            hasChanges = true;
            // Extract app name from stream.name for settings lookup
            const appName = stream.name;
            // Load from settings.app_eq if available, otherwise default to 0
            const persistedEq = settings?.app_eq?.[appName];
            const gains = persistedEq
              ? [...persistedEq, ...Array(10 - persistedEq.length).fill(0)].slice(0, 10)
              : Array(10).fill(0);
            newStreamGains[stream.id] = gains;

            // Apply persisted EQ to backend immediately for new streams
            // This ensures the backend matches the UI state on startup
            if (!initialSyncComplete[stream.id]) {
              gains.forEach((gain, band) => {
                if (gain !== 0) {
                  invoke("set_stream_band_gain", { streamId: stream.id, band, gainDb: gain }).catch(
                    (e) => console.error("Failed to apply persisted EQ:", e)
                  );
                }
              });
              newInitialSyncHelper[stream.id] = true;
              syncChanged = true;
            }
          }
        });

        if (syncChanged) {
          setInitialSyncComplete(newInitialSyncHelper);
        }

        // Only return new object if we actually added new streams
        return hasChanges ? newStreamGains : prev;
      });

      // Initialize volumes for new streams - separate from streamGains to keep logic clear
      setStreamVolumes((prev) => {
        const newStreamVolumes = { ...prev };
        let hasChanges = false;

        // Reuse sync helper from gains or just check separately?
        // Volumes are simpler, let's check directly against our new state or just always check undefined
        // Actually, we should check if we already applied volume to backend
        // We can reuse the same initialSyncComplete map since we do both at same time usually
        // But let's be safe and check if volume is 1.0 (default)

        result.forEach((stream) => {
          if (newStreamVolumes[stream.id] === undefined) {
            hasChanges = true;
            const appName = stream.name;
            const persistedVolume = settings?.app_volumes?.[appName] ?? 1.0;
            newStreamVolumes[stream.id] = persistedVolume;

            // Apply persisted volume to backend if different from default
            // And use the sync tracker to ensure we do it safely once
            if (persistedVolume !== 1.0) {
              invoke("set_stream_volume", { streamId: stream.id, volume: persistedVolume }).catch(
                (e) => console.error("Failed to apply persisted volume:", e)
              );
            }
          }
        });
        return hasChanges ? newStreamVolumes : prev;
      });
    } catch (e) {
      console.error("Failed to fetch audio streams:", e);
      setError(String(e));
    } finally {
      setIsLoading(false);
    }
  }, [isRunning, settings?.app_eq, settings?.app_volumes, settingsLoading, streams, initialSyncComplete]);

  // Fetch streams on mount and when running state changes
  useEffect(() => {
    fetchStreams();

    // Poll for stream changes every 5 seconds when running
    // Longer interval reduces UI lag from re-renders during audio playback
    // Stream list changes (new apps) don't need to be instant
    if (isRunning) {
      const interval = setInterval(fetchStreams, 5000);
      return () => clearInterval(interval);
    }
  }, [isRunning]); // eslint-disable-line react-hooks/exhaustive-deps

  const handleMasterBandChange = useCallback((band: number, gain: number) => {
    setMasterGains((prev) => {
      const next = [...prev];
      next[band] = gain;
      return next;
    });
  }, []);

  const handleStreamBandChange = useCallback(
    (streamId: string, band: number, gain: number) => {
      setStreamGains((prev) => {
        const next = { ...prev };
        if (!next[streamId]) {
          next[streamId] = Array(10).fill(0);
        }
        next[streamId] = [...next[streamId]];
        next[streamId][band] = gain;
        return next;
      });
    },
    []
  );

  // Handle per-app volume change (update UI state, persist to settings, backend already handled by AudioStreamItem)
  const handleStreamVolumeChange = useCallback(
    (streamId: string, volume: number) => {
      setStreamVolumes((prev) => ({
        ...prev,
        [streamId]: volume,
      }));

      // Persist volume to settings by app name
      const stream = streams.find((s) => s.id === streamId);
      if (stream && settings) {
        const appName = getAppName(stream);
        const newAppVolumes = { ...settings.app_volumes, [appName]: volume };
        updateSettings({ ...settings, app_volumes: newAppVolumes });
      }
    },
    [streams, settings, updateSettings]
  );

  // Handle bypass toggle for an app
  const handleBypassChange = useCallback(
    async (appName: string, bypassed: boolean) => {
      try {
        await invoke("set_app_bypass", { appName, bypassed });
        // Update local settings - context handles persistence via its own save_settings call
        if (settings) {
          const newBypassedApps = bypassed
            ? [...(settings.bypassed_apps || []), appName]
            : (settings.bypassed_apps || []).filter((a) => a !== appName);
          updateSettings({ ...settings, bypassed_apps: newBypassedApps });
        }
      } catch (e) {
        console.error("Failed to set app bypass:", e);
      }
    },
    [settings, updateSettings]
  );

  // Handle hide for an app
  const handleHide = useCallback(
    async (appName: string) => {
      if (settings) {
        const newHiddenApps = [...(settings.hidden_apps || []), appName];
        updateSettings({ ...settings, hidden_apps: newHiddenApps });
      }
    },
    [settings, updateSettings]
  );

  // Handle unhide for an app
  const handleUnhide = useCallback(
    async (appName: string) => {
      if (settings) {
        const newHiddenApps = (settings.hidden_apps || []).filter((a) => a !== appName);
        updateSettings({ ...settings, hidden_apps: newHiddenApps });
      }
    },
    [settings, updateSettings]
  );

  // Handle macOS capture toggle (start/stop Process Tap capture)
  const handleCaptureToggle = useCallback(
    async (stream: AudioStream, capture: boolean) => {
      try {
        if (capture) {
          // Start capturing this app's audio via Process Tap
          await invoke("start_app_capture", { pid: stream.pid, appName: stream.name });
        } else {
          // Stop capturing this app's audio
          await invoke("stop_app_capture", { pid: stream.pid });
        }
        // Refresh streams to update is_routed_to_gecko status
        fetchStreams();
      } catch (e) {
        console.error("Failed to toggle app capture:", e);
        setError(`Failed to ${capture ? "start" : "stop"} capture: ${e}`);
      }
    },
    [fetchStreams]
  );

  // Filter streams into visible and hidden
  const visibleStreams = streams.filter((s) => !(settings?.hidden_apps || []).includes(getAppName(s)));
  const hiddenStreams = streams.filter((s) => (settings?.hidden_apps || []).includes(getAppName(s)));

  return (
    <div className="flex flex-col gap-3">
      {/* Header */}
      <div className="flex items-center justify-between">
        <h2 className="text-sm font-medium uppercase tracking-wider text-gecko-text-secondary">
          Audio Streams
        </h2>
        {isRunning && (
          <button
            onClick={fetchStreams}
            className="text-xs text-gecko-text-secondary hover:text-gecko-text-primary transition-colors"
            disabled={isLoading}
          >
            {isLoading ? "Refreshing..." : "Refresh"}
          </button>
        )}
      </div>

      {/* Error message */}
      {error && (
        <div className="text-sm text-gecko-danger bg-gecko-danger/10 rounded p-2">
          {error}
        </div>
      )}

      {/* Master stream (always shown) */}
      <AudioStreamItem
        id="master"
        name="Master"
        pid={0}
        isActive={isRunning}
        isRoutedToGecko={true}
        isMaster={true}
        disabled={disabled || !isRunning}
        bandGains={masterGains}
        onBandChange={handleMasterBandChange}
        eqBandCount={eqBandCount}
      />

      {/* Application streams */}
      {!isRunning ? (
        <div className="text-center text-gecko-text-muted py-8 bg-gecko-bg-secondary rounded-lg border border-gecko-border">
          <p className="text-sm">Start the engine to see audio streams</p>
          <p className="text-xs mt-1">
            Applications playing audio will appear here
          </p>
        </div>
      ) : visibleStreams.length === 0 && hiddenStreams.length === 0 ? (
        <div className="text-center text-gecko-text-muted py-8 bg-gecko-bg-secondary rounded-lg border border-gecko-border">
          <p className="text-sm">No audio streams detected</p>
          <p className="text-xs mt-1">
            Play audio in an application to see it here
          </p>
        </div>
      ) : (
        <>
          {/* Visible streams */}
          {visibleStreams.map((stream) => (
            <AudioStreamItem
              key={stream.id}
              id={stream.id}
              name={stream.name}
              pid={stream.pid}
              isActive={stream.is_active}
              isRoutedToGecko={stream.is_routed_to_gecko}
              disabled={disabled}
              bandGains={streamGains[stream.id] || Array(10).fill(0)}
              onBandChange={(band, gain) =>
                handleStreamBandChange(stream.id, band, gain)
              }
              eqBandCount={eqBandCount}
              isBypassed={(settings?.bypassed_apps || []).includes(getAppName(stream))}
              onBypassChange={(bypassed) => handleBypassChange(getAppName(stream), bypassed)}
              onHide={() => handleHide(getAppName(stream))}
              volume={streamVolumes[stream.id] ?? 1.0}
              onVolumeChange={(volume) => handleStreamVolumeChange(stream.id, volume)}
              showCaptureToggle={isMacOS && usesProcessTap}
              onCaptureToggle={(capture) => handleCaptureToggle(stream, capture)}
              isTappable={stream.is_tappable}
              untappableReason={stream.untappable_reason}
            />
          ))}

          {/* Hidden streams section */}
          {hiddenStreams.length > 0 && (
            <div className="mt-4">
              <button
                onClick={() => setShowHidden(!showHidden)}
                className="flex items-center gap-2 text-sm text-gecko-text-secondary hover:text-gecko-text-primary transition-colors"
              >
                <span className={`transition-transform duration-200 ${showHidden ? "rotate-90" : ""}`}>
                  ▶
                </span>
                Hidden Apps ({hiddenStreams.length})
              </button>

              {showHidden && (
                <div className="mt-2 space-y-2">
                  {hiddenStreams.map((stream) => (
                    <div
                      key={stream.id}
                      className="flex items-center justify-between p-2 bg-gecko-bg-tertiary rounded border border-gecko-border/50"
                    >
                      <span className="text-sm text-gecko-text-secondary">{stream.name}</span>
                      <button
                        onClick={() => handleUnhide(getAppName(stream))}
                        className="text-xs px-2 py-1 rounded bg-gecko-bg-secondary text-gecko-text-secondary hover:text-gecko-text-primary transition-colors"
                      >
                        Show
                      </button>
                    </div>
                  ))}
                </div>
              )}
            </div>
          )}
        </>
      )}

    </div>
  );
}
