import { useRef, useState } from 'react'
import { Crosshair, RotateCcw } from 'lucide-react'
import { getNavNode, getNavSuggest } from '@/api/ragnarock'
import { messageFromError } from '@/api/client'

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

export function ThinkNavigator({ collection }: { collection: string }) {
  const [nodes, setNodes] = useState<Map<string, NavMapNode>>(new Map())
  const [edges, setEdges] = useState<Set<string>>(new Set())
  const [busy, setBusy] = useState<string | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [tema, setTema] = useState('')
  const [sugestoes, setSugestoes] = useState<{ valor: string; valor_norm: string }[]>([])
  const [sugVazio, setSugVazio] = useState(false)
  const [view, setView] = useState({ x: 0, y: 0, zoom: 1 })
  const drag = useRef<{ px: number; py: number; vx: number; vy: number } | null>(null)
  const svgRef = useRef<SVGSVGElement>(null)

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
    drag.current = { px: e.clientX, py: e.clientY, vx: view.x, vy: view.y }
  }
  function onMove(e: React.MouseEvent) {
    if (!drag.current) return
    setView((v) => ({ ...v, x: drag.current!.vx + (e.clientX - drag.current!.px), y: drag.current!.vy + (e.clientY - drag.current!.py) }))
  }
  function onWheel(e: React.WheelEvent) {
    setView((v) => ({ ...v, zoom: Math.min(2.5, Math.max(0.25, v.zoom * (e.deltaY < 0 ? 1.12 : 0.89))) }))
  }

  const lista = [...nodes.values()]
  const raioDe = (n: NavMapNode) => Math.min(34, 14 + Math.sqrt(n.peso) * 1.4)

  return (
    <div className="space-y-3">
      {/* tema central */}
      <div className="relative flex flex-wrap items-center gap-2">
        <Crosshair size={15} className="text-[var(--color-muted)]" />
        <input
          value={tema}
          onChange={(e) => buscarTema(e.target.value)}
          placeholder={`tema central em «${collection}» (ex.: Jesus, Gandalf)…`}
          className="w-[300px] rounded-md border border-[var(--color-border)] bg-[var(--color-panel-2)] px-3 py-2 text-[13px] outline-none focus:border-[var(--color-accent)]"
        />
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
                nenhum assunto casa com "{tema.trim()}" na coleção <b>{collection}</b> — troque a coleção nas abas acima (ex.: Jesus vive em «livros»).
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
        onWheel={onWheel}
        className="h-[calc(100vh-260px)] min-h-[420px] w-full cursor-grab rounded-lg border border-[var(--color-border)] bg-[var(--color-panel-2)] active:cursor-grabbing"
      >
        <g transform={`translate(${view.x + (svgRef.current?.clientWidth ?? 900) / 2}, ${view.y + (svgRef.current?.clientHeight ?? 500) / 2}) scale(${view.zoom})`}>
          {/* arestas */}
          {[...edges].map((ek) => {
            const [a, b] = ek.split('|')
            const na = nodes.get(a); const nb = nodes.get(b)
            if (!na || !nb) return null
            return <line key={ek} x1={na.x} y1={na.y} x2={nb.x} y2={nb.y} stroke="var(--color-border)" strokeWidth={1.4} />
          })}
          {/* nós */}
          {lista.map((n) => {
            const r = raioDe(n)
            return (
              <g key={n.norm} className="cursor-pointer" onClick={(e) => { e.stopPropagation(); expandir(n.norm) }}>
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
              o pensamento começa por um tema ☝
            </text>
          )}
        </g>
      </svg>
    </div>
  )
}
