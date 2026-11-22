import type { Config } from "tailwindcss";

const config: Config = {
  darkMode: ["class"],
  content: ["./index.html", "./src/**/*.{ts,tsx}"],
  theme: {
    extend: {
      fontFamily: {
        sans: [
          '"Geist Variable"',
          '"Inter Variable"',
          '"Segoe UI Variable"',
          "system-ui",
          "sans-serif",
        ],
        mono: ['"Cascadia Code"', "ui-monospace", "SFMono-Regular", "monospace"],
      },
      borderRadius: {
        "2xl": "16px",
        xl: "12px",
        lg: "10px",
        md: "8px",
        sm: "6px",
      },
      colors: {
        surface: {
          base: "rgb(var(--cf-surface-base) / <alpha-value>)",
          elevated: "rgb(var(--cf-surface-elevated) / <alpha-value>)",
          subtle: "rgb(var(--cf-surface-subtle) / <alpha-value>)",
        },
        border: {
          DEFAULT: "rgb(var(--cf-border) / <alpha-value>)",
          strong: "rgb(var(--cf-border-strong) / <alpha-value>)",
        },
        fg: {
          DEFAULT: "rgb(var(--cf-fg) / <alpha-value>)",
          muted: "rgb(var(--cf-fg-muted) / <alpha-value>)",
          subtle: "rgb(var(--cf-fg-subtle) / <alpha-value>)",
        },
        accent: {
          violet: "rgb(var(--cf-violet) / <alpha-value>)",
          cyan: "rgb(var(--cf-cyan) / <alpha-value>)",
        },
        status: {
          success: "rgb(var(--cf-success) / <alpha-value>)",
          warning: "rgb(var(--cf-warning) / <alpha-value>)",
          danger: "rgb(var(--cf-danger) / <alpha-value>)",
        },
      },
      boxShadow: {
        glass:
          "0 12px 32px -12px rgba(0, 0, 0, 0.45), inset 0 1px 0 0 rgba(255, 255, 255, 0.06)",
        "glass-light":
          "0 12px 28px -14px rgba(15, 23, 42, 0.16), inset 0 1px 0 0 rgba(255, 255, 255, 0.7)",
      },
    },
  },
  plugins: [],
};

export default config;
