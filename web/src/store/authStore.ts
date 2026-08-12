import { create } from 'zustand'
import { persist } from 'zustand/middleware'
import { setAuthToken } from '@/api/client'
import { apiLogin, apiRefresh } from '@/api/auth'

// authStore — JWT real. Access curto (15min) renovado pelo refresh (session_ttl do
// ragd, 12h default); as caps vêm RESOLVIDAS no token (guard não consulta o server).
export interface Usuario {
  login: string
  nome: string
  perfil: string
  caps: string[]
  colls: string[]
}

interface AuthState {
  usuario: Usuario | null
  access: string | null
  refresh: string | null
  isAuthenticated: boolean
  login: (login: string, password: string) => Promise<void>
  renovar: () => Promise<boolean>
  logout: () => void
  hasCap: (cap: string) => boolean
}

export const useAuthStore = create<AuthState>()(
  persist(
    (set, get) => ({
      usuario: null,
      access: null,
      refresh: null,
      isAuthenticated: false,
      login: async (login, password) => {
        const r = await apiLogin(login, password) // deixa o HttpError subir pro form
        setAuthToken(r.access)
        set({ usuario: r.usuario, access: r.access, refresh: r.refresh, isAuthenticated: true })
      },
      renovar: async () => {
        const rt = get().refresh
        if (!rt) return false
        try {
          const r = await apiRefresh(rt)
          setAuthToken(r.access)
          set({ access: r.access })
          return true
        } catch {
          setAuthToken(null)
          set({ usuario: null, access: null, refresh: null, isAuthenticated: false })
          return false
        }
      },
      logout: () => {
        setAuthToken(null)
        set({ usuario: null, access: null, refresh: null, isAuthenticated: false })
      },
      hasCap: (cap) => {
        const caps = get().usuario?.caps ?? []
        return caps.includes('*') || caps.includes(cap)
      },
    }),
    {
      name: 'valhalla-auth',
      onRehydrateStorage: () => (state) => {
        // re-injeta o token no client após reload; access curto → renova em background
        if (state?.access) { setAuthToken(state.access); void state.renovar() }
      },
    },
  ),
)
