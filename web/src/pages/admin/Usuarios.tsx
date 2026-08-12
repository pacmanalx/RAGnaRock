import { Panel } from '@/components/ui'
import { UserPlus } from 'lucide-react'

// Cadastro de usuários — estrutura da modelagem. Hoje o ragd tem só admin_user/admin_pass
// único no cfg; a UI prevê CRUD de usuários (precisa de backend novo + JWT). Dado ilustrativo.
const MOCK = [
  { login: 'admin', nome: 'Administrador', perfil: 'admin', ativo: true },
  { login: 'operador', nome: 'Operador RAG', perfil: 'operador', ativo: true },
  { login: 'leitor', nome: 'Convidado', perfil: 'leitor', ativo: false },
]

export function Usuarios() {
  return (
    <div className="mx-auto max-w-4xl space-y-5">
      <div className="flex items-center justify-between">
        <h1 className="text-lg font-semibold">Usuários</h1>
        <button className="flex items-center gap-1.5 rounded-md bg-[var(--color-accent)] px-3 py-1.5 text-[13px] font-semibold text-[var(--color-accent-fg)] hover:opacity-90">
          <UserPlus size={15} /> Novo usuário
        </button>
      </div>
      <Panel>
        <table className="w-full text-[13px]">
          <thead>
            <tr className="border-b border-[var(--color-border)] text-left text-[11px] uppercase tracking-wider text-[var(--color-muted)]">
              <th className="pb-2 font-medium">Login</th>
              <th className="pb-2 font-medium">Nome</th>
              <th className="pb-2 font-medium">Perfil</th>
              <th className="pb-2 font-medium">Ativo</th>
            </tr>
          </thead>
          <tbody>
            {MOCK.map((u) => (
              <tr key={u.login} className="border-b border-[var(--color-border)]/50">
                <td className="py-2.5 font-medium">{u.login}</td>
                <td className="py-2.5">{u.nome}</td>
                <td className="py-2.5"><span className="rounded bg-[var(--color-panel-2)] px-1.5 py-0.5 text-[11px] text-[var(--color-accent)]">{u.perfil}</span></td>
                <td className="py-2.5">{u.ativo ? 'sim' : 'não'}</td>
              </tr>
            ))}
          </tbody>
        </table>
        <div className="mt-3 text-[11px] text-[var(--color-muted)]">estrutura da modelagem — CRUD real depende de backend de usuários + JWT.</div>
      </Panel>
    </div>
  )
}
