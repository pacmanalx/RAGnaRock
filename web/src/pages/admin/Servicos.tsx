import { useState } from 'react'
import { RefreshCw, Play } from 'lucide-react'
import { useAsync } from '@/hooks/useAsync'
import { getStats, getNidhoggStatus, setNidhogg, runNidhoggCycle } from '@/api/ragnarock'
import { messageFromError } from '@/api/client'
import { Panel, Dot, Spinner, ErrorBox } from '@/components/ui'

// Painel de serviços: telemetria REAL do ragd (/stats) + cockpit do nidhoggd
// (GET/POST /api/nidhogg: on/off, nível 0-3, cadência, ciclo forçado).
// Restart de daemon fica FORA de propósito: é systemctl no servidor, não API
// do próprio processo (um daemon não se mata com segurança).

function humanUptime(s: number): string {
  if (s < 60) return `${s}s`
  if (s < 3600) return `${Math.floor(s / 60)}min`
  if (s < 86400) return `${Math.floor(s / 3600)}h ${Math.floor((s % 3600) / 60)}min`
  return `${Math.floor(s / 86400)}d ${Math.floor((s % 86400) / 3600)}h`
}
const mb = (v?: number) => (v == null ? '—' : v >= 1024 ? `${(v / 1024).toFixed(1)} GB` : `${v.toFixed(0)} MB`)

export function Servicos() {
  const stats = useAsync(getStats, [])
  const nid = useAsync(getNidhoggStatus, [])
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [note, setNote] = useState<string | null>(null)
  const [cadence, setCadence] = useState<number | null>(null)

  async function agir(fn: () => Promise<unknown>, msg?: string) {
    if (busy) return
    setBusy(true); setError(null); setNote(null)
    try { await fn(); if (msg) setNote(msg); nid.reload() }
    catch (e) { setError(messageFromError(e)) }
    finally { setBusy(false) }
  }

  const s = stats.data
  const n = nid.data
  const ramPct = s ? Math.min(100, (s.mem.rss_mb / s.mem.sys_total_mb) * 100) : 0
  // semáforo de pressão (herdado do dashboard legado, que morreu aqui): abaixo de 60% a
  // máquina respira; a partir de 85% o thrash é questão de tempo — foi o que precedeu o
  // freeze de 13/ago. A cor é o aviso que uma barra monocromática não dá.
  const ramCor = ramPct < 60 ? 'bg-[var(--color-ok)]' : ramPct < 85 ? 'bg-[var(--color-warn)]' : 'bg-[var(--color-crit)]'

  const tile = (label: string, v: string | number, hint?: string) => (
    <div title={hint} className="rounded-md border border-[var(--color-border)] bg-[var(--color-panel-2)] px-4 py-3">
      <div className="text-[11px] uppercase tracking-wide text-[var(--color-muted)]">{label}</div>
      <div className="mt-0.5 text-[20px] font-semibold tabular-nums">{v}</div>
    </div>
  )

  return (
    <div className="space-y-5">
      <div className="flex items-center justify-between">
        <h1 className="text-lg font-semibold">Serviços do servidor</h1>
        <button
          onClick={() => { stats.reload(); nid.reload() }}
          className="flex items-center gap-1.5 rounded-md border border-[var(--color-border)] px-3 py-1.5 text-[12px] hover:bg-[var(--color-panel-2)]"
        >
          <RefreshCw size={13} /> atualizar
        </button>
      </div>

      {error && <ErrorBox message={error} />}
      {note && <div className="rounded-md border border-[var(--color-ok)]/40 bg-[var(--color-ok)]/10 px-4 py-2.5 text-[13px]">{note}</div>}

      {/* ─────────── ragd ─────────── */}
      <Panel
        title={
          <span className="flex items-center gap-2">
            <Dot on={!!s && !stats.error} /> ragd — motor de busca
            {s && <span className="font-normal text-[var(--color-muted)]">v{s.version} · no ar há {humanUptime(s.uptime_secs)}</span>}
          </span>
        }
        actions={s && (
          <span className="rounded bg-[var(--color-panel-2)] px-2 py-0.5 text-[11px] text-[var(--color-accent)]" title="modo de armazenamento (memory = tudo em RAM; hybrid = texto/tokens no disco)">
            storage: {s.mem.storage}
          </span>
        )}
      >
        {stats.loading && <Spinner />}
        {stats.error && <ErrorBox message={stats.error} onRetry={stats.reload} />}
        {s && (
          <div className="space-y-4">
            <div className="grid grid-cols-2 gap-3 sm:grid-cols-4 xl:grid-cols-6">
              {tile('coleções', s.collections)}
              {tile('bases', s.bases)}
              {tile('chunks', s.chunks.toLocaleString('pt-BR'))}
              {tile('drivers', s.drivers)}
              {tile('dicionários', s.dicts_active, 'dicionários de sinônimos ativos (busca expandida)')}
              {tile('sinônimos', s.word_syn_entries.toLocaleString('pt-BR'), 'palavras com expansão carregadas dos dicionários ativos')}
            </div>

            {/* memória: rss do processo sobre o total da máquina */}
            <div>
              <div className="mb-1 flex items-baseline justify-between text-[12px]">
                <span className="text-[var(--color-muted)]">memória do processo</span>
                <span className="tabular-nums">
                  {mb(s.mem.rss_mb)} de {mb(s.mem.sys_total_mb)} da máquina · {mb(s.mem.sys_avail_mb)} livres
                </span>
              </div>
              <div className="h-2 overflow-hidden rounded-full bg-[var(--color-panel-2)]">
                <div className={`h-full rounded-full ${ramCor}`} style={{ width: `${ramPct}%` }} />
              </div>
              {(s.mem.est_text_mb != null || s.mem.est_vec_mb != null) && (
                <div className="mt-1 text-[11px] text-[var(--color-muted)]">
                  estimado: texto {mb(s.mem.est_text_mb)} · vetores {mb(s.mem.est_vec_mb)}{s.mem.est_words_mb != null ? ` · tokens ${mb(s.mem.est_words_mb)}` : ''}
                </div>
              )}
            </div>

            <table className="w-full text-[13px]">
              <thead>
                <tr className="border-b border-[var(--color-border)] text-left text-[11px] uppercase tracking-wider text-[var(--color-muted)]">
                  <th className="pb-1.5 font-medium">coleção</th>
                  <th className="pb-1.5 text-right font-medium">bases</th>
                  <th className="pb-1.5 text-right font-medium">chunks</th>
                </tr>
              </thead>
              <tbody>
                {s.collections_detail.map((c) => (
                  <tr key={c.collection} className="border-b border-[var(--color-border)]/40">
                    <td className="py-1.5 font-medium">{c.collection}</td>
                    <td className="py-1.5 text-right tabular-nums">{c.bases}</td>
                    <td className="py-1.5 text-right tabular-nums">{c.chunks.toLocaleString('pt-BR')}</td>
                  </tr>
                ))}
              </tbody>
            </table>
            <div className="text-[11px] text-[var(--color-muted)]">ragfiles: <code>{s.ragfiles_dir}</code></div>
          </div>
        )}
      </Panel>

      {/* ─────────── nidhoggd ─────────── */}
      <Panel
        title={
          <span className="flex items-center gap-2">
            <Dot on={!!n && !nid.error} /> nidhoggd — camada de inteligência
            {n && <span className="font-normal text-[var(--color-muted)]">v{n.version} · no ar há {humanUptime(n.uptime_secs)}</span>}
          </span>
        }
        actions={n && (
          <span className="flex items-center gap-2 text-[11px] text-[var(--color-muted)]">
            <span className="flex items-center gap-1"><Dot on={n.ragd_online} /> ragd</span>
            {n.cycle_running && <span className="animate-pulse text-[var(--color-accent)]">⚙ ciclo rodando…</span>}
          </span>
        )}
      >
        {nid.loading && <Spinner />}
        {nid.error && <ErrorBox message={nid.error} onRetry={nid.reload} />}
        {n && (
          <div className="space-y-4">
            <div className="flex flex-wrap items-center gap-4">
              {/* liga/desliga o worm */}
              <button
                onClick={() => agir(() => setNidhogg({ on: !n.on }), n.on ? 'Nidhogg DESLIGADO' : 'Nidhogg LIGADO')}
                disabled={busy}
                className={`rounded-md px-4 py-2 text-[13px] font-semibold transition-colors disabled:opacity-50 ${
                  n.on
                    ? 'bg-[var(--color-ok)]/15 text-[var(--color-ok)] hover:bg-[var(--color-ok)]/25'
                    : 'bg-[var(--color-crit)]/15 text-[var(--color-crit)] hover:bg-[var(--color-crit)]/25'
                }`}
              >
                {n.on ? '● ligado — desligar' : '○ desligado — ligar'}
              </button>

              {/* cadência */}
              <div className="flex items-end gap-2" title="intervalo entre ciclos do worm (segundos, mínimo 10)">
                <div>
                  <div className="mb-1 text-[11px] uppercase tracking-wide text-[var(--color-muted)]">cadência (s)</div>
                  <input
                    type="number" min={10}
                    value={cadence ?? n.cadence_secs}
                    onChange={(e) => setCadence(Math.max(10, +e.target.value || 10))}
                    className="w-[90px] rounded-md border border-[var(--color-border)] bg-[var(--color-panel-2)] px-3 py-2 text-[13px] outline-none focus:border-[var(--color-accent)]"
                  />
                </div>
                {cadence != null && cadence !== n.cadence_secs && (
                  <button onClick={() => agir(() => setNidhogg({ cadence: cadence }), `cadência: ${cadence}s`)} disabled={busy}
                    className="rounded-md border border-[var(--color-accent)] px-3 py-2 text-[12px] font-semibold text-[var(--color-accent)] hover:bg-[var(--color-accent)] hover:text-[var(--color-accent-fg)]">
                    aplicar
                  </button>
                )}
              </div>

              {/* ciclo forçado */}
              <button
                onClick={() => agir(runNidhoggCycle, 'ciclo forçado disparado — acompanhe o ⚙ no topo do painel')}
                disabled={busy || n.cycle_running}
                title="re-minera o nível 0 AGORA, ignorando o source_hash (refresh mesmo sem dado novo)"
                className="flex items-center gap-1.5 rounded-md bg-[var(--color-accent)] px-4 py-2 text-[13px] font-semibold text-[var(--color-accent-fg)] hover:opacity-90 disabled:opacity-50"
              >
                <Play size={14} /> rodar ciclo agora
              </button>
            </div>

            {/* nível 0-4 — o coração do worm (L4 é FUTURO: aparece na régua, não seleciona) */}
            <div>
              <div className="mb-1.5 text-[11px] uppercase tracking-wide text-[var(--color-muted)]">nível de inteligência</div>
              <div className="grid gap-2 sm:grid-cols-2 xl:grid-cols-5">
                {n.levels.map((lv) => (
                  <button
                    key={lv.n}
                    onClick={() => agir(() => setNidhogg({ level: lv.n }), `nível ${lv.n} · ${lv.name}`)}
                    disabled={busy || lv.future}
                    title={lv.desc}
                    className={`rounded-md border p-3 text-left transition-colors disabled:opacity-50 ${
                      n.level === lv.n
                        ? 'border-[var(--color-accent)] bg-[var(--color-accent)]/10'
                        : 'border-[var(--color-border)] hover:border-[var(--color-muted)]'
                    }`}
                  >
                    <div className="flex items-center justify-between">
                      <span className="text-[13px] font-semibold">L{lv.n} · {lv.name}</span>
                      <span className="flex gap-1">
                        {lv.ia && <span className="rounded bg-[var(--color-panel-2)] px-1.5 text-[10px] text-[var(--color-accent)]">IA</span>}
                        {lv.future && <span className="rounded bg-[var(--color-panel-2)] px-1.5 text-[10px] text-[var(--color-muted)]">por vir</span>}
                      </span>
                    </div>
                    <div className="mt-1 line-clamp-3 text-[11px] leading-snug text-[var(--color-muted)]">{lv.desc}</div>
                  </button>
                ))}
              </div>
            </div>

            {n.last_cycle && <div className="text-[11px] text-[var(--color-muted)]">último ciclo: {n.last_cycle}</div>}
          </div>
        )}
      </Panel>

      <div className="text-[11px] text-[var(--color-muted)]">
        Restart dos daemons é via <code>systemctl</code> no servidor (fora da API de propósito — um daemon não se reinicia com segurança por dentro).
      </div>
    </div>
  )
}
