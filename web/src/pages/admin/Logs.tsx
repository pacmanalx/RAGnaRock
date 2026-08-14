import { useEffect, useRef, useState } from 'react'
import { ChevronDown, ChevronRight, Pause, Play, RefreshCw } from 'lucide-react'
import { getLogs, getLlmLedger } from '@/api/ragnarock'
import type { LlmLedgerEntry } from '@/api/types'
import { messageFromError } from '@/api/client'
import { ErrorBox, Spinner } from '@/components/ui'

// Duas fontes: o tail do log do ragd (todas as requests + motor) e o DIÁRIO DE MASTIGAÇÃO
// do nidhoggd — toda consulta/resposta de LLM, mesmo interna, pra ver a evolução do
// entendimento ciclo a ciclo ("o esquilinho mastigando").

const LINHAS = [100, 300, 1000, 5000]

export function Logs() {
  const [fonte, setFonte] = useState<'ragd' | 'llm'>('ragd')

  return (
    <div className="flex h-full flex-col space-y-3">
      <div className="flex flex-wrap items-center gap-3">
        <h1 className="text-lg font-semibold">Logs</h1>
        <div className="flex overflow-hidden rounded-md border border-[var(--color-border)]">
          {(['ragd', 'llm'] as const).map((f) => (
            <button key={f} onClick={() => setFonte(f)}
              className={`px-3 py-1.5 text-[12px] font-medium transition-colors ${
                fonte === f ? 'bg-[var(--color-accent)] text-[var(--color-accent-fg)]' : 'bg-[var(--color-panel-2)] text-[var(--color-muted)] hover:text-[var(--color-fg)]'
              }`}>
              {f === 'ragd' ? '📜 ragd' : '🐿️ LLM (nidhogg)'}
            </button>
          ))}
        </div>
        <div className="grow" />
      </div>
      {fonte === 'ragd' ? <LogRagd /> : <DiarioLlm />}
    </div>
  )
}

// ── fonte 1: tail ao vivo do log do ragd (GET /logs?n= — guard admin.servicos) ──
function LogRagd() {
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
    <div className="flex min-h-0 grow flex-col space-y-3">
      <div className="flex flex-wrap items-center gap-3">
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
        <BotaoAoVivo live={live} onToggle={() => setLive((v) => !v)} />
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
        <span>o diário completo das chamadas de IA está na aba 🐿️ LLM</span>
        {ts && <span>atualizado {ts.toLocaleTimeString('pt-BR')}{live ? ' · a cada 3s' : ''}</span>}
      </div>
    </div>
  )
}

// ── fonte 2: diário de mastigação — cada card = 1 chamada de LLM (prompt + resposta) ──
function DiarioLlm() {
  const [n, setN] = useState(30)
  const [live, setLive] = useState(true)
  const [entries, setEntries] = useState<LlmLedgerEntry[]>([])
  const [file, setFile] = useState('')
  const [error, setError] = useState<string | null>(null)
  const [ts, setTs] = useState<Date | null>(null)
  const [carregou, setCarregou] = useState(false)
  const [aberto, setAberto] = useState<number | null>(null)

  async function carregar(qtd: number) {
    try {
      const r = await getLlmLedger(qtd)
      setEntries(r.entries); setFile(r.file); setError(null); setTs(new Date())
    } catch (e) { setError(messageFromError(e)) }
    finally { setCarregou(true) }
  }

  useEffect(() => {
    carregar(n)
    if (!live) return
    const id = setInterval(() => carregar(n), 5000)
    return () => clearInterval(id)
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [n, live])

  const tagCor: Record<string, string> = {
    classificador: 'text-[var(--color-accent)]',
    modelador: 'text-[var(--color-warn)]',
    extrator: 'text-[var(--color-ok)]',
  }
  const fmt = (c: number) => c >= 1_000_000 ? `${(c / 1_000_000).toFixed(1)}M` : c >= 1000 ? `${(c / 1000).toFixed(1)}k` : `${c}`

  return (
    <div className="flex min-h-0 grow flex-col space-y-3">
      <div className="flex flex-wrap items-center gap-3">
        <span className="font-mono text-[11px] text-[var(--color-muted)]">{file}</span>
        <div className="grow" />
        <select value={n} onChange={(e) => setN(+e.target.value)}
          className="rounded-md border border-[var(--color-border)] bg-[var(--color-panel-2)] px-3 py-1.5 text-[12px] outline-none">
          {[30, 60, 120, 200].map((l) => <option key={l} value={l}>{l} chamadas</option>)}
        </select>
        <BotaoAoVivo live={live} onToggle={() => setLive((v) => !v)} />
        <button onClick={() => carregar(n)} title="atualizar agora" className="rounded-md border border-[var(--color-border)] p-2 text-[var(--color-muted)] hover:text-[var(--color-fg)]">
          <RefreshCw size={13} />
        </button>
      </div>

      {error && <ErrorBox message={error} onRetry={() => carregar(n)} />}
      {!carregou && !error && <Spinner label="lendo o diário…" />}
      {carregou && entries.length === 0 && !error && (
        <div className="text-[13px] text-[var(--color-muted)]">
          diário vazio — o esquilinho ainda não mastigou nada com IA desde que o registro nasceu.
          Toda chamada de LLM (classificador, modelador, extrator) cai aqui com prompt e resposta.
        </div>
      )}

      <div className="min-h-0 grow space-y-1.5 overflow-auto pr-1">
        {entries.map((e, i) => {
          const exp = aberto === i
          return (
            <div key={`${e.ts}-${i}`} className={`rounded-md border ${e.ok ? 'border-[var(--color-border)]' : 'border-[var(--color-crit)]/50'}`}>
              <button onClick={() => setAberto(exp ? null : i)} className="flex w-full items-center gap-2 px-3 py-2 text-left">
                {exp ? <ChevronDown size={13} className="shrink-0 text-[var(--color-muted)]" /> : <ChevronRight size={13} className="shrink-0 text-[var(--color-muted)]" />}
                <span className="shrink-0 font-mono text-[11px] text-[var(--color-muted)]">{e.ts}</span>
                <span className={`shrink-0 text-[12px] font-semibold ${tagCor[e.tag] ?? ''}`}>{e.tag}</span>
                <span className="grow truncate font-mono text-[11px]">{e.ctx}</span>
                <span className="shrink-0 text-[11px] tabular-nums text-[var(--color-muted)]">
                  {(e.ms / 1000).toFixed(1)}s · in {fmt(e.user_len)} → out {fmt(e.resposta_len)}
                </span>
                {!e.ok && <span className="shrink-0 rounded bg-[var(--color-crit)]/15 px-1.5 py-0.5 text-[10px] font-semibold text-[var(--color-crit)]">SEM RESPOSTA</span>}
                {e.finish === 'length' && <span className="shrink-0 rounded bg-[var(--color-warn)]/15 px-1.5 py-0.5 text-[10px] font-semibold text-[var(--color-warn)]">CORTADO</span>}
              </button>
              {exp && (
                <div className="space-y-2 border-t border-[var(--color-border)] px-3 py-2.5">
                  <BlocoTexto titulo={`system (${fmt(e.system_len)} chars)`} texto={e.system} />
                  <BlocoTexto titulo={`user (${fmt(e.user_len)} chars${e.user_len > 4000 ? ' — truncado aqui; inteiro no arquivo' : ''})`} texto={e.user} />
                  <BlocoTexto titulo={e.ok ? `resposta (${fmt(e.resposta_len)} chars)` : 'resposta — o LLM não respondeu (timeout/erro)'} texto={e.resposta || '—'} destaque />
                </div>
              )}
            </div>
          )
        })}
      </div>

      <div className="flex items-center justify-between text-[11px] text-[var(--color-muted)]">
        <span>mais recente primeiro · o inteiro teor (sem truncar) fica no llm-ledger.jsonl do servidor</span>
        {ts && <span>atualizado {ts.toLocaleTimeString('pt-BR')}{live ? ' · a cada 5s' : ''}</span>}
      </div>
    </div>
  )
}

function BlocoTexto({ titulo, texto, destaque }: { titulo: string; texto: string; destaque?: boolean }) {
  return (
    <div>
      <div className="mb-1 text-[10px] font-semibold uppercase tracking-wide text-[var(--color-muted)]">{titulo}</div>
      <pre className={`max-h-[300px] overflow-auto whitespace-pre-wrap rounded-md p-2.5 font-mono text-[11px] leading-[1.5] ${
        destaque ? 'border border-[var(--color-accent)]/30 bg-[var(--color-accent)]/5' : 'bg-[var(--color-panel-2)]'
      }`}>{texto}</pre>
    </div>
  )
}

function BotaoAoVivo({ live, onToggle }: { live: boolean; onToggle: () => void }) {
  return (
    <button
      onClick={onToggle}
      title={live ? 'pausar o acompanhamento' : 'retomar o acompanhamento'}
      className={`flex items-center gap-1.5 rounded-md border px-3 py-1.5 text-[12px] font-semibold ${
        live
          ? 'border-[var(--color-ok)] text-[var(--color-ok)]'
          : 'border-[var(--color-border)] text-[var(--color-muted)]'
      }`}
    >
      {live ? <><Pause size={13} /> ao vivo</> : <><Play size={13} /> pausado</>}
    </button>
  )
}
