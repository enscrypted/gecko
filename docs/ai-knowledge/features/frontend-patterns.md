# Frontend Patterns

**Last Updated**: December 2025
**Context**: Read when building React components, working with styling, or managing state

## Component Architecture

### Base Components (src/components/ui/)
Reusable, unstyled building blocks using CVA (class-variance-authority):
- `Button` - Action triggers with variants
- `Slider` - Range inputs (horizontal/vertical)
- `Card` - Container with header/content
- `Select` - Dropdown selections
- `Badge` - Status indicators

### Feature Components (src/components/)
Domain-specific components that compose base components:
- `Equalizer` - 10-band EQ with sliders
- `Controls` - Play/stop, bypass, volume
- `LevelMeter` - Audio level visualization
- `SpectrumAnalyzer` - Real-time FFT visualization
- `StreamList` - Per-app audio streams with EQ
- `Settings` - Configuration modal with themes

## CVA Pattern

All base components use class-variance-authority for variant styling:

```tsx
import { cva, type VariantProps } from "class-variance-authority";
import { cn } from "../../lib/utils";

// 1. Define variants with cva()
const buttonVariants = cva(
  // Base styles (use array for readability)
  [
    "inline-flex items-center justify-center",
    "rounded font-medium text-sm",
    "transition-colors duration-150",
    "focus-visible:outline-none focus-visible:ring-2",
    "disabled:pointer-events-none disabled:opacity-50",
  ],
  {
    variants: {
      variant: {
        default: [
          "bg-gecko-bg-tertiary border border-gecko-border",
          "text-gecko-text-primary",
          "hover:bg-gecko-border-hover",
        ],
        primary: [
          "bg-gecko-accent border border-gecko-accent",
          "text-gecko-bg-primary font-semibold",
          "hover:bg-gecko-accent-hover",
        ],
        // ... more variants
      },
      size: {
        sm: "h-8 px-3 text-xs",
        md: "h-9 px-4",
        lg: "h-10 px-6",
      },
    },
    defaultVariants: {
      variant: "default",
      size: "md",
    },
  }
);

// 2. Define props interface extending VariantProps
export interface ButtonProps
  extends ButtonHTMLAttributes<HTMLButtonElement>,
    VariantProps<typeof buttonVariants> {}

// 3. Use forwardRef for base components
export const Button = forwardRef<HTMLButtonElement, ButtonProps>(
  ({ className, variant, size, ...props }, ref) => {
    return (
      <button
        className={cn(buttonVariants({ variant, size, className }))}
        ref={ref}
        {...props}
      />
    );
  }
);

// 4. Set displayName for DevTools
Button.displayName = "Button";
```

## Design Tokens & Theming

Colors use CSS custom properties for theme support. The `gecko-*` tokens in `tailwind.config.js` reference CSS variables:

```javascript
// tailwind.config.js - references CSS variables
colors: {
  gecko: {
    bg: {
      primary: "var(--gecko-bg-primary)",
      secondary: "var(--gecko-bg-secondary)",
      // ...
    },
    // ...
  },
}
```

Theme definitions are in `src/styles.css`:

```css
/* Default dark theme */
:root {
  --gecko-bg-primary: #0a0a0a;
  --gecko-text-primary: #fafafa;
  --gecko-accent: #4ade80;
  /* ... */
}

/* Other themes use data-theme attribute */
[data-theme="light"] { /* ... */ }
[data-theme="midnight"] { /* ... */ }
[data-theme="colorblind"] { /* ... */ }
```

### Available Themes
| Theme | Description |
|-------|-------------|
| `dark` | Default dark theme (green accent) |
| `light` | Bright theme for well-lit environments |
| `midnight` | Deep blue tones, easy on eyes at night |
| `nord` | Arctic-inspired, soft muted colors |
| `solarized` | Low-contrast, reduces eye strain |
| `high-contrast` | Maximum readability, WCAG AAA |
| `colorblind` | Optimized for red-green color blindness |

### Theme Application

Themes are managed via `SettingsContext`:

```tsx
// Apply theme to document root
function applyTheme(theme: ThemeName) {
  if (theme === "dark") {
    document.documentElement.removeAttribute("data-theme");
  } else {
    document.documentElement.setAttribute("data-theme", theme);
  }
}
```

### Usage

```tsx
// GOOD: Use design tokens (works with all themes)
<div className="bg-gecko-bg-primary text-gecko-text-primary">
<button className="bg-gecko-accent hover:bg-gecko-accent-hover">

// BAD: Don't use raw colors (breaks theming)
<div className="bg-gray-900 text-white">
<button className="bg-green-400 hover:bg-green-500">
```

## The cn() Utility

Located in `src/lib/utils.ts`, combines clsx and tailwind-merge:

```typescript
import { type ClassValue, clsx } from "clsx";
import { twMerge } from "tailwind-merge";

export function cn(...inputs: ClassValue[]) {
  return twMerge(clsx(inputs));
}
```

### Usage

```tsx
// Merge conditional classes without conflicts
<div className={cn(
  "base-class",
  isActive && "active-class",
  className  // Allow override from props
)} />

// tailwind-merge handles conflicts:
cn("px-4", "px-2")  // Results in "px-2" (last wins)
```

## Performance Patterns

### Memoization

```tsx
// Memoize components receiving stable props
const EqBand = memo(function EqBand({ band, onGainChange }: EqBandProps) {
  // Component only re-renders when props change
  return <Slider value={band.gain_db} onChange={...} />;
});
```

### Stable Callbacks

```tsx
// Use useCallback for handlers passed to children
const handleBandChange = useCallback(
  async (index: number, gainDb: number) => {
    // Optimistic update for responsiveness
    setBands(prev =>
      prev.map(b => b.index === index ? { ...b, gain_db: gainDb } : b)
    );
    // Sync with backend
    await invoke("set_band_gain", { band: index, gainDb });
  },
  []  // Empty deps - function is stable
);
```

### Optimistic Updates

```tsx
// Update UI immediately, then sync with backend
const handleVolumeChange = useCallback(async (volume: number) => {
  setMasterVolume(volume);  // Instant UI feedback
  try {
    await invoke("set_master_volume", { volume });
  } catch (e) {
    setError(String(e));
    // Could revert here if needed
  }
}, []);
```

## Tauri IPC

### Invoking Commands

```tsx
import { invoke } from "@tauri-apps/api/core";

// Type-safe invocation
const devices = await invoke<DeviceInfo[]>("list_devices");
await invoke("set_band_gain", { band: 0, gainDb: 6.0 });
```

### Polling Pattern (for events)

```tsx
useEffect(() => {
  if (!isRunning) return;

  const interval = setInterval(async () => {
    try {
      const events = await invoke<string[]>("poll_events");
      for (const eventJson of events) {
        const event = JSON.parse(eventJson);
        if (event.type === "LevelUpdate") {
          setLevels(event.payload);
        }
      }
    } catch {
      // Ignore polling errors
    }
  }, 50);  // 20 FPS update rate

  return () => clearInterval(interval);
}, [isRunning]);
```

## File Organization

```
src/
├── components/
│   ├── ui/                    # Base components (CVA)
│   │   ├── button.tsx
│   │   ├── slider.tsx
│   │   ├── card.tsx
│   │   ├── select.tsx
│   │   ├── badge.tsx
│   │   └── index.ts           # Barrel export
│   ├── Equalizer.tsx          # EQ slider controls
│   ├── Controls.tsx           # Play/stop, volume, bypass
│   ├── LevelMeter.tsx         # Audio level visualization
│   ├── StreamList.tsx         # Per-app stream list
│   ├── AudioStreamItem.tsx    # Individual app with EQ
│   └── Settings.tsx           # Settings modal with themes
├── contexts/
│   └── SettingsContext.tsx    # Settings & theme management
├── lib/
│   └── utils.ts               # cn() and other utilities
├── App.tsx                    # Root component
├── main.tsx                   # Entry point
└── styles.css                 # Global styles + theme definitions
```

## Related Files

- `tailwind.config.js` - Design token definitions (CSS variables)
- `src/styles.css` - Theme color definitions
- `src/contexts/SettingsContext.tsx` - Theme constants and application
- `src/lib/utils.ts` - cn() utility
- `src/components/ui/index.ts` - Component exports
