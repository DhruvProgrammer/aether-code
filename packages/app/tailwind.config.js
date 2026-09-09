import containerQueries from '@tailwindcss/container-queries';
import forms from '@tailwindcss/forms';

/** Tailwind v3 config — ported 1:1 from the former Play CDN inline
 *  `tailwind.config` in index.html (v0.29.1: styles are built locally
 *  so the desktop app no longer depends on a runtime CDN). */
export default {
  darkMode: 'class',
  content: ['./index.html', './src/**/*.{ts,html}'],
  theme: {
    extend: {
      colors: {
        app: {
          bg: '#0f1115',
          topbar: '#161922',
          surface: '#161922',
          hover: '#1d2230',
          border: '#2a2f3d',
          textPrimary: '#e8e8e8',
          textSecondary: '#8c9296',
          brand: '#7ca6b4',
          warning: '#e8a94c',
          error: '#bc484a',
          inputBg: '#161922',
        },
      },
      fontFamily: {
        sans: ['Inter', 'ui-sans-serif', 'system-ui', '-apple-system', 'BlinkMacSystemFont', '"Segoe UI"', 'Roboto', '"Helvetica Neue"', 'Arial', 'sans-serif'],
        mono: ['"JetBrains Mono"', 'ui-monospace', 'SFMono-Regular', 'Menlo', 'Monaco', 'Consolas', '"Liberation Mono"', '"Courier New"', 'monospace'],
      },
    },
  },
  plugins: [forms, containerQueries],
};
