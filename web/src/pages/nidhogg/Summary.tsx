import { useState } from 'react'
import { ChevronDown, ChevronRight, Plus, Save, X } from 'lucide-react'
import { useAsync } from '@/hooks/useAsync'
import {
  getNidhoggCollections, getNidhoggClasses, getNidhoggDoctypes, setNidhoggDoctypes, getDoctypesUso,
  getNidhoggTemplates, getNidhoggPrompts, saveNidhoggPrompt,
} from '@/api/ragnarock'
import type { PromptTemplate } from '@/api/types'
import { messageFromError } from '@/api/client'
import { Panel, Spinner, ErrorBox } from '@/components/ui'

// L1 · Summary — a consciência (1º nível COM IA). O que ela decide e com que régua:
//   Classes    → cada documento {natureza, tipo} — origem humana é STICKY (LLM não reverte)
//   Doctypes   → o VOCABULÁRIO editável do classificador (editar = reclassifica no próximo ciclo)
//   Moldes     → registry de extração regex (L1 cria 1×, L0 aplica determinístico aos N)
//   Prompts    → a biblioteca que direciona o que cada nível pergunta ao LLM

const inputCls = 'rounded-md border border-[var(--color-border)] bg-[var(--color-panel-2)] px-3 py-2 text-[13px] outline-none focus:border-[var(--color-accent)]'

export function NidhoggSummary() {
  const cols = useAsync(getNidhoggCollections, [])
  const habilitadas = (cols.data?.collections ?? []).filter((c) => c.enabled)
  const [coll, setColl] = useState<string | null>(null)
  const ativa = coll ?? habilitadas[0]?.collection ?? null

  return (
    <div className="space-y-5">
      <div className="flex flex-wrap items-center gap-3">
        <h1 className="text-lg font-semibold">L1 · Summary</h1>
        <span className="text-[12px] text-[var(--color-muted)]">a consciência — classifica, normaliza e cria os moldes; confiança e origem sempre visíveis</span>
      </div>

      {cols.loading && <Spinner />}
      {cols.error && <ErrorBox message={cols.error} onRetry={cols.reload} />}

      {/* ── classes por coleção ── */}
      {habilitadas.length > 0 && (
        <div className="flex gap-1 border-b border-[var(--color-border)]">
          {habilitadas.map((c) => (
            <button
              key={c.collection}
              onClick={() => setColl(c.collection)}
              className={`border-b-2 px-4 py-2 text-[13px] ${
                ativa === c.collection
                  ? 'border-[var(--color-accent)] font-semibold text-[var(--color-fg)]'
                  : 'border-transparent text-[var(--color-muted)] hover:text-[var(--color-fg)]'
              }`}
            >
              {c.collection}
            </button>
          ))}
        </div>
      )}
      {ativa && <ClassesDaColecao key={ativa} collection={ativa} />}

      <Doctypes />
      <Moldes />
      <Prompts />
    </div>
  )
}

// ───────────────────────── classes de UMA coleção ─────────────────────────
function ClassesDaColecao({ collection }: { collection: string }) {
  const cls = useAsync(() => getNidhoggClasses(collection), [collection])
  return (
    <Panel title={`Classes — ${collection} · ${cls.data?.count ?? '–'} documento(s)`}>
      {cls.error && <ErrorBox message={cls.error} onRetry={cls.reload} />}
      {cls.loading ? <Spinner /> : cls.data && (
        <table className="w-full text-[13px]">
          <thead>
            <tr className="border-b border-[var(--color-border)] text-left text-[11px] uppercase tracking-wider text-[var(--color-muted)]">
              <th className="pb-2 font-medium">Base</th>
              <th className="pb-2 font-medium">Natureza</th>
              <th className="pb-2 font-medium">Tipo</th>
              <th className="pb-2 font-medium">Origem</th>
              <th className="pb-2 text-right font-medium">Confiança</th>
              <th className="pb-2 text-right font-medium">Quando</th>
            </tr>
          </thead>
          <tbody>
            {cls.data.bases.map((b) => (
              <tr key={b.name} className="border-b border-[var(--color-border)]/40">
                <td className="max-w-0 truncate py-2" title={b.name}>{b.name}</td>
                <td className="py-2">
                  <span className="rounded bg-[var(--color-panel-2)] px-1.5 py-0.5 text-[11px]">{b.natureza}</span>
                  {b.csv === 1 && <span className="ml-1 rounded bg-[var(--color-panel-2)] px-1.5 py-0.5 text-[10px] text-[var(--color-accent)]" title="tabular regular — parser CSV determinístico, zero LLM">csv</span>}
                </td>
                <td className="py-2 text-[12px]">
                  {b.tipo}
                  {b.forma && (
                    <span className="ml-1.5 rounded bg-[var(--color-panel-2)] px-1.5 py-0.5 font-mono text-[10px] text-[var(--color-muted)]"
                      title={`forma ${b.forma} — assinatura estrutural (esqueleto de rótulos); documentos irmãos de forma compartilham o molde`}>
                      ⌗{b.forma}
                    </span>
                  )}
                </td>
                <td className="py-2">
                  <span
                    title={b.origem === 'humano' ? 'decisão humana — STICKY: o LLM nunca sobrescreve' : 'classificado pela IA leve (Fase 1)'}
                    className={`rounded px-1.5 py-0.5 text-[11px] ${b.origem === 'humano' ? 'bg-[var(--color-warn)]/15 font-semibold text-[var(--color-warn)]' : 'bg-[var(--color-panel-2)] text-[var(--color-muted)]'}`}
                  >
                    {b.origem === 'humano' ? '👤 humano' : '🧠 llm'}
                  </span>
                </td>
                <td className="py-2 text-right tabular-nums">{(b.confianca ?? 0).toFixed(2)}</td>
                <td className="py-2 text-right text-[11px] tabular-nums text-[var(--color-muted)]">{b.classified_at}</td>
              </tr>
            ))}
            {cls.data.bases.length === 0 && <tr><td colSpan={6} className="py-3 text-[12px] text-[var(--color-muted)]">nenhum documento classificado ainda — o ciclo da Fase 1 preenche aqui.</td></tr>}
          </tbody>
        </table>
      )}
    </Panel>
  )
}

// ───────────────────────── doctypes: o vocabulário do classificador ─────────────────────────
function Doctypes() {
  const dt = useAsync(getNidhoggDoctypes, [])
  // o que cada tipo carrega HOJE. Remover um tipo não é editar texto: as bases de origem LLM
  // voltam pro classificador (gasta IA), as re-tipadas à mão ficam PRESAS apontando pro tipo
  // que sumiu (needs_class curto-circuita em origem='humano') e o molde do tipo fica órfão.
  const uso = useAsync(getDoctypesUso, [])
  const [edit, setEdit] = useState<{ naturezas: string[]; tipos: string[] } | null>(null)
  const [novo, setNovo] = useState<{ nat: string; tip: string }>({ nat: '', tip: '' })
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [ok, setOk] = useState(false)

  const atual = edit ?? { naturezas: dt.data?.naturezas ?? [], tipos: dt.data?.tipos ?? [] }
  const mudou = edit != null

  async function salvar() {
    if (!edit || busy) return
    setBusy(true); setError(null)
    try { await setNidhoggDoctypes(edit.naturezas, edit.tipos); setEdit(null); setOk(true); setTimeout(() => setOk(false), 2500); dt.reload(); uso.reload() }
    catch (e) { setError(messageFromError(e)) }
    finally { setBusy(false) }
  }

  const custo = (v: string) => (uso.data?.uso ?? []).find((u) => u.tipo === v)

  // `comCusto` liga o aviso: nas naturezas não há o que medir (o uso é por TIPO), então a
  // mesma função serve os dois blocos com o custo desligado no primeiro.
  const chips = (lista: string[], onRemove: (v: string) => void, comCusto = false) => (
    <div className="flex flex-wrap gap-1.5">
      {lista.map((v) => {
        const u = comCusto ? custo(v) : undefined
        const emUso = (u?.bases ?? 0) + (u?.moldes ?? 0) > 0
        return (
        <span key={v} title={u ? `${u.bases} base(s) classificada(s)${u.humano ? `, ${u.humano} fixada(s) à mão` : ''}${u.moldes ? `, ${u.moldes} molde(s)` : ''}` : undefined}
          className="group flex items-center gap-1 rounded-full border border-[var(--color-border)] px-2.5 py-0.5 text-[12px]">
          {v}
          {emUso && (
            <span className={`font-mono text-[10px] ${u?.humano ? 'text-[var(--color-crit)]' : 'text-[var(--color-muted)]'}`}>
              {u?.bases ?? 0}{u?.humano ? `·${u.humano}✋` : ''}{u?.moldes ? `·${u.moldes}⬚` : ''}
            </span>
          )}
          <button onClick={() => {
            if (u?.humano) {
              // as fixadas à mão são o dano irreversível: ninguém as reclassifica depois
              if (!confirm(`O tipo "${v}" tem ${u.humano} base(s) re-tipada(s) À MÃO. Elas NÃO são reclassificadas — vão continuar apontando para um tipo que não existe mais. Remover mesmo assim?`)) return
            } else if (emUso) {
              if (!confirm(`O tipo "${v}" tem ${u?.bases ?? 0} base(s) classificada(s)${u?.moldes ? ` e ${u.moldes} molde(s)` : ''}. Elas voltam para o classificador no próximo ciclo (gasta IA)${u?.moldes ? ' e o molde fica órfão' : ''}. Remover?`)) return
            }
            onRemove(v)
          }} title="remover" className="hidden text-[var(--color-muted)] hover:text-[var(--color-crit)] group-hover:block"><X size={11} /></button>
        </span>
        )
      })}
    </div>
  )

  return (
    <Panel
      title="Doctypes — o vocabulário do classificador (Fase 1)"
      actions={
        <span className="flex items-center gap-2">
          {ok && <span className="text-[11px] text-[var(--color-ok)]">salvo ✓ — reclassifica no próximo ciclo</span>}
          {mudou && (
            <>
              <button onClick={() => setEdit(null)} className="text-[11px] text-[var(--color-muted)] hover:text-[var(--color-fg)]">descartar</button>
              <button onClick={salvar} disabled={busy} className="flex items-center gap-1 rounded-md border border-[var(--color-accent)] px-2.5 py-1 text-[11px] font-semibold text-[var(--color-accent)] hover:bg-[var(--color-accent)] hover:text-[var(--color-accent-fg)] disabled:opacity-50">
                <Save size={12} /> {busy ? 'salvando…' : 'salvar'}
              </button>
            </>
          )}
        </span>
      }
    >
      {dt.error && <ErrorBox message={dt.error} onRetry={dt.reload} />}
      {error && <ErrorBox message={error} />}
      {dt.loading ? <Spinner /> : (
        <div className="space-y-4">
          <div>
            <div className="mb-1.5 text-[11px] uppercase tracking-wide text-[var(--color-muted)]">naturezas ({atual.naturezas.length})</div>
            {chips(atual.naturezas, (v) => setEdit({ ...atual, naturezas: atual.naturezas.filter((x) => x !== v) }))}
            <div className="mt-2 flex gap-2">
              <input value={novo.nat} onChange={(e) => setNovo({ ...novo, nat: e.target.value })} placeholder="nova natureza…" className={`w-[180px] ${inputCls}`} />
              <button
                onClick={() => { const v = novo.nat.trim().toLowerCase(); if (v && !atual.naturezas.includes(v)) { setEdit({ ...atual, naturezas: [...atual.naturezas, v] }); setNovo({ ...novo, nat: '' }) } }}
                className="rounded-md border border-[var(--color-border)] px-2.5 text-[12px] hover:bg-[var(--color-panel-2)]"
              ><Plus size={13} /></button>
            </div>
          </div>
          <div>
            <div className="mb-1.5 text-[11px] uppercase tracking-wide text-[var(--color-muted)]">tipos ({atual.tipos.length})</div>
            {chips(atual.tipos, (v) => setEdit({ ...atual, tipos: atual.tipos.filter((x) => x !== v) }), true)}
            <div className="mt-2 flex gap-2">
              <input value={novo.tip} onChange={(e) => setNovo({ ...novo, tip: e.target.value })} placeholder="novo tipo…" className={`w-[180px] ${inputCls}`} />
              <button
                onClick={() => { const v = novo.tip.trim().toLowerCase(); if (v && !atual.tipos.includes(v)) { setEdit({ ...atual, tipos: [...atual.tipos, v] }); setNovo({ ...novo, tip: '' }) } }}
                className="rounded-md border border-[var(--color-border)] px-2.5 text-[12px] hover:bg-[var(--color-panel-2)]"
              ><Plus size={13} /></button>
            </div>
          </div>
          <div className="text-[11px] text-[var(--color-muted)]">
            no chip do tipo: <b>nº de bases</b> classificadas nele · <b>✋</b> quantas foram fixadas à mão
            (essas NÃO são reclassificadas — some com o tipo e elas ficam órfãs) · <b>⬚</b> moldes de extração.<br />
            editar esta lista muda o enum que o classificador enxerga — TODO o corpus reclassifica no próximo ciclo (checkpoint por hash).
          </div>
        </div>
      )}
    </Panel>
  )
}

// ───────────────────────── moldes: o registry de extração ─────────────────────────
function Moldes() {
  const tpls = useAsync(getNidhoggTemplates, [])
  const [aberto, setAberto] = useState<string | null>(null)
  const lista = Object.entries(tpls.data?.templates ?? {})

  return (
    <Panel title={`Moldes de extração — ${lista.length} tipo(s) no registry`}>
      {tpls.error && <ErrorBox message={tpls.error} onRetry={tpls.reload} />}
      {tpls.loading ? <Spinner /> : (
        <div className="space-y-2">
          {lista.map(([tipo, t]) => (
            <div key={tipo} className="rounded-md border border-[var(--color-border)] bg-[var(--color-panel-2)]">
              <button onClick={() => setAberto(aberto === tipo ? null : tipo)} className="flex w-full items-center justify-between px-3 py-2 text-left">
                <span className="flex items-center gap-2 text-[13px]">
                  {aberto === tipo ? <ChevronDown size={14} /> : <ChevronRight size={14} />}
                  <b>{tipo}</b>
                  <span className="text-[11px] text-[var(--color-muted)]">{t.schema?.length ?? 0} campo(s)</span>
                </span>
                <span className="flex items-center gap-3 text-[11px] text-[var(--color-muted)]">
                  {t.cobertura != null && <span className="tabular-nums">cobertura {(t.cobertura * 100).toFixed(0)}%</span>}
                  {t.origem && <span className={t.origem === 'humano' ? 'font-semibold text-[var(--color-warn)]' : ''}>{t.origem === 'humano' ? '👤 dirigido' : t.origem === 'herdado' ? '♻ herdado' : '🧠 llm'}</span>}
                  {t.created_at && <span className="tabular-nums">{t.created_at}</span>}
                </span>
              </button>
              {aberto === tipo && (
                <pre className="overflow-x-auto border-t border-[var(--color-border)] p-3 text-[11px] leading-relaxed">
                  {(t.schema ?? []).join('\n') || '(sem schema)'}
                </pre>
              )}
            </div>
          ))}
          {lista.length === 0 && <div className="text-[12px] text-[var(--color-muted)]">registry vazio — o L1 cria moldes quando encontra tipos estruturados; ou crie um dirigido na L3.</div>}
          <div className="text-[11px] text-[var(--color-muted)]">molde = regex ancorado no rótulo. O L1 cria UMA vez; o L0 aplica aos N documentos, determinístico — o LLM aprende a se dispensar.</div>
        </div>
      )}
    </Panel>
  )
}

// ───────────────────────── biblioteca de prompts ─────────────────────────
function Prompts() {
  const pr = useAsync(getNidhoggPrompts, [])
  const [aberto, setAberto] = useState<string | null>(null)
  const [draft, setDraft] = useState<PromptTemplate | null>(null)
  const [novoNome, setNovoNome] = useState('')
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)

  const templates = Object.entries(pr.data?.templates ?? {})

  function abrir(nome: string, t: PromptTemplate) {
    setAberto(aberto === nome ? null : nome)
    setDraft({ ...t })
    setError(null)
  }

  async function salvar(nome: string) {
    if (!draft || busy) return
    setBusy(true); setError(null)
    try { await saveNidhoggPrompt(nome, draft.system, draft.description, draft.max_tokens); setAberto(null); pr.reload() }
    catch (e) { setError(messageFromError(e)) }
    finally { setBusy(false) }
  }

  return (
    <Panel
      title={`Biblioteca de prompts — ${templates.length} template(s)`}
      actions={
        <span className="flex items-center gap-2">
          <input value={novoNome} onChange={(e) => setNovoNome(e.target.value)} placeholder="nome do novo template…" className={`w-[190px] ${inputCls} py-1 text-[12px]`} />
          <button
            onClick={() => { const n = novoNome.trim(); if (n) { setAberto(n); setDraft({ description: '', system: '', updated: '' }); setNovoNome('') } }}
            disabled={!novoNome.trim()}
            className="flex items-center gap-1 rounded-md border border-[var(--color-accent)] px-2.5 py-1 text-[11px] font-semibold text-[var(--color-accent)] hover:bg-[var(--color-accent)] hover:text-[var(--color-accent-fg)] disabled:opacity-40"
          ><Plus size={12} /> novo</button>
        </span>
      }
    >
      {pr.error && <ErrorBox message={pr.error} onRetry={pr.reload} />}
      {error && <ErrorBox message={error} />}
      {pr.loading ? <Spinner /> : (
        <div className="space-y-2">
          {/* template novo (ainda não salvo) aparece primeiro */}
          {aberto && !pr.data?.templates[aberto] && draft && (
            <EditorPrompt nome={aberto} draft={draft} setDraft={setDraft} onSave={() => salvar(aberto)} onCancel={() => setAberto(null)} busy={busy} novo />
          )}
          {templates.map(([nome, t]) => (
            <div key={nome} className="rounded-md border border-[var(--color-border)] bg-[var(--color-panel-2)]">
              <button onClick={() => abrir(nome, t)} className="flex w-full items-center justify-between px-3 py-2 text-left">
                <span className="flex items-center gap-2 text-[13px]">
                  {aberto === nome ? <ChevronDown size={14} /> : <ChevronRight size={14} />}
                  <b>{nome}</b>
                  <span className="max-w-[420px] truncate text-[11px] text-[var(--color-muted)]">{t.description}</span>
                </span>
                <span className="text-[11px] tabular-nums text-[var(--color-muted)]">{t.max_tokens ? `${t.max_tokens} tok · ` : ''}{t.updated || 'nunca editado'}</span>
              </button>
              {aberto === nome && draft && (
                <EditorPrompt nome={nome} draft={draft} setDraft={setDraft} onSave={() => salvar(nome)} onCancel={() => setAberto(null)} busy={busy} />
              )}
            </div>
          ))}
          <div className="text-[11px] text-[var(--color-muted)]">
            os templates <b>classificador</b> e <b>extrator</b> são os motores das Fases 1 e 2 — editar muda o comportamento do worm no próximo ciclo (e reclassifica/re-extrai pelo checkpoint de hash).
          </div>
        </div>
      )}
    </Panel>
  )
}

function EditorPrompt({ nome, draft, setDraft, onSave, onCancel, busy, novo }: {
  nome: string
  draft: PromptTemplate
  setDraft: (t: PromptTemplate) => void
  onSave: () => void
  onCancel: () => void
  busy: boolean
  novo?: boolean
}) {
  return (
    <div className={`space-y-2 p-3 ${novo ? 'rounded-md border border-[var(--color-accent)]/50 bg-[var(--color-panel-2)]' : 'border-t border-[var(--color-border)]'}`}>
      {novo && <div className="text-[12px] font-semibold">novo template: {nome}</div>}
      <div className="flex flex-wrap gap-2">
        <input value={draft.description} onChange={(e) => setDraft({ ...draft, description: e.target.value })} placeholder="descrição curta…" className={`grow ${inputCls}`} />
        <input
          type="number" min={64} value={draft.max_tokens ?? ''} placeholder="max_tokens"
          onChange={(e) => setDraft({ ...draft, max_tokens: +e.target.value || undefined })}
          title="teto de resposta do LLM pra este template (vazio = default global)"
          className={`w-[110px] ${inputCls}`}
        />
      </div>
      <textarea
        value={draft.system}
        onChange={(e) => setDraft({ ...draft, system: e.target.value })}
        rows={6}
        placeholder="system prompt — o que o LLM deve fazer e o formato EXATO da resposta…"
        className={`w-full resize-y font-mono text-[12px] ${inputCls}`}
      />
      <div className="flex gap-2">
        <button onClick={onSave} disabled={busy || !draft.system.trim()} className="flex items-center gap-1 rounded-md bg-[var(--color-accent)] px-3 py-1.5 text-[12px] font-semibold text-[var(--color-accent-fg)] hover:opacity-90 disabled:opacity-50">
          <Save size={12} /> {busy ? 'salvando…' : 'salvar'}
        </button>
        <button onClick={onCancel} className="rounded-md border border-[var(--color-border)] px-3 py-1.5 text-[12px] hover:bg-[var(--color-panel)]">cancelar</button>
      </div>
    </div>
  )
}
