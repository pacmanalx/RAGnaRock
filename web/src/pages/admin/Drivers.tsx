import { useState } from 'react'
import { ArrowDownToLine, ArrowUpFromLine, Power } from 'lucide-react'
import { useAsync } from '@/hooks/useAsync'
import { getDrivers, getDriversOut, getThesaurus, getIngestors, moveDriver, toggleDict } from '@/api/ragnarock'
import type { Driver } from '@/api/types'
import { messageFromError } from '@/api/client'
import { Panel, Spinner, ErrorBox, Dot } from '@/components/ui'

// Os 3 tipos de driver do RAGnaRock, agora com CONTROLE real:
//   linguagem  → instala/desinstala (move drivers ↔ drivers.out) — reflete na hora
//   dicionário → liga/desliga (inuse.flag) — a busca expandida 🧠 recarrega na hora
//   ingestão   → listagem real do ingestors_dir (conversores da entrada; sem on/off)
type Tab = 'lang' | 'dicts' | 'ingestors'
const TABS: { id: Tab; label: string; hint: string }[] = [
  { id: 'lang', label: 'Linguagem', hint: '.drv — tokeniza código-fonte por linguagem; instalar/desinstalar reflete na hora, sem reiniciar' },
  { id: 'dicts', label: 'Dicionários', hint: 'thesaurus por-palavra — só os ATIVOS alimentam a expansão 🧠; ligar/desligar recarrega na hora' },
  { id: 'ingestors', label: 'Ingestão', hint: 'scripts que convertem pdf/docx/xlsx/csv/banco na entrada (roteados por MIME/extensão)' },
]

const FALLBACK = 'tokens_PTBR.drv' // a matriz do projeto — o motor recusa desinstalar

const kb = (b: number) => (b >= 1048576 ? `${(b / 1048576).toFixed(1)} MB` : `${Math.max(1, Math.round(b / 1024))} KB`)

export function Drivers() {
  const [tab, setTab] = useState<Tab>('lang')
  const inst = useAsync(getDrivers, [])
  const out = useAsync(getDriversOut, [])
  const dicts = useAsync(getThesaurus, [])
  const ing = useAsync(getIngestors, [])
  const [busy, setBusy] = useState<string | null>(null)
  const [error, setError] = useState<string | null>(null)

  async function mover(file: string, action: 'install' | 'uninstall') {
    if (busy) return
    setBusy(file); setError(null)
    try { await moveDriver(file, action); inst.reload(); out.reload() }
    catch (e) { setError(messageFromError(e)) }
    finally { setBusy(null) }
  }

  async function ligar(code: string, enable: boolean) {
    if (busy) return
    setBusy(code); setError(null)
    try { await toggleDict(code, enable); dicts.reload() }
    catch (e) { setError(messageFromError(e)) }
    finally { setBusy(null) }
  }

  const card = (d: Driver, installed: boolean) => {
    const isFallback = d.name === FALLBACK
    return (
      <div key={d.name} className="flex items-center justify-between gap-3 rounded-md border border-[var(--color-border)] bg-[var(--color-panel-2)] px-3 py-2">
        <div className="min-w-0">
          <div className="flex items-baseline gap-2">
            <span className="text-[13px] font-semibold">{d.language || d.name}</span>
            <span className="text-[11px] text-[var(--color-muted)]">
              {d.syllables.toLocaleString('pt-BR')}s{d.keywords ? ` · ${d.keywords}kw` : ''}
            </span>
            {isFallback && <span className="rounded bg-[var(--color-panel)] px-1.5 text-[10px] text-[var(--color-warn)]" title="a matriz do projeto — todo arquivo sem driver específico cai nela">matriz</span>}
          </div>
          <div className="truncate text-[11px] text-[var(--color-muted)]" title={d.description}>
            {(d.extensions ?? []).slice(0, 6).join(' ') || d.description || '—'}
          </div>
        </div>
        {installed ? (
          <button
            onClick={() => mover(d.name, 'uninstall')}
            disabled={busy != null || isFallback}
            title={isFallback ? 'é o fallback de tokenização — não desinstala' : 'mover pra drivers.out (deixa de tokenizar essa linguagem)'}
            className="flex shrink-0 items-center gap-1 rounded border border-[var(--color-border)] px-2 py-1 text-[11px] text-[var(--color-muted)] transition-colors hover:border-[var(--color-warn)] hover:text-[var(--color-warn)] disabled:cursor-not-allowed disabled:opacity-40"
          >
            <ArrowUpFromLine size={12} /> desinstalar
          </button>
        ) : (
          <button
            onClick={() => mover(d.name, 'install')}
            disabled={busy != null}
            title="mover pra drivers/ (passa a tokenizar essa linguagem na hora)"
            className="flex shrink-0 items-center gap-1 rounded border border-[var(--color-accent)] px-2 py-1 text-[11px] font-semibold text-[var(--color-accent)] transition-colors hover:bg-[var(--color-accent)] hover:text-[var(--color-accent-fg)] disabled:opacity-40"
          >
            <ArrowDownToLine size={12} /> instalar
          </button>
        )}
      </div>
    )
  }

  return (
    <div className="space-y-4">
      <h1 className="text-lg font-semibold">Drivers</h1>

      <div className="flex gap-1 border-b border-[var(--color-border)]">
        {TABS.map((t) => (
          <button
            key={t.id}
            onClick={() => setTab(t.id)}
            className={`border-b-2 px-4 py-2 text-[13px] ${
              tab === t.id
                ? 'border-[var(--color-accent)] text-[var(--color-fg)]'
                : 'border-transparent text-[var(--color-muted)] hover:text-[var(--color-fg)]'
            }`}
          >
            {t.label}
          </button>
        ))}
      </div>
      <div className="text-[11px] text-[var(--color-muted)]">{TABS.find((t) => t.id === tab)!.hint}</div>

      {error && <ErrorBox message={error} />}

      {/* ───── linguagem: instalados × disponíveis ───── */}
      {tab === 'lang' && (
        <div className="grid gap-4 xl:grid-cols-2">
          <Panel title={`▣ Instalados · ${inst.data?.drivers.length ?? '–'}`}>
            {inst.error && <ErrorBox message={inst.error} onRetry={inst.reload} />}
            {inst.loading ? <Spinner /> : (
              <div className="space-y-1.5">
                {(inst.data?.drivers ?? []).slice().sort((a, b) => (a.language || a.name).localeCompare(b.language || b.name)).map((d) => card(d, true))}
                {(inst.data?.drivers.length ?? 0) === 0 && <div className="text-[12px] text-[var(--color-muted)]">vazio</div>}
              </div>
            )}
          </Panel>
          <Panel title={`▢ Disponíveis · ${out.data?.drivers.length ?? '–'}`}>
            {out.error && <ErrorBox message={out.error} onRetry={out.reload} />}
            {out.loading ? <Spinner /> : (
              <div className="space-y-1.5">
                {(out.data?.drivers ?? []).slice().sort((a, b) => (a.language || a.name).localeCompare(b.language || b.name)).map((d) => card(d, false))}
                {(out.data?.drivers.length ?? 0) === 0 && (
                  <div className="text-[12px] text-[var(--color-muted)]">vazio — desinstale um driver pra ele aparecer aqui</div>
                )}
              </div>
            )}
          </Panel>
        </div>
      )}

      {/* ───── dicionários: liga/desliga ───── */}
      {tab === 'dicts' && (
        <Panel title={dicts.data ? `${dicts.data.count} dicionário(s) · ${dicts.data.active} ativo(s)` : 'Dicionários'}>
          {dicts.error && <ErrorBox message={dicts.error} onRetry={dicts.reload} />}
          {dicts.loading ? <Spinner /> : dicts.data && (
            <table className="w-full text-[13px]">
              <thead>
                <tr className="border-b border-[var(--color-border)] text-left text-[11px] uppercase tracking-wider text-[var(--color-muted)]">
                  <th className="pb-2 font-medium">Código</th>
                  <th className="pb-2 font-medium">Estado</th>
                  <th className="pb-2 text-right font-medium">Entradas</th>
                  <th className="pb-2 font-medium">Fonte</th>
                  <th className="pb-2 font-medium">Licença</th>
                  <th className="pb-2 text-right font-medium"></th>
                </tr>
              </thead>
              <tbody>
                {dicts.data.dicts.map((d) => (
                  <tr key={d.code} className="border-b border-[var(--color-border)]/40">
                    <td className="py-2 font-mono text-[12px] font-semibold">{d.code}</td>
                    <td className="py-2"><span className="flex items-center gap-1.5"><Dot on={d.active} /> {d.active ? 'ativo' : 'desligado'}</span></td>
                    <td className="py-2 text-right tabular-nums">{d.entries.toLocaleString('pt-BR')}</td>
                    <td className="py-2 text-[12px] text-[var(--color-muted)]" title={d.source_url}>{d.source || '—'}</td>
                    <td className="py-2 text-[12px] text-[var(--color-muted)]">{d.license || '—'}</td>
                    <td className="py-2 text-right">
                      <button
                        onClick={() => ligar(d.code, !d.active)}
                        disabled={busy != null}
                        title={d.active ? 'desligar (sai da expansão 🧠 na hora)' : 'ligar (entra na expansão 🧠 na hora)'}
                        className={`inline-flex items-center gap-1 rounded border px-2 py-1 text-[11px] transition-colors disabled:opacity-40 ${
                          d.active
                            ? 'border-[var(--color-border)] text-[var(--color-muted)] hover:border-[var(--color-warn)] hover:text-[var(--color-warn)]'
                            : 'border-[var(--color-accent)] font-semibold text-[var(--color-accent)] hover:bg-[var(--color-accent)] hover:text-[var(--color-accent-fg)]'
                        }`}
                      >
                        <Power size={12} /> {busy === d.code ? '…' : d.active ? 'desligar' : 'ligar'}
                      </button>
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          )}
        </Panel>
      )}

      {/* ───── ingestão: listagem real ───── */}
      {tab === 'ingestors' && (
        <Panel title={ing.data ? `${ing.data.count} driver(s) de ingestão` : 'Ingestão'}>
          {ing.error && <ErrorBox message={ing.error} onRetry={ing.reload} />}
          {ing.loading ? <Spinner /> : ing.data && (
            <div className="space-y-3">
              <table className="w-full text-[13px]">
                <thead>
                  <tr className="border-b border-[var(--color-border)] text-left text-[11px] uppercase tracking-wider text-[var(--color-muted)]">
                    <th className="pb-2 font-medium">Script</th>
                    <th className="pb-2 font-medium">Descrição</th>
                    <th className="pb-2 text-right font-medium">Tamanho</th>
                  </tr>
                </thead>
                <tbody>
                  {ing.data.ingestors.map((i) => (
                    <tr key={i.name} className="border-b border-[var(--color-border)]/40">
                      <td className="py-2 font-mono text-[12px] font-semibold">{i.name}</td>
                      <td className="py-2 text-[12px] text-[var(--color-muted)]">{i.description || '—'}</td>
                      <td className="py-2 text-right tabular-nums text-[12px]">{kb(i.bytes)}</td>
                    </tr>
                  ))}
                </tbody>
              </table>
              <div className="text-[11px] text-[var(--color-muted)]">
                {ing.data.ingestors_dir} · roteamento por MIME/extensão na entrada do /ingest_any; sem liga/desliga — remover o script do diretório desativa o formato.
              </div>
            </div>
          )}
        </Panel>
      )}
    </div>
  )
}
