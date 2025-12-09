import { forwardRef, type InputHTMLAttributes } from "react";
import { cva, type VariantProps } from "class-variance-authority";
import { cn } from "../../lib/utils";

const sliderVariants = cva(
  [
    "appearance-none cursor-pointer",
    "bg-gecko-bg-tertiary rounded-full",
    // Webkit (Chrome, Safari, Edge)
    "[&::-webkit-slider-thumb]:appearance-none",
    "[&::-webkit-slider-thumb]:rounded-full",
    "[&::-webkit-slider-thumb]:bg-gecko-accent",
    "[&::-webkit-slider-thumb]:cursor-pointer",
    "[&::-webkit-slider-thumb]:transition-transform",
    "[&::-webkit-slider-thumb]:hover:scale-110",
    // Firefox
    "[&::-moz-range-thumb]:border-none",
    "[&::-moz-range-thumb]:rounded-full",
    "[&::-moz-range-thumb]:bg-gecko-accent",
    "[&::-moz-range-thumb]:cursor-pointer",
    // Disabled state
    "disabled:opacity-50 disabled:cursor-not-allowed",
    "[&:disabled::-webkit-slider-thumb]:cursor-not-allowed",
  ],
  {
    variants: {
      orientation: {
        horizontal: [
          "h-1.5 w-full",
          "[&::-webkit-slider-thumb]:w-4 [&::-webkit-slider-thumb]:h-4",
          "[&::-moz-range-thumb]:w-4 [&::-moz-range-thumb]:h-4",
        ],
        vertical: [
          // For vertical, we use a horizontal slider rotated -90deg
          "h-1.5",
          "[&::-webkit-slider-thumb]:w-4 [&::-webkit-slider-thumb]:h-4",
          "[&::-moz-range-thumb]:w-4 [&::-moz-range-thumb]:h-4",
        ],
      },
    },
    defaultVariants: {
      orientation: "horizontal",
    },
  }
);

export interface SliderProps
  extends Omit<InputHTMLAttributes<HTMLInputElement>, "type">,
    VariantProps<typeof sliderVariants> {}

export const Slider = forwardRef<HTMLInputElement, SliderProps>(
  ({ className, orientation, style, ...props }, ref) => {
    // For vertical sliders, we rotate a horizontal slider
    // This provides much better cross-browser compatibility
    if (orientation === "vertical") {
      return (
        <div
          className={cn("flex items-center justify-center", className)}
          style={{ width: "1.5rem" }}
        >
          <input
            type="range"
            className={cn(sliderVariants({ orientation, className: "w-32" }))}
            style={{
              ...style,
              transform: "rotate(-90deg)",
              transformOrigin: "center center",
            }}
            ref={ref}
            {...props}
          />
        </div>
      );
    }

    return (
      <input
        type="range"
        className={cn(sliderVariants({ orientation, className }))}
        style={style}
        ref={ref}
        {...props}
      />
    );
  }
);

Slider.displayName = "Slider";
