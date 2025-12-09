import { type HTMLAttributes } from "react";
import { cva, type VariantProps } from "class-variance-authority";
import { cn } from "../../lib/utils";

const badgeVariants = cva(
  [
    "inline-flex items-center",
    "rounded px-2 py-0.5",
    "text-2xs font-medium uppercase tracking-wide",
  ],
  {
    variants: {
      variant: {
        default: "bg-gecko-bg-tertiary text-gecko-text-secondary",
        success: "bg-gecko-accent-muted text-gecko-accent",
        warning: "bg-amber-900/30 text-gecko-warning",
        danger: "bg-red-900/30 text-gecko-danger",
      },
    },
    defaultVariants: {
      variant: "default",
    },
  }
);

export interface BadgeProps
  extends HTMLAttributes<HTMLSpanElement>,
    VariantProps<typeof badgeVariants> {}

export function Badge({ className, variant, ...props }: BadgeProps) {
  return (
    <span className={cn(badgeVariants({ variant, className }))} {...props} />
  );
}
