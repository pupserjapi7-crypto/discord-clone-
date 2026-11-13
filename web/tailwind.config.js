/** @type {import('tailwindcss').Config} */
export default {
  content: ['./src/**/*.{html,js,svelte,ts}'],
  theme: {
    extend: {
      colors: {
        bg: 'var(--nx-bg)',
        surface: 'var(--nx-surface)',
        surface2: 'var(--nx-surface-2)',
        line: 'var(--nx-line)',
        ink: 'var(--nx-text)',
        ink2: 'var(--nx-text-2)',
        ink3: 'var(--nx-text-3)',
        accent: 'var(--nx-accent)',
        accenta: 'var(--nx-accent-text)'
      },
      borderRadius: { panel: '18px', card: '12px' },
      fontFamily: { sans: ['Inter', 'system-ui', 'sans-serif'] }
    }
  },
  plugins: []
};
