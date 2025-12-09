import { useState, useEffect, useCallback, memo } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Button } from "./ui";
import { cn } from "../lib/utils";

interface Preset {
    name: string;
    gains: number[];
    isUser: boolean;
}

interface PresetSelectorProps {
    currentGains: number[];
    disabled?: boolean;
    onApply: (gains: number[]) => void;
}

export const PresetSelector = memo(function PresetSelector({
    currentGains,
    disabled = false,
    onApply,
}: PresetSelectorProps) {
    const [presets, setPresets] = useState<Preset[]>([]);
    const [activePreset, setActivePreset] = useState<string | null>(null);
    const [isOpen, setIsOpen] = useState(false);
    const [showSaveDialog, setShowSaveDialog] = useState(false);
    const [newPresetName, setNewPresetName] = useState("");

    // Load presets on mount
    useEffect(() => {
        const loadPresets = async () => {
            try {
                const result = await invoke<[string, number[], boolean][]>("get_presets");
                setPresets(result.map(([name, gains, isUser]) => ({ name, gains, isUser })));
            } catch (e) {
                console.error("Failed to load presets:", e);
            }
        };
        loadPresets();
    }, []);

    const handleApplyPreset = useCallback(async (preset: Preset) => {
        try {
            await invoke("apply_preset", { name: preset.name, gains: preset.gains });
            setActivePreset(preset.name);
            onApply(preset.gains);
            setIsOpen(false);
        } catch (e) {
            console.error("Failed to apply preset:", e);
        }
    }, [onApply]);

    const handleSavePreset = useCallback(async () => {
        if (!newPresetName.trim()) return;

        try {
            await invoke("save_preset", { name: newPresetName, gains: currentGains });

            // Reload presets
            const result = await invoke<[string, number[], boolean][]>("get_presets");
            setPresets(result.map(([name, gains, isUser]) => ({ name, gains, isUser })));

            setActivePreset(newPresetName);
            setNewPresetName("");
            setShowSaveDialog(false);
        } catch (e) {
            console.error("Failed to save preset:", e);
        }
    }, [newPresetName, currentGains]);

    const handleDeletePreset = useCallback(async (name: string, e: React.MouseEvent) => {
        e.stopPropagation();
        try {
            await invoke("delete_preset", { name });
            setPresets(prev => prev.filter(p => p.name !== name));
            if (activePreset === name) setActivePreset(null);
        } catch (e) {
            console.error("Failed to delete preset:", e);
        }
    }, [activePreset]);

    return (
        <div className={cn("flex items-center gap-2", disabled && "opacity-50 pointer-events-none")}>
            {/* Dropdown button */}
            <div className="relative">
                <Button
                    size="sm"
                    variant="default"
                    onClick={() => setIsOpen(!isOpen)}
                    className="min-w-[120px] justify-between"
                >
                    <span className="truncate">{activePreset || "Select Preset"}</span>
                    <span className="ml-2">{isOpen ? "▲" : "▼"}</span>
                </Button>

                {/* Dropdown menu */}
                {isOpen && (
                    <div className="absolute top-full left-0 mt-1 z-50 min-w-[180px] max-h-[200px] overflow-y-auto bg-gecko-bg-tertiary border border-gecko-border rounded-lg shadow-lg">
                        {presets.length === 0 ? (
                            <div className="px-3 py-2 text-sm text-gecko-text-muted">No presets</div>
                        ) : (
                            presets.map(preset => (
                                <div
                                    key={preset.name}
                                    onClick={() => handleApplyPreset(preset)}
                                    className={cn(
                                        "flex items-center justify-between px-3 py-2 text-sm cursor-pointer hover:bg-gecko-bg-secondary transition-colors",
                                        activePreset === preset.name && "bg-gecko-accent/20 text-gecko-accent"
                                    )}
                                >
                                    <span className="truncate">{preset.name}</span>
                                    {preset.isUser && (
                                        <button
                                            onClick={(e) => handleDeletePreset(preset.name, e)}
                                            className="ml-2 text-gecko-text-muted hover:text-gecko-danger transition-colors"
                                            title="Delete"
                                        >
                                            ×
                                        </button>
                                    )}
                                </div>
                            ))
                        )}
                    </div>
                )}
            </div>

            {/* Save button */}
            {!showSaveDialog ? (
                <Button
                    size="sm"
                    variant="default"
                    onClick={() => setShowSaveDialog(true)}
                >
                    Save
                </Button>
            ) : (
                <div className="flex gap-1 items-center">
                    <input
                        type="text"
                        value={newPresetName}
                        onChange={e => setNewPresetName(e.target.value)}
                        placeholder="Name"
                        className="px-2 py-1 text-sm bg-gecko-bg-tertiary border border-gecko-border rounded text-gecko-text-primary w-24"
                        autoFocus
                        onKeyDown={e => e.key === 'Enter' && handleSavePreset()}
                    />
                    <Button size="sm" variant="primary" onClick={handleSavePreset}>✓</Button>
                    <Button size="sm" variant="default" onClick={() => setShowSaveDialog(false)}>×</Button>
                </div>
            )}
        </div>
    );
});

