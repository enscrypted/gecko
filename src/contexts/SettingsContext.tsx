import { createContext, useContext, useState, useEffect, useCallback, useRef, ReactNode } from "react";
import { invoke } from "@tauri-apps/api/core";

// Available themes - must match CSS [data-theme="..."] selectors in styles.css
export const AVAILABLE_THEMES = [
    "dark",
    "light",
    "midnight",
    "nord",
    "solarized",
    "high-contrast",
    "colorblind",
] as const;
export type ThemeName = typeof AVAILABLE_THEMES[number];

// Human-readable theme names and descriptions for UI
export const THEME_INFO: Record<ThemeName, { label: string; description: string }> = {
    dark: { label: "Dark", description: "Default dark theme" },
    light: { label: "Light", description: "Bright theme for well-lit environments" },
    midnight: { label: "Midnight", description: "Deep blue tones, easy on eyes at night" },
    nord: { label: "Nord", description: "Arctic-inspired, soft muted colors" },
    solarized: { label: "Solarized", description: "Low-contrast, reduces eye strain" },
    "high-contrast": { label: "High Contrast", description: "Maximum readability, WCAG AAA" },
    colorblind: { label: "Colorblind", description: "Optimized for red-green color blindness" },
};

export interface UiSettings {
    theme: ThemeName;
    show_level_meters: boolean;
    start_minimized: boolean;
    eq_bands_ui: number;
    soft_clip_enabled: boolean;
}

export interface GeckoSettings {
    master_volume: number;
    master_eq: number[];
    app_eq: { [key: string]: number[] };
    /** Per-app volume settings (0.0-2.0, default 1.0) */
    app_volumes: { [key: string]: number };
    bypassed: boolean;
    /** Set of app names that have per-app EQ bypassed */
    bypassed_apps: string[];
    /** Set of app names hidden from the UI */
    hidden_apps: string[];
    active_preset: string | null;
    user_presets: any[];
    ui_settings: UiSettings;
}

interface SettingsContextValue {
    settings: GeckoSettings | null;
    loading: boolean;
    updateSettings: (updates: Partial<GeckoSettings>) => Promise<void>;
    updateUiSettings: <K extends keyof UiSettings>(key: K, value: UiSettings[K]) => Promise<void>;
    reloadSettings: () => Promise<void>;
}

const defaultSettings: GeckoSettings = {
    master_volume: 1.0,
    master_eq: Array(10).fill(0),
    app_eq: {},
    app_volumes: {},
    bypassed: false,
    bypassed_apps: [],
    hidden_apps: [],
    active_preset: "Flat",
    user_presets: [],
    ui_settings: {
        theme: "dark",
        show_level_meters: true,
        start_minimized: false,
        eq_bands_ui: 10,
        soft_clip_enabled: true,
    },
};

// Apply theme to document root element
function applyTheme(theme: ThemeName) {
    // "dark" is the default (no data-theme attribute needed, uses :root styles)
    if (theme === "dark") {
        document.documentElement.removeAttribute("data-theme");
    } else {
        document.documentElement.setAttribute("data-theme", theme);
    }
}

const SettingsContext = createContext<SettingsContextValue | null>(null);

export function SettingsProvider({ children }: { children: ReactNode }) {
    const [settings, setSettings] = useState<GeckoSettings | null>(null);
    const [loading, setLoading] = useState(true);

    const reloadSettings = useCallback(async () => {
        try {
            const s = await invoke<GeckoSettings>("get_settings");
            setSettings(s);
        } catch (e) {
            console.error("Failed to load settings:", e);
            setSettings(defaultSettings);
        } finally {
            setLoading(false);
        }
    }, []);

    // Load settings on mount
    useEffect(() => {
        reloadSettings();
    }, [reloadSettings]);

    // Apply theme whenever settings change
    useEffect(() => {
        if (settings?.ui_settings?.theme) {
            applyTheme(settings.ui_settings.theme);
        }
    }, [settings?.ui_settings?.theme]);

    // Debounce timer for saving settings
    const saveTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

    // Actual save function
    const persistSettings = async (settingsToSave: GeckoSettings) => {
        try {
            await invoke("save_settings", { settings: settingsToSave });
        } catch (e) {
            console.error("Failed to save settings:", e);
        }
    };

    const updateSettings = useCallback(async (updates: Partial<GeckoSettings>) => {
        if (!settings) return;

        const newSettings = { ...settings, ...updates };
        setSettings(newSettings);

        // Debounce save
        if (saveTimerRef.current) clearTimeout(saveTimerRef.current);

        saveTimerRef.current = setTimeout(() => {
            persistSettings(newSettings);
            saveTimerRef.current = null;
        }, 1000); // 1s debounce
    }, [settings]);

    const updateUiSettings = useCallback(async <K extends keyof UiSettings>(key: K, value: UiSettings[K]) => {
        if (!settings) return;

        const newSettings = {
            ...settings,
            ui_settings: { ...settings.ui_settings, [key]: value },
        };
        setSettings(newSettings);

        // Debounce save
        if (saveTimerRef.current) clearTimeout(saveTimerRef.current);

        saveTimerRef.current = setTimeout(() => {
            persistSettings(newSettings);
            saveTimerRef.current = null;
        }, 1000);
    }, [settings]);

    // Cleanup timer
    useEffect(() => {
        return () => {
            if (saveTimerRef.current) clearTimeout(saveTimerRef.current);
        };
    }, []);

    return (
        <SettingsContext.Provider value={{ settings, loading, updateSettings, updateUiSettings, reloadSettings }}>
            {children}
        </SettingsContext.Provider>
    );
}

export function useSettings() {
    const context = useContext(SettingsContext);
    if (!context) {
        throw new Error("useSettings must be used within SettingsProvider");
    }
    return context;
}
