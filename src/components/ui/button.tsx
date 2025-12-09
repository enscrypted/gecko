import { forwardRef, type ButtonHTMLAttributes } from "react";
import { cva, type VariantProps } from "class-variance-authority";
import { cn } from "../../lib/utils";

const buttonVariants = cva(
  // Base styles shared by all variants
  [
    "inline-flex items-center justify-center",
    "rounded font-medium text-sm",
    "transition-colors duration-150",
    "focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-gecko-accent focus-visible:ring-offset-2 focus-visible:ring-offset-gecko-bg-primary",
    "disabled:pointer-events-none disabled:opacity-50",
  ],
  {
    variants: {
      variant: {
        default: [
          "bg-gecko-bg-tertiary border border-gecko-border",
          "text-gecko-text-primary",
          "hover:bg-gecko-border-hover hover:border-gecko-border-hover",
        ],
        primary: [
          "bg-gecko-accent border border-gecko-accent",
          "text-gecko-bg-primary font-semibold",
          "hover:bg-gecko-accent-hover hover:border-gecko-accent-hover",
        ],
        danger: [
          "bg-gecko-danger border border-gecko-danger",
          "text-white",
          "hover:bg-gecko-danger-hover hover:border-gecko-danger-hover",
        ],
        warning: [
          "bg-gecko-warning border border-gecko-warning",
          "text-gecko-bg-primary",
          "hover:bg-gecko-warning-hover hover:border-gecko-warning-hover",
        ],
        ghost: [
          "bg-transparent border border-transparent",
          "text-gecko-text-secondary",
          "hover:bg-gecko-bg-tertiary hover:text-gecko-text-primary",
        ],
      },
      size: {
        sm: "h-8 px-3 text-xs",
        md: "h-9 px-4",
        lg: "h-10 px-6",
        icon: "h-9 w-9",
      },
    },
    defaultVariants: {
      variant: "default",
      size: "md",
    },
  }
);

export interface ButtonProps
  extends ButtonHTMLAttributes<HTMLButtonElement>,
    VariantProps<typeof buttonVariants> {}

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

Button.displayName = "Button";
