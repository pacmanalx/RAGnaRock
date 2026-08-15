import { useState } from 'react'
import { Link } from 'react-router-dom'
import { Power } from 'lucide-react'
import { useAsync } from '@/hooks/useAsync'
import {
  getNidhoggStatus, getNidhoggCollections, getNidhoggClasses, getNidhoggEntities,
  getNidhoggTemplates, getNidhoggRejeitados, toggleNidhoggCollection,
} from '@/api/ragnarock'
import { messageFromError } from '@/api/client'
import { Panel, Dot, Spinner, ErrorBox } from '@/components/ui'

// Nidhogg — Visão geral: o cockpit da camada de inteligência. Observa o worm
// (status/nível/ciclo), o acumulado (classes, dump denso + NQI, moldes, rejeitados)
// e controla o ACESSO por coleção. Controles do daemon em si ficam em Serviços.

// Barra horizontal de magnitude — um tom só (accent); rótulo e valor em texto.
function Bars({ data, total }: { data: Record<string, number>; total: number }) {
  const rows = Object.entries(data).sort((a, b) => b[1] - a[1])
  const max = rows[0]?.[1] ?? 1
  return (
    <div className="space-y-1.5">
      {rows.map(([k, v]) => (
        <div key={k} className="flex items-center gap-2 text-[12px]">
          <span className="w-[110px] shrink-0 truncate text-right text-[var(--color-muted)]" title={k}>{k}</span>
          <div className="h-[14px] grow overflow-hidden rounded-sm bg-[var(--color-panel-2)]">
            <div className="h-full rounded-sm bg-[var(--color-accent)]" style={{ width: `${(v / max) * 100}%`, opacity: 0.45 + 0.55 * (v / max) }} />
          </div>
          <span className="w-[70px] shrink-0 tabular-nums">{v} <span className="text-[var(--color-muted)]">({total ? Math.round((v / total) * 100) : 0}%)</span></span>
        </div>
      ))}
      {rows.length === 0 && <div className="text-[12px] text-[var(--color-muted)]">—</div>}
    </div>
  )
}

export function NidhoggVisao() {
  const st = useAsync(getNidhoggStatus, [])
  const cols = useAsync(getNidhoggCollections, [])
  const classes = useAsync(() => getNidhoggClasses(), [])
  const ents = useAsync(getNidhoggEntities, [])
  const tpls = useAsync(getNidhoggTemplates, [])
  const rej = useAsync(getNidhoggRejeitados, [])
  const [busy, setBusy] = useState<string | null>(null)
  const [error, setError] = useState<string | null>(null)

  async function toggleColl(collection: string, enabled: boolean) {
    if (busy) return
    setBusy(collection); setError(null)
    try { await toggleNidhoggCollection(collection, enabled); cols.reload() }
    catch (e) { setError(messageFromError(e)) }
    finally { setBusy(null) }
  }

  const s = st.data
  const nTpl = tpls.data ? Object.keys(tpls.data.templates).length : null
  const nqi = ents.data?.nqi_global

  const tile = (label: string, v: React.ReactNode, hint?: string, to?: string) => {
    const inner = (
      <div title={hint} className={`rounded-md border border-[var(--color-border)] bg-[var(--color-panel-2)] px-4 py-3 ${to ? 'transition-colors hover:border-[var(--color-accent)]' : ''}`}>
        <div className="text-[11px] uppercase tracking-wide text-[var(--color-muted)]">{label}</div>
        <div className="mt-0.5 text-[20px] font-semibold tabular-nums">{v}</div>
      </div>
    )
    return to ? <Link key={label} to={to}>{inner}</Link> : <div key={label}>{inner}</div>
  }

  return (
    <div className="space-y-5">
      <div className="flex flex-wrap items-center gap-3">
        <h1 className="text-lg font-semibold">Nidhogg — Visão geral</h1>
        {s && (
          <span className="flex items-center gap-2 text-[12px] text-[var(--color-muted)]">
            <Dot on={s.on} /> {s.on ? 'ligado' : 'desligado'} · <b className="text-[var(--color-fg)]">L{s.level} {s.level_name}</b>
            · cadência {s.cadence_secs}s
            {s.cycle_running && <span className="animate-pulse text-[var(--color-accent)]">· ⚙ ciclo rodando…</span>}
          </span>
        )}
        {s?.needs_ia && (
          <span
            title={s.llm_online === false ? (s.llm_erro || 'endpoint não respondeu') : 'modelo respondendo'}
            className={`flex items-center gap-1.5 rounded px-2 py-0.5 text-[12px] ${
              s.llm_online === false
                ? 'bg-[var(--color-crit)]/15 font-semibold text-[var(--color-crit)]'
                : 'text-[var(--color-muted)]'}`}
          >
            <Dot on={s.llm_online !== false} />
            IA: <b className={s.llm_online === false ? '' : 'text-[var(--color-fg)]'}>{s.llm_tag || '—'}</b>
            {s.llm_online === false && ' · FORA DO AR'}
          </span>
        )}
        <div className="grow" />
        <Link to="/admin/servicos" className="text-[12px] text-[var(--color-accent)] hover:opacity-80">controlar em Serviços →</Link>
      </div>

      {error && <ErrorBox message={error} />}
      {st.loading && <Spinner label="lendo o worm…" />}
      {st.error && <ErrorBox message={st.error} onRetry={st.reload} />}

      {/* ── o acumulado, em números ── */}
      <div className="grid grid-cols-2 gap-3 sm:grid-cols-5">
        {tile('classes', classes.data?.count ?? '…', 'documentos classificados {natureza, tipo} — Fase 1', '/nidhogg/summary')}
        {tile('entidades no dump', ents.data?.count ?? '…', 'registros extraídos (o dump denso que as inferências leem)', '/nidhogg/miner')}
        {tile('NQI global', nqi != null ? nqi.toFixed(2) : '…', 'qualidade da normalização (cobertura × precisão, agregável)')}
        {tile('moldes', nTpl ?? '…', 'templates regex ancorados no rótulo (L1 cria 1×, L0 aplica determinístico)')}
        {tile('rejeitados', rej.data?.count ?? '…', 'documentos que o motor não processou — humano decide na L3', '/nidhogg/gaps')}
      </div>

      {/* NQI como barra (0..1) */}
      {nqi != null && (
        <div>
          <div className="mb-1 flex justify-between text-[11px] text-[var(--color-muted)]">
            <span>NQI global — qualidade da normalização</span><span className="tabular-nums">{(nqi * 100).toFixed(0)}%</span>
          </div>
          <div className="h-2 overflow-hidden rounded-full bg-[var(--color-panel-2)]">
            <div className="h-full rounded-full" style={{ width: `${nqi * 100}%`, background: nqi >= 0.8 ? 'var(--color-ok)' : nqi >= 0.5 ? 'var(--color-warn)' : 'var(--color-crit)' }} />
          </div>
        </div>
      )}

      {/* ── o modelo: quem faz o trabalho dos níveis com IA ── */}
      {s?.needs_ia && (
        <Panel
          title={<span className="flex items-center gap-2">
            <Dot on={s.llm_online !== false} /> Modelo de IA
            <span className="font-normal text-[var(--color-muted)]">
              — do nível 1 pra cima o worm depende dele
            </span>
          </span>}
          actions={<Link to="/admin/config" className="text-[12px] text-[var(--color-accent)] hover:opacity-80">configurar →</Link>}
        >
          {s.llm_online === false && (
            <div className="mb-3 rounded-md border border-[var(--color-crit)] bg-[var(--color-crit)]/10 px-3 py-2 text-[13px] font-semibold text-[var(--color-crit)]">
              ⛔ fora do ar — nada novo será produzido até o endpoint responder
            </div>
          )}
          <div className="grid gap-x-6 gap-y-1.5 text-[13px] sm:grid-cols-[auto_1fr]">
            <span className="text-[var(--color-muted)]">modelo</span>
            <code className="font-medium">{s.llm_tag || '—'}</code>
            <span className="text-[var(--color-muted)]">endpoint</span>
            <code className="break-all text-[12px]">{s.llm_url || '—'}</code>
            <span className="text-[var(--color-muted)]">estado</span>
            <span className={s.llm_online === false ? 'font-semibold text-[var(--color-crit)]' : 'text-[var(--color-ok)]'}>
              {s.llm_online === false ? 'não responde' : 'respondendo'}
            </span>
            {s.llm_erro && (<>
              <span className="text-[var(--color-muted)]">motivo</span>
              <span className="text-[var(--color-crit)]">{s.llm_erro}</span>
            </>)}
            <span className="text-[var(--color-muted)]">última sondagem</span>
            <span className="tabular-nums text-[12px]">{s.llm_checked || '—'} <span className="text-[var(--color-muted)]">(a cada 15s)</span></span>
          </div>
        </Panel>
      )}

      {/* ── coleções: o que o worm pode comer ── */}
      <Panel title="Coleções — acesso do worm">
        {cols.error && <ErrorBox message={cols.error} onRetry={cols.reload} />}
        {cols.loading ? <Spinner /> : cols.data && (
          <table className="w-full text-[13px]">
            <thead>
              <tr className="border-b border-[var(--color-border)] text-left text-[11px] uppercase tracking-wider text-[var(--color-muted)]">
                <th className="pb-2 font-medium">Coleção</th>
                <th className="pb-2 text-right font-medium">Bases</th>
                <th className="pb-2 font-medium">Acesso</th>
                <th className="pb-2 font-medium" title="fração do corpus com a digestão da camada ativa em dia (L1 = classificação)">Saturação</th>
                <th className="pb-2 font-medium">Conhecimento</th>
                <th className="pb-2 font-medium">Última digestão</th>
                <th className="pb-2 text-right font-medium"></th>
              </tr>
            </thead>
            <tbody>
              {cols.data.collections.map((c) => (
                <tr key={c.collection} className="border-b border-[var(--color-border)]/40">
                  <td className="py-2 font-semibold">{c.collection}</td>
                  <td className="py-2 text-right tabular-nums">{c.bases}</td>
                  <td className="py-2"><span className="flex items-center gap-1.5"><Dot on={c.enabled} /> {c.enabled ? 'liberado' : 'bloqueado'}</span></td>
                  <td className="py-2">
                    {c.has_knowledge ? (
                      <span className="flex items-center gap-2">
                        <span className="h-1.5 w-[64px] overflow-hidden rounded-full bg-[var(--color-panel-2)]">
                          <span className="block h-full rounded-full" style={{ width: `${c.saturation * 100}%`, background: c.saturation >= 0.999 ? 'var(--color-ok)' : 'var(--color-accent)' }} />
                        </span>
                        <span className="text-[11px] tabular-nums text-[var(--color-muted)]">{(c.saturation * 100).toFixed(0)}%</span>
                      </span>
                    ) : <span className="text-[12px] text-[var(--color-muted)]">—</span>}
                  </td>
                  <td className="py-2 text-[12px] text-[var(--color-muted)]">{c.has_knowledge ? 'minerado ✓' : '—'}</td>
                  <td className="py-2 text-[12px] tabular-nums text-[var(--color-muted)]">{c.updated || '—'}</td>
                  <td className="py-2 text-right">
                    <button
                      onClick={() => toggleColl(c.collection, !c.enabled)}
                      disabled={busy != null}
                      title={c.enabled ? 'bloquear: o worm para de digerir esta coleção' : 'liberar: o worm passa a digerir esta coleção'}
                      className={`inline-flex items-center gap-1 rounded border px-2 py-1 text-[11px] transition-colors disabled:opacity-40 ${
                        c.enabled
                          ? 'border-[var(--color-border)] text-[var(--color-muted)] hover:border-[var(--color-warn)] hover:text-[var(--color-warn)]'
                          : 'border-[var(--color-accent)] font-semibold text-[var(--color-accent)] hover:bg-[var(--color-accent)] hover:text-[var(--color-accent-fg)]'
                      }`}
                    >
                      <Power size={12} /> {busy === c.collection ? '…' : c.enabled ? 'bloquear' : 'liberar'}
                    </button>
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        )}
      </Panel>

      {/* ── distribuição das classes (Fase 1) ── */}
      <div className="grid gap-4 xl:grid-cols-2">
        <Panel title={`Naturezas · ${classes.data?.count ?? '–'} documento(s)`}>
          {classes.error && <ErrorBox message={classes.error} onRetry={classes.reload} />}
          {classes.loading ? <Spinner /> : classes.data && <Bars data={classes.data.naturezas} total={classes.data.count} />}
        </Panel>
        <Panel title="Tipos">
          {classes.loading ? <Spinner /> : classes.data && <Bars data={classes.data.tipos} total={classes.data.count} />}
        </Panel>
      </div>

      {/* ── rejeitados por motivo + NQI por tipo ── */}
      <div className="grid gap-4 xl:grid-cols-2">
        <Panel title={`Rejeitados por motivo · ${rej.data?.count ?? '–'}`}>
          {rej.error && <ErrorBox message={rej.error} onRetry={rej.reload} />}
          {rej.loading ? <Spinner /> : rej.data && (
            <div className="space-y-2">
              <Bars data={rej.data.por_motivo} total={rej.data.count} />
              <div className="text-[11px] text-[var(--color-muted)]">tratamento (re-tipar / molde dirigido) na tela L3 · Gaps & Propostas</div>
            </div>
          )}
        </Panel>
        <Panel title="Extração por tipo (dump denso)">
          {ents.error && <ErrorBox message={ents.error} onRetry={ents.reload} />}
          {ents.loading ? <Spinner /> : ents.data && (
            <table className="w-full text-[13px]">
              <thead>
                <tr className="border-b border-[var(--color-border)] text-left text-[11px] uppercase tracking-wider text-[var(--color-muted)]">
                  <th className="pb-1.5 font-medium">Tipo</th>
                  <th className="pb-1.5 font-medium">Modo</th>
                  <th className="pb-1.5 text-right font-medium">Registros</th>
                  <th className="pb-1.5 text-right font-medium">Bases</th>
                  <th className="pb-1.5 text-right font-medium">NQI</th>
                </tr>
              </thead>
              <tbody>
                {ents.data.por_tipo.map((t) => (
                  <tr key={`${t.tipo}-${t.modo}`} className="border-b border-[var(--color-border)]/40">
                    <td className="py-1.5 font-semibold">{t.tipo}</td>
                    <td className="py-1.5 text-[12px] text-[var(--color-muted)]" title={t.modo === 'det' ? 'determinístico (parser CSV / molde regex)' : t.modo}>{t.modo}</td>
                    <td className="py-1.5 text-right tabular-nums">{t.c}</td>
                    <td className="py-1.5 text-right tabular-nums">{t.bases}</td>
                    <td className="py-1.5 text-right tabular-nums">{t.nqi.toFixed(2)}</td>
                  </tr>
                ))}
                {ents.data.por_tipo.length === 0 && <tr><td colSpan={5} className="py-2 text-[12px] text-[var(--color-muted)]">dump vazio — nada extraído ainda</td></tr>}
              </tbody>
            </table>
          )}
        </Panel>
      </div>
    </div>
  )
}
