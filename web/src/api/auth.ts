// API de autenticação/RBAC do ragd — /login, /refresh e CRUD de perfis/usuários. [#33 JWT]
import { ragd } from './client'

export interface Perfil {
  nome: string
  desc: string
  caps: string[]
  colls: string[]
}
export interface UsuarioApi {
  login: string
  nome: string
  perfil: string
  ativo: boolean
}
export interface LoginResponse {
  access: string
  refresh: string
  expires_in: number
  usuario: { login: string; nome: string; perfil: string; caps: string[]; colls: string[] }
}

export const apiLogin = (login: string, password: string) =>
  ragd.post<LoginResponse>('/login', { login, password })
export const apiRefresh = (refresh: string) =>
  ragd.post<{ access: string; expires_in: number }>('/refresh', { refresh })

export const getCaps = () => ragd.get<{ caps: string[] }>('/auth/caps')
export const listPerfis = () => ragd.get<{ perfis: Perfil[] }>('/auth/perfis')
export const upsertPerfil = (p: Perfil) => ragd.post<{ ok: boolean; perfil: Perfil }>('/auth/perfis', { ...p })
export const deletePerfil = (nome: string) => ragd.del<{ ok: boolean }>(`/auth/perfis/${encodeURIComponent(nome)}`)

export const listUsuarios = () => ragd.get<{ usuarios: UsuarioApi[] }>('/auth/usuarios')
export const upsertUsuario = (u: UsuarioApi & { password?: string }) =>
  ragd.post<{ ok: boolean; usuario: UsuarioApi }>('/auth/usuarios', { ...u })
export const deleteUsuario = (login: string) => ragd.del<{ ok: boolean }>(`/auth/usuarios/${encodeURIComponent(login)}`)
