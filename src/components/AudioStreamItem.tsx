import { useState, useCallback, useRef, useEffect } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Slider } from "./ui/slider";
import { Badge } from "./ui/badge";
import { EditableValue } from "./ui/editable-value";
import { PresetSelector } from "./PresetSelector";
import { cn } from "../lib/utils";

// Debounce delay for backend calls (ms)
// UI updates immediately, but backend calls are debounced to reduce IPC overhead
const BACKEND_DEBOUNCE_MS = 16; // ~60fps update rate to backend

interface AudioStreamItemProps {
  id: string;
  name: string;
  pid: number;
  isActive: boolean;
  isRoutedToGecko: boolean;
  isMaster?: boolean;
  disabled?: boolean;
  bandGains: number[];
  onBandChange?: (band: number, gain: number) => void;
  eqBandCount?: number;
  /** Whether this app's EQ is bypassed (audio passes through unprocessed) */
  isBypassed?: boolean;
  /** Callback when bypass state changes */
  onBypassChange?: (bypassed: boolean) => void;
  /** Callback when hide is requested */
  onHide?: () => void;
  /** Per-app volume (0.0 - 2.0, default 1.0) */
  volume?: number;
  /** Callback when volume changes */
  onVolumeChange?: (volume: number) => void;
}

const EQ_FREQUENCIES = [31, 62, 125, 250, 500, 1000, 2000, 4000, 8000, 16000];

function formatFrequency(freq: number): string {
  if (freq >= 1000) {
    return `${freq / 1000}k`;
  }
  return String(freq);
}

export function AudioStreamItem({
  id, // Used for per-stream EQ commands
  name,
  pid,
  isActive,
  isRoutedToGecko,
  isMaster = false,
  disabled = false,
  bandGains,
  onBandChange,
  eqBandCount = 10,
  isBypassed = false,
  onBypassChange,
  onHide,
  volume = 1.0,
  onVolumeChange,
}: AudioStreamItemProps) {
  const [isExpanded, setIsExpanded] = useState(isMaster);
  const [localGains, setLocalGains] = useState<number[]>(bandGains);
  const [localVolume, setLocalVolume] = useState<number>(volume);

  // Refs for debouncing backend calls per band
  // Each band gets its own timer so changes to different bands don't interfere
  const debounceTimers = useRef<Map<number, ReturnType<typeof setTimeout>>>(new Map());
  const pendingValues = useRef<Map<number, number>>(new Map());

  // Sync localGains when bandGains prop changes (e.g., from preset)
  useEffect(() => {
    setLocalGains(bandGains);
  }, [bandGains]);

  // Sync localVolume when volume prop changes
  useEffect(() => {
    setLocalVolume(volume);
  }, [volume]);

  // Debounce timer ref for volume changes
  const volumeDebounceTimer = useRef<ReturnType<typeof setTimeout> | null>(null);

  // Cleanup timers on unmount
  useEffect(() => {
    return () => {
      debounceTimers.current.forEach((timer) => clearTimeout(timer));
      if (volumeDebounceTimer.current) clearTimeout(volumeDebounceTimer.current);
    };
  }, []);

  const handleBandChange = useCallback(
    (band: number, gain: number) => {
      // Update local state immediately for responsive UI
      setLocalGains((prev) => {
        const next = [...prev];
        next[band] = gain;
        return next;
      });

      // Notify parent immediately for UI sync
      if (onBandChange) {
        onBandChange(band, gain);
      }

      // Store the pending value
      pendingValues.current.set(band, gain);

      // Clear existing timer for this band
      const existingTimer = debounceTimers.current.get(band);
      if (existingTimer) {
        clearTimeout(existingTimer);
      }

      // Set new debounced timer to send to backend
      const timer = setTimeout(() => {
        const value = pendingValues.current.get(band);
        if (value !== undefined) {
          // Use set_band_gain for master, set_stream_band_gain for apps
          if (isMaster) {
            invoke("set_band_gain", { band, gainDb: value }).catch((e) => {
              console.error("Failed to set band gain:", e);
            });
          } else {
            invoke("set_stream_band_gain", { streamId: id, band, gainDb: value }).catch((e) => {
              console.error("Failed to set stream band gain:", e);
            });
          }
          pendingValues.current.delete(band);
        }
        debounceTimers.current.delete(band);
      }, BACKEND_DEBOUNCE_MS);

      debounceTimers.current.set(band, timer);
    },
    [isMaster, id, onBandChange]
  );

  const handleReset = useCallback(() => {
    // Reset all bands to 0
    const resetGains = Array(10).fill(0);
    setLocalGains(resetGains);

    // Notify parent of ALL band changes so parent state stays in sync
    resetGains.forEach((gain, band) => {
      if (onBandChange) {
        onBandChange(band, gain);
      }
      // Send to backend
      if (isMaster) {
        invoke("set_band_gain", { band, gainDb: 0 }).catch(console.error);
      } else {
        invoke("set_stream_band_gain", { streamId: id, band, gainDb: 0 }).catch(console.error);
      }
    });
  }, [isMaster, id, onBandChange]);

  const handleVolumeChange = useCallback(
    (newVolume: number) => {
      // Update local state immediately for responsive UI
      setLocalVolume(newVolume);

      // Notify parent
      if (onVolumeChange) {
        onVolumeChange(newVolume);
      }

      // Clear existing timer
      if (volumeDebounceTimer.current) {
        clearTimeout(volumeDebounceTimer.current);
      }

      // Debounce backend call
      volumeDebounceTimer.current = setTimeout(() => {
        invoke("set_stream_volume", { streamId: id, volume: newVolume }).catch((e) => {
          console.error("Failed to set stream volume:", e);
        });
        volumeDebounceTimer.current = null;
      }, BACKEND_DEBOUNCE_MS);
    },
    [id, onVolumeChange]
  );

  return (
    <div
      className={cn(
        // Use shadow instead of border to avoid subpixel rendering artifacts
        "rounded-lg transition-colors",
        isMaster
          ? "bg-gecko-bg-secondary shadow-[inset_0_0_0_1px_var(--gecko-accent)]"
          : "bg-gecko-bg-secondary shadow-[inset_0_0_0_1px_var(--gecko-border)]",
        disabled && "opacity-50"
      )}
      style={isMaster ? { '--tw-shadow-color': 'var(--gecko-accent)' } as React.CSSProperties : undefined}
    >
      {/* Header - Always visible */}
      <button
        onClick={() => setIsExpanded(!isExpanded)}
        className="w-full flex items-center justify-between p-3 text-left hover:bg-gecko-bg-tertiary/50 rounded-t-lg transition-colors"
        disabled={disabled}
      >
        <div className="flex items-center gap-3">
          {/* Expand/collapse indicator */}
          <span
            className={cn(
              "text-gecko-text-secondary transition-transform duration-200",
              isExpanded && "rotate-90"
            )}
          >
            ▶
          </span>

          {/* Stream info */}
          <div className="flex flex-col">
            <span className="font-medium text-gecko-text-primary">
              {isMaster ? "Master (All Apps)" : name}
            </span>
            {!isMaster && (
              <span className="text-xs text-gecko-text-muted">PID: {pid}</span>
            )}
          </div>
        </div>

        {/* Status badges */}
        <div className="flex items-center gap-2">
          {isBypassed && !isMaster && (
            <Badge variant="default" className="text-xs bg-gecko-warning/20 text-gecko-warning">
              Bypassed
            </Badge>
          )}
          {isActive && (
            <Badge variant="success" className="text-xs">
              Active
            </Badge>
          )}
          {isRoutedToGecko && (
            <Badge variant="default" className="text-xs">
              Routed
            </Badge>
          )}
          {isMaster && (
            <Badge variant="default" className="text-xs bg-gecko-accent/20 text-gecko-accent">
              Master
            </Badge>
          )}
        </div>
      </button>

      {/* Per-app controls (bypass/hide/volume) - shown in header area for non-master streams */}
      {/* Clicking empty area in this row also toggles expand/collapse */}
      {!isMaster && (
        <div
          className="flex items-center gap-3 px-3 pb-2 shadow-[0_1px_0_0_var(--gecko-border)] transition-colors"
        >
          {/* Volume slider */}
          <div
            className="flex items-center gap-2 flex-1"
          >
            <span className="text-xs text-gecko-text-muted">Vol</span>
            <input
              type="range"
              min={0}
              max={2}
              step={0.01}
              value={localVolume}
              onChange={(e) => handleVolumeChange(parseFloat(e.target.value))}
              className="flex-1 h-1 bg-gecko-bg-tertiary rounded-lg appearance-none cursor-pointer accent-gecko-accent"
              title={`Volume: ${Math.round(localVolume * 100)}%`}
            />
            <EditableValue
              value={Math.round(localVolume * 100)}
              onChange={(percent) => handleVolumeChange(percent / 100)}
              min={0}
              max={200}
              decimals={0}
              suffix="%"
              className="text-gecko-text-secondary w-10 text-right"
              inputWidth="w-10"
            />
          </div>
          <button
            onClick={() => {
              onBypassChange?.(!isBypassed);
            }}
            className={cn(
              "text-xs px-2 py-1 rounded transition-colors",
              isBypassed
                ? "bg-gecko-warning/20 text-gecko-warning hover:bg-gecko-warning/30"
                : "bg-gecko-bg-tertiary text-gecko-text-secondary hover:bg-gecko-bg-tertiary/80"
            )}
            title={isBypassed ? "Enable EQ processing for this app" : "Bypass EQ processing for this app"}
          >
            {isBypassed ? "Enable EQ" : "Bypass"}
          </button>
          <button
            onClick={() => {
              onHide?.();
            }}
            className="text-xs px-2 py-1 rounded bg-gecko-bg-tertiary text-gecko-text-secondary hover:bg-gecko-bg-tertiary/80 transition-colors"
            title="Hide this app from the list (still processed)"
          >
            Hide
          </button>
        </div>
      )}

      {/* Expanded content - EQ sliders */}
      {isExpanded && (
        <div className="p-4 pt-3 space-y-3">
          {/* Preset Selector */}
          <PresetSelector
            currentGains={localGains}
            disabled={disabled}
            onApply={(gains) => {
              // Update local state
              setLocalGains(gains);
              // Notify parent
              gains.forEach((gain, band) => {
                if (onBandChange) onBandChange(band, gain);
              });
              // Apply all bands to backend - use correct command based on master vs per-app
              gains.forEach((gain, band) => {
                if (isMaster) {
                  invoke("set_band_gain", { band, gainDb: gain }).catch(console.error);
                } else {
                  invoke("set_stream_band_gain", { streamId: id, band, gainDb: gain }).catch(console.error);
                }
              });
            }}
          />

          {/* EQ Sliders */}
          <div className="flex justify-between items-end gap-1">
            {EQ_FREQUENCIES.filter((_, i) => {
              // 5-band mode: show bands 0, 2, 4, 6, 8 (31, 125, 500, 2k, 8k)
              if (eqBandCount === 5) return i % 2 === 0;
              return true;
            }).map((freq) => {
              const band = EQ_FREQUENCIES.indexOf(freq);
              return (
                <div key={band} className="flex flex-col items-center flex-1 min-w-[40px] gap-1">
                  <div className="h-32 flex items-center justify-center">
                    <Slider
                      orientation="vertical"
                      min={-24}
                      max={24}
                      step={0.5}
                      value={localGains[band] ?? 0}
                      onChange={(e) =>
                        handleBandChange(band, parseFloat(e.target.value))
                      }
                      disabled={disabled}
                      className="h-full"
                    />
                  </div>
                  <span className="text-[10px] text-gecko-text-muted">
                    {formatFrequency(freq)}
                  </span>
                  <EditableValue
                    value={localGains[band] ?? 0}
                    onChange={(gain) => handleBandChange(band, gain)}
                    min={-24}
                    max={24}
                    decimals={1}
                    suffix="dB"
                    showPositive
                    disabled={disabled}
                    className="text-[10px] text-gecko-text-secondary"
                    inputWidth="w-10"
                  />
                </div>
              )
            })}
          </div>

          {/* Reset button */}
          <div className="flex justify-end">
            <button
              onClick={handleReset}
              className="text-xs text-gecko-text-secondary hover:text-gecko-text-primary transition-colors"
              disabled={disabled}
            >
              Reset EQ
            </button>
          </div>
        </div>
      )}
    </div>
  );
}
