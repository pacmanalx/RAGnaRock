import { useEffect, useState } from 'react'
import { ChevronDown, ChevronRight, Search } from 'lucide-react'
import { useAsync } from '@/hooks/useAsync'
import { getNidhoggCollections, getNidhoggTree, getNidhoggStatus } from '@/api/ragnarock'
import type { TreeNode } from '@/api/types'
import { messageFromError } from '@/api/client'
import { Panel, Spinner, ErrorBox } from '@/components/ui'
import { ChunkModal, type ChunkTarget } from '@/components/ChunkModal'
import { ThinkNavigator } from './Navigator'

// L2 · KnowledgeTree — a árvore de ASSUNTOS destilada do dump denso (zero IA).
// Nó = valor-entidade (CNPJ, nome…) que aparece em ≥2 registros; ramos = tipos de
// documento; folhas = os registros, clicáveis (abrem o documento chunk a chunk).
// A aresta contrato↔comprovante é o próprio nó: quem compartilha valor está ligado.

export function NidhoggTree() {
  const st = useAsync(getNidhoggStatus, [])
  const cols = useAsync(getNidhoggCollections, [])
  // a árvore é do ACUMULADO (sobrevive ao acesso do worm) — todas as coleções aparecem
  const todas = cols.data?.collections ?? []
  const [coll, setColl] = useState<string | null>(null)
  const ativa = coll ?? todas[0]?.collection ?? null
  const [q, setQ] = useState('')
  const [qAplicado, setQAplicado] = useState('')
  const [inspect, setInspect] = useState<ChunkTarget | null>(null)
  const [vista, setVista] = useState<'arvore' | 'navigator'>('arvore')

  // debounce da busca (400ms)
  useEffect(() => {
    const id = setTimeout(() => setQAplicado(q.trim()), 400)
    return () => clearTimeout(id)
  }, [q])

  const nivelBaixo = st.data != null && st.data.level < 2

  return (
    <div className="space-y-4">
      <div className="flex flex-wrap items-center gap-3">
        <h1 className="text-lg font-semibold">L2 · KnowledgeTree</h1>
        <span className="text-[12px] text-[var(--color-muted)]">assuntos destilados do dump — quem compartilha valor está ligado (zero IA)</span>
        <div className="grow" />
        <div className="flex overflow-hidden rounded-md border border-[var(--color-border)]">
          {(['arvore', 'navigator'] as const).map((v) => (
            <button key={v} onClick={() => setVista(v)}
              className={`px-3 py-1.5 text-[12px] font-medium transition-colors ${
                vista === v ? 'bg-[var(--color-accent)] text-[var(--color-accent-fg)]' : 'bg-[var(--color-panel-2)] text-[var(--color-muted)] hover:text-[var(--color-fg)]'
              }`}>
              {v === 'arvore' ? '🌳 Árvore' : '🧠 Think Navigator'}
            </button>
          ))}
        </div>
      </div>

      {nivelBaixo && (
        <div className="rounded-md border border-[var(--color-warn)]/40 bg-[var(--color-warn)]/10 px-4 py-2.5 text-[13px]">
          o worm está no nível <b>L{st.data!.level} · {st.data!.level_name}</b> — a ligação roda a partir do <b>L2</b>.
          Suba o nível em <a href="/admin/servicos" className="text-[var(--color-accent)]">Serviços</a> e rode um ciclo.
        </div>
      )}

      {cols.loading && <Spinner />}
      {vista === 'arvore' && todas.length > 0 && (
        <div className="flex flex-wrap items-center gap-1 border-b border-[var(--color-border)]">
          {todas.map((c) => (
            <button
              key={c.collection}
              onClick={() => setColl(c.collection)}
              title={c.enabled ? undefined : 'acesso do worm bloqueado — a árvore mostra o já acumulado'}
              className={`border-b-2 px-4 py-2 text-[13px] ${
                ativa === c.collection
                  ? 'border-[var(--color-accent)] font-semibold text-[var(--color-fg)]'
                  : 'border-transparent text-[var(--color-muted)] hover:text-[var(--color-fg)]'
              }`}
            >
              {c.collection}{!c.enabled && ' ⏸'}
            </button>
          ))}
          <div className="grow" />
          {vista === 'arvore' && (
            <div className="relative pb-1.5">
              <Search size={13} className="absolute left-2.5 top-1/2 -translate-y-[60%] text-[var(--color-muted)]" />
              <input
                value={q}
                onChange={(e) => setQ(e.target.value)}
                placeholder="buscar assunto (nome, CNPJ…)…"
                className="w-[240px] rounded-md border border-[var(--color-border)] bg-[var(--color-panel-2)] py-1.5 pl-8 pr-3 text-[12px] outline-none focus:border-[var(--color-accent)]"
              />
            </div>
          )}
        </div>
      )}

      {ativa && vista === 'arvore' && <Arvore key={ativa} collection={ativa} q={qAplicado} onOpen={setInspect} onJump={(v) => setQ(v)} />}
      {vista === 'navigator' && <ThinkNavigator colecoes={todas.map((c) => c.collection)} />}
      {inspect && <ChunkModal target={inspect} onClose={() => setInspect(null)} />}
    </div>
  )
}

function Arvore({ collection, q, onOpen, onJump }: { collection: string; q: string; onOpen: (t: ChunkTarget) => void; onJump: (valor: string) => void }) {
  const tree = useAsync(() => getNidhoggTree(collection, q), [collection, q])
  const [abertos, setAbertos] = useState<Set<string>>(new Set())

  function toggle(k: string) {
    setAbertos((s) => { const n = new Set(s); if (n.has(k)) n.delete(k); else n.add(k); return n })
  }

  if (tree.loading) return <Spinner label="destilando a árvore…" />
  if (tree.error) return <ErrorBox message={messageFromError(tree.error)} onRetry={tree.reload} />
  const nodes = tree.data?.nodes ?? []

  return (
    <Panel title={`${nodes.length} assunto(s)${q ? ` · filtro: "${q}"` : ''}`}>
      {nodes.length === 0 && (
        <div className="space-y-1.5 text-[13px] text-[var(--color-muted)]">
          {q ? <div>nenhum assunto casa com a busca.</div> : (
            <>
              <div>nenhum assunto nesta coleção — assunto nasce quando <b>≥2 registros do dump compartilham um valor</b>. Três causas possíveis:</div>
              <div>· <b>dump vazio</b>: coleção narrativa (memorial, contratos…) não gera registro — a destilação de temas narrativos é a Camada 2 (LLM), a caminho;</div>
              <div>· <b>valores todos únicos</b>: há registros, mas nada se repete entre eles (nada a ligar — ainda);</div>
              <div>· <b>worm abaixo do L2</b> ou ciclo ainda não rodou após a extração.</div>
            </>
          )}
        </div>
      )}
      <div className="space-y-1">
        {nodes.map((n: TreeNode) => {
          const aberto = abertos.has(n.valor_norm)
          return (
            <div key={n.valor_norm}>
              {/* nó-assunto */}
              <button
                onClick={() => toggle(n.valor_norm)}
                className="flex w-full items-center gap-2 rounded-md px-2 py-1.5 text-left hover:bg-[var(--color-panel-2)]"
              >
                {aberto ? <ChevronDown size={14} className="shrink-0 text-[var(--color-muted)]" /> : <ChevronRight size={14} className="shrink-0 text-[var(--color-muted)]" />}
                <span className="truncate text-[14px] font-semibold">{n.valor}</span>
                <span className="shrink-0 text-[11px] text-[var(--color-muted)]">
                  {n.registros} registro(s) · {n.bases} documento(s) · {n.ramos.length} tipo(s)
                </span>
              </button>
              {/* ramos por tipo + co-assuntos */}
              {aberto && (
                <div className="ml-[13px] border-l border-[var(--color-border)] pl-4">
                  {/* profundidade: assuntos que co-ocorrem NO MESMO registro deste */}
                  {(n.co?.length ?? 0) > 0 && (
                    <div className="flex flex-wrap items-center gap-1.5 px-2 py-1">
                      <span className="text-[11px] text-[var(--color-muted)]">🔗 ligado a:</span>
                      {n.co.map((c) => (
                        <button
                          key={c.valor_norm}
                          onClick={() => onJump(c.valor)}
                          title={`co-ocorre em ${c.n} registro(s) — clique pra focar a árvore neste assunto`}
                          className="rounded-full border border-[var(--color-border)] px-2 py-0.5 text-[11px] transition-colors hover:border-[var(--color-accent)] hover:text-[var(--color-accent)]"
                        >
                          {c.valor} <span className="text-[var(--color-muted)]">×{c.n}</span>
                        </button>
                      ))}
                    </div>
                  )}
                  {n.ramos.map((r) => {
                    const rk = `${n.valor_norm}|${r.tipo}`
                    const rAberto = abertos.has(rk)
                    return (
                      <div key={rk}>
                        <button
                          onClick={() => toggle(rk)}
                          className="flex w-full items-center gap-2 rounded px-2 py-1 text-left hover:bg-[var(--color-panel-2)]"
                        >
                          {rAberto ? <ChevronDown size={12} className="shrink-0 text-[var(--color-muted)]" /> : <ChevronRight size={12} className="shrink-0 text-[var(--color-muted)]" />}
                          <span className="text-[13px]">{r.tipo}</span>
                          <span className="rounded bg-[var(--color-panel-2)] px-1.5 text-[10px] tabular-nums text-[var(--color-muted)]">{r.n}</span>
                        </button>
                        {rAberto && (
                          <div className="ml-[11px] border-l border-[var(--color-border)] pl-4">
                            {r.itens.map((it, i) => (
                              <button
                                key={`${it.base}-${it.idx}-${i}`}
                                onClick={() => onOpen({ collection, base: it.base, id: 0 })}
                                title={`campo de ligação: ${it.campo} · registro #${it.idx} · NQI ${it.nqi.toFixed(2)} — clique pra abrir o documento`}
                                className="flex w-full items-center gap-2 rounded px-2 py-1 text-left text-[12px] hover:bg-[var(--color-panel-2)]"
                              >
                                <span className="truncate">{it.base}</span>
                                <span className="shrink-0 text-[10px] text-[var(--color-muted)]">via {it.campo} · nqi {it.nqi.toFixed(2)}</span>
                              </button>
                            ))}
                          </div>
                        )}
                      </div>
                    )
                  })}
                </div>
              )}
            </div>
          )
        })}
      </div>
      <div className="mt-3 text-[11px] text-[var(--color-muted)]">
        nó = valor que aparece em ≥2 registros do dump (CNPJ normalizado por dígitos, nomes sem caixa/acento).
        A cadeia contrato → cadastro → comprovantes de uma entidade É a árvore dela.
      </div>
    </Panel>
  )
}
