import { useMemo, useState } from 'react'
import { useAsync } from '@/hooks/useAsync'
import { getNidhoggCollections, getNidhoggKnowledge, getNidhoggCacheDigest } from '@/api/ragnarock'
import { Panel, Spinner, ErrorBox } from '@/components/ui'

// L0 · Minerador — o nível determinístico (zero IA). Mostra o que o worm MINEROU:
//   RootIndex  → a assinatura léxica (as raízes que definem a coleção)
//   CorpusDict → a anatomia do vocabulário (unificado / compartilhado / único, por base)
//   CacheDigest→ o que os HUMANOS andaram perguntando (digest global do cache 🧠 do ragd)
// A versão legada jogava uidf/dim crus num scatter — aqui cada número vem traduzido.

export function NidhoggMiner() {
  const cols = useAsync(getNidhoggCollections, [])
  const digest = useAsync(getNidhoggCacheDigest, [])
  const [coll, setColl] = useState<string | null>(null)

  const mineradas = (cols.data?.collections ?? []).filter((c) => c.has_knowledge)
  const ativa = coll ?? mineradas[0]?.collection ?? null

  return (
    <div className="space-y-5">
      <div className="flex flex-wrap items-center gap-3">
        <h1 className="text-lg font-semibold">L0 · Minerador</h1>
        <span className="text-[12px] text-[var(--color-muted)]">determinístico, zero IA — o material bruto sobre o qual os níveis com IA trabalham</span>
      </div>

      {cols.loading && <Spinner label="lendo coleções mineradas…" />}
      {cols.error && <ErrorBox message={cols.error} onRetry={cols.reload} />}

      {mineradas.length > 0 && (
        <div className="flex gap-1 border-b border-[var(--color-border)]">
          {mineradas.map((c) => (
            <button
              key={c.collection}
              onClick={() => setColl(c.collection)}
              className={`border-b-2 px-4 py-2 text-[13px] ${
                ativa === c.collection
                  ? 'border-[var(--color-accent)] font-semibold text-[var(--color-fg)]'
                  : 'border-transparent text-[var(--color-muted)] hover:text-[var(--color-fg)]'
              }`}
            >
              {c.collection} <span className="text-[11px] text-[var(--color-muted)]">({c.bases})</span>
            </button>
          ))}
        </div>
      )}
      {!cols.loading && mineradas.length === 0 && (
        <div className="text-[13px] text-[var(--color-muted)]">nenhuma coleção minerada — libere o acesso do worm na Visão geral e rode um ciclo.</div>
      )}

      {ativa && <ColecaoMinerada key={ativa} collection={ativa} />}

      {/* ── pilar GLOBAL: o que os humanos perguntam (digest do cache de expansão) ── */}
      <Panel title="CacheDigest — o que os humanos andaram perguntando (global)">
        {digest.error && <ErrorBox message={digest.error} onRetry={digest.reload} />}
        {digest.loading ? <Spinner /> : digest.data && (
          <div className="space-y-3">
            <div className="text-[12px] text-[var(--color-muted)]">
              {digest.data.content.n_queries} consulta(s) expandidas pela cascata 🧠 · {digest.data.content.n_variants_total} sinônimo(s) gerados
              · média {digest.data.content.avg_variants.toFixed(1)} por consulta · atualizado {digest.data.updated}
            </div>
            <div className="space-y-2">
              {digest.data.content.entries.map((e) => (
                <div key={e.query} className="flex flex-wrap items-baseline gap-1.5">
                  <span className="rounded bg-[var(--color-accent)]/15 px-2 py-0.5 text-[12px] font-semibold text-[var(--color-accent)]">{e.query}</span>
                  <span className="text-[11px] text-[var(--color-muted)]">→</span>
                  {e.variants.map((v) => (
                    <span key={v} className="rounded-full border border-[var(--color-border)] px-2 py-0.5 text-[11px]">{v}</span>
                  ))}
                </div>
              ))}
              {digest.data.content.entries.length === 0 && <div className="text-[12px] text-[var(--color-muted)]">cache vazio — nenhuma busca expandida ainda.</div>}
            </div>
            <div className="text-[11px] text-[var(--color-muted)]">
              é a memória de intenção dos usuários: quando os níveis com IA olham pra cá, sabem O QUE o corpus precisa responder.
            </div>
          </div>
        )}
      </Panel>
    </div>
  )
}

// ───────────────────────── uma coleção minerada (RootIndex + CorpusDict) ─────────────────────────
function ColecaoMinerada({ collection }: { collection: string }) {
  const kn = useAsync(() => getNidhoggKnowledge(collection), [collection])
  const k = kn.data
  const root = k?.knowledge.find((x) => x.type === 'RootIndex')?.content
  const dict = k?.knowledge.find((x) => x.type === 'CorpusDict')?.content

  // saliência = freq × uidf: fala MUITO e fala SÓ AQUI. É o que rankeia a assinatura.
  const roots = useMemo(() => (root?.salient_roots ?? [])
    .map((r) => ({ ...r, score: r.freq * r.uidf }))
    .sort((a, b) => b.score - a.score), [root])
  const maxFreq = Math.max(1, ...roots.map((r) => r.freq))
  const maxUidf = Math.max(0.001, ...roots.map((r) => r.uidf))

  const shared = dict?.shared_vocab ?? 0
  const unique = dict?.unique_vocab ?? 0
  const vocab = dict?.unified_vocab_size ?? 0
  const resto = Math.max(0, vocab - shared - unique)

  return (
    <div className="space-y-4">
      {kn.loading && <Spinner label={`lendo o minerado de ${collection}…`} />}
      {kn.error && <ErrorBox message={kn.error} onRetry={kn.reload} />}
      {k && (
        <>
          {/* proveniência: o carimbo de auditoria da digestão */}
          <div className="flex flex-wrap gap-x-5 gap-y-1 text-[11px] text-[var(--color-muted)]">
            <span>digestão <b className="font-mono text-[var(--color-fg)]">{k.provenance?.digestion_id}</b></span>
            <span>via <b className="text-[var(--color-fg)]">{k.provenance?.via}</b></span>
            <span>em {k.provenance?.at}</span>
            <span>{k.provenance?.inputs.bases} base(s) · {k.provenance?.inputs.total_chunks} chunk(s)</span>
            <span>source_hash <span className="font-mono">{k.source_hash}</span></span>
          </div>

          <div className="grid gap-4 xl:grid-cols-2">
            {/* ── RootIndex: a assinatura léxica ── */}
            <Panel title="RootIndex — a assinatura léxica da coleção">
              <div className="mb-3 text-[11px] text-[var(--color-muted)]">
                as raízes silábicas que DEFINEM esta coleção, rankeadas por <b>saliência = frequência × raridade</b>.
                Barra = quanto fala; ponto na régua = quão exclusivo (uidf alto = quase só esta coleção usa).
              </div>
              <div className="space-y-1.5">
                {roots.slice(0, 20).map((r) => (
                  <div key={r.dim} className="flex items-center gap-2 text-[12px]">
                    <span className="w-[64px] shrink-0 truncate text-right font-mono font-semibold" title={`dim ${r.dim}`}>{r.syllable}</span>
                    <div className="h-[13px] grow overflow-hidden rounded-sm bg-[var(--color-panel-2)]" title={`${r.freq} ocorrências em ${r.df} chunk(s)`}>
                      <div className="h-full rounded-sm bg-[var(--color-accent)]" style={{ width: `${(r.freq / maxFreq) * 100}%`, opacity: 0.45 + 0.55 * (r.freq / maxFreq) }} />
                    </div>
                    <span className="w-[52px] shrink-0 text-right tabular-nums text-[var(--color-muted)]">{r.freq}×</span>
                    {/* régua de exclusividade */}
                    <div className="relative h-[6px] w-[64px] shrink-0 rounded-full bg-[var(--color-panel-2)]" title={`raridade (uidf) ${r.uidf.toFixed(2)} — quanto mais à direita, mais exclusiva da coleção`}>
                      <div className="absolute top-1/2 h-[10px] w-[3px] -translate-y-1/2 rounded-full bg-[var(--color-ok)]" style={{ left: `${Math.min(97, (r.uidf / maxUidf) * 100)}%` }} />
                    </div>
                  </div>
                ))}
                {roots.length === 0 && <div className="text-[12px] text-[var(--color-muted)]">sem raízes salientes.</div>}
              </div>
              {root && (
                <div className="mt-3 text-[11px] text-[var(--color-muted)]">
                  vocabulário unificado: {root.unified_vocab_size?.toLocaleString('pt-BR')} sílabas · {root.bases_count} base(s) · {root.total_chunks} chunk(s)
                </div>
              )}
            </Panel>

            {/* ── CorpusDict: a anatomia do vocabulário ── */}
            <Panel title="CorpusDict — a anatomia do vocabulário">
              {dict ? (
                <div className="space-y-4">
                  <div className="grid grid-cols-3 gap-3">
                    {[
                      { l: 'unificado', v: vocab, hint: 'todas as sílabas distintas da coleção' },
                      { l: 'compartilhado', v: shared, hint: 'sílabas presentes em MAIS de uma base — o assunto comum' },
                      { l: 'único', v: unique, hint: 'sílabas de UMA base só — o que cada documento traz de próprio' },
                    ].map((t) => (
                      <div key={t.l} title={t.hint} className="rounded-md border border-[var(--color-border)] bg-[var(--color-panel-2)] px-3 py-2.5">
                        <div className="text-[11px] uppercase tracking-wide text-[var(--color-muted)]">{t.l}</div>
                        <div className="mt-0.5 text-[18px] font-semibold tabular-nums">{t.v.toLocaleString('pt-BR')}</div>
                      </div>
                    ))}
                  </div>
                  {vocab > 0 && (
                    <div>
                      <div className="flex h-3 overflow-hidden rounded-full">
                        <div className="bg-[var(--color-accent)]" style={{ width: `${(shared / vocab) * 100}%` }} title={`compartilhado: ${shared}`} />
                        <div className="ml-[2px] bg-[var(--color-ok)]" style={{ width: `${(unique / vocab) * 100}%` }} title={`único: ${unique}`} />
                        <div className="ml-[2px] bg-[var(--color-panel-2)]" style={{ width: `${(resto / vocab) * 100}%` }} title={`intermediário: ${resto}`} />
                      </div>
                      <div className="mt-1 flex gap-4 text-[11px] text-[var(--color-muted)]">
                        <span><span className="mr-1 inline-block h-[8px] w-[8px] rounded-sm bg-[var(--color-accent)]" />compartilhado</span>
                        <span><span className="mr-1 inline-block h-[8px] w-[8px] rounded-sm bg-[var(--color-ok)]" />único de 1 base</span>
                        <span><span className="mr-1 inline-block h-[8px] w-[8px] rounded-sm bg-[var(--color-panel-2)] ring-1 ring-[var(--color-border)]" />intermediário</span>
                      </div>
                    </div>
                  )}
                  <table className="w-full text-[13px]">
                    <thead>
                      <tr className="border-b border-[var(--color-border)] text-left text-[11px] uppercase tracking-wider text-[var(--color-muted)]">
                        <th className="pb-1.5 font-medium">Base</th>
                        <th className="pb-1.5 text-right font-medium">Chunks</th>
                      </tr>
                    </thead>
                    <tbody>
                      {(dict.bases ?? []).slice().sort((a, b) => b.n_chunks - a.n_chunks).map((b) => (
                        <tr key={b.name} className="border-b border-[var(--color-border)]/40">
                          <td className="max-w-0 truncate py-1.5" title={b.corpus}>{b.name}</td>
                          <td className="py-1.5 text-right tabular-nums">{b.n_chunks}</td>
                        </tr>
                      ))}
                    </tbody>
                  </table>
                </div>
              ) : <div className="text-[12px] text-[var(--color-muted)]">sem CorpusDict nesta digestão.</div>}
            </Panel>
          </div>
        </>
      )}
    </div>
  )
}
