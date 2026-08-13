import { useEffect, useRef, useState } from 'react'
import { Crosshair, RotateCcw, X } from 'lucide-react'
import { getNavNode, getNavSuggest, search } from '@/api/ragnarock'
import type { SearchResponse } from '@/api/types'
import { messageFromError } from '@/api/client'
import { Spinner } from '@/components/ui'
import { ChunkModal, type ChunkTarget } from '@/components/ChunkModal'

// Think Navigator — mindmap infinito sobre o grafo de co-ocorrência do L2.
// Tema central → clica → relacionados irradiam → clica num relacionado → a teia cresce.
// Nó já existente NÃO duplica: ganha aresta nova (é onde o mapa "liga com os anteriores").
// Zero lib de grafo: SVG + layout radial determinístico + pan/zoom manuais.

interface NavMapNode {
  norm: string
  valor: string
  x: number
  y: number
  expanded: boolean
  peso: number // registros ligados (engorda o nó)
}

const R_EXPAND = 190      // raio dos filhos ao expandir
const MAX_FILHOS = 10     // legibilidade: os 10 relacionados mais fortes por expansão

export function ThinkNavigator({ colecoes }: { colecoes: string[] }) {
  // coleção é FILTRO, não jaula: default = TODAS ('*'). Com 2500 coleções o select
  // vira autocomplete — a API já aceita qualquer nome.
  const [escopo, setEscopo] = useState('*')
  const collection = escopo
  const [nodes, setNodes] = useState<Map<string, NavMapNode>>(new Map())
  const [edges, setEdges] = useState<Set<string>>(new Set())
  const [busy, setBusy] = useState<string | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [tema, setTema] = useState('')
  const [sugestoes, setSugestoes] = useState<{ valor: string; valor_norm: string }[]>([])
  const [sugVazio, setSugVazio] = useState(false)
  const [refs, setRefs] = useState<{ valor: string } | null>(null)   // modal de referências (duplo clique)
  const [inspect, setInspect] = useState<ChunkTarget | null>(null)   // documento chunk a chunk
  const clickTimer = useRef<ReturnType<typeof setTimeout> | null>(null)
  const [view, setView] = useState({ x: 0, y: 0, zoom: 1 })
  const viewRef = useRef(view)
  viewRef.current = view
  const animRef = useRef<number | null>(null)
  const drag = useRef<{ px: number; py: number; vx: number; vy: number } | null>(null)
  const svgRef = useRef<SVGSVGElement>(null)

  // desliza o mapa até o nó clicado virar o CENTRO (ease-out cúbico, ~450ms)
  function panPara(nx: number, ny: number) {
    if (animRef.current) cancelAnimationFrame(animRef.current)
    const from = { x: viewRef.current.x, y: viewRef.current.y }
    const zoom = viewRef.current.zoom
    const alvo = { x: -nx * zoom, y: -ny * zoom }
    const t0 = performance.now()
    const dur = 450
    const step = (t: number) => {
      const k = Math.min(1, (t - t0) / dur)
      const e = 1 - Math.pow(1 - k, 3)
      setView((v) => ({ ...v, x: from.x + (alvo.x - from.x) * e, y: from.y + (alvo.y - from.y) * e }))
      if (k < 1) animRef.current = requestAnimationFrame(step)
    }
    animRef.current = requestAnimationFrame(step)
  }

  const num = (v: number | string) => typeof v === 'number' ? v : parseInt(v || '0', 10) || 0
  const debounceRef = useRef<ReturnType<typeof setTimeout> | null>(null)

  // ── busca do tema central (endpoint leve + debounce 300ms) ──
  function buscarTema(q: string) {
    setTema(q)
    if (debounceRef.current) clearTimeout(debounceRef.current)
    if (q.trim().length < 2) { setSugestoes([]); setSugVazio(false); return }
    debounceRef.current = setTimeout(async () => {
      try {
        const r = await getNavSuggest(collection, q.trim())
        setSugestoes(r.nodes.map((n) => ({ valor: n.valor, valor_norm: n.valor_norm })))
        setSugVazio(r.nodes.length === 0)
      } catch (e) { setError(messageFromError(e)); setSugestoes([]) }
    }, 300)
  }

  async function centrar(valor: string, norm: string) {
    setSugestoes([]); setTema(valor); setError(null)
    const m = new Map<string, NavMapNode>()
    m.set(norm, { norm, valor, x: 0, y: 0, expanded: false, peso: 1 })
    setNodes(m); setEdges(new Set()); setView({ x: 0, y: 0, zoom: 1 })
    await expandir(norm, m, new Set())
  }

  // ── expansão: os relacionados irradiam; nó existente só ganha aresta ──
  async function expandir(norm: string, curNodes?: Map<string, NavMapNode>, curEdges?: Set<string>) {
    if (busy) return
    setBusy(norm); setError(null)
    try {
      const r = await getNavNode(collection, norm)
      const ns = new Map(curNodes ?? nodes)
      const es = new Set(curEdges ?? edges)
      const pai = ns.get(norm)
      if (!pai || !r.found) { setBusy(null); return }
      panPara(pai.x, pai.y)   // o nó clicado vira o centro do pensamento
      pai.expanded = true
      pai.peso = Math.max(pai.peso, num(r.registros))
      const novos = r.co.slice(0, MAX_FILHOS)
      // ângulo de entrada (do pai em direção ao centro) pra abrir o leque pro lado livre
      const entrada = Math.atan2(pai.y, pai.x)
      const aNovos = novos.filter((c) => !ns.has(c.valor_norm))
      let slot = 0
      for (const c of novos) {
        const ekey = [norm, c.valor_norm].sort().join('|')
        es.add(ekey)
        if (!ns.has(c.valor_norm)) {
          // leque de 300° centrado no rumo "pra fora" (evita voltar por cima do caminho)
          const base = (pai.x === 0 && pai.y === 0) ? -Math.PI / 2 : entrada
          const spread = (Math.PI * 5) / 3
          const ang = base - spread / 2 + (aNovos.length <= 1 ? spread / 2 : (slot * spread) / (aNovos.length - 1))
          // raio com respiro progressivo pra mapas densos
          const raio = R_EXPAND + (ns.size > 14 ? 40 : 0)
          ns.set(c.valor_norm, {
            norm: c.valor_norm, valor: c.valor,
            x: pai.x + raio * Math.cos(ang),
            y: pai.y + raio * Math.sin(ang),
            expanded: false, peso: num(c.n),
          })
          slot += 1
        }
      }
      setNodes(ns); setEdges(es)
    } catch (e) { setError(messageFromError(e)) }
    finally { setBusy(null) }
  }

  // ── pan & zoom ──
  function onDown(e: React.MouseEvent) {
    if (animRef.current) cancelAnimationFrame(animRef.current)   // arrasto interrompe o glide
    drag.current = { px: e.clientX, py: e.clientY, vx: view.x, vy: view.y }
  }
  function onMove(e: React.MouseEvent) {
    if (!drag.current) return
    setView((v) => ({ ...v, x: drag.current!.vx + (e.clientX - drag.current!.px), y: drag.current!.vy + (e.clientY - drag.current!.py) }))
  }

  // zoom na RODA, ancorado no CURSOR (o ponto sob o mouse fica parado — comportamento de
  // mapa). Listener manual com passive:false — o onWheel do React é passivo e a página
  // rolava junto, brigando com o zoom.
  useEffect(() => {
    const svg = svgRef.current
    if (!svg) return
    const handler = (e: WheelEvent) => {
      e.preventDefault()
      if (animRef.current) cancelAnimationFrame(animRef.current)
      const rect = svg.getBoundingClientRect()
      const mx = e.clientX - rect.left - rect.width / 2   // mouse relativo ao centro do palco
      const my = e.clientY - rect.top - rect.height / 2
      setView((v) => {
        const z2 = Math.min(3, Math.max(0.2, v.zoom * (e.deltaY < 0 ? 1.15 : 0.87)))
        // mantém o ponto do mundo sob o cursor no mesmo lugar da tela
        const wx = (mx - v.x) / v.zoom
        const wy = (my - v.y) / v.zoom
        return { zoom: z2, x: mx - wx * z2, y: my - wy * z2 }
      })
    }
    svg.addEventListener('wheel', handler, { passive: false })
    return () => svg.removeEventListener('wheel', handler)
  }, [])

  const lista = [...nodes.values()]
  const raioDe = (n: NavMapNode) => Math.min(34, 14 + Math.sqrt(n.peso) * 1.4)

  return (
    <div className="space-y-3">
      <style>{'@keyframes navpop { from { opacity: 0 } to { opacity: 1 } }'}</style>
      {/* tema central */}
      <div className="relative flex flex-wrap items-center gap-2">
        <Crosshair size={15} className="text-[var(--color-muted)]" />
        <input
          value={tema}
          onChange={(e) => buscarTema(e.target.value)}
          placeholder="tema central (ex.: Jesus, Gandalf, EssenciaViva)…"
          className="w-[300px] rounded-md border border-[var(--color-border)] bg-[var(--color-panel-2)] px-3 py-2 text-[13px] outline-none focus:border-[var(--color-accent)]"
        />
        <select
          value={escopo}
          onChange={(e) => { setEscopo(e.target.value); setSugestoes([]); setSugVazio(false) }}
          title="escopo da navegação — coleção é filtro; o default pensa sobre TODO o acumulado"
          className="rounded-md border border-[var(--color-border)] bg-[var(--color-panel-2)] px-2 py-2 text-[12px] outline-none focus:border-[var(--color-accent)]"
        >
          <option value="*">🌐 todas as coleções</option>
          {colecoes.map((c) => <option key={c} value={c}>{c}</option>)}
        </select>
        {nodes.size > 0 && (
          <button onClick={() => { setNodes(new Map()); setEdges(new Set()); setTema('') }}
            className="flex items-center gap-1.5 rounded-md border border-[var(--color-border)] px-3 py-2 text-[12px] text-[var(--color-muted)] hover:text-[var(--color-fg)]">
            <RotateCcw size={13} /> recomeçar
          </button>
        )}
        <span className="text-[11px] text-[var(--color-muted)]">
          {nodes.size > 0 ? `${nodes.size} nó(s) · ${edges.size} ligação(ões) · clique num nó pra expandir · arraste pra navegar · roda = zoom` : 'digite um tema e escolha na lista — o mapa nasce dele'}
        </span>
        {(sugestoes.length > 0 || (sugVazio && tema.trim().length >= 2)) && (
          <div className="absolute left-6 top-11 z-20 w-[320px] rounded-md border border-[var(--color-border)] bg-[var(--color-panel)] py-1 shadow-xl">
            {sugestoes.map((s) => (
              <button key={s.valor_norm} onClick={() => centrar(s.valor, s.valor_norm)}
                className="block w-full px-3 py-1.5 text-left text-[13px] hover:bg-[var(--color-panel-2)]">
                {s.valor}
              </button>
            ))}
            {sugestoes.length === 0 && (
              <div className="px-3 py-2 text-[12px] text-[var(--color-muted)]">
                nenhum assunto casa com "{tema.trim()}"{escopo === '*' ? ' em nenhuma coleção' : <> na coleção <b>{escopo}</b> — tente «🌐 todas»</>}.
              </div>
            )}
          </div>
        )}
      </div>

      {error && <div className="text-[12px] text-[var(--color-crit)]">{error}</div>}

      {/* o mapa */}
      <svg
        ref={svgRef}
        onMouseDown={onDown} onMouseMove={onMove} onMouseUp={() => (drag.current = null)} onMouseLeave={() => (drag.current = null)}
        className="h-[calc(100vh-260px)] min-h-[420px] w-full cursor-grab touch-none rounded-lg border border-[var(--color-border)] bg-[var(--color-panel-2)] active:cursor-grabbing"
      >
        <g transform={`translate(${view.x + (svgRef.current?.clientWidth ?? 900) / 2}, ${view.y + (svgRef.current?.clientHeight ?? 500) / 2}) scale(${view.zoom})`}>
          {/* arestas */}
          {[...edges].map((ek) => {
            const [a, b] = ek.split('|')
            const na = nodes.get(a); const nb = nodes.get(b)
            if (!na || !nb) return null
            return <line key={ek} x1={na.x} y1={na.y} x2={nb.x} y2={nb.y} stroke="var(--color-border)" strokeWidth={1.4} style={{ animation: 'navpop .5s ease-out' }} />
          })}
          {/* nós */}
          {lista.map((n) => {
            const r = raioDe(n)
            return (
              <g
                key={n.norm} className="cursor-pointer" style={{ animation: 'navpop .45s ease-out' }}
                onClick={(e) => {
                  e.stopPropagation()
                  // clique simples espera 260ms — se vier o segundo, é duplo (referências)
                  if (clickTimer.current) return
                  clickTimer.current = setTimeout(() => { clickTimer.current = null; expandir(n.norm) }, 260)
                }}
                onDoubleClick={(e) => {
                  e.stopPropagation()
                  if (clickTimer.current) { clearTimeout(clickTimer.current); clickTimer.current = null }
                  setRefs({ valor: n.valor })
                }}
              >
                <circle
                  cx={n.x} cy={n.y} r={r}
                  fill={n.expanded ? 'var(--color-accent)' : 'var(--color-panel)'}
                  fillOpacity={n.expanded ? 0.25 : 1}
                  stroke={busy === n.norm ? 'var(--color-warn)' : n.expanded ? 'var(--color-accent)' : 'var(--color-muted)'}
                  strokeWidth={n.expanded ? 2 : 1.4}
                />
                <text
                  x={n.x} y={n.y + r + 13} textAnchor="middle"
                  className="select-none"
                  style={{ fill: 'var(--color-fg)', fontSize: 11, fontWeight: n.expanded ? 700 : 400 }}
                >
                  {n.valor.length > 26 ? n.valor.slice(0, 25) + '…' : n.valor}
                </text>
                <text x={n.x} y={n.y + 4} textAnchor="middle" className="select-none" style={{ fill: 'var(--color-muted)', fontSize: 9 }}>
                  {n.peso}
                </text>
              </g>
            )
          })}
          {nodes.size === 0 && (
            <text x={0} y={0} textAnchor="middle" style={{ fill: 'var(--color-muted)', fontSize: 13 }}>
              o pensamento começa por um tema ☝ (clique expande · duplo clique abre o corpus)
            </text>
          )}
        </g>
      </svg>

      {/* referências ficam VIVAS por baixo do documento — fechar o doc volta pra lista
          (folhear referência a referência sem repetir o duplo clique) */}
      {refs && <RefsModal valor={refs.valor} escopo={escopo} docAberto={!!inspect}
        onClose={() => setRefs(null)} onOpenDoc={(t) => setInspect(t)} />}
      {inspect && <ChunkModal target={inspect} onClose={() => setInspect(null)} />}
    </div>
  )
}

// ───────── modal de referências: o valor buscado no CORPUS (a camada RAGnaRock) ─────────
function RefsModal({ valor, escopo, docAberto, onClose, onOpenDoc }: {
  valor: string
  escopo: string
  docAberto: boolean   // ChunkModal empilhado por cima — o Esc é dele, não desta
  onClose: () => void
  onOpenDoc: (t: ChunkTarget) => void
}) {
  const [res, setRes] = useState<SearchResponse | null>(null)
  const [error, setError] = useState<string | null>(null)

  useEffect(() => {
    let alive = true
    search(valor, { collection: escopo === '*' ? undefined : escopo, k: 12 })
      .then((r) => { if (alive) setRes(r) })
      .catch((e) => { if (alive) setError(messageFromError(e)) })
    return () => { alive = false }
  }, [valor, escopo])

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => { if (e.key === 'Escape' && !docAberto) onClose() }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [onClose, docAberto])

  return (
    <div className="fixed inset-0 z-40 flex items-center justify-center bg-black/50" onClick={onClose}>
      <div onClick={(e) => e.stopPropagation()}
        className="flex max-h-[80vh] w-[720px] max-w-[92vw] flex-col rounded-lg border border-[var(--color-border)] bg-[var(--color-panel)]">
        <header className="flex items-center justify-between border-b border-[var(--color-border)] px-4 py-3">
          <div className="text-[14px] font-semibold">
            referências no corpus — <span className="text-[var(--color-accent)]">{valor}</span>
            <span className="ml-2 text-[11px] font-normal text-[var(--color-muted)]">
              {escopo === '*' ? 'todas as coleções' : `coleção ${escopo}`}
            </span>
          </div>
          <button onClick={onClose} className="rounded p-1 text-[var(--color-muted)] hover:text-[var(--color-fg)]"><X size={16} /></button>
        </header>
        <div className="min-h-0 flex-1 space-y-2 overflow-y-auto p-4">
          {error && <div className="text-[12px] text-[var(--color-crit)]">{error}</div>}
          {!res && !error && <Spinner label="buscando no corpus…" />}
          {res?.hits.map((h) => (
            <button
              key={`${h.collection}-${h.base}-${h.chunk}`}
              onClick={() => onOpenDoc({ collection: h.collection, base: h.base, id: h.chunk })}
              title="abrir o documento chunk a chunk"
              className="block w-full rounded-md border border-[var(--color-border)] bg-[var(--color-panel-2)] p-3 text-left transition-colors hover:border-[var(--color-accent)]"
            >
              <div className="mb-1 flex items-center justify-between text-[11px] text-[var(--color-muted)]">
                <span><span className="text-[var(--color-accent)]">{h.collection}</span> / {h.base} · chunk {h.chunk}</span>
                <span className="tabular-nums">cos {(h.cos ?? 0).toFixed(3)}</span>
              </div>
              <div className="text-[13px] leading-relaxed">
                {(h.snippet ?? '').split(/[«»]/).map((p, i) =>
                  i % 2 === 1 ? <b key={i} className="text-[var(--color-accent)]">{p}</b> : <span key={i}>{p}</span>)}
              </div>
            </button>
          ))}
          {res && res.hits.length === 0 && <div className="text-[13px] text-[var(--color-muted)]">nenhuma referência no corpus para este escopo.</div>}
        </div>
      </div>
    </div>
  )
}
