import { useState } from 'react'
import { Plus, Pencil, Trash2 } from 'lucide-react'
import { Panel, Spinner, ErrorBox } from '@/components/ui'
import { useAsync } from '@/hooks/useAsync'
import { listPerfis, getCaps, upsertPerfil, deletePerfil, type Perfil } from '@/api/auth'
import { messageFromError } from '@/api/client'

// CRUD de perfis (papéis do RBAC). Perfil = capacidades (verbos) + escopo de coleções;
// o JWT carrega as caps resolvidas — edição vale no próximo login/refresh.
const VAZIO: Perfil = { nome: '', desc: '', caps: [], colls: ['*'] }

export function Perfis() {
  const perfis = useAsync(listPerfis, [])
  const caps = useAsync(getCaps, [])
  const [edit, setEdit] = useState<Perfil | null>(null)
  const [isNew, setIsNew] = useState(false)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)

  async function salvar() {
    if (!edit || busy) return
    setBusy(true); setError(null)
    try { await upsertPerfil(edit); setEdit(null); perfis.reload() }
    catch (e) { setError(messageFromError(e)) }
    finally { setBusy(false) }
  }

  async function remover(nome: string) {
    setError(null)
    try { await deletePerfil(nome); perfis.reload() }
    catch (e) { setError(messageFromError(e)) }
  }

  function toggleCap(c: string) {
    if (!edit) return
    const has = edit.caps.includes(c)
    // "*" é exclusivo: marcou, desmarca o resto; marcou outra, tira o "*"
    const caps = c === '*'
      ? (has ? [] : ['*'])
      : (has ? edit.caps.filter((x) => x !== c) : [...edit.caps.filter((x) => x !== '*'), c])
    setEdit({ ...edit, caps })
  }

  const inputCls = 'w-full rounded-md border border-[var(--color-border)] bg-[var(--color-panel-2)] px-3 py-2 text-[13px] outline-none focus:border-[var(--color-accent)]'
  const catalogo = ['*', ...(caps.data?.caps ?? [])]

  return (
    <div className="space-y-5">
      <div className="flex items-center justify-between">
        <h1 className="text-lg font-semibold">Perfis</h1>
        <button
          onClick={() => { setEdit({ ...VAZIO }); setIsNew(true) }}
          className="flex items-center gap-1.5 rounded-md bg-[var(--color-accent)] px-3 py-1.5 text-[13px] font-semibold text-[var(--color-accent-fg)] hover:opacity-90"
        >
          <Plus size={15} /> Novo perfil
        </button>
      </div>

      {error && <ErrorBox message={error} />}
      {perfis.error && <ErrorBox message={perfis.error} onRetry={perfis.reload} />}
      {perfis.loading && <Spinner label="carregando perfis…" />}

      {/* form de edição/criação */}
      {edit && (
        <Panel title={isNew ? 'Novo perfil' : `Editar: ${edit.nome}`}>
          <div className="space-y-3">
            <div className="flex flex-wrap gap-3">
              <div className="w-[180px]">
                <div className="mb-1 text-[11px] uppercase tracking-wide text-[var(--color-muted)]">nome</div>
                <input value={edit.nome} disabled={!isNew} onChange={(e) => setEdit({ ...edit, nome: e.target.value })} className={`${inputCls} disabled:opacity-60`} />
              </div>
              <div className="grow">
                <div className="mb-1 text-[11px] uppercase tracking-wide text-[var(--color-muted)]">descrição</div>
                <input value={edit.desc} onChange={(e) => setEdit({ ...edit, desc: e.target.value })} className={inputCls} />
              </div>
              <div className="w-[200px]" title='lista de coleções separadas por vírgula; "*" = todas'>
                <div className="mb-1 text-[11px] uppercase tracking-wide text-[var(--color-muted)]">coleções (escopo)</div>
                <input
                  value={edit.colls.join(', ')}
                  onChange={(e) => setEdit({ ...edit, colls: e.target.value.split(',').map((s) => s.trim()).filter(Boolean) })}
                  className={inputCls}
                />
              </div>
            </div>
            <div>
              <div className="mb-1.5 text-[11px] uppercase tracking-wide text-[var(--color-muted)]">capacidades</div>
              <div className="flex flex-wrap gap-2">
                {catalogo.map((c) => (
                  <button
                    key={c}
                    type="button"
                    onClick={() => toggleCap(c)}
                    className={`rounded-full border px-3 py-1 text-[12px] transition-colors ${
                      edit.caps.includes(c)
                        ? 'border-[var(--color-accent)] bg-[var(--color-accent)] text-[var(--color-accent-fg)]'
                        : 'border-[var(--color-border)] text-[var(--color-muted)] hover:text-[var(--color-fg)]'
                    }`}
                  >
                    {c === '*' ? '* (todas)' : c}
                  </button>
                ))}
              </div>
            </div>
            <div className="flex gap-2">
              <button onClick={salvar} disabled={busy || !edit.nome.trim() || edit.caps.length === 0}
                className="rounded-md bg-[var(--color-accent)] px-4 py-2 text-[13px] font-semibold text-[var(--color-accent-fg)] hover:opacity-90 disabled:opacity-50">
                {busy ? 'salvando…' : 'salvar'}
              </button>
              <button onClick={() => setEdit(null)} className="rounded-md border border-[var(--color-border)] px-4 py-2 text-[13px] hover:bg-[var(--color-panel-2)]">cancelar</button>
            </div>
          </div>
        </Panel>
      )}

      <div className="grid gap-3 sm:grid-cols-2 xl:grid-cols-3">
        {(perfis.data?.perfis ?? []).map((p) => (
          <Panel key={p.nome} title={p.nome} actions={
            <span className="flex gap-1">
              <button onClick={() => { setEdit({ ...p }); setIsNew(false) }} title="editar" className="rounded p-1 text-[var(--color-muted)] hover:text-[var(--color-fg)]"><Pencil size={14} /></button>
              <button onClick={() => remover(p.nome)} title="remover (recusa se em uso)" className="rounded p-1 text-[var(--color-muted)] hover:text-[var(--color-crit)]"><Trash2 size={14} /></button>
            </span>
          }>
            <div className="text-[12px] text-[var(--color-muted)]">{p.desc || '—'}</div>
            <div className="mt-3 flex flex-wrap gap-1.5">
              {p.caps.map((c) => (
                <span key={c} className="rounded bg-[var(--color-panel-2)] px-2 py-0.5 text-[11px]">{c}</span>
              ))}
            </div>
            <div className="mt-2 text-[11px] text-[var(--color-muted)]">coleções: {p.colls.join(', ')}</div>
          </Panel>
        ))}
      </div>
    </div>
  )
}
