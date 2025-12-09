import { forwardRef, type SelectHTMLAttributes } from "react";
import { cn } from "../../lib/utils";

export interface SelectProps extends SelectHTMLAttributes<HTMLSelectElement> {}

export const Select = forwardRef<HTMLSelectElement, SelectProps>(
  ({ className, children, ...props }, ref) => {
    return (
      <select
        ref={ref}
        className={cn(
          "flex h-9 w-full rounded",
          "bg-gecko-bg-tertiary border border-gecko-border",
          "px-3 py-2 text-sm",
          "text-gecko-text-primary",
          // Ensure dropdown options have proper contrast
          "[&>option]:bg-gecko-bg-tertiary [&>option]:text-gecko-text-primary",
          "focus:outline-none focus:ring-2 focus:ring-gecko-accent focus:ring-offset-2 focus:ring-offset-gecko-bg-primary",
          "disabled:cursor-not-allowed disabled:opacity-50",
          className
        )}
        {...props}
      >
        {children}
      </select>
    );
  }
);

Select.displayName = "Select";
