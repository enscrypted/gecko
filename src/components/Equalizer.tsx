import { useEffect, useState, useCallback, memo } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Card, CardHeader, CardTitle, CardContent, Slider } from "./ui";
import { PresetSelector } from "./PresetSelector";
import { cn, formatFrequency } from "../lib/utils";

interface BandInfo {
  index: number;
  frequency: number;
  gain_db: number;
  enabled: boolean;
}

interface EqualizerProps {
  disabled?: boolean;
}

interface EqBandProps {
  band: BandInfo;
  disabled: boolean;
  onGainChange: (index: number, gainDb: number) => void;
}

// Memoized band component to prevent unnecessary re-renders
const EqBand = memo(function EqBand({
  band,
  disabled,
  onGainChange,
}: EqBandProps) {
  const handleChange = useCallback(
    (e: React.ChangeEvent<HTMLInputElement>) => {
      onGainChange(band.index, parseFloat(e.target.value));
    },
    [band.index, onGainChange]
  );

  return (
    <div className="flex flex-col items-center flex-1 min-w-[40px] gap-1">
      <div className="h-32 flex items-center justify-center">
        <Slider
          orientation="vertical"
          min={-24}
          max={24}
          step={0.5}
          value={band.gain_db}
          onChange={handleChange}
          disabled={disabled}
          className="h-32"
          aria-label={`${formatFrequency(band.frequency)}Hz band`}
        />
      </div>
      <span className="text-2xs font-mono text-gecko-text-muted">
        {band.gain_db > 0 ? "+" : ""}
        {band.gain_db.toFixed(1)}
      </span>
      <span className="text-xs font-medium text-gecko-accent">
        {formatFrequency(band.frequency)}
      </span>
    </div>
  );
});

export function Equalizer({ disabled = false }: EqualizerProps) {
  const [bands, setBands] = useState<BandInfo[]>([]);

  useEffect(() => {
    const loadBands = async () => {
      try {
        const bandInfo = await invoke<BandInfo[]>("get_eq_bands");
        setBands(bandInfo);
      } catch (e) {
        console.error("Failed to load EQ bands:", e);
      }
    };
    loadBands();
  }, []);

  const handleBandChange = useCallback(async (index: number, gainDb: number) => {
    // Optimistic update for responsiveness
    setBands((prev) =>
      prev.map((b) => (b.index === index ? { ...b, gain_db: gainDb } : b))
    );

    try {
      await invoke("set_band_gain", { band: index, gainDb });
    } catch (e) {
      console.error("Failed to set band gain:", e);
    }
  }, []);

  return (
    <Card className={cn("flex-1", disabled && "opacity-50 pointer-events-none")}>
      <CardHeader>
        <CardTitle>10-Band Equalizer</CardTitle>
      </CardHeader>
      <CardContent className="space-y-4">
        {/* Preset Selector */}
        <PresetSelector
          currentGains={bands.map(b => b.gain_db)}
          disabled={disabled}
          onApply={(gains) => {
            setBands(prev => prev.map((b, i) => ({ ...b, gain_db: gains[i] ?? 0 })));
          }}
        />

        <div className="flex justify-between gap-1">
          {bands.map((band) => (
            <EqBand
              key={band.index}
              band={band}
              disabled={disabled}
              onGainChange={handleBandChange}
            />
          ))}
        </div>

        {/* Scale indicators */}
        <div className="flex justify-between text-2xs text-gecko-text-muted mt-3 pt-3 border-t border-gecko-border">
          <span>+24dB</span>
          <span>0dB</span>
          <span>-24dB</span>
        </div>
      </CardContent>
    </Card>
  );
}
