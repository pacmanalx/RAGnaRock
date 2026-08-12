import { useState } from 'react'
import { search } from '@/api/ragnarock'
import type { SearchResponse } from '@/api/types'
import { messageFromError } from '@/api/client'
import { Panel, Spinner, ErrorBox } from '@/components/ui'
import { ChunkModal, type ChunkTarget } from '@/components/ChunkModal'

export function Buscar() {
  const [q, setQ] = useState('Frodo Bolseiro')
  const [res, setRes] = useState<SearchResponse | null>(null)
  const [loading, setLoading] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [inspect, setInspect] = useState<ChunkTarget | null>(null)

  async function run(e?: React.FormEvent) {
    e?.preventDefault()
    if (!q.trim()) return
    setLoading(true); setError(null)
    try { setRes(await search(q.trim(), 10)) }
    catch (err) { setError(messageFromError(err)) }
    finally { setLoading(false) }
  }

  return (
    <div className="space-y-5">
      <h1 className="text-lg font-semibold">Buscar</h1>

      <form onSubmit={run} className="flex gap-2">
        <input
          value={q}
          onChange={(e) => setQ(e.target.value)}
          placeholder="consulta em linguagem natural…"
          className="flex-1 rounded-md border border-[var(--color-border)] bg-[var(--color-panel-2)] px-3 py-2 text-[14px] outline-none focus:border-[var(--color-accent)]"
        />
        <button className="rounded-md bg-[var(--color-accent)] px-4 py-2 text-[13px] font-semibold text-[var(--color-accent-fg)] hover:opacity-90">buscar</button>
      </form>

      {error && <ErrorBox message={error} onRetry={() => run()} />}
      {loading && <Spinner label="buscando…" />}

      {res && !loading && (
        <Panel title={`${res.hits.length} resultado(s)`} actions={
          res.query_syllables ? <span className="text-[11px] text-[var(--color-muted)]">sílabas: {res.query_syllables}</span> : null
        }>
          <div className="space-y-2">
            {res.hits.map((h) => (
              <button
                key={`${h.base}-${h.chunk}`}
                onClick={() => setInspect({ collection: h.collection, base: h.base, id: h.chunk })}
                className="block w-full rounded-md border border-[var(--color-border)] bg-[var(--color-panel-2)] p-3 text-left transition-colors hover:border-[var(--color-accent)]"
                title="abrir e navegar o documento chunk a chunk"
              >
                <div className="mb-1 flex items-center justify-between text-[11px] text-[var(--color-muted)]">
                  <span><span className="text-[var(--color-accent)]">{h.collection}</span> / {h.base} · chunk {h.chunk}</span>
                  <span className="tabular-nums">mf {h.matchpoint.toFixed(2)} · cos {h.cos.toFixed(3)}</span>
                </div>
                <div className="text-[13px] leading-relaxed">{h.snippet}</div>
              </button>
            ))}
            {res.hits.length === 0 && <div className="text-[13px] text-[var(--color-muted)]">nada encontrado.</div>}
          </div>
        </Panel>
      )}

      {inspect && <ChunkModal target={inspect} onClose={() => setInspect(null)} />}
    </div>
  )
}
