/** @type {import('tailwindcss').Config} */
export default {
  content: ["./index.html", "./src/**/*.{js,ts,jsx,tsx}"],
  theme: {
    extend: {
      colors: {
        // Gecko brand colors - uses CSS variables for theming
        // See src/styles.css for theme definitions
        gecko: {
          bg: {
            primary: "var(--gecko-bg-primary)",
            secondary: "var(--gecko-bg-secondary)",
            tertiary: "var(--gecko-bg-tertiary)",
            elevated: "var(--gecko-bg-elevated)",
          },
          text: {
            primary: "var(--gecko-text-primary)",
            secondary: "var(--gecko-text-secondary)",
            muted: "var(--gecko-text-muted)",
          },
          accent: {
            DEFAULT: "var(--gecko-accent)",
            hover: "var(--gecko-accent-hover)",
            muted: "var(--gecko-accent-muted)",
          },
          danger: {
            DEFAULT: "var(--gecko-danger)",
            hover: "var(--gecko-danger-hover)",
          },
          warning: {
            DEFAULT: "var(--gecko-warning)",
            hover: "var(--gecko-warning-hover)",
          },
          border: {
            DEFAULT: "var(--gecko-border)",
            hover: "var(--gecko-border-hover)",
          },
        },
      },
      fontFamily: {
        sans: [
          "-apple-system",
          "BlinkMacSystemFont",
          "Segoe UI",
          "Roboto",
          "sans-serif",
        ],
        mono: ["SF Mono", "Fira Code", "Consolas", "monospace"],
      },
      fontSize: {
        "2xs": ["0.625rem", { lineHeight: "0.75rem" }],
      },
      spacing: {
        18: "4.5rem",
      },
      borderRadius: {
        DEFAULT: "6px",
      },
      animation: {
        "meter-pulse": "meter-pulse 0.1s ease-out",
      },
      keyframes: {
        "meter-pulse": {
          "0%": { opacity: "1" },
          "100%": { opacity: "0.8" },
        },
      },
    },
  },
  plugins: [],
};
