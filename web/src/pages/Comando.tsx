import { useState } from 'react'
import { search, searchExpand, getCollections } from '@/api/ragnarock'
import type { SearchExpandResponse } from '@/api/types'
import { messageFromError } from '@/api/client'
import { useAsync } from '@/hooks/useAsync'
import { Panel, Spinner, ErrorBox } from '@/components/ui'
import { ChunkModal, type ChunkTarget } from '@/components/ChunkModal'

// Modos de busca — todos vão em POST /search (#39); o modo decide `expand`/`two_phase`:
//   lexico    → expand=false        (silábico puro: tf-idf + matched filter)
//   semantico → expand=true         (two-phase: expande 📚→📖→🧠 SÓ quando o léxico é fraco)
//   inferir   → expand=true, two_phase=false (SEMPRE roda a cascata, incl. a IA)
// As demais opções da rota ficam no bloco "opções do /search".
type Modo = 'lexico' | 'semantico' | 'inferir'

const MODOS: { id: Modo; label: string; hint: string }[] = [
  { id: 'lexico', label: 'léxico', hint: 'busca silábica pura — tf-idf + matched filter, sem expansão' },
  { id: 'semantico', label: 'semântico 🧠', hint: 'expande por sinônimos quando o léxico é fraco. Cascata: dicionários ativos (📚) → cache (📖) → IA (🧠)' },
  { id: 'inferir', label: 'inferência forçada', hint: 'sempre roda a cascata de expansão, mesmo quando a busca pura já acha (two_phase off)' },
]

// [#39] estágios efetivos que o motor devolve em `via`
const VIA_LABEL: Record<string, string> = {
  silabico: 'silábico', phonetic: 'fonético', literal: '🔎 literal', literal_fallback: '🔎 literal',
  dict: '📚 dicionário', cache: '📖 cache', llm: '🧠 IA',
}

const SOURCE_LABEL: Record<string, string> = {
  phase1: '⚡ léxico forte (fase 1, sem expansão)',
  dict: '📚 dicionário',
  cache: '📖 cache',
  llm: '🧠 IA',
  literal: '🔎 literal (needle alfanumérico)',
  literal_fallback: '🔎 literal (fallback)',
}

// Geração — a linguagem do RAGnaRock vive em lib/geracao.ts (módulo puro, semente
// da spec do parser #35). Nesta fase os comandos são SÓ definição: nada executa.
import { genSelect, genDeleteChunk, genDeleteBase, type Lang, type FormState } from '@/lib/geracao'

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
  // opções do POST /search (padrões = os do motor)
  const [rerank, setRerank] = useState(true)
  const [literal, setLiteral] = useState(true)
  const [unified, setUnified] = useState<'auto' | 'sim' | 'nao'>('auto')
  const [recallN, setRecallN] = useState(20)
  const [merge, setMerge] = useState(false)
  const [mergeMax, setMergeMax] = useState(3)
  const [contexto, setContexto] = useState(0)
  const [ctxMax, setCtxMax] = useState(20000)
  const [abertos, setAbertos] = useState<Set<string>>(new Set())
  // páginas + IA local: páginas 1–2 vêm rápidas; da 3 em diante a IA junta traduções e contexto
  const [deep, setDeep] = useState(true)
  const [pagina, setPagina] = useState(1)
  const [res, setRes] = useState<SearchExpandResponse | null>(null)
  const [ms, setMs] = useState<number | null>(null)
  const [loading, setLoading] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [inspect, setInspect] = useState<ChunkTarget | null>(null)
  // Geração: linguagem escolhida + ÚLTIMO comando gerado pelos botões da listagem
  // (substitutivo, não cumulativo — o novo troca o anterior)
  const [lang, setLang] = useState<Lang>('sql')
  const [gerado, setGerado] = useState<string | null>(null)
  const [copied, setCopied] = useState(false)

  const form: FormState = { q, modo, coll, base, k, phonetic }

  async function run(e?: React.FormEvent, pag = 1) {
    e?.preventDefault()
    setPagina(pag)
    const query = q.trim()
    if (!query) return
    setLoading(true); setError(null)
    const opts = {
      collection: coll || undefined, base, k, phonetic,
      rerank, recall_n: recallN, literal_fallback: literal,
      unified: unified === 'auto' ? undefined : unified === 'sim',
      // passagens e contexto só valem sem expansão (o motor ignora com expand)
      ...(modo === 'lexico' ? { merge_adjacent: merge && rerank, merge_max: mergeMax, context: contexto, context_max_chars: ctxMax } : {}),
      page: pag, deep,
    }
    setAbertos(new Set())
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

  async function copiar() {
    // a sequência completa: a busca atual + o comando gerado (se houver)
    const texto = [genSelect(lang, form), ...(gerado ? [gerado] : [])].join('\n\n')
    // clipboard API exige contexto seguro (https) — em http cai no execCommand
    try {
      if (navigator.clipboard && window.isSecureContext) {
        await navigator.clipboard.writeText(texto)
      } else {
        const ta = document.createElement('textarea')
        ta.value = texto
        ta.style.position = 'fixed'; ta.style.opacity = '0'
        document.body.appendChild(ta)
        ta.select()
        document.execCommand('copy')
        ta.remove()
      }
      setCopied(true); setTimeout(() => setCopied(false), 1500)
    } catch { /* clipboard bloqueado pelo browser */ }
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
              placeholder={'ex: cláusula de rescisão do contrato\ncole aqui um parágrafo inteiro pra buscar por similaridade…'}
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
            <label className="flex cursor-pointer items-center gap-2 pb-2 text-[13px]" title="buscas complexas disparam a IA local (tradução e contexto) em segundo plano; as páginas 1–2 não esperam por ela, e os resultados dela entram da página 3 em diante">
              <input type="checkbox" checked={deep} onChange={(e) => setDeep(e.target.checked)} className="accent-[var(--color-accent)]" />
              IA local (pág. 3+)
            </label>
            <button className="rounded-md bg-[var(--color-accent)] px-5 py-2 text-[13px] font-semibold text-[var(--color-accent-fg)] hover:opacity-90">
              buscar
            </button>
          </div>

          {/* ── todas as opções do POST /search (#39/#44/#45) ── */}
          <details className="rounded-md border border-[var(--color-border)] bg-[var(--color-panel-2)] px-3 py-2">
            <summary className="cursor-pointer text-[11px] uppercase tracking-wide text-[var(--color-muted)]">
              opções do /search <span className="normal-case">— padrões do motor; mude para testar</span>
            </summary>
            <div className="mt-3 flex flex-wrap items-end gap-x-5 gap-y-3 text-[13px]">
              <label className="flex cursor-pointer items-center gap-2" title="estágio 2: ordena por cobertura e proximidade; desligado = só o recall (cosseno)">
                <input type="checkbox" checked={rerank} onChange={(e) => setRerank(e.target.checked)} className="accent-[var(--color-accent)]" />
                reordenar (rerank)
              </label>
              <label className="flex cursor-pointer items-center gap-2" title="códigos com dígito (OE-6016, M31May-23h28): busca literal, achados exatos na frente">
                <input type="checkbox" checked={literal} onChange={(e) => setLiteral(e.target.checked)} className="accent-[var(--color-accent)]" />
                literal (literal_fallback)
              </label>
              <div title="vocabulário e idf unificados da coleção; automático = ligado quando a coleção tem mais de uma base no escopo">
                <div className="mb-1 text-[11px] uppercase tracking-wide text-[var(--color-muted)]">vocabulário unificado</div>
                <select value={unified} onChange={(e) => setUnified(e.target.value as 'auto' | 'sim' | 'nao')} className={inputCls}>
                  <option value="auto">automático</option>
                  <option value="sim">sim</option>
                  <option value="nao">não (idf local)</option>
                </select>
              </div>
              <div title="candidatos de cada base que vão para o reordenamento">
                <div className="mb-1 text-[11px] uppercase tracking-wide text-[var(--color-muted)]">candidatos/base</div>
                <input type="number" min={1} max={200} value={recallN} onChange={(e) => setRecallN(Math.max(1, +e.target.value || 20))} className={`w-[70px] ${inputCls}`} />
              </div>
              <label
                className={`flex items-center gap-2 ${modo === 'lexico' && rerank ? 'cursor-pointer' : 'opacity-45'}`}
                title={modo !== 'lexico' ? 'não se aplica com expansão (modo léxico apenas)' : !rerank ? 'exige reordenar' : 'trechos consecutivos da mesma base viram uma passagem, com a cobertura recalculada sobre o texto junto (#44) — custa ~25% de tempo'}
              >
                <input type="checkbox" checked={merge} disabled={modo !== 'lexico' || !rerank} onChange={(e) => setMerge(e.target.checked)} className="accent-[var(--color-accent)]" />
                juntar trechos vizinhos
              </label>
              <div className={modo === 'lexico' && merge && rerank ? '' : 'opacity-45'} title="máximo de trechos por passagem">
                <div className="mb-1 text-[11px] uppercase tracking-wide text-[var(--color-muted)]">trechos/passagem</div>
                <input type="number" min={2} max={8} value={mergeMax} disabled={modo !== 'lexico' || !merge || !rerank} onChange={(e) => setMergeMax(Math.min(8, Math.max(2, +e.target.value || 3)))} className={`w-[60px] ${inputCls}`} />
              </div>
              <div className={modo === 'lexico' ? '' : 'opacity-45'} title={modo !== 'lexico' ? 'não se aplica com expansão (modo léxico apenas)' : 'trechos vizinhos antes e depois de cada resultado, na própria resposta (#45)'}>
                <div className="mb-1 text-[11px] uppercase tracking-wide text-[var(--color-muted)]">contexto (± trechos)</div>
                <input type="number" min={0} max={5} value={contexto} disabled={modo !== 'lexico'} onChange={(e) => setContexto(Math.min(5, Math.max(0, +e.target.value || 0)))} className={`w-[60px] ${inputCls}`} />
              </div>
              <div className={modo === 'lexico' && contexto > 0 ? '' : 'opacity-45'} title="teto do texto de contexto somado; passou, corta e avisa">
                <div className="mb-1 text-[11px] uppercase tracking-wide text-[var(--color-muted)]">teto do contexto (caracteres)</div>
                <input type="number" min={1000} max={200000} step={1000} value={ctxMax} disabled={modo !== 'lexico' || contexto === 0} onChange={(e) => setCtxMax(Math.max(1000, +e.target.value || 20000))} className={`w-[100px] ${inputCls}`} />
              </div>
            </div>
          </details>
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
              {gerado && (
                <button type="button" onClick={() => setGerado(null)} className="text-[11px] text-[var(--color-muted)] hover:text-[var(--color-fg)]">limpar</button>
              )}
              <button
                type="button"
                onClick={copiar}
                title="copia a sequência (busca + comando gerado) — pronta pra colar num código ou rodar num client SQL"
                className="rounded-md border border-[var(--color-accent)] px-2.5 py-1 text-[11px] font-semibold text-[var(--color-accent)] transition-colors hover:bg-[var(--color-accent)] hover:text-[var(--color-accent-fg)]"
              >
                {copied ? 'copiado ✓' : '📋 copiar sequência'}
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
            {gerado ? (
              <pre className="overflow-x-auto rounded-md border border-[var(--color-border)] bg-[var(--color-panel-2)] p-3 text-[12px] leading-relaxed">
                {gerado}
              </pre>
            ) : (
              <div className="text-[11px] text-[var(--color-muted)]">
                use <b>⌦ chunk</b> / <b>⌦ base</b> num resultado pra gerar o DELETE aqui (o novo substitui o anterior).
              </div>
            )}
          </div>
        </Panel>
      </div>

      {error && <ErrorBox message={error} onRetry={() => run()} />}
      {loading && <Spinner label={pagina >= 3 && deep ? '🧠 juntando os resultados da IA local…' : modo === 'lexico' ? 'buscando…' : '🧠 expandindo + buscando…'} />}

      {res && !loading && (
        <>
          {/* linha de info: fonte da cascata + sinônimos (riscado = fora do corpus) */}
          {(source || res.expansions?.length || res.query_syllables || res.via?.length || res.page) && (
            <div className="space-y-1 text-[12px] text-[var(--color-muted)]">
              <div>
                {source && <span>{source}{res.source === 'llm' && res.provider ? ` (${res.provider})` : ''}</span>}
                {ms != null && <span> · <b className="text-[var(--color-ok,#3fb950)]">{ms.toFixed(0)} ms</b></span>}
                {res.query_syllables && <span> · sílabas: {res.query_syllables}</span>}
                {(res.via?.length ?? 0) > 0 && <span> · via: <b>{res.via!.map((v) => VIA_LABEL[v] ?? v).join(' → ')}</b></span>}
                {res.context_truncated && <span className="text-[var(--color-warn,#d29922)]"> · contexto cortado pelo teto</span>}
                {res.page && <span> · página <b>{res.page}</b></span>}
                {res.deep && res.deep !== 'off' && (
                  <span title={res.deep_variants?.join(' · ')}>
                    {' · '}🧠 IA: {res.deep === 'pending' ? 'buscando em segundo plano (entra da página 3)' : res.deep === 'ready' ? `pronta (${res.deep_variants?.length ?? 0} variantes)` : res.deep}
                  </span>
                )}
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
            <Panel
              title={`${res.hits.length} resultado(s)${res.page ? ` · página ${res.page}` : ''}`}
              actions={
                <div className="flex items-center gap-2 text-[12px]">
                  <button type="button" disabled={pagina <= 1 || loading} onClick={() => run(undefined, pagina - 1)}
                    className="rounded border border-[var(--color-border)] px-2 py-0.5 disabled:opacity-40 hover:border-[var(--color-accent)]">◀ anterior</button>
                  <button type="button" disabled={loading || res.hits.length < k} onClick={() => run(undefined, pagina + 1)}
                    title={pagina + 1 >= 3 && deep ? 'da página 3 em diante entram os resultados da IA local (pode levar alguns segundos)' : undefined}
                    className="rounded border border-[var(--color-border)] px-2 py-0.5 disabled:opacity-40 hover:border-[var(--color-accent)]">próxima ▶</button>
                </div>
              }
            >
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
                        #{h.rank} · <span className="text-[var(--color-accent)]">{h.collection}</span> / {h.base} · {h.chunks && h.chunks.length > 1 ? <>passagem: trechos {h.chunks.join('+')}</> : <>chunk {h.chunk}</>}
                        {h.via && h.via !== 'original' && (
                          <span className="ml-1.5 rounded-full border border-[var(--color-border)] px-1.5 text-[9px]" title={`casou via: ${h.via}`}>{h.deep ? '🧠 IA: ' : '🧠 '}{h.via}</span>
                        )}
                      </span>
                      <span className="flex items-center gap-2">
                        <span className="tabular-nums">
                          cov {(h.coverage ?? h.matchpoint ?? 0).toFixed(2)} · span {h.span ?? '–'} · cos {(h.cos ?? 0).toFixed(3)}
                        </span>
                        {/* geração de DELETE — só escreve no painel Geração, NÃO apaga nada */}
                        <button
                          type="button"
                          onClick={(e) => { e.stopPropagation(); setGerado(genDeleteChunk(lang, h.collection, h.base, h.chunk)) }}
                          className={miniBtn}
                          title="gerar o DELETE deste chunk no painel Geração (não executa)"
                        >⌦ chunk</button>
                        <button
                          type="button"
                          onClick={(e) => { e.stopPropagation(); setGerado(genDeleteBase(lang, h.collection, h.base)) }}
                          className={miniBtn}
                          title="gerar o DELETE da base inteira (PURGE) no painel Geração (não executa)"
                        >⌦ base</button>
                      </span>
                    </div>
                    <div className="text-[13px] leading-relaxed"><Snippet text={h.snippet ?? ''} /></div>
                    {h.context && (h.context.before.length + h.context.after.length) > 0 && (() => {
                      const chave = `${h.collection}/${h.base}/${h.chunk}`
                      const aberto = abertos.has(chave)
                      return (
                        <div className="mt-2" onClick={(e) => e.stopPropagation()}>
                          <button
                            type="button"
                            onClick={() => setAbertos((s) => { const n = new Set(s); if (n.has(chave)) n.delete(chave); else n.add(chave); return n })}
                            className="text-[11px] text-[var(--color-accent)] hover:underline"
                          >
                            {aberto ? '▾' : '▸'} contexto ({h.context.before.length} antes, {h.context.after.length} depois)
                          </button>
                          {aberto && (
                            <div className="mt-1 space-y-1.5 text-[12px] leading-relaxed text-[var(--color-muted)]">
                              {h.context.before.map((c) => <div key={`b${c.id}`}><b>trecho {c.id} ↑</b> {c.text}</div>)}
                              {h.context.after.map((c) => <div key={`a${c.id}`}><b>trecho {c.id} ↓</b> {c.text}</div>)}
                            </div>
                          )}
                        </div>
                      )
                    })()}
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
