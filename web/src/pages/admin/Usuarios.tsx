import { useState } from 'react'
import { UserPlus, Pencil, Trash2 } from 'lucide-react'
import { Panel, Spinner, ErrorBox } from '@/components/ui'
import { useAsync } from '@/hooks/useAsync'
import { listUsuarios, listPerfis, upsertUsuario, deleteUsuario, type UsuarioApi } from '@/api/auth'
import { messageFromError } from '@/api/client'

// CRUD de usuários. Senha obrigatória na criação; em branco no update = mantém a atual.
// O backend trava: último admin ativo não desativa/rebaixa/remove.
type Form = UsuarioApi & { password: string }
const VAZIO: Form = { login: '', nome: '', perfil: 'leitor', ativo: true, password: '' }

export function Usuarios() {
  const usuarios = useAsync(listUsuarios, [])
  const perfis = useAsync(listPerfis, [])
  const [edit, setEdit] = useState<Form | null>(null)
  const [isNew, setIsNew] = useState(false)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)

  async function salvar() {
    if (!edit || busy) return
    setBusy(true); setError(null)
    try {
      await upsertUsuario({ ...edit, password: edit.password || undefined })
      setEdit(null); usuarios.reload()
    } catch (e) { setError(messageFromError(e)) }
    finally { setBusy(false) }
  }

  async function remover(login: string) {
    setError(null)
    try { await deleteUsuario(login); usuarios.reload() }
    catch (e) { setError(messageFromError(e)) }
  }

  const inputCls = 'w-full rounded-md border border-[var(--color-border)] bg-[var(--color-panel-2)] px-3 py-2 text-[13px] outline-none focus:border-[var(--color-accent)]'

  return (
    <div className="space-y-5">
      <div className="flex items-center justify-between">
        <h1 className="text-lg font-semibold">Usuários</h1>
        <button
          onClick={() => { setEdit({ ...VAZIO }); setIsNew(true) }}
          className="flex items-center gap-1.5 rounded-md bg-[var(--color-accent)] px-3 py-1.5 text-[13px] font-semibold text-[var(--color-accent-fg)] hover:opacity-90"
        >
          <UserPlus size={15} /> Novo usuário
        </button>
      </div>

      {error && <ErrorBox message={error} />}
      {usuarios.error && <ErrorBox message={usuarios.error} onRetry={usuarios.reload} />}
      {usuarios.loading && <Spinner label="carregando usuários…" />}

      {edit && (
        <Panel title={isNew ? 'Novo usuário' : `Editar: ${edit.login}`}>
          <div className="space-y-3">
            <div className="flex flex-wrap gap-3">
              <div className="w-[160px]">
                <div className="mb-1 text-[11px] uppercase tracking-wide text-[var(--color-muted)]">login</div>
                <input value={edit.login} disabled={!isNew} onChange={(e) => setEdit({ ...edit, login: e.target.value })} className={`${inputCls} disabled:opacity-60`} autoComplete="off" />
              </div>
              <div className="grow">
                <div className="mb-1 text-[11px] uppercase tracking-wide text-[var(--color-muted)]">nome</div>
                <input value={edit.nome} onChange={(e) => setEdit({ ...edit, nome: e.target.value })} className={inputCls} />
              </div>
              <div className="w-[160px]">
                <div className="mb-1 text-[11px] uppercase tracking-wide text-[var(--color-muted)]">perfil</div>
                <select value={edit.perfil} onChange={(e) => setEdit({ ...edit, perfil: e.target.value })} className={inputCls}>
                  {(perfis.data?.perfis ?? []).map((p) => <option key={p.nome} value={p.nome}>{p.nome}</option>)}
                </select>
              </div>
              <div className="w-[180px]" title={isNew ? 'obrigatória' : 'em branco = mantém a atual'}>
                <div className="mb-1 text-[11px] uppercase tracking-wide text-[var(--color-muted)]">senha {isNew ? '' : '(opcional)'}</div>
                <input type="password" value={edit.password} onChange={(e) => setEdit({ ...edit, password: e.target.value })} className={inputCls} autoComplete="new-password" />
              </div>
              <label className="flex cursor-pointer items-center gap-2 self-end pb-2 text-[13px]">
                <input type="checkbox" checked={edit.ativo} onChange={(e) => setEdit({ ...edit, ativo: e.target.checked })} className="accent-[var(--color-accent)]" />
                ativo
              </label>
            </div>
            <div className="flex gap-2">
              <button onClick={salvar} disabled={busy || !edit.login.trim() || (isNew && !edit.password)}
                className="rounded-md bg-[var(--color-accent)] px-4 py-2 text-[13px] font-semibold text-[var(--color-accent-fg)] hover:opacity-90 disabled:opacity-50">
                {busy ? 'salvando…' : 'salvar'}
              </button>
              <button onClick={() => setEdit(null)} className="rounded-md border border-[var(--color-border)] px-4 py-2 text-[13px] hover:bg-[var(--color-panel-2)]">cancelar</button>
            </div>
          </div>
        </Panel>
      )}

      <Panel>
        <table className="w-full text-[13px]">
          <thead>
            <tr className="border-b border-[var(--color-border)] text-left text-[11px] uppercase tracking-wider text-[var(--color-muted)]">
              <th className="pb-2 font-medium">Login</th>
              <th className="pb-2 font-medium">Nome</th>
              <th className="pb-2 font-medium">Perfil</th>
              <th className="pb-2 font-medium">Ativo</th>
              <th className="pb-2 font-medium"></th>
            </tr>
          </thead>
          <tbody>
            {(usuarios.data?.usuarios ?? []).map((u) => (
              <tr key={u.login} className="border-b border-[var(--color-border)]/50">
                <td className="py-2.5 font-medium">{u.login}</td>
                <td className="py-2.5">{u.nome}</td>
                <td className="py-2.5"><span className="rounded bg-[var(--color-panel-2)] px-1.5 py-0.5 text-[11px] text-[var(--color-accent)]">{u.perfil}</span></td>
                <td className="py-2.5">{u.ativo ? 'sim' : 'não'}</td>
                <td className="py-2.5 text-right">
                  <button onClick={() => { setEdit({ ...u, password: '' }); setIsNew(false) }} title="editar" className="rounded p-1 text-[var(--color-muted)] hover:text-[var(--color-fg)]"><Pencil size={14} /></button>
                  <button onClick={() => remover(u.login)} title="remover" className="rounded p-1 text-[var(--color-muted)] hover:text-[var(--color-crit)]"><Trash2 size={14} /></button>
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      </Panel>
    </div>
  )
}
