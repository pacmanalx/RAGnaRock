import { create } from 'zustand'
import { persist } from 'zustand/middleware'

// authStore — molde Innova. PLACEHOLDER: o JWT (access/refresh) entra aqui depois. Por ora
// segura só a identidade exibida na topbar; a UI usa os endpoints públicos, sem token.
export interface Usuario {
  login: string
  nome: string
  perfil: string
}

interface AuthState {
  usuario: Usuario | null
  isAuthenticated: boolean
  login: (u: Usuario) => void
  logout: () => void
}

export const useAuthStore = create<AuthState>()(
  persist(
    (set) => ({
      // pré-preenchido só pra modelar a topbar; troca por login real quando o JWT chegar
      usuario: { login: 'admin', nome: 'Administrador', perfil: 'admin' },
      isAuthenticated: true,
      login: (usuario) => set({ usuario, isAuthenticated: true }),
      logout: () => set({ usuario: null, isAuthenticated: false }),
    }),
    { name: 'valhalla-auth' },
  ),
)
