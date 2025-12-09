import { useRef, useEffect, useImperativeHandle, forwardRef, memo } from "react";
import { cn } from "../lib/utils";

// Number of frequency bins from the backend
const NUM_BINS = 32;

// Smoothing constants - same as LevelMeter for consistency
const SPECTRUM_DECAY = 0.85;

// Cache theme colors to avoid getComputedStyle on every frame
let cachedAccentColor = "#4ade80";
let cachedBgColor = "#1f1f1f";
let cachedWarningColor = "#f59e0b";
let cachedDangerColor = "#ef4444";
let colorsCached = false;

function cacheThemeColors() {
  if (colorsCached) return;
  const computedStyle = getComputedStyle(document.documentElement);
  cachedAccentColor = computedStyle.getPropertyValue("--gecko-accent").trim() || "#4ade80";
  cachedBgColor = computedStyle.getPropertyValue("--gecko-bg-tertiary").trim() || "#1f1f1f";
  cachedWarningColor = computedStyle.getPropertyValue("--gecko-warning").trim() || "#f59e0b";
  cachedDangerColor = computedStyle.getPropertyValue("--gecko-danger").trim() || "#ef4444";
  colorsCached = true;
}

// Reset cache when theme changes (call this from theme switcher)
export function resetSpectrumColorCache() {
  colorsCached = false;
}

/** Handle for imperatively updating spectrum data without React re-renders */
export interface SpectrumAnalyzerHandle {
  /** Update the spectrum bins directly (no re-render) */
  updateBins: (bins: number[]) => void;
}

interface SpectrumAnalyzerProps {
  /** Initial bins (optional, can be updated via ref) */
  initialBins?: number[];
  /** Width of the component */
  width?: number;
  /** Height of the component */
  height?: number;
  /** Additional class names */
  className?: string;
}

/**
 * Real-time FFT spectrum analyzer visualization
 *
 * Renders frequency spectrum as vertical bars, with logarithmically-spaced
 * frequency bins from ~20Hz to 20kHz. Uses canvas for efficient rendering.
 *
 * For best performance, use the ref handle to update bins imperatively
 * rather than passing bins as props (avoids React re-renders).
 */
export const SpectrumAnalyzer = memo(forwardRef<SpectrumAnalyzerHandle, SpectrumAnalyzerProps>(
  function SpectrumAnalyzer({
    initialBins,
    width = 200,
    height = 40,
    className,
  }, ref) {
    const canvasRef = useRef<HTMLCanvasElement>(null);
    const smoothedBins = useRef<number[]>(new Array(NUM_BINS).fill(0));
    // Store target bins in ref - updated imperatively, not through props
    const targetBins = useRef<number[]>(initialBins || new Array(NUM_BINS).fill(0));

    // Expose imperative handle for updating bins without re-render
    useImperativeHandle(ref, () => ({
      updateBins: (bins: number[]) => {
        targetBins.current = bins;
      },
    }), []);

    // Animation loop - runs continuously for smooth visualization
    useEffect(() => {
      const canvas = canvasRef.current;
      if (!canvas) return;

      const ctx = canvas.getContext("2d");
      if (!ctx) return;

      // Cache theme colors on first render
      cacheThemeColors();

      let animationId: number;

      const draw = () => {
        // Smooth the bins with exponential decay for smoother visualization
        const currentBins = targetBins.current;
        for (let i = 0; i < NUM_BINS; i++) {
          const targetValue = currentBins[i] ?? 0;
          // Rise fast, fall slow for better visual appeal
          if (targetValue > smoothedBins.current[i]) {
            smoothedBins.current[i] = targetValue; // Instant rise
          } else {
            smoothedBins.current[i] = smoothedBins.current[i] * SPECTRUM_DECAY + targetValue * (1 - SPECTRUM_DECAY);
          }
        }

        // Clear canvas
        ctx.fillStyle = cachedBgColor;
        ctx.fillRect(0, 0, width, height);

        // Draw bars
        const barWidth = width / NUM_BINS;
        const gap = 1;

        for (let i = 0; i < NUM_BINS; i++) {
          const magnitude = smoothedBins.current[i];
          const barHeight = magnitude * height;
          const x = i * barWidth;
          const y = height - barHeight;

          // Color based on magnitude (green -> yellow -> red)
          let color = cachedAccentColor;
          if (magnitude > 0.9) {
            color = cachedDangerColor; // Red for clipping
          } else if (magnitude > 0.7) {
            color = cachedWarningColor; // Yellow/orange for loud
          }

          ctx.fillStyle = color;
          ctx.fillRect(x + gap / 2, y, barWidth - gap, barHeight);
        }

        // Continue animation loop
        animationId = requestAnimationFrame(draw);
      };

      // Start animation loop
      animationId = requestAnimationFrame(draw);

      return () => {
        cancelAnimationFrame(animationId);
      };
    }, [width, height]); // Only restart loop if dimensions change

    return (
      <div className={cn("flex flex-col gap-1", className)}>
        <canvas
          ref={canvasRef}
          width={width}
          height={height}
          className="rounded"
          style={{ width, height }}
          aria-label="Audio spectrum analyzer"
          role="img"
        />
      </div>
    );
  }
));

/**
 * Compact spectrum analyzer for inline display next to level meters
 * Forwards ref for imperative updates
 */
export const CompactSpectrumAnalyzer = forwardRef<SpectrumAnalyzerHandle, {
  className?: string;
}>(function CompactSpectrumAnalyzer({ className }, ref) {
  return (
    <SpectrumAnalyzer
      ref={ref}
      width={140}
      height={32}
      className={className}
    />
  );
});
