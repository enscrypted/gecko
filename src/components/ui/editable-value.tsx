import { useState, useRef, useEffect, useCallback, KeyboardEvent, FocusEvent } from "react";
import { cn } from "../../lib/utils";

interface EditableValueProps {
    /** Current value to display */
    value: number;
    /** Callback when value is committed */
    onChange: (value: number) => void;
    /** Minimum allowed value */
    min?: number;
    /** Maximum allowed value */
    max?: number;
    /** Decimal places to show/allow */
    decimals?: number;
    /** Suffix to display (e.g., "dB", "%") */
    suffix?: string;
    /** Prefix to display (e.g., "+") */
    showPositive?: boolean;
    /** Additional className for styling */
    className?: string;
    /** Disable editing */
    disabled?: boolean;
    /** Width of the input field */
    inputWidth?: string;
}

/**
 * Accessible editable numeric value
 *
 * Displays as text, becomes editable on click.
 * Validates input and shows visual feedback when editing.
 */
export function EditableValue({
    value,
    onChange,
    min = -Infinity,
    max = Infinity,
    decimals = 1,
    suffix = "",
    showPositive = false,
    className,
    disabled = false,
    inputWidth = "w-12",
}: EditableValueProps) {
    const [isEditing, setIsEditing] = useState(false);
    const [editValue, setEditValue] = useState("");
    const [isValid, setIsValid] = useState(true);
    const inputRef = useRef<HTMLInputElement>(null);

    // Format the display value
    const formattedValue = (() => {
        const prefix = showPositive && value > 0 ? "+" : "";
        return `${prefix}${value.toFixed(decimals)}${suffix}`;
    })();

    // Start editing
    const startEditing = useCallback(() => {
        if (disabled) return;
        setEditValue(value.toFixed(decimals));
        setIsValid(true);
        setIsEditing(true);
    }, [disabled, value, decimals]);

    // Focus input when editing starts
    useEffect(() => {
        if (isEditing && inputRef.current) {
            inputRef.current.focus();
            inputRef.current.select();
        }
    }, [isEditing]);

    // Validate and parse input
    const validateInput = useCallback((input: string): { valid: boolean; value: number } => {
        // Allow empty, negative sign, or decimal point during typing
        if (input === "" || input === "-" || input === ".") {
            return { valid: true, value: 0 };
        }

        const num = parseFloat(input);
        if (isNaN(num)) {
            return { valid: false, value: 0 };
        }

        if (num < min || num > max) {
            return { valid: false, value: num };
        }

        return { valid: true, value: num };
    }, [min, max]);

    // Handle input change
    const handleInputChange = useCallback((e: React.ChangeEvent<HTMLInputElement>) => {
        const input = e.target.value;
        // Allow typing numbers, negative sign, and decimal point
        if (/^-?\d*\.?\d*$/.test(input)) {
            setEditValue(input);
            const { valid } = validateInput(input);
            setIsValid(valid);
        }
    }, [validateInput]);

    // Commit the value
    const commitValue = useCallback(() => {
        const { valid, value: newValue } = validateInput(editValue);

        if (valid && editValue !== "" && editValue !== "-" && editValue !== ".") {
            // Clamp to range
            const clampedValue = Math.max(min, Math.min(max, newValue));
            onChange(clampedValue);
        }

        setIsEditing(false);
    }, [editValue, validateInput, min, max, onChange]);

    // Cancel editing
    const cancelEditing = useCallback(() => {
        setIsEditing(false);
    }, []);

    // Handle keyboard
    const handleKeyDown = useCallback((e: KeyboardEvent<HTMLInputElement>) => {
        if (e.key === "Enter") {
            e.preventDefault();
            commitValue();
        } else if (e.key === "Escape") {
            e.preventDefault();
            cancelEditing();
        }
    }, [commitValue, cancelEditing]);

    // Handle blur
    const handleBlur = useCallback((_e: FocusEvent<HTMLInputElement>) => {
        // Small delay to allow click events to fire first
        setTimeout(() => {
            commitValue();
        }, 100);
    }, [commitValue]);

    if (isEditing) {
        return (
            <input
                ref={inputRef}
                type="text"
                inputMode="decimal"
                value={editValue}
                onChange={handleInputChange}
                onKeyDown={handleKeyDown}
                onBlur={handleBlur}
                className={cn(
                    "text-center font-mono text-xs bg-transparent outline-none",
                    "border-b transition-colors",
                    isValid
                        ? "border-gecko-accent text-gecko-text-primary"
                        : "border-gecko-danger text-gecko-danger",
                    inputWidth,
                    className
                )}
                aria-label="Edit value"
                aria-invalid={!isValid}
            />
        );
    }

    return (
        <button
            type="button"
            onClick={startEditing}
            disabled={disabled}
            className={cn(
                "font-mono text-xs cursor-pointer transition-colors",
                "hover:text-gecko-accent hover:underline",
                "focus:outline-none focus:text-gecko-accent focus:underline",
                disabled && "cursor-default hover:text-inherit hover:no-underline",
                className
            )}
            title={disabled ? undefined : "Click to edit"}
            aria-label={`${formattedValue}, click to edit`}
        >
            {formattedValue}
        </button>
    );
}
