import { type ClassValue, clsx } from "clsx";
import { twMerge } from "tailwind-merge";

/**
 * Merge Tailwind CSS classes with proper precedence handling.
 * Combines clsx for conditional classes with tailwind-merge for deduplication.
 */
export function cn(...inputs: ClassValue[]) {
  return twMerge(clsx(inputs));
}

/**
 * Format frequency for display (e.g., 1000 -> "1k", 16000 -> "16k")
 */
export function formatFrequency(freq: number): string {
  if (freq >= 1000) {
    return `${(freq / 1000).toFixed(freq >= 10000 ? 0 : 0)}k`;
  }
  return freq.toFixed(0);
}

/**
 * Convert linear amplitude (0-1) to display percentage with perceptual scaling
 */
export function amplitudeToPercent(value: number): number {
  // Apply square root for better visual response (perceptual scaling)
  return Math.min(100, Math.pow(value, 0.5) * 100);
}

/**
 * Debounce function for rate-limiting UI updates
 */
export function debounce<T extends (...args: unknown[]) => unknown>(
  fn: T,
  delay: number
): (...args: Parameters<T>) => void {
  let timeoutId: ReturnType<typeof setTimeout>;
  return (...args: Parameters<T>) => {
    clearTimeout(timeoutId);
    timeoutId = setTimeout(() => fn(...args), delay);
  };
}
