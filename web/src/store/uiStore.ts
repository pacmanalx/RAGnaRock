import { create } from 'zustand'
import { persist } from 'zustand/middleware'

// Estado de UI (molde Innova: Zustand + persist, sem Context global). Guarda a preferência
// de sidebar colapsada. authStore virá aqui do lado quando JWT entrar.
interface UiState {
  sidebarCollapsed: boolean
  toggleSidebar: () => void
}

export const useUiStore = create<UiState>()(
  persist(
    (set) => ({
      sidebarCollapsed: false,
      toggleSidebar: () => set((s) => ({ sidebarCollapsed: !s.sidebarCollapsed })),
    }),
    { name: 'valhalla-ui' },
  ),
)
