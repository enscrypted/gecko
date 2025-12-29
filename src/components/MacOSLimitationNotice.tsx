import { useState, useEffect, useCallback } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Badge } from "./ui/badge";

interface MacOSAudioInfo {
  macos_version: string;
  process_tap_available: boolean;
  per_app_audio_available: boolean;
}

interface MacOSLimitationNoticeProps {
  /** Compact mode shows only a badge, full mode shows detailed info */
  compact?: boolean;
}

/**
 * MacOSLimitationNotice - Displays information about macOS audio capture limitations
 *
 * On macOS, Safari, FaceTime, iMessage, and system sounds cannot be individually
 * routed due to sandboxing. They DO receive master EQ when Gecko is the default output.
 *
 * This component shows:
 * - macOS version and API being used (Process Tap vs HAL Plugin)
 * - List of apps that cannot be individually EQ'd
 * - Explanation of the workaround (master EQ applies to all)
 */
export function MacOSLimitationNotice({ compact = false }: MacOSLimitationNoticeProps) {
  const [macOSInfo, setMacOSInfo] = useState<MacOSAudioInfo | null>(null);
  const [isExpanded, setIsExpanded] = useState(false);
  const [isMac, setIsMac] = useState(false);

  useEffect(() => {
    // Check if we're on macOS
    const checkPlatform = async () => {
      try {
        const info = await invoke<MacOSAudioInfo>("get_macos_audio_info");
        setMacOSInfo(info);
        setIsMac(true);
      } catch {
        // Not on macOS or command not available
        setIsMac(false);
      }
    };

    checkPlatform();
  }, []);

  const toggleExpanded = useCallback(() => {
    setIsExpanded((prev) => !prev);
  }, []);

  // Don't render on non-macOS platforms
  if (!isMac) {
    return null;
  }

  // Compact mode: just show a badge
  if (compact) {
    return (
      <button
        onClick={toggleExpanded}
        className="flex items-center gap-1.5 hover:opacity-80 transition-opacity"
        title="Click for macOS audio info"
      >
        <Badge variant="warning">macOS</Badge>
        <span className="text-2xs text-gecko-text-muted">Process Tap</span>
      </button>
    );
  }

  // Full mode: show detailed information
  return (
    <div className="bg-gecko-bg-secondary rounded-lg border border-gecko-border p-4">
      <div className="flex items-start gap-3">
        {/* Warning icon */}
        <div className="text-gecko-warning text-lg">⚠️</div>

        <div className="flex-1 space-y-2">
          <div className="flex items-center justify-between">
            <h3 className="text-sm font-medium text-gecko-text-primary">
              macOS Audio Limitations
            </h3>
            <Badge variant="warning">Process Tap API</Badge>
          </div>

          <p className="text-xs text-gecko-text-secondary">
            macOS {macOSInfo?.macos_version || "?"} detected.{" "}
            {macOSInfo?.process_tap_available
              ? "Using native Process Tap API for per-app audio capture."
              : "macOS 14.4+ required for per-app audio capture."}
          </p>

          <button
            onClick={toggleExpanded}
            className="text-xs text-gecko-accent hover:text-gecko-accent-bright transition-colors"
          >
            {isExpanded ? "Hide details ▲" : "Show details ▼"}
          </button>

          {isExpanded && (
            <div className="mt-3 space-y-3 pt-3 border-t border-gecko-border">
              {/* Unsupported apps section */}
              <div>
                <h4 className="text-xs font-medium text-gecko-text-secondary mb-1">
                  Apps without per-app EQ:
                </h4>
                <div className="flex flex-wrap gap-1.5">
                  {["Safari", "FaceTime", "Messages", "System Sounds"].map(
                    (app) => (
                      <span
                        key={app}
                        className="text-2xs bg-gecko-bg-tertiary px-2 py-0.5 rounded text-gecko-text-muted"
                      >
                        {app}
                      </span>
                    )
                  )}
                </div>
              </div>

              {/* Explanation */}
              <div className="text-xs text-gecko-text-muted space-y-1">
                <p>
                  <strong className="text-gecko-text-secondary">Why?</strong>{" "}
                  Apple's sandboxing prevents routing these apps' audio to
                  virtual devices.
                </p>
                <p>
                  <strong className="text-gecko-text-secondary">
                    Workaround:
                  </strong>{" "}
                  Set Gecko as your default audio output. These apps will then
                  receive <span className="text-gecko-accent">master EQ</span>{" "}
                  (affects all audio equally).
                </p>
              </div>

              {/* API info */}
              <div className="text-2xs text-gecko-text-muted">
                {macOSInfo?.process_tap_available ? (
                  <p>
                    ✓ Process Tap API (macOS 14.4+) - No driver installation
                    required
                  </p>
                ) : (
                  <p className="text-gecko-danger">
                    ✗ macOS 14.4+ required - Update your system for per-app audio capture
                  </p>
                )}
              </div>
            </div>
          )}
        </div>
      </div>
    </div>
  );
}
