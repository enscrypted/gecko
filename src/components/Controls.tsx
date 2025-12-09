import { memo } from "react";
import { Button, Slider, EditableValue } from "./ui";

interface ControlsProps {
  isRunning: boolean;
  isBypassed: boolean;
  masterVolume: number;
  onStart: () => void;
  onStop: () => void;
  onBypassChange: (bypassed: boolean) => void;
  onVolumeChange: (volume: number) => void;
}

export const Controls = memo(function Controls({
  isRunning,
  isBypassed,
  masterVolume,
  onStart,
  onStop,
  onBypassChange,
  onVolumeChange,
}: ControlsProps) {
  return (
    <div className="flex items-center gap-6 flex-wrap">
      {/* Transport Controls */}
      <div className="flex gap-2">
        <Button
          variant={isRunning ? "danger" : "primary"}
          onClick={isRunning ? onStop : onStart}
        >
          {isRunning ? "Stop" : "Start"}
        </Button>

        <Button
          variant={isBypassed ? "warning" : "default"}
          onClick={() => onBypassChange(!isBypassed)}
          disabled={!isRunning}
        >
          Bypass
        </Button>
      </div>

      {/* Volume Control */}
      <div className="flex items-center gap-3">
        <label
          htmlFor="master-volume"
          className="text-sm text-gecko-text-secondary"
        >
          Master Volume
        </label>
        <Slider
          id="master-volume"
          min={0}
          max={1}
          step={0.01}
          value={masterVolume}
          onChange={(e) => onVolumeChange(parseFloat(e.target.value))}
          className="w-28"
          aria-label="Master volume"
        />
        <EditableValue
          value={Math.round(masterVolume * 100)}
          onChange={(percent) => onVolumeChange(percent / 100)}
          min={0}
          max={100}
          decimals={0}
          suffix="%"
          className="text-sm text-gecko-text-primary min-w-[3rem]"
          inputWidth="w-12"
        />
      </div>
    </div>
  );
});
