import { useRef, useState } from 'react'
import { UploadCloud, FileText, X } from 'lucide-react'
import { useAsync } from '@/hooks/useAsync'
import { getCollections, ingestAny } from '@/api/ragnarock'
import { messageFromError } from '@/api/client'
import { Panel, Dot } from '@/components/ui'

// Formatos que a UI aceita: fonte/texto + os que os drivers de ingestão convertem.
const SRC_EXT = /\.(cs|sql|sh|py|yml|yaml|json|md|markdown|ini|toml|csproj|js|mjs|ts|jsx|tsx|java|go|rs|rb|php|swift|kt|kts|c|cc|cpp|cxx|h|hpp|cshtml|razor|html|css|scss|xml|txt|pas|vb|csv|pdf|docx|xlsx)$/i
const ING_MAX_MB = 64
const ING_MAX = ING_MAX_MB * 1024 * 1024

type Status = 'fila' | 'enviando' | 'ok' | 'erro'
interface Row {
  file: File
  status: Status
  driver?: string
  chunks?: number
  error?: string
}

// nome da base: mantém a extensão como sufixo (_pdf/_docx) pra irmãos multi-formato não colidirem.
function baseName(f: File): string {
  const rel = (f as File & { webkitRelativePath?: string }).webkitRelativePath || f.name
  return rel.replace(/[\/\\]/g, '__').replace(/\.([^.]+)$/, '_$1')
}

function fmtSize(b: number): string {
  if (b < 1024) return `${b} B`
  if (b < 1024 * 1024) return `${(b / 1024).toFixed(0)} KB`
  return `${(b / 1024 / 1024).toFixed(1)} MB`
}

export function Ingestao() {
  const cols = useAsync(getCollections, [])
  const [rows, setRows] = useState<Row[]>([])
  const [coll, setColl] = useState('')
  const [chunk, setChunk] = useState(2048)
  const [dragOver, setDragOver] = useState(false)
  const [running, setRunning] = useState(false)
  const [ignored, setIgnored] = useState(0)
  const inputRef = useRef<HTMLInputElement>(null)

  function addFiles(list: FileList | null) {
    if (!list) return
    const all = [...list]
    const accepted = all.filter((f) => SRC_EXT.test(f.name) && f.size < ING_MAX)
    setIgnored(all.length - accepted.length)
    setRows((prev) => {
      const seen = new Set(prev.map((r) => r.file.name + r.file.size))
      const fresh = accepted.filter((f) => !seen.has(f.name + f.size)).map((f) => ({ file: f, status: 'fila' as Status }))
      return [...prev, ...fresh]
    })
  }

  function onDrop(e: React.DragEvent) {
    e.preventDefault()
    setDragOver(false)
    addFiles(e.dataTransfer.files)
  }

  function removeRow(i: number) {
    setRows((prev) => prev.filter((_, idx) => idx !== i))
  }

  async function subir() {
    if (!rows.length || running) return
    setRunning(true)
    const collection = coll.trim() || 'default'
    for (let i = 0; i < rows.length; i++) {
      if (rows[i].status === 'ok') continue
      setRows((prev) => prev.map((r, idx) => (idx === i ? { ...r, status: 'enviando' } : r)))
      try {
        const res = await ingestAny(rows[i].file, { collection, name: baseName(rows[i].file), chunk })
        setRows((prev) => prev.map((r, idx) =>
          idx === i ? { ...r, status: res.ok ? 'ok' : 'erro', driver: res.driver, chunks: res.n_chunks, error: res.error } : r))
      } catch (e) {
        setRows((prev) => prev.map((r, idx) => (idx === i ? { ...r, status: 'erro', error: messageFromError(e) } : r)))
      }
    }
    setRunning(false)
    cols.reload()
  }

  const done = rows.filter((r) => r.status === 'ok')
  const failed = rows.filter((r) => r.status === 'erro')
  const totalChunks = done.reduce((s, r) => s + (r.chunks ?? 0), 0)

  return (
    <div className="space-y-5">
      <h1 className="text-lg font-semibold">Ingestão de documentos</h1>

      {/* dropzone */}
      <div
        onDragOver={(e) => { e.preventDefault(); setDragOver(true) }}
        onDragLeave={() => setDragOver(false)}
        onDrop={onDrop}
        onClick={() => inputRef.current?.click()}
        className={`flex cursor-pointer flex-col items-center justify-center gap-2 rounded-lg border-2 border-dashed py-12 transition-colors ${
          dragOver ? 'border-[var(--color-accent)] bg-[var(--color-accent)]/10' : 'border-[var(--color-border)] bg-[var(--color-panel)] hover:border-[var(--color-muted)]'
        }`}
      >
        <UploadCloud size={32} className="text-[var(--color-muted)]" />
        <div className="text-[14px]">Arraste documentos aqui ou <span className="text-[var(--color-accent)]">clique pra escolher</span></div>
        <div className="text-[11px] text-[var(--color-muted)]">PDF · DOCX · XLSX · CSV · código-fonte e texto — até {ING_MAX_MB} MB por arquivo</div>
        <input
          ref={inputRef}
          type="file"
          multiple
          className="hidden"
          onChange={(e) => { addFiles(e.target.files); e.target.value = '' }}
        />
      </div>

      {/* controles */}
      <Panel>
        <div className="flex flex-wrap items-end gap-4">
          <div className="grow">
            <label className="mb-1 block text-[11px] uppercase tracking-wider text-[var(--color-muted)]">Coleção (existente ou nova)</label>
            <input
              value={coll}
              onChange={(e) => setColl(e.target.value)}
              list="colls"
              placeholder="default"
              className="w-full rounded-md border border-[var(--color-border)] bg-[var(--color-panel-2)] px-3 py-2 text-[14px] outline-none focus:border-[var(--color-accent)]"
            />
            <datalist id="colls">
              {cols.data?.collections.map((c) => <option key={c.collection} value={c.collection} />)}
            </datalist>
          </div>
          <div>
            <label className="mb-1 block text-[11px] uppercase tracking-wider text-[var(--color-muted)]">Chunk (chars)</label>
            <input
              type="number"
              value={chunk}
              onChange={(e) => setChunk(+e.target.value || 2048)}
              className="w-28 rounded-md border border-[var(--color-border)] bg-[var(--color-panel-2)] px-3 py-2 text-[14px] outline-none focus:border-[var(--color-accent)]"
            />
          </div>
          <button
            onClick={subir}
            disabled={!rows.length || running}
            className="rounded-md bg-[var(--color-accent)] px-5 py-2 text-[13px] font-semibold text-[var(--color-accent-fg)] hover:opacity-90 disabled:cursor-not-allowed disabled:opacity-40"
          >
            {running ? 'enviando…' : `subir ${rows.length || ''}`.trim()}
          </button>
          {rows.length > 0 && !running && (
            <button onClick={() => { setRows([]); setIgnored(0) }} className="rounded-md border border-[var(--color-border)] px-3 py-2 text-[13px] text-[var(--color-muted)] hover:bg-[var(--color-panel-2)]">
              limpar
            </button>
          )}
        </div>
        {(rows.length > 0 || ignored > 0) && (
          <div className="mt-3 text-[11px] text-[var(--color-muted)]">
            {rows.length} arquivo(s) na fila{ignored > 0 && ` · ${ignored} ignorado(s) (formato não suportado ou > ${ING_MAX_MB} MB)`}
            {done.length > 0 && <> · <span className="text-[var(--color-ok)]">{done.length} ok</span></>}
            {failed.length > 0 && <> · <span className="text-[var(--color-crit)]">{failed.length} falha(s)</span></>}
            {totalChunks > 0 && <> · {totalChunks.toLocaleString('pt-BR')} chunks</>}
          </div>
        )}
      </Panel>

      {/* lista de arquivos */}
      {rows.length > 0 && (
        <Panel title="Fila">
          <table className="w-full text-[13px]">
            <thead>
              <tr className="border-b border-[var(--color-border)] text-left text-[11px] uppercase tracking-wider text-[var(--color-muted)]">
                <th className="pb-2 font-medium">Arquivo</th>
                <th className="pb-2 text-right font-medium">Tamanho</th>
                <th className="pb-2 font-medium">Status</th>
                <th className="pb-2 w-8"></th>
              </tr>
            </thead>
            <tbody>
              {rows.map((r, i) => (
                <tr key={r.file.name + r.file.size} className="border-b border-[var(--color-border)]/50">
                  <td className="py-2">
                    <span className="flex items-center gap-2"><FileText size={14} className="shrink-0 text-[var(--color-muted)]" /> {r.file.name}</span>
                  </td>
                  <td className="py-2 text-right tabular-nums text-[var(--color-muted)]">{fmtSize(r.file.size)}</td>
                  <td className="py-2">
                    {r.status === 'fila' && <span className="text-[var(--color-muted)]">na fila</span>}
                    {r.status === 'enviando' && <span className="text-[var(--color-accent)]">enviando…</span>}
                    {r.status === 'ok' && (
                      <span className="flex items-center gap-1.5 text-[var(--color-ok)]">
                        <Dot on /> ok · {r.chunks} chunks{r.driver && <span className="text-[var(--color-muted)]">· {r.driver}</span>}
                      </span>
                    )}
                    {r.status === 'erro' && <span className="text-[var(--color-crit)]">{r.error || 'erro'}</span>}
                  </td>
                  <td className="py-2 text-right">
                    {!running && r.status !== 'ok' && (
                      <button onClick={() => removeRow(i)} className="text-[var(--color-muted)] hover:text-[var(--color-crit)]" title="remover"><X size={14} /></button>
                    )}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </Panel>
      )}
    </div>
  )
}
