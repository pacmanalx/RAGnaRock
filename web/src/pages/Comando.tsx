import { useState } from 'react'
import { search, searchExpand, getCollections } from '@/api/ragnarock'
import type { SearchExpandResponse } from '@/api/types'
import { messageFromError } from '@/api/client'
import { useAsync } from '@/hooks/useAsync'
import { Panel, Spinner, ErrorBox } from '@/components/ui'
import { ChunkModal, type ChunkTarget } from '@/components/ChunkModal'

// Modos de busca (espelham a aba Buscar do dashboard legado do ragd):
//   lexico    → POST /search        (silábico puro: tf-idf + matched filter)
//   semantico → POST /search_expand (two-phase: expande 📚→📖→🧠 SÓ quando o léxico é fraco)
//   inferir   → POST /search_expand two_phase=false (SEMPRE roda a cascata, incl. a IA)
type Modo = 'lexico' | 'semantico' | 'inferir'

const MODOS: { id: Modo; label: string; hint: string }[] = [
  { id: 'lexico', label: 'léxico', hint: 'busca silábica pura — tf-idf + matched filter, sem expansão' },
  { id: 'semantico', label: 'semântico 🧠', hint: 'expande por sinônimos quando o léxico é fraco. Cascata: dicionários ativos (📚) → cache (📖) → IA (🧠)' },
  { id: 'inferir', label: 'inferência forçada', hint: 'sempre roda a cascata de expansão, mesmo quando a busca pura já acha (two_phase off)' },
]

const SOURCE_LABEL: Record<string, string> = {
  phase1: '⚡ léxico forte (fase 1, sem expansão)',
  dict: '📚 dicionário',
  cache: '📖 cache',
  llm: '🧠 IA',
  literal: '🔎 literal (needle alfanumérico)',
  literal_fallback: '🔎 literal (fallback)',
}

// ───────── Geração — a linguagem do RAGnaRock (superset ANSI SQL / GraphQL) ─────────
// Nesta fase os comandos são SÓ definição de linguagem: nada executa. O motor
// (issue #35, POST /sql) vem depois — a tela já cria o vocabulário.
type Lang = 'sql' | 'graphql'
type FormState = { q: string; modo: Modo; coll: string; base: string; k: number; phonetic: boolean }

const sqlStr = (s: string) => `'${s.replace(/'/g, "''")}'`
const sqlId = (s: string) => (/^[a-zA-Z_][a-zA-Z0-9_]*$/.test(s) ? s : `"${s.replace(/"/g, '""')}"`)
const gqlStr = (s: string) => JSON.stringify(s)
const MODE_SQL: Record<Modo, string> = { lexico: 'LEXICAL', semantico: 'SEMANTIC', inferir: 'INFER' }

function genSelect(lang: Lang, f: FormState): string {
  const scope = `${f.coll ? sqlId(f.coll) : '*'}.${f.base.trim() && f.base.trim() !== '*' ? sqlId(f.base.trim()) : '*'}`
  if (lang === 'sql')
    return [
      `SELECT rank, collection, base, chunk, cov, span, cos, snippet`,
      `  FROM ${scope}`,
      ` WHERE MATCH(${sqlStr(f.q.trim() || '…')})`,
      `  WITH MODE = ${MODE_SQL[f.modo]}, PHONETIC = ${f.phonetic ? 'ON' : 'OFF'}`,
      ` LIMIT ${f.k};`,
    ].join('\n')
  return [
    `query {`,
    `  search(query: ${gqlStr(f.q.trim() || '…')}, mode: ${MODE_SQL[f.modo]},`,
    `         collection: ${f.coll ? gqlStr(f.coll) : 'null'}, base: ${gqlStr(f.base.trim() || '*')},`,
    `         phonetic: ${f.phonetic}, k: ${f.k}) {`,
    `    rank collection base chunk cov span cos snippet`,
    `  }`,
    `}`,
  ].join('\n')
}

function genDeleteChunk(lang: Lang, coll: string, base: string, chunk: number): string {
  if (lang === 'sql') return `DELETE FROM ${sqlId(coll)}.${sqlId(base)} WHERE chunk = ${chunk};`
  return `mutation { deleteChunk(collection: ${gqlStr(coll)}, base: ${gqlStr(base)}, chunk: ${chunk}) { ok } }`
}

function genDeleteBase(lang: Lang, coll: string, base: string): string {
  // sem WHERE = a base inteira; PURGE = apaga também o JSON do disco (contrato do DELETE /bases?purge=1)
  if (lang === 'sql') return `DELETE FROM ${sqlId(coll)}.${sqlId(base)} PURGE;`
  return `mutation { deleteBase(collection: ${gqlStr(coll)}, base: ${gqlStr(base)}, purge: true) { ok removed } }`
}

// Snippet vem com os trechos que casaram marcados entre « e » — vira <b>.
function Snippet({ text }: { text: string }) {
  const parts = text.split(/[«»]/)
  return (
    <>
      {parts.map((p, i) => (i % 2 === 1 ? <b key={i} className="text-[var(--color-accent)]">{p}</b> : <span key={i}>{p}</span>))}
    </>
  )
}

export function Comando() {
  const cols = useAsync(getCollections, [])
  const [q, setQ] = useState('')
  const [modo, setModo] = useState<Modo>('semantico')
  const [coll, setColl] = useState('')
  const [base, setBase] = useState('*')
  const [k, setK] = useState(8)
  const [phonetic, setPhonetic] = useState(false)
  const [res, setRes] = useState<SearchExpandResponse | null>(null)
  const [ms, setMs] = useState<number | null>(null)
  const [loading, setLoading] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [inspect, setInspect] = useState<ChunkTarget | null>(null)
  // Geração: linguagem escolhida + comandos gerados pelos botões da listagem
  const [lang, setLang] = useState<Lang>('sql')
  const [gerados, setGerados] = useState<string[]>([])
  const [copied, setCopied] = useState(false)

  const form: FormState = { q, modo, coll, base, k, phonetic }

  async function run(e?: React.FormEvent) {
    e?.preventDefault()
    const query = q.trim()
    if (!query) return
    setLoading(true); setError(null)
    const opts = { collection: coll || undefined, base, k, phonetic }
    const t0 = performance.now()
    try {
      const r = modo === 'lexico'
        ? await search(query, opts)
        : await searchExpand(query, { ...opts, forceInfer: modo === 'inferir' })
      setRes(r)
      setMs(performance.now() - t0)
    } catch (err) { setError(messageFromError(err)) }
    finally { setLoading(false) }
  }

  function onKeyDown(e: React.KeyboardEvent) {
    if (e.key === 'Enter' && (e.ctrlKey || e.metaKey)) run()
  }

  function gerar(cmd: string) {
    setGerados((g) => (g.includes(cmd) ? g : [...g, cmd]))
  }

  async function copiar() {
    const texto = [genSelect(lang, form), ...gerados].join('\n\n')
    try { await navigator.clipboard.writeText(texto); setCopied(true); setTimeout(() => setCopied(false), 1500) } catch { /* clipboard indisponível (http) */ }
  }

  const dropped = new Set(res?.dropped ?? [])
  const source = res?.source ? (SOURCE_LABEL[res.source] ?? res.source) : null

  const inputCls = 'rounded-md border border-[var(--color-border)] bg-[var(--color-panel-2)] px-3 py-2 text-[13px] outline-none focus:border-[var(--color-accent)]'
  const segBtn = (active: boolean) =>
    `px-3 py-2 text-[12px] font-medium transition-colors ${active
      ? 'bg-[var(--color-accent)] text-[var(--color-accent-fg)]'
      : 'bg-[var(--color-panel-2)] text-[var(--color-muted)] hover:text-[var(--color-fg)]'}`
  const miniBtn = 'rounded border border-[var(--color-border)] px-1.5 py-0.5 text-[10px] text-[var(--color-muted)] transition-colors hover:border-[var(--color-danger,#f85149)] hover:text-[var(--color-danger,#f85149)]'

  return (
    <div className="space-y-5">
      <h1 className="text-lg font-semibold">Comando</h1>

      {/* comando + geração lado a lado */}
      <div className="grid gap-4 xl:grid-cols-[minmax(0,3fr)_minmax(0,2fr)]">
        <form onSubmit={run} className="space-y-3">
          <div>
            <div className="mb-1 text-[11px] uppercase tracking-wide text-[var(--color-muted)]">
              query <span className="normal-case">— pode colar trechos longos · Ctrl+Enter busca</span>
            </div>
            <textarea
              value={q}
              onChange={(e) => setQ(e.target.value)}
              onKeyDown={onKeyDown}
              rows={3}
              placeholder={'ex: cálculo de média do aluno\ncole aqui um parágrafo inteiro pra buscar por similaridade…'}
              className={`w-full resize-y ${inputCls} text-[14px]`}
            />
          </div>

          <div className="flex flex-wrap items-end gap-3">
            <div>
              <div className="mb-1 text-[11px] uppercase tracking-wide text-[var(--color-muted)]">modo</div>
              <div className="flex overflow-hidden rounded-md border border-[var(--color-border)]">
                {MODOS.map((m) => (
                  <button key={m.id} type="button" title={m.hint} onClick={() => setModo(m.id)} className={segBtn(modo === m.id)}>
                    {m.label}
                  </button>
                ))}
              </div>
            </div>
            <div>
              <div className="mb-1 text-[11px] uppercase tracking-wide text-[var(--color-muted)]">coleção</div>
              <select value={coll} onChange={(e) => setColl(e.target.value)} className={inputCls}>
                <option value="">(todas)</option>
                {(cols.data?.collections ?? []).map((c) => (
                  <option key={c.collection} value={c.collection}>{c.collection} ({c.bases})</option>
                ))}
              </select>
            </div>
            <div>
              <div className="mb-1 text-[11px] uppercase tracking-wide text-[var(--color-muted)]">base (wildcard)</div>
              <input value={base} onChange={(e) => setBase(e.target.value)} className={`w-[130px] ${inputCls}`} />
            </div>
            <div title="top-K: quantos resultados o motor devolve (os K chunks mais bem rankeados)">
              <div className="mb-1 text-[11px] uppercase tracking-wide text-[var(--color-muted)]">resultados</div>
              <input
                type="number" min={1} max={50} value={k}
                onChange={(e) => setK(Math.max(1, +e.target.value || 8))}
                className={`w-[70px] ${inputCls}`}
              />
            </div>
            <label className="flex cursor-pointer items-center gap-2 pb-2 text-[13px]">
              <input type="checkbox" checked={phonetic} onChange={(e) => setPhonetic(e.target.checked)} className="accent-[var(--color-accent)]" />
              fonético
            </label>
            <button className="rounded-md bg-[var(--color-accent)] px-5 py-2 text-[13px] font-semibold text-[var(--color-accent-fg)] hover:opacity-90">
              buscar
            </button>
          </div>
        </form>

        {/* ───── painel Geração: a linguagem, nada executa (motor = #35, depois) ───── */}
        <Panel
          title="Geração"
          actions={
            <div className="flex items-center gap-2">
              <div className="flex overflow-hidden rounded-md border border-[var(--color-border)]">
                <button type="button" onClick={() => setLang('sql')} className={segBtn(lang === 'sql')}>SQL</button>
                <button type="button" onClick={() => setLang('graphql')} className={segBtn(lang === 'graphql')}>GraphQL</button>
              </div>
              {gerados.length > 0 && (
                <button type="button" onClick={() => setGerados([])} className="text-[11px] text-[var(--color-muted)] hover:text-[var(--color-fg)]">limpar</button>
              )}
              <button type="button" onClick={copiar} className="text-[11px] text-[var(--color-accent)] hover:opacity-80">
                {copied ? 'copiado ✓' : 'copiar'}
              </button>
            </div>
          }
        >
          <div className="space-y-2">
            <div className="text-[11px] text-[var(--color-muted)]">
              comando equivalente à busca atual — definição da linguagem; o motor que executa vem depois
            </div>
            <pre className="overflow-x-auto rounded-md border border-[var(--color-border)] bg-[var(--color-panel-2)] p-3 text-[12px] leading-relaxed">
              {genSelect(lang, form)}
            </pre>
            {gerados.map((g, i) => (
              <pre key={i} className="group relative overflow-x-auto rounded-md border border-[var(--color-border)] bg-[var(--color-panel-2)] p-3 text-[12px] leading-relaxed">
                {g}
                <button
                  type="button"
                  onClick={() => setGerados((arr) => arr.filter((_, j) => j !== i))}
                  className="absolute right-1.5 top-1.5 hidden rounded px-1 text-[10px] text-[var(--color-muted)] hover:text-[var(--color-fg)] group-hover:block"
                  title="descartar este comando"
                >✕</button>
              </pre>
            ))}
            {gerados.length === 0 && (
              <div className="text-[11px] text-[var(--color-muted)]">
                use <b>⌦ chunk</b> / <b>⌦ base</b> num resultado pra gerar o DELETE aqui.
              </div>
            )}
          </div>
        </Panel>
      </div>

      {error && <ErrorBox message={error} onRetry={() => run()} />}
      {loading && <Spinner label={modo === 'lexico' ? 'buscando…' : '🧠 expandindo + buscando…'} />}

      {res && !loading && (
        <>
          {/* linha de info: fonte da cascata + sinônimos (riscado = fora do corpus) */}
          {(source || res.expansions?.length || res.query_syllables) && (
            <div className="space-y-1 text-[12px] text-[var(--color-muted)]">
              <div>
                {source && <span>{source}{res.source === 'llm' && res.provider ? ` (${res.provider})` : ''}</span>}
                {ms != null && <span> · <b className="text-[var(--color-ok,#3fb950)]">{ms.toFixed(0)} ms</b></span>}
                {res.query_syllables && <span> · sílabas: {res.query_syllables}</span>}
              </div>
              {(res.expansions?.length ?? 0) > 0 && (
                <div className="flex flex-wrap items-center gap-1.5">
                  <span>sinônimos:</span>
                  {res.expansions!.map((e) => (
                    <span
                      key={e}
                      title={dropped.has(e) ? 'fora do corpus deste escopo — não foi buscada' : undefined}
                      className={`rounded-full border border-[var(--color-border)] px-2 py-0.5 text-[11px] ${dropped.has(e) ? 'line-through opacity-45' : ''}`}
                    >
                      {e}
                    </span>
                  ))}
                </div>
              )}
            </div>
          )}

          {/* query ausente do corpus: mostra o did-you-mean do motor */}
          {res.absent ? (
            <Panel title="⚠ ausente do corpus">
              <div className="space-y-2 text-[13px]">
                <div className="text-[var(--color-muted)]">este escopo não tem essas sílabas. o mais parecido que ele tem:</div>
                <div className="flex flex-wrap gap-1.5">
                  {(res.did_you_mean ?? []).map((t) => (
                    <button
                      key={t}
                      onClick={() => { setQ(t) }}
                      className="rounded-full border border-[var(--color-accent)] px-2.5 py-0.5 text-[12px] text-[var(--color-accent)] hover:bg-[var(--color-accent)] hover:text-[var(--color-accent-fg)]"
                      title="usar este termo como query"
                    >
                      {t}
                    </button>
                  ))}
                  {(res.did_you_mean ?? []).length === 0 && <span className="text-[var(--color-muted)]">—</span>}
                </div>
              </div>
            </Panel>
          ) : (
            <Panel title={`${res.hits.length} resultado(s)`}>
              <div className="space-y-2">
                {res.hits.map((h) => (
                  <div
                    key={`${h.collection}-${h.base}-${h.chunk}`}
                    role="button"
                    tabIndex={0}
                    onClick={() => setInspect({ collection: h.collection, base: h.base, id: h.chunk })}
                    onKeyDown={(e) => { if (e.key === 'Enter') setInspect({ collection: h.collection, base: h.base, id: h.chunk }) }}
                    className="block w-full cursor-pointer rounded-md border border-[var(--color-border)] bg-[var(--color-panel-2)] p-3 text-left transition-colors hover:border-[var(--color-accent)]"
                    title={h.via && h.via !== 'original' ? `casou via: ${h.via}` : 'abrir e navegar o documento chunk a chunk'}
                  >
                    <div className="mb-1 flex flex-wrap items-center justify-between gap-2 text-[11px] text-[var(--color-muted)]">
                      <span>
                        #{h.rank} · <span className="text-[var(--color-accent)]">{h.collection}</span> / {h.base} · chunk {h.chunk}
                        {h.via && h.via !== 'original' && (
                          <span className="ml-1.5 rounded-full border border-[var(--color-border)] px-1.5 text-[9px]" title={`casou via: ${h.via}`}>🧠 {h.via}</span>
                        )}
                      </span>
                      <span className="flex items-center gap-2">
                        <span className="tabular-nums">
                          cov {(h.coverage ?? h.matchpoint ?? 0).toFixed(2)} · span {h.span ?? '–'} · cos {(h.cos ?? 0).toFixed(3)}
                        </span>
                        {/* geração de DELETE — só escreve no painel Geração, NÃO apaga nada */}
                        <button
                          type="button"
                          onClick={(e) => { e.stopPropagation(); gerar(genDeleteChunk(lang, h.collection, h.base, h.chunk)) }}
                          className={miniBtn}
                          title="gerar o DELETE deste chunk no painel Geração (não executa)"
                        >⌦ chunk</button>
                        <button
                          type="button"
                          onClick={(e) => { e.stopPropagation(); gerar(genDeleteBase(lang, h.collection, h.base)) }}
                          className={miniBtn}
                          title="gerar o DELETE da base inteira (PURGE) no painel Geração (não executa)"
                        >⌦ base</button>
                      </span>
                    </div>
                    <div className="text-[13px] leading-relaxed"><Snippet text={h.snippet ?? ''} /></div>
                  </div>
                ))}
                {res.hits.length === 0 && <div className="text-[13px] text-[var(--color-muted)]">sem resultados.</div>}
              </div>
            </Panel>
          )}
        </>
      )}

      {inspect && <ChunkModal target={inspect} onClose={() => setInspect(null)} />}
    </div>
  )
}
