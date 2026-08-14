import { create } from 'zustand'
import { persist } from 'zustand/middleware'

export type Theme = 'dark' | 'light'

// Aplica o tema no <html data-theme=...> — os tokens --color-* do index.css trocam sozinhos.
function apply(theme: Theme) {
  document.documentElement.dataset.theme = theme
}

interface ThemeState {
  theme: Theme
  setTheme: (t: Theme) => void
  toggle: () => void
}

export const useThemeStore = create<ThemeState>()(
  persist(
    (set, get) => ({
      theme: 'dark', // default cockpit escuro
      setTheme: (theme) => { apply(theme); set({ theme }) },
      toggle: () => { const t = get().theme === 'dark' ? 'light' : 'dark'; apply(t); set({ theme: t }) },
    }),
    {
      name: 'valhalla-theme',
      onRehydrateStorage: () => (state) => { if (state) apply(state.theme) },
    },
  ),
)

// Garante o data-theme no boot (antes/independente da rehidratação assíncrona).
apply(useThemeStore.getState().theme)
