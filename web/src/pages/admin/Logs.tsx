import { useEffect, useRef, useState } from 'react'
import { Pause, Play, RefreshCw } from 'lucide-react'
import { getLogs } from '@/api/ragnarock'
import { messageFromError } from '@/api/client'
import { ErrorBox } from '@/components/ui'

// Tail ao vivo do log do ragd (GET /logs?n= — guard admin.servicos). O arquivo registra
// TODAS as requests (tags [api] e [valhalla]) + eventos do motor (slog da busca expandida).
// O nidhoggd loga no journald (systemd) — ver `journalctl -u nidhoggd` no servidor.

const LINHAS = [100, 300, 1000, 5000]

export function Logs() {
  const [n, setN] = useState(300)
  const [live, setLive] = useState(true)
  const [filtro, setFiltro] = useState('')
  const [file, setFile] = useState('')
  const [log, setLog] = useState('')
  const [error, setError] = useState<string | null>(null)
  const [ts, setTs] = useState<Date | null>(null)
  const preRef = useRef<HTMLPreElement>(null)
  const stickBottom = useRef(true)

  async function carregar(nLinhas: number) {
    try {
      const r = await getLogs(nLinhas)
      setFile(r.file); setLog(r.log); setError(null); setTs(new Date())
    } catch (e) { setError(messageFromError(e)) }
  }

  // poll de 3s enquanto "ao vivo"; busca imediata ao mudar n
  useEffect(() => {
    carregar(n)
    if (!live) return
    const id = setInterval(() => carregar(n), 3000)
    return () => clearInterval(id)
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [n, live])

  // autoscroll pro fim SÓ se o usuário já estava no fim (não rouba o scroll de quem leu pra trás)
  useEffect(() => {
    const el = preRef.current
    if (el && stickBottom.current) el.scrollTop = el.scrollHeight
  }, [log, filtro])

  function onScroll() {
    const el = preRef.current
    if (el) stickBottom.current = el.scrollHeight - el.scrollTop - el.clientHeight < 40
  }

  const linhas = filtro.trim()
    ? log.split('\n').filter((l) => l.toLowerCase().includes(filtro.trim().toLowerCase())).join('\n')
    : log

  const inputCls = 'rounded-md border border-[var(--color-border)] bg-[var(--color-panel-2)] px-3 py-1.5 text-[12px] outline-none focus:border-[var(--color-accent)]'

  return (
    <div className="flex h-full flex-col space-y-3">
      <div className="flex flex-wrap items-center gap-3">
        <h1 className="text-lg font-semibold">Logs</h1>
        <span className="font-mono text-[11px] text-[var(--color-muted)]">{file}</span>
        <div className="grow" />
        <input
          value={filtro}
          onChange={(e) => setFiltro(e.target.value)}
          placeholder="filtrar (ex: search, POST, 404)…"
          className={`w-[220px] ${inputCls}`}
        />
        <select value={n} onChange={(e) => setN(+e.target.value)} className={inputCls}>
          {LINHAS.map((l) => <option key={l} value={l}>{l} linhas</option>)}
        </select>
        <button
          onClick={() => setLive((v) => !v)}
          title={live ? 'pausar o acompanhamento (3s)' : 'retomar o acompanhamento'}
          className={`flex items-center gap-1.5 rounded-md border px-3 py-1.5 text-[12px] font-semibold ${
            live
              ? 'border-[var(--color-ok)] text-[var(--color-ok)]'
              : 'border-[var(--color-border)] text-[var(--color-muted)]'
          }`}
        >
          {live ? <><Pause size={13} /> ao vivo</> : <><Play size={13} /> pausado</>}
        </button>
        <button onClick={() => carregar(n)} title="atualizar agora" className="rounded-md border border-[var(--color-border)] p-2 text-[var(--color-muted)] hover:text-[var(--color-fg)]">
          <RefreshCw size={13} />
        </button>
      </div>

      {error && <ErrorBox message={error} onRetry={() => carregar(n)} />}

      <pre
        ref={preRef}
        onScroll={onScroll}
        className="min-h-0 grow overflow-auto rounded-lg border border-[var(--color-border)] bg-[var(--color-panel-2)] p-4 font-mono text-[11px] leading-[1.55]"
      >
        {linhas || '(vazio)'}
      </pre>

      <div className="flex items-center justify-between text-[11px] text-[var(--color-muted)]">
        <span>nidhoggd loga no journald — <code>journalctl -u nidhoggd -f</code> no servidor</span>
        {ts && <span>atualizado {ts.toLocaleTimeString('pt-BR')}{live ? ' · a cada 3s' : ''}</span>}
      </div>
    </div>
  )
}
