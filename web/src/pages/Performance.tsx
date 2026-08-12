import { useEffect, useRef, useState } from 'react'
import { search, getHistogram, getCollections } from '@/api/ragnarock'
import type { HistogramResponse } from '@/api/types'
import { messageFromError } from '@/api/client'
import { useAsync } from '@/hooks/useAsync'
import { useThemeStore } from '@/store/themeStore'
import { Panel, ErrorBox } from '@/components/ui'

// ───────────────────────── rampa de calor (intensidade do sinal) ─────────────────────────
// Herdada do dashboard legado: frio (azul) → quente (vermelho). É encoding de intensidade
// física do matched filter, não paleta categórica — a legenda gradiente explica a régua.
const HEAT: [number, [number, number, number]][] = [
  [0, [68, 119, 255]], [0.25, [34, 221, 204]], [0.5, [63, 185, 80]],
  [0.7, [255, 216, 61]], [0.85, [255, 140, 43]], [1, [255, 77, 77]],
]
function heat(t: number): string {
  t = t < 0 ? 0 : t > 1 ? 1 : t
  for (let i = 1; i < HEAT.length; i++) {
    if (t <= HEAT[i][0]) {
      const [ta, a] = HEAT[i - 1]; const [tb, b] = HEAT[i]
      const u = (t - ta) / (tb - ta)
      return `rgb(${a.map((v, k) => Math.round(v + (b[k] - v) * u)).join(',')})`
    }
  }
  return 'rgb(255,77,77)'
}
const HEATCSS = 'linear-gradient(90deg,#4477ff,#22ddcc,#3fb950,#ffd83d,#ff8c2b,#ff4d4d)'

// Tokens do tema lidos na hora do draw — o canvas redesenha quando o tema troca.
function tone() {
  const s = getComputedStyle(document.documentElement)
  const v = (n: string) => s.getPropertyValue(n).trim()
  return {
    bg: v('--color-panel-2'), grid: v('--color-border'), axis: v('--color-muted'),
    fg: v('--color-fg'), muted: v('--color-muted'), ok: v('--color-ok'), accent: v('--color-accent'),
  }
}

function HeatBar({ label }: { label: string }) {
  return (
    <span className="inline-flex items-center gap-1.5 text-[11px] text-[var(--color-muted)]">
      {label} frio
      <span className="inline-block h-[9px] w-[90px] rounded-full" style={{ background: HEATCSS }} />
      quente
    </span>
  )
}

// ─────────────────────────────── medição de latência ───────────────────────────────
interface PerfStats { avg: number; p50: number; min: number; max: number; n: number }

export function Performance() {
  const cols = useAsync(getCollections, [])
  const theme = useThemeStore((s) => s.theme)
  const [q, setQ] = useState('cálculo de média do aluno')
  const [coll, setColl] = useState('')
  const [reps, setReps] = useState(30)
  const [stats, setStats] = useState<PerfStats | null>(null)
  const [measuring, setMeasuring] = useState(false)
  const [progress, setProgress] = useState(0)
  const [hist, setHist] = useState<HistogramResponse | null>(null)
  const [histLoading, setHistLoading] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const mfRef = useRef<HTMLCanvasElement>(null)
  const histRef = useRef<HTMLCanvasElement>(null)
  const [mfHover, setMfHover] = useState<string | null>(null)

  async function medir() {
    if (!q.trim() || measuring) return
    setMeasuring(true); setError(null); setStats(null)
    const opts = { collection: coll || undefined, k: 5 }
    const ts: number[] = []
    try {
      for (let i = 0; i < reps; i++) {
        const t = performance.now()
        await search(q.trim(), opts)
        ts.push(performance.now() - t)
        setProgress(i + 1)
      }
    } catch (err) { setError(messageFromError(err)); setMeasuring(false); return }
    ts.sort((a, b) => a - b)
    setStats({
      avg: ts.reduce((a, b) => a + b, 0) / ts.length,
      p50: ts[ts.length >> 1], min: ts[0], max: ts[ts.length - 1], n: ts.length,
    })
    setMeasuring(false)
  }

  async function histograma() {
    if (!q.trim() || histLoading) return
    setHistLoading(true); setError(null)
    try { setHist(await getHistogram(q.trim(), { collection: coll || undefined, k: 5 })) }
    catch (err) { setError(messageFromError(err)) }
    finally { setHistLoading(false) }
  }

  // ── matched filter: spikes por posição, coloridos pela fração que casa ──
  useEffect(() => {
    const cv = mfRef.current
    if (!cv) return
    const W = cv.clientWidth || 900, H = 240, dpr = window.devicePixelRatio || 1
    cv.width = W * dpr; cv.height = H * dpr
    const ctx = cv.getContext('2d')!
    ctx.scale(dpr, dpr)
    const t = tone()
    ctx.fillStyle = t.bg; ctx.fillRect(0, 0, W, H)
    const padL = 48, padR = 14, padT = 18, padB = 34, pw = W - padL - padR, ph = H - padT - padB, base = padT + ph
    const d = hist
    if (!d?.found || !d.mf?.length || !d.seq_len) {
      ctx.fillStyle = t.muted; ctx.font = '12px monospace'; ctx.textAlign = 'center'
      ctx.fillText(d?.found ? 'chunk sem texto guardado' : 'rode o histograma acima…', W / 2, base - ph / 2)
      return
    }
    const N = Math.max(1, d.seq_len)
    const X = (p: number) => padL + (p / N) * pw
    const Y = (f: number) => base - f * ph
    ctx.strokeStyle = t.grid; ctx.lineWidth = 1
    ctx.beginPath(); ctx.moveTo(padL, padT); ctx.lineTo(padL, base); ctx.lineTo(padL + pw, base); ctx.stroke()
    ctx.fillStyle = t.muted; ctx.font = '10px monospace'; ctx.textAlign = 'right'
    ;[0, 0.5, 1].forEach((f) => {
      ctx.fillText(f.toFixed(1), padL - 6, Y(f) + 3)
      ctx.strokeStyle = t.grid; ctx.globalAlpha = 0.4
      ctx.beginPath(); ctx.moveTo(padL, Y(f)); ctx.lineTo(padL + pw, Y(f)); ctx.stroke()
      ctx.globalAlpha = 1
    })
    ctx.textAlign = 'center'
    for (let i = 0; i <= 4; i++) { const p = Math.round((N * i) / 4); ctx.fillText(String(p), X(p), base + 15) }
    let best: { peak: number; peak_pos: number; term: string } | null = null
    for (const m of d.mf) {
      for (const [p, f] of m.points) {
        const x = Math.round(X(p)) + 0.5
        ctx.strokeStyle = heat(f); ctx.lineWidth = 2.5 + f * 3.5
        ctx.beginPath(); ctx.moveTo(x, base); ctx.lineTo(x, Y(f)); ctx.stroke()
      }
      if (!best || m.peak > best.peak) best = { peak: m.peak, peak_pos: m.peak_pos, term: m.term }
    }
    // ponto de convergência (maior pico entre os termos)
    if (best && best.peak > 0) {
      const x = X(best.peak_pos), y = Y(best.peak)
      ctx.strokeStyle = t.fg; ctx.fillStyle = heat(best.peak); ctx.lineWidth = 1.5
      ctx.beginPath(); ctx.arc(x, y, 5, 0, 7); ctx.fill(); ctx.stroke()
      const tx = Math.min(x + 22, padL + pw - 250), ty = Math.min(y + 30, base - 14)
      ctx.strokeStyle = t.axis; ctx.beginPath(); ctx.moveTo(tx, ty); ctx.lineTo(x + 6, y + 5); ctx.stroke()
      ctx.fillStyle = t.fg; ctx.font = '11px monospace'; ctx.textAlign = 'left'
      ctx.fillText(`convergência: p=${best.peak_pos} · ${best.term} · ${best.peak.toFixed(2)}`, tx, ty + 13)
    }
    ctx.fillStyle = t.muted; ctx.font = '11px monospace'; ctx.textAlign = 'center'
    ctx.fillText(`posição no chunk (sequência de sílabas, 0..${N})`, padL + pw / 2, H - 6)
  }, [hist, theme])

  // hover do MF: posição na sequência + melhor fração ali por perto
  function onMfMove(e: React.MouseEvent<HTMLCanvasElement>) {
    const d = hist
    if (!d?.found || !d.mf?.length || !d.seq_len) { setMfHover(null); return }
    const cv = mfRef.current!
    const r = cv.getBoundingClientRect()
    const padL = 48, padR = 14, pw = (cv.clientWidth || 900) - padL - padR
    const p = Math.round(((e.clientX - r.left - padL) / pw) * d.seq_len)
    if (p < 0 || p > d.seq_len) { setMfHover(null); return }
    let bestF = 0, bestTerm = ''
    for (const m of d.mf) for (const [pos, f] of m.points) {
      if (Math.abs(pos - p) <= 2 && f > bestF) { bestF = f; bestTerm = m.term }
    }
    setMfHover(bestF > 0 ? `p=${p} · ${bestTerm} · fração ${bestF.toFixed(2)}` : `p=${p} · —`)
  }

  // ── histograma: embedding do chunk (calor) × dimensões da query (verde/azul) ──
  useEffect(() => {
    const cv = histRef.current
    if (!cv) return
    const W = cv.clientWidth || 900, H = 340, dpr = window.devicePixelRatio || 1
    cv.width = W * dpr; cv.height = H * dpr
    const ctx = cv.getContext('2d')!
    ctx.scale(dpr, dpr)
    const t = tone()
    ctx.fillStyle = t.bg; ctx.fillRect(0, 0, W, H)
    const padL = 48, padR = 14, padT = 30, padB = 36, pw = W - padL - padR, ph = H - padT - padB, base = padT + ph
    const d = hist
    if (!d?.found || !d.chunk || !d.query) {
      ctx.fillStyle = t.muted; ctx.font = '13px monospace'; ctx.textAlign = 'center'
      ctx.fillText('rode o histograma acima…', W / 2, base - ph / 2)
      return
    }
    const vocab = Math.max(1, d.vocab_size ?? 1)
    let maxC = 1
    d.chunk.forEach((p) => { maxC = Math.max(maxC, p.c) })
    d.query.forEach((p) => { maxC = Math.max(maxC, p.c) })
    const X = (dim: number) => padL + (dim / vocab) * pw
    const Y = (c: number) => base - (c / maxC) * ph
    ctx.strokeStyle = t.grid; ctx.lineWidth = 1
    ctx.beginPath(); ctx.moveTo(padL, padT); ctx.lineTo(padL, base); ctx.lineTo(padL + pw, base); ctx.stroke()
    ctx.fillStyle = t.muted; ctx.font = '10px monospace'; ctx.textAlign = 'right'
    ctx.fillText('0', padL - 6, base + 3)
    ctx.fillText(String(maxC), padL - 6, padT + 8)
    ctx.fillText(String(Math.floor(maxC / 2)), padL - 6, padT + ph / 2)
    ctx.textAlign = 'center'
    for (let i = 0; i <= 4; i++) {
      const dim = Math.round((vocab * i) / 4), x = X(dim)
      ctx.fillText(String(dim), x, base + 16)
      ctx.strokeStyle = t.grid; ctx.globalAlpha = 0.4
      ctx.beginPath(); ctx.moveTo(x, padT); ctx.lineTo(x, base); ctx.stroke()
      ctx.globalAlpha = 1
    }
    // embedding do chunk: cor pela contagem (gamma pra aquecer) + grossura conforme altura
    d.chunk.forEach((p) => {
      const f = p.c / maxC, x = Math.round(X(p.dim)) + 0.5
      ctx.strokeStyle = heat(Math.pow(f, 0.6)); ctx.lineWidth = 2 + f * 3
      ctx.beginPath(); ctx.moveTo(x, base); ctx.lineTo(x, Y(p.c)); ctx.stroke()
    })
    // dimensões da query: linha de altura total — VERDE converge (conta no cosseno), AZUL só-query
    d.query.forEach((p) => {
      const x = Math.round(X(p.dim)) + 0.5
      ctx.strokeStyle = p.hit ? 'rgba(63,185,80,0.40)' : 'rgba(88,166,255,0.26)'
      ctx.lineWidth = 2.5
      ctx.beginPath(); ctx.moveTo(x, padT); ctx.lineTo(x, base); ctx.stroke()
    })
    d.query.forEach((p) => {
      const x = Math.round(X(p.dim)) + 0.5
      ctx.strokeStyle = p.hit ? '#3fb950' : '#58a6ff'; ctx.lineWidth = 4.5
      ctx.beginPath(); ctx.moveTo(x, base); ctx.lineTo(x, Y(p.c)); ctx.stroke()
    })
    // rótulo da sílaba no topo de cada dimensão da query (zig-zag pra não colidir)
    ctx.font = '9px monospace'; ctx.textAlign = 'center'
    d.query.forEach((p, i) => {
      ctx.fillStyle = p.hit ? '#56d364' : '#79c0ff'
      ctx.fillText(p.syl, X(p.dim), padT - 12 + (i % 2) * 9)
    })
    ctx.fillStyle = t.muted; ctx.font = '11px monospace'; ctx.textAlign = 'center'
    ctx.fillText(`dimensão (índice fixo do vocabulário, 0..${vocab})`, padL + pw / 2, H - 6)
  }, [hist, theme])

  const qHits = hist?.query?.filter((p) => p.hit).length ?? 0
  const inputCls = 'rounded-md border border-[var(--color-border)] bg-[var(--color-panel-2)] px-3 py-2 text-[13px] outline-none focus:border-[var(--color-accent)]'
  const tile = (label: string, v: string) => (
    <div className="rounded-md border border-[var(--color-border)] bg-[var(--color-panel-2)] px-4 py-3">
      <div className="text-[11px] uppercase tracking-wide text-[var(--color-muted)]">{label}</div>
      <div className="mt-0.5 text-[22px] font-semibold tabular-nums">{v}<span className="ml-1 text-[12px] font-normal text-[var(--color-muted)]">ms</span></div>
    </div>
  )

  return (
    <div className="space-y-5">
      <h1 className="text-lg font-semibold">Performance</h1>

      <div className="flex flex-wrap items-end gap-3">
        <div className="min-w-[260px] grow">
          <div className="mb-1 text-[11px] uppercase tracking-wide text-[var(--color-muted)]">query</div>
          <input value={q} onChange={(e) => setQ(e.target.value)} className={`w-full ${inputCls}`} />
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
        <div title="quantas buscas idênticas rodar pra medir a latência">
          <div className="mb-1 text-[11px] uppercase tracking-wide text-[var(--color-muted)]">repetições</div>
          <input type="number" min={1} max={200} value={reps} onChange={(e) => setReps(Math.max(1, +e.target.value || 30))} className={`w-[80px] ${inputCls}`} />
        </div>
        <button
          onClick={medir} disabled={measuring}
          className="rounded-md bg-[var(--color-accent)] px-5 py-2 text-[13px] font-semibold text-[var(--color-accent-fg)] hover:opacity-90 disabled:opacity-50"
        >
          {measuring ? `medindo… ${progress}/${reps}` : 'medir'}
        </button>
        <button
          onClick={histograma} disabled={histLoading}
          className="rounded-md border border-[var(--color-accent)] px-5 py-2 text-[13px] font-semibold text-[var(--color-accent)] hover:bg-[var(--color-accent)] hover:text-[var(--color-accent-fg)] disabled:opacity-50"
        >
          {histLoading ? 'calculando…' : 'histograma'}
        </button>
      </div>

      {error && <ErrorBox message={error} />}

      {stats && (
        <div className="grid grid-cols-2 gap-3 sm:grid-cols-4">
          {tile('média', stats.avg.toFixed(1))}
          {tile('p50', stats.p50.toFixed(1))}
          {tile('mín', stats.min.toFixed(1))}
          {tile('máx', stats.max.toFixed(1))}
          <div className="col-span-2 self-center text-[11px] text-[var(--color-muted)] sm:col-span-4">
            {stats.n} requisições sequenciais, medidas do browser (inclui rede + proxy + motor)
          </div>
        </div>
      )}

      <Panel
        title="Matched filter — query deslizando sobre o chunk"
        actions={
          <span className="flex items-center gap-3 text-[11px] text-[var(--color-muted)]">
            {mfHover && <span className="tabular-nums">{mfHover}</span>}
            <HeatBar label="fração que casa:" />
          </span>
        }
      >
        {hist?.found && (
          <div className="mb-2 text-[12px] text-[var(--color-muted)]">
            <span className="text-[var(--color-accent)]">{hist.collection}/{hist.base}</span> · chunk #{hist.chunk_id} ·
            cov {(hist.coverage ?? 0).toFixed(2)} · cos {(hist.cos ?? 0).toFixed(3)} · sílabas: {hist.query_syllables || '—'}
            {(hist.mf?.length ?? 0) > 0 && (
              <span> · {hist.mf!.map((m) => `${m.term} pico ${m.peak.toFixed(2)}`).join(' · ')}</span>
            )}
          </div>
        )}
        {hist && !hist.found && <div className="mb-2 text-[12px] text-[var(--color-warn,#d29922)]">nenhum chunk convergiu pra essa query neste escopo.</div>}
        <canvas ref={mfRef} onMouseMove={onMfMove} onMouseLeave={() => setMfHover(null)} className="block h-[240px] w-full" />
      </Panel>

      <Panel title="Histograma — query × chunk mais próximo">
        <canvas ref={histRef} className="block h-[340px] w-full" />
        {hist?.found && hist.query && (
          <div className="mt-2 flex flex-wrap gap-4 text-[11px] text-[var(--color-muted)]">
            <span className="flex items-center gap-1.5">chunk (embedding · {hist.chunk?.length ?? 0} dims) — <HeatBar label="contagem:" /></span>
            <span className="flex items-center gap-1.5"><span className="inline-block h-[3px] w-[14px] bg-[#3fb950]" /> query convergente ({qHits} dims no cosseno)</span>
            <span className="flex items-center gap-1.5"><span className="inline-block h-[3px] w-[14px] bg-[#58a6ff]" /> query só-query ({(hist.query.length - qHits)}){hist.query_oov ? ` · ${hist.query_oov} oov` : ''}</span>
          </div>
        )}
      </Panel>
    </div>
  )
}
