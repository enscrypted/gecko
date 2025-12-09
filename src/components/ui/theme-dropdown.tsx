import { useState, useRef, useEffect } from "react";
import { cn } from "../../lib/utils";

interface ThemeOption {
    value: string;
    label: string;
    description: string;
}

interface ThemeDropdownProps {
    value: string;
    options: ThemeOption[];
    onChange: (value: string) => void;
    disabled?: boolean;
}

/**
 * Compact dropdown for theme selection
 * Stays open until clicked outside, allowing quick theme switching
 */
export function ThemeDropdown({ value, options, onChange, disabled = false }: ThemeDropdownProps) {
    const [isOpen, setIsOpen] = useState(false);
    const dropdownRef = useRef<HTMLDivElement>(null);

    // Find current option
    const currentOption = options.find(opt => opt.value === value) || options[0];

    // Close on click outside
    useEffect(() => {
        function handleClickOutside(event: MouseEvent) {
            if (dropdownRef.current && !dropdownRef.current.contains(event.target as Node)) {
                setIsOpen(false);
            }
        }

        if (isOpen) {
            document.addEventListener("mousedown", handleClickOutside);
            return () => document.removeEventListener("mousedown", handleClickOutside);
        }
    }, [isOpen]);

    // Close on escape
    useEffect(() => {
        function handleEscape(event: KeyboardEvent) {
            if (event.key === "Escape") {
                setIsOpen(false);
            }
        }

        if (isOpen) {
            document.addEventListener("keydown", handleEscape);
            return () => document.removeEventListener("keydown", handleEscape);
        }
    }, [isOpen]);

    return (
        <div ref={dropdownRef} className="relative">
            {/* Trigger button */}
            <button
                type="button"
                onClick={() => !disabled && setIsOpen(!isOpen)}
                disabled={disabled}
                className={cn(
                    "w-full flex items-center justify-between px-3 py-2 rounded border transition-colors text-left",
                    "bg-gecko-bg-tertiary border-gecko-border",
                    "hover:bg-gecko-bg-elevated hover:border-gecko-border-hover",
                    disabled && "opacity-50 cursor-not-allowed"
                )}
                aria-haspopup="listbox"
                aria-expanded={isOpen}
            >
                <div className="flex-1 min-w-0">
                    <div className="text-sm font-medium text-gecko-text-primary truncate">
                        {currentOption.label}
                    </div>
                    <div className="text-xs text-gecko-text-muted truncate">
                        {currentOption.description}
                    </div>
                </div>
                <span className={cn(
                    "ml-2 text-gecko-text-muted transition-transform",
                    isOpen && "rotate-180"
                )}>
                    ▼
                </span>
            </button>

            {/* Dropdown menu */}
            {isOpen && (
                <div
                    className={cn(
                        "absolute z-50 w-full mt-1 py-1 rounded border shadow-lg",
                        "bg-gecko-bg-secondary border-gecko-border",
                        "max-h-64 overflow-y-auto"
                    )}
                    role="listbox"
                >
                    {options.map((option) => (
                        <button
                            key={option.value}
                            type="button"
                            onClick={() => {
                                onChange(option.value);
                                // Don't close - allow multiple switches
                            }}
                            className={cn(
                                "w-full px-3 py-2 text-left transition-colors",
                                option.value === value
                                    ? "bg-gecko-accent/20 border-l-2 border-gecko-accent"
                                    : "hover:bg-gecko-bg-tertiary border-l-2 border-transparent"
                            )}
                            role="option"
                            aria-selected={option.value === value}
                        >
                            <div className={cn(
                                "text-sm font-medium",
                                option.value === value ? "text-gecko-accent" : "text-gecko-text-primary"
                            )}>
                                {option.label}
                            </div>
                            <div className="text-xs text-gecko-text-muted">
                                {option.description}
                            </div>
                        </button>
                    ))}
                </div>
            )}
        </div>
    );
}
