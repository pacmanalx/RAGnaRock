import { useCallback, useEffect, useState } from 'react'
import { ChevronsLeft, ChevronLeft, ChevronRight, ChevronsRight, X, Download } from 'lucide-react'
import { fetchChunk, fetchDocument } from '@/api/ragnarock'
import type { ChunkData } from '@/api/types'
import { messageFromError } from '@/api/client'

// Modal de inspeção de chunk — copia o comportamento do dashboard legado: navega chunk a chunk
// (início / anterior / próximo / fim), teclado ←/→/Home/End/Esc, + download do documento em .md.
export interface ChunkTarget {
  collection: string
  base: string
  id: number
}

export function ChunkModal({ target, onClose }: { target: ChunkTarget; onClose: () => void }) {
  const [id, setId] = useState(target.id)
  const [corpus, setCorpus] = useState<string>('')
  const [nChunks, setNChunks] = useState<number>(0)
  const [chunk, setChunk] = useState<ChunkData | null>(null)
  const [loading, setLoading] = useState(true)
  const [error, setError] = useState<string | null>(null)
  const [downloading, setDownloading] = useState(false)

  const path = target.base.replaceAll('__', '/')

  const load = useCallback(async (cid: number) => {
    setLoading(true); setError(null)
    try {
      const r = await fetchChunk(target.collection, target.base, cid)
      setCorpus(r.corpus ?? '?')
      setNChunks(r.n_chunks ?? 0)
      setChunk(r.chunks[0] ?? null)
    } catch (e) { setError(messageFromError(e)) }
    finally { setLoading(false) }
  }, [target.collection, target.base])

  useEffect(() => { load(id) }, [id, load])

  const last = nChunks > 0 ? nChunks - 1 : id

  // teclado: ←/→ navega, Home/End extremos, Esc fecha
  useEffect(() => {
    function onKey(e: KeyboardEvent) {
      if (e.key === 'Escape') onClose()
      else if (e.key === 'ArrowLeft') setId((c) => Math.max(0, c - 1))
      else if (e.key === 'ArrowRight') setId((c) => (nChunks > 0 ? Math.min(last, c + 1) : c + 1))
      else if (e.key === 'Home') setId(0)
      else if (e.key === 'End' && nChunks > 0) setId(last)
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [onClose, last, nChunks])

  async function baixarMd() {
    if (!nChunks) return
    setDownloading(true)
    try {
      const doc = await fetchDocument(target.collection, target.base, nChunks)
      const md = doc.chunks.map((c) => c.text ?? '').join('\n\n')
      const blob = new Blob([md], { type: 'text/markdown;charset=utf-8' })
      const url = URL.createObjectURL(blob)
      const a = document.createElement('a')
      a.href = url
      a.download = `${target.base}.md`
      a.click()
      URL.revokeObjectURL(url)
    } catch (e) { setError(messageFromError(e)) }
    finally { setDownloading(false) }
  }

  const navbtn = 'rounded border border-[var(--color-border)] p-1.5 text-[var(--color-muted)] hover:border-[var(--color-accent)] hover:text-[var(--color-accent)] disabled:opacity-30 disabled:hover:border-[var(--color-border)] disabled:hover:text-[var(--color-muted)]'

  const pill = (label: string, value: React.ReactNode) => (
    <span className="rounded bg-[var(--color-panel-2)] px-2 py-0.5 text-[11px] text-[var(--color-muted)]">
      {label} <span className="text-[var(--color-fg)]">{value}</span>
    </span>
  )

  return (
    <div
      onClick={(e) => { if (e.target === e.currentTarget) onClose() }}
      className="fixed inset-0 z-50 flex items-start justify-center overflow-auto bg-black/60 p-8"
    >
      <div className="w-full max-w-4xl rounded-lg border border-[var(--color-border)] bg-[var(--color-panel)] shadow-2xl">
        {/* header */}
        <div className="flex items-center gap-3 border-b border-[var(--color-border)] bg-[var(--color-panel-2)] px-4 py-3">
          <div className="min-w-0 grow">
            <div className="truncate text-[13px] font-semibold text-[var(--color-ok)]">{corpus || '…'}</div>
            <div className="truncate text-[11px] text-[var(--color-muted)]">{target.collection} · {path}</div>
          </div>
          <button onClick={baixarMd} disabled={downloading || !nChunks} title="baixar documento completo (.md)"
            className="flex items-center gap-1.5 rounded-md border border-[var(--color-border)] px-2.5 py-1.5 text-[12px] hover:border-[var(--color-accent)] hover:text-[var(--color-accent)] disabled:opacity-40">
            <Download size={14} /> {downloading ? 'baixando…' : '.md'}
          </button>
          <button onClick={onClose} className="rounded p-1.5 text-[var(--color-muted)] hover:bg-[var(--color-panel)] hover:text-[var(--color-fg)]"><X size={18} /></button>
        </div>

        {/* nav */}
        <div className="flex items-center gap-2 border-b border-[var(--color-border)] px-4 py-2">
          <button onClick={() => setId(0)} disabled={id <= 0} title="início (Home)" className={navbtn}><ChevronsLeft size={16} /></button>
          <button onClick={() => setId((c) => Math.max(0, c - 1))} disabled={id <= 0} title="anterior (←)" className={navbtn}><ChevronLeft size={16} /></button>
          <span className="min-w-[120px] text-center text-[12px] tabular-nums text-[var(--color-muted)]">
            chunk <span className="text-[var(--color-fg)]">{id}</span>{nChunks ? ` / ${last}` : ''}
          </span>
          <button onClick={() => setId((c) => (nChunks > 0 ? Math.min(last, c + 1) : c + 1))} disabled={nChunks > 0 && id >= last} title="próximo (→)" className={navbtn}><ChevronRight size={16} /></button>
          <button onClick={() => setId(last)} disabled={!nChunks || id >= last} title="fim (End)" className={navbtn}><ChevronsRight size={16} /></button>
        </div>

        {/* body */}
        <div className="max-h-[65vh] overflow-auto p-4">
          {error && <div className="text-[13px] text-[var(--color-crit)]">falha: {error}</div>}
          {loading && !chunk && <div className="py-8 text-center text-[13px] text-[var(--color-muted)]">carregando…</div>}
          {!loading && !chunk && !error && <div className="py-8 text-center text-[13px] text-[var(--color-muted)]">(chunk inexistente — fim da base)</div>}
          {chunk && (
            <>
              <div className="mb-3 flex flex-wrap gap-1.5">
                {pill('id', chunk.id)}
                {pill('start', chunk.start.toLocaleString('pt-BR'))}
                {pill('len', chunk.len.toLocaleString('pt-BR'))}
                {pill('tokens', chunk.tokens.toLocaleString('pt-BR'))}
                {pill('oov', chunk.oov)}
                {pill('norm', chunk.norm?.toFixed(4))}
              </div>
              <pre className="whitespace-pre-wrap break-words rounded-md border border-[var(--color-border)] bg-[var(--color-panel-2)] p-3 text-[13px] leading-relaxed">
                {chunk.text ?? '(base sem texto guardado)'}
              </pre>
            </>
          )}
        </div>
      </div>
    </div>
  )
}
