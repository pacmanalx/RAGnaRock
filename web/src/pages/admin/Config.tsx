import { useState } from 'react'
import { FlaskConical, Save, Trash2 } from 'lucide-react'
import { useAsync } from '@/hooks/useAsync'
import { getConfig, setConfig, testProvider } from '@/api/ragnarock'
import { messageFromError } from '@/api/client'
import { Panel, Spinner, ErrorBox } from '@/components/ui'

// Configuração do ragd — GET/POST /config (guard admin.config no backend, chaves
// mascaradas no GET). Cada mudança persiste no ragnarock.cfg via set_cfg_key.

const PROVIDERS = [
  { id: 'none', label: 'nenhum', desc: 'busca expandida sem IA (só dicionários + cache)' },
  { id: 'local', label: 'local 🏠', desc: 'llama-server OpenAI-compat (sem chave, sem nuvem — corpus sensível fica em casa)' },
  { id: 'anthropic', label: 'Anthropic', desc: 'Claude (exige chave cadastrada)' },
  { id: 'openai', label: 'OpenAI', desc: 'GPT (exige chave cadastrada)' },
]

export function Config() {
  const cfg = useAsync(getConfig, [])
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [notes, setNotes] = useState<string[]>([])
  const [anthropicKey, setAnthropicKey] = useState('')
  const [openaiKey, setOpenaiKey] = useState('')
  const [localUrl, setLocalUrl] = useState<string | null>(null)
  const [ttl, setTtl] = useState<number | null>(null)
  const [testMsg, setTestMsg] = useState<Record<string, string>>({})

  async function aplicar(patch: Record<string, unknown>, limpar?: () => void) {
    if (busy) return
    setBusy(true); setError(null); setNotes([])
    try {
      const r = await setConfig(patch)
      setNotes(r.notes)
      limpar?.()
      cfg.reload()
    } catch (e) { setError(messageFromError(e)) }
    finally { setBusy(false) }
  }

  async function testar(provider: string) {
    setTestMsg((m) => ({ ...m, [provider]: 'testando…' }))
    try {
      const r = await testProvider(provider)
      setTestMsg((m) => ({ ...m, [provider]: r.message }))
    } catch (e) { setTestMsg((m) => ({ ...m, [provider]: messageFromError(e) })) }
  }

  const c = cfg.data
  const inputCls = 'rounded-md border border-[var(--color-border)] bg-[var(--color-panel-2)] px-3 py-2 text-[13px] outline-none focus:border-[var(--color-accent)]'
  const btnCls = 'flex items-center gap-1.5 rounded-md border border-[var(--color-accent)] px-3 py-2 text-[12px] font-semibold text-[var(--color-accent)] hover:bg-[var(--color-accent)] hover:text-[var(--color-accent-fg)] disabled:opacity-50'

  const info = (label: string, v: React.ReactNode) => (
    <div className="flex items-baseline justify-between gap-4 border-b border-[var(--color-border)]/40 py-1.5 text-[13px]">
      <span className="text-[var(--color-muted)]">{label}</span>
      <span className="text-right font-mono text-[12px]">{v}</span>
    </div>
  )

  return (
    <div className="space-y-5">
      <h1 className="text-lg font-semibold">Configuração</h1>

      {cfg.loading && <Spinner label="carregando configuração…" />}
      {cfg.error && <ErrorBox message={cfg.error} onRetry={cfg.reload} />}
      {error && <ErrorBox message={error} />}
      {notes.length > 0 && (
        <div className="rounded-md border border-[var(--color-ok)]/40 bg-[var(--color-ok)]/10 px-4 py-2.5 text-[13px]">
          {notes.map((n, i) => <div key={i}>✓ {n}</div>)}
        </div>
      )}

      {c && (
        <>
          {/* ───── provider de IA (busca expandida 🧠) ───── */}
          <Panel title="Provider de IA — expansão semântica 🧠">
            <div className="space-y-4">
              <div className="grid gap-2 sm:grid-cols-2 xl:grid-cols-4">
                {PROVIDERS.map((p) => (
                  <button
                    key={p.id}
                    onClick={() => aplicar({ active_provider: p.id })}
                    disabled={busy}
                    title={p.desc}
                    className={`rounded-md border p-3 text-left transition-colors disabled:opacity-50 ${
                      c.active_provider === p.id
                        ? 'border-[var(--color-accent)] bg-[var(--color-accent)]/10'
                        : 'border-[var(--color-border)] hover:border-[var(--color-muted)]'
                    }`}
                  >
                    <div className="text-[13px] font-semibold">{p.label}{c.active_provider === p.id ? ' · ativo' : ''}</div>
                    <div className="mt-1 text-[11px] leading-snug text-[var(--color-muted)]">{p.desc}</div>
                  </button>
                ))}
              </div>

              {/* chave anthropic */}
              <div className="flex flex-wrap items-end gap-2">
                <div className="grow">
                  <div className="mb-1 text-[11px] uppercase tracking-wide text-[var(--color-muted)]">
                    chave Anthropic {c.anthropic_key_set ? <span className="text-[var(--color-ok)]">· cadastrada ({c.anthropic_key_masked})</span> : '· não cadastrada'}
                  </div>
                  <input type="password" value={anthropicKey} onChange={(e) => setAnthropicKey(e.target.value)}
                    placeholder="cole a nova chave pra cadastrar/trocar" autoComplete="off" className={`w-full ${inputCls}`} />
                </div>
                <button onClick={() => aplicar({ anthropic_key: anthropicKey }, () => setAnthropicKey(''))} disabled={busy || !anthropicKey.trim()} className={btnCls}><Save size={13} /> salvar</button>
                <button onClick={() => testar('anthropic')} disabled={busy || !c.anthropic_key_set} className={btnCls}><FlaskConical size={13} /> testar</button>
                <button onClick={() => aplicar({ clear_anthropic: true })} disabled={busy || !c.anthropic_key_set}
                  className="flex items-center gap-1.5 rounded-md border border-[var(--color-border)] px-3 py-2 text-[12px] text-[var(--color-muted)] hover:border-[var(--color-crit)] hover:text-[var(--color-crit)] disabled:opacity-50"><Trash2 size={13} /> limpar</button>
                {testMsg['anthropic'] && <span className="pb-2 text-[12px] text-[var(--color-muted)]">{testMsg['anthropic']}</span>}
              </div>

              {/* chave openai */}
              <div className="flex flex-wrap items-end gap-2">
                <div className="grow">
                  <div className="mb-1 text-[11px] uppercase tracking-wide text-[var(--color-muted)]">
                    chave OpenAI {c.openai_key_set ? <span className="text-[var(--color-ok)]">· cadastrada ({c.openai_key_masked})</span> : '· não cadastrada'}
                  </div>
                  <input type="password" value={openaiKey} onChange={(e) => setOpenaiKey(e.target.value)}
                    placeholder="cole a nova chave pra cadastrar/trocar" autoComplete="off" className={`w-full ${inputCls}`} />
                </div>
                <button onClick={() => aplicar({ openai_key: openaiKey }, () => setOpenaiKey(''))} disabled={busy || !openaiKey.trim()} className={btnCls}><Save size={13} /> salvar</button>
                <button onClick={() => testar('openai')} disabled={busy || !c.openai_key_set} className={btnCls}><FlaskConical size={13} /> testar</button>
                <button onClick={() => aplicar({ clear_openai: true })} disabled={busy || !c.openai_key_set}
                  className="flex items-center gap-1.5 rounded-md border border-[var(--color-border)] px-3 py-2 text-[12px] text-[var(--color-muted)] hover:border-[var(--color-crit)] hover:text-[var(--color-crit)] disabled:opacity-50"><Trash2 size={13} /> limpar</button>
                {testMsg['openai'] && <span className="pb-2 text-[12px] text-[var(--color-muted)]">{testMsg['openai']}</span>}
              </div>

              {/* llama local */}
              <div className="flex flex-wrap items-end gap-2">
                <div className="grow">
                  <div className="mb-1 text-[11px] uppercase tracking-wide text-[var(--color-muted)]">endpoint local (llama-server, OpenAI-compat)</div>
                  <input value={localUrl ?? c.local_url} onChange={(e) => setLocalUrl(e.target.value)} className={`w-full font-mono text-[12px] ${inputCls}`} />
                </div>
                {localUrl != null && localUrl !== c.local_url && (
                  <button onClick={() => aplicar({ local_url: localUrl }, () => setLocalUrl(null))} disabled={busy} className={btnCls}><Save size={13} /> salvar</button>
                )}
              </div>
            </div>
          </Panel>

          {/* ───── armazenamento ───── */}
          <Panel title="Armazenamento">
            <div className="flex flex-wrap items-center gap-3">
              {(['memory', 'hybrid'] as const).map((s) => (
                <button
                  key={s}
                  onClick={() => aplicar({ storage: s })}
                  disabled={busy || c.storage === s}
                  className={`rounded-md border p-3 text-left transition-colors disabled:opacity-100 ${
                    c.storage === s
                      ? 'border-[var(--color-accent)] bg-[var(--color-accent)]/10'
                      : 'border-[var(--color-border)] opacity-70 hover:border-[var(--color-muted)] hover:opacity-100'
                  }`}
                >
                  <div className="text-[13px] font-semibold">{s}{c.storage === s ? ' · ativo' : ''}</div>
                  <div className="mt-1 max-w-[320px] text-[11px] leading-snug text-[var(--color-muted)]">
                    {s === 'memory'
                      ? 'tudo em RAM — máxima velocidade, maior consumo'
                      : 'texto/tokens ficam no disco — corta ~80% da RAM; busca ampla um pouco mais lenta'}
                  </div>
                </button>
              ))}
              <div className="max-w-[260px] text-[11px] text-[var(--color-muted)]">
                trocar recarrega as bases na hora; a RAM só volta pro SO no próximo restart do daemon.
              </div>
            </div>
          </Panel>

          {/* ───── sessão ───── */}
          <Panel title="Sessão (JWT)">
            <div className="flex flex-wrap items-end gap-2">
              <div title="validade do refresh token — o access é sempre 15 min">
                <div className="mb-1 text-[11px] uppercase tracking-wide text-[var(--color-muted)]">validade do login (segundos)</div>
                <input type="number" min={60} value={ttl ?? c.session_ttl} onChange={(e) => setTtl(Math.max(60, +e.target.value || 60))} className={`w-[130px] ${inputCls}`} />
              </div>
              <span className="pb-2 text-[12px] text-[var(--color-muted)]">= {((ttl ?? c.session_ttl) / 3600).toFixed(1)}h · vale pros próximos logins</span>
              {ttl != null && ttl !== c.session_ttl && (
                <button onClick={() => aplicar({ session_ttl: ttl }, () => setTtl(null))} disabled={busy} className={btnCls}><Save size={13} /> aplicar</button>
              )}
            </div>
          </Panel>

          {/* ───── infraestrutura (leitura) ───── */}
          <Panel title="Infraestrutura (somente leitura — muda no cfg/CLI e reinicia)">
            <div>
              {info('arquivo de config', c.config_path)}
              {info('ragfiles (persistência)', c.ragfiles_dir)}
              {info('drivers de linguagem', c.drivers_dir)}
              {info('drivers de ingestão', c.ingestors_dir)}
              {info('dicionários (thesaurus)', `${c.thesaurus_dir} · ${c.dicts_active} ativo(s) · ${c.word_syn_entries.toLocaleString('pt-BR')} sinônimos`)}
              {info('cache de expansões', `${c.cache_dir} · ${c.expansions_entries} entrada(s)`)}
              {info('nidhoggd', c.nidhogg_url)}
              {info('upload máximo', `${c.max_upload_mb} MB`)}
              {info('teto de bases / chunks por base', `${c.max_bases || '∞'} / ${c.max_chunks_per_base || '∞'}`)}
              {info('modo dev', c.dev_mode ? 'SIM (aceita credenciais padrão)' : 'não')}
            </div>
          </Panel>
        </>
      )}
    </div>
  )
}
