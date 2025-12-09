import { memo, useRef, useEffect, forwardRef, useImperativeHandle } from "react";
import { amplitudeToPercent } from "../lib/utils";

// Smoothing constants for meter animation
// Higher decay = smoother but slower falloff
const METER_DECAY = 0.85;
// Attack is instant for responsive feel
const METER_ATTACK = 1.0;

// Color thresholds (percent)
const WARNING_THRESHOLD = 70;
const DANGER_THRESHOLD = 90;

// Get Tailwind color values - these match our gecko theme CSS variables
const COLOR_ACCENT = "var(--gecko-accent)";
const COLOR_WARNING = "var(--gecko-warning)";
const COLOR_DANGER = "var(--gecko-danger)";

function getBarColor(percent: number): string {
  if (percent > DANGER_THRESHOLD) return COLOR_DANGER;
  if (percent > WARNING_THRESHOLD) return COLOR_WARNING;
  return COLOR_ACCENT;
}

/** Handle for imperatively updating level data without React re-renders */
export interface LevelMeterHandle {
  /** Update the levels directly (no re-render) */
  updateLevels: (left: number, right: number) => void;
}

interface LevelMeterProps {
  /** Initial left level (optional, can be updated via ref) */
  initialLeft?: number;
  /** Initial right level (optional, can be updated via ref) */
  initialRight?: number;
}

/**
 * Real-time stereo level meter visualization
 *
 * Uses direct DOM manipulation for smooth 60fps animation without React re-renders.
 * For best performance, use the ref handle to update levels imperatively.
 */
export const LevelMeter = memo(forwardRef<LevelMeterHandle, LevelMeterProps>(
  function LevelMeter({ initialLeft = 0, initialRight = 0 }, ref) {
    // Store target values in refs - updated imperatively, not through props
    const targetLeft = useRef(initialLeft);
    const targetRight = useRef(initialRight);

    // Smoothed values for animation (not React state - direct DOM manipulation)
    const smoothedLeft = useRef(0);
    const smoothedRight = useRef(0);

    // DOM refs for direct manipulation (avoids React re-renders)
    const leftBarRef = useRef<HTMLDivElement>(null);
    const rightBarRef = useRef<HTMLDivElement>(null);

    // Expose imperative handle for updating levels without re-render
    useImperativeHandle(ref, () => ({
      updateLevels: (left: number, right: number) => {
        targetLeft.current = left;
        targetRight.current = right;
      },
    }), []);

    // Animation loop - directly manipulates DOM, no React state updates
    useEffect(() => {
      let animationId: number;

      const animate = () => {
        // Apply asymmetric smoothing: instant attack, smooth decay
        // Left channel
        if (targetLeft.current > smoothedLeft.current) {
          smoothedLeft.current = targetLeft.current * METER_ATTACK +
            smoothedLeft.current * (1 - METER_ATTACK);
        } else {
          smoothedLeft.current = smoothedLeft.current * METER_DECAY +
            targetLeft.current * (1 - METER_DECAY);
        }

        // Right channel
        if (targetRight.current > smoothedRight.current) {
          smoothedRight.current = targetRight.current * METER_ATTACK +
            smoothedRight.current * (1 - METER_ATTACK);
        } else {
          smoothedRight.current = smoothedRight.current * METER_DECAY +
            targetRight.current * (1 - METER_DECAY);
        }

        // Direct DOM manipulation - bypasses React entirely for smooth 60fps
        if (leftBarRef.current) {
          const percent = amplitudeToPercent(smoothedLeft.current);
          leftBarRef.current.style.width = `${percent}%`;
          leftBarRef.current.style.backgroundColor = getBarColor(percent);
        }

        if (rightBarRef.current) {
          const percent = amplitudeToPercent(smoothedRight.current);
          rightBarRef.current.style.width = `${percent}%`;
          rightBarRef.current.style.backgroundColor = getBarColor(percent);
        }

        animationId = requestAnimationFrame(animate);
      };

      animationId = requestAnimationFrame(animate);

      return () => {
        cancelAnimationFrame(animationId);
      };
    }, []);

    return (
      <div className="flex flex-col gap-1 min-w-[140px]">
        {/* Left channel */}
        <div className="flex items-center gap-2">
          <span className="text-2xs text-gecko-text-muted w-3">L</span>
          <div className="flex-1 h-2 bg-gecko-bg-tertiary rounded-full overflow-hidden">
            <div
              ref={leftBarRef}
              className="h-full rounded-full"
              style={{ width: "0%", backgroundColor: COLOR_ACCENT }}
              role="meter"
              aria-valuemin={0}
              aria-valuemax={100}
              aria-label="Left channel level"
            />
          </div>
        </div>
        {/* Right channel */}
        <div className="flex items-center gap-2">
          <span className="text-2xs text-gecko-text-muted w-3">R</span>
          <div className="flex-1 h-2 bg-gecko-bg-tertiary rounded-full overflow-hidden">
            <div
              ref={rightBarRef}
              className="h-full rounded-full"
              style={{ width: "0%", backgroundColor: COLOR_ACCENT }}
              role="meter"
              aria-valuemin={0}
              aria-valuemax={100}
              aria-label="Right channel level"
            />
          </div>
        </div>
      </div>
    );
  }
));
