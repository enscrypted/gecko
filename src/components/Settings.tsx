import { useCallback, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Button, ThemeDropdown } from "./ui";
import { useSettings, UiSettings, AVAILABLE_THEMES, THEME_INFO, ThemeName } from "../contexts/SettingsContext";

interface SettingsProps {
    isOpen: boolean;
    onClose: () => void;
}

export function Settings({ isOpen, onClose }: SettingsProps) {
    const { settings, loading, updateUiSettings } = useSettings();
    const [autoStart, setAutoStart] = useState(false);
    const [autoStartLoading, setAutoStartLoading] = useState(true);

    // Load autostart state when modal opens
    useEffect(() => {
        if (isOpen) {
            invoke<boolean>("get_autostart")
                .then(setAutoStart)
                .catch(() => setAutoStart(false))
                .finally(() => setAutoStartLoading(false));
        }
    }, [isOpen]);

    const handleAutoStartChange = useCallback(async (enabled: boolean) => {
        try {
            await invoke("set_autostart", { enabled });
            setAutoStart(enabled);
        } catch (e) {
            console.error("Failed to set autostart:", e);
        }
    }, []);

    const handleChange = useCallback(async <K extends keyof UiSettings>(key: K, value: UiSettings[K]) => {
        await updateUiSettings(key, value);
    }, [updateUiSettings]);

    if (!isOpen || loading || !settings) return null;

    return (
        <div className="fixed inset-0 bg-black/50 flex items-center justify-center z-50" onClick={onClose}>
            <div
                className="bg-gecko-bg-primary rounded-lg shadow-xl w-full max-w-md max-h-[80vh] flex flex-col border border-gecko-border"
                onClick={e => e.stopPropagation()}
            >
                <div className="flex items-center justify-between p-4 border-b border-gecko-border">
                    <h2 className="text-lg font-semibold text-gecko-text-primary">Settings</h2>
                    <button
                        onClick={onClose}
                        className="text-gecko-text-muted hover:text-gecko-text-primary transition-colors"
                    >
                        ✕
                    </button>
                </div>

                <div className="p-4 space-y-6 overflow-y-auto flex-1">
                    {/* Appearance Section */}
                    <section>
                        <h3 className="text-sm font-medium text-gecko-text-secondary mb-3">Appearance</h3>
                        <div className="space-y-3">
                            {/* Theme Selector */}
                            <div className="space-y-2">
                                <span className="text-sm text-gecko-text-primary">Theme</span>
                                <ThemeDropdown
                                    value={settings.ui_settings.theme}
                                    options={AVAILABLE_THEMES.map((theme) => ({
                                        value: theme,
                                        label: THEME_INFO[theme].label,
                                        description: THEME_INFO[theme].description,
                                    }))}
                                    onChange={(value) => handleChange("theme", value as ThemeName)}
                                />
                            </div>
                        </div>
                    </section>

                    {/* Audio Section */}
                    <section>
                        <h3 className="text-sm font-medium text-gecko-text-secondary mb-3">Audio</h3>
                        <div className="space-y-3">
                            {/* Soft Clipping (Limiter) */}
                            <div className="flex items-center justify-between">
                                <div className="flex flex-col">
                                    <span className="text-sm text-gecko-text-primary">Soft Clipping</span>
                                    <span className="text-xs text-gecko-text-muted">Prevent harsh digital distortion</span>
                                </div>
                                <input
                                    type="checkbox"
                                    checked={settings.ui_settings.soft_clip_enabled ?? true}
                                    onChange={e => {
                                        handleChange("soft_clip_enabled", e.target.checked);
                                        invoke("set_soft_clip", { enabled: e.target.checked });
                                    }}
                                    className="w-4 h-4"
                                />
                            </div>
                        </div>
                    </section>

                    {/* Display Section */}
                    <section>
                        <h3 className="text-sm font-medium text-gecko-text-secondary mb-3">Display</h3>
                        <div className="space-y-3">
                            {/* EQ Bands */}
                            <div className="flex items-center justify-between">
                                <span className="text-sm text-gecko-text-primary">EQ Bands</span>
                                <div className="flex gap-2">
                                    <Button
                                        size="sm"
                                        variant={settings.ui_settings.eq_bands_ui === 5 ? "primary" : "default"}
                                        onClick={() => handleChange("eq_bands_ui", 5)}
                                    >
                                        5
                                    </Button>
                                    <Button
                                        size="sm"
                                        variant={settings.ui_settings.eq_bands_ui === 10 ? "primary" : "default"}
                                        onClick={() => handleChange("eq_bands_ui", 10)}
                                    >
                                        10
                                    </Button>
                                </div>
                            </div>

                            {/* Show Level Meters */}
                            <div className="flex items-center justify-between">
                                <span className="text-sm text-gecko-text-primary">Show Level Meters</span>
                                <input
                                    type="checkbox"
                                    checked={settings.ui_settings.show_level_meters}
                                    onChange={e => handleChange("show_level_meters", e.target.checked)}
                                    className="w-4 h-4"
                                />
                            </div>
                        </div>
                    </section>

                    {/* Behavior Section */}
                    <section>
                        <h3 className="text-sm font-medium text-gecko-text-secondary mb-3">Behavior</h3>
                        <div className="space-y-3">
                            {/* Auto-start on login */}
                            <div className="flex items-center justify-between">
                                <div className="flex flex-col">
                                    <span className="text-sm text-gecko-text-primary">Start on Login</span>
                                    <span className="text-xs text-gecko-text-muted">Launch Gecko when you log in</span>
                                </div>
                                <input
                                    type="checkbox"
                                    checked={autoStart}
                                    disabled={autoStartLoading}
                                    onChange={e => handleAutoStartChange(e.target.checked)}
                                    className="w-4 h-4"
                                />
                            </div>
                            {/* Start Minimized */}
                            <div className="flex items-center justify-between">
                                <div className="flex flex-col">
                                    <span className="text-sm text-gecko-text-primary">Start Minimized</span>
                                    <span className="text-xs text-gecko-text-muted">Launch to system tray</span>
                                </div>
                                <input
                                    type="checkbox"
                                    checked={settings.ui_settings.start_minimized}
                                    onChange={e => handleChange("start_minimized", e.target.checked)}
                                    className="w-4 h-4"
                                />
                            </div>
                        </div>
                    </section>
                </div>

                {/* Footer */}
                <div className="flex justify-end gap-2 p-4 border-t border-gecko-border">
                    <Button variant="primary" onClick={onClose}>
                        Done
                    </Button>
                </div>
            </div>
        </div>
    );
}

