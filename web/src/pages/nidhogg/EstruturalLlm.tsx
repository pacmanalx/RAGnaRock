import { useMemo, useState } from 'react'
import { Pencil, Sparkles } from 'lucide-react'
import { useAsync } from '@/hooks/useAsync'
import {
  getNidhoggRelacoes, getNidhoggStatus, getNidhoggCollections, getNidhoggPrompts, saveNidhoggPrompt,
} from '@/api/ragnarock'
import type { RelacaoItem } from '@/api/types'
import { messageFromError } from '@/api/client'
import { Panel, Metric, Spinner, ErrorBox } from '@/components/ui'
import { ThinkNavigator } from './Navigator'

// L3 · Estrutural LLM — a régua recravada (13/ago): o MESMO objetivo do L2 (grafar relações),
// só que 100% LLM-bound, sem medo de usar inferência. O worm pega as CENAS mais densas do
// censo determinístico (chunks onde mais entidades co-ocorrem), manda o trecho pro LLM local
// e destila {a, rel, b, tema}. Tudo entra no MESMO grafo do L2 (nó do LLM funde com o nó do
// censo pela mesma normalização) com selo de origem 🧠 (tipo="relacao" no dump).

// espelho TS do norm_valor("mencao", …) do nidhoggd: NFD sem acento + minúsculas + espaço único
const norm = (s: string) =>
  s.normalize('NFD').replace(/[\u0300-\u036f]/g, '').toLowerCase().trim().split(/\s+/).join(' ')

export function NidhoggEstruturalLlm() {
  const st = useAsync(getNidhoggStatus, [])
  const colls = useAsync(getNidhoggCollections, [])
  const [escopo, setEscopo] = useState('*')
  const rel = useAsync(() => getNidhoggRelacoes(escopo), [escopo])

  const [navInicial, setNavInicial] = useState<{ valor: string; norm: string; escopo?: string } | null>(null)
  const [fTema, setFTema] = useState('')

  const relacoes = rel.data?.relacoes ?? []
  const temas = useMemo(() => {
    const m = new Map<string, number>()
    for (const r of relacoes) if (r.dado.tema) m.set(r.dado.tema, (m.get(r.dado.tema) ?? 0) + 1)
    return [...m.entries()].sort((a, b) => b[1] - a[1])
  }, [relacoes])
  const visiveis = fTema ? relacoes.filter((r) => r.dado.tema === fTema) : relacoes

  const nivel = st.data?.level ?? 0
  const todas = colls.data?.collections ?? []

  function navegar(valor: string, coll: string) {
    setNavInicial({ valor, norm: norm(valor), escopo: coll })
  }

  return (
    <div className="space-y-4">
      <div className="flex flex-wrap items-center gap-3">
        <h1 className="text-lg font-semibold">L3 · Estrutural LLM</h1>
        <span className="text-[12px] text-[var(--color-muted)]">
          o mesmo que o L2, 100% LLM-bound — destila quem-é-o-quê-de-quem e temas de cena; grava no mesmo grafo com selo 🧠
        </span>
        <div className="grow" />
        <select value={escopo} onChange={(e) => setEscopo(e.target.value)}
          className="rounded-md border border-[var(--color-border)] bg-[var(--color-panel-2)] px-2.5 py-1.5 text-[12px] outline-none">
          <option value="*">todas as coleções</option>
          {todas.map((c) => <option key={c.collection} value={c.collection}>{c.collection}</option>)}
        </select>
      </div>

      {st.data && nivel < 3 && (
        <div className="rounded-md border border-[var(--color-warn)]/40 bg-[var(--color-warn)]/10 px-3 py-2 text-[13px]">
          o worm está no <b>L{nivel} · {st.data.level_name}</b> — a mastigação L3 só roda com o nível em
          <b> L3 · estrutural-llm</b> (Admin → Serviços). O que já foi destilado continua legível abaixo.
        </div>
      )}

      <div className="grid gap-3 sm:grid-cols-3">
        <Panel><Metric label="relações destiladas" value={rel.data ? String(rel.data.count) : '…'} hint="tipo=relacao no dump (sentinelas fora)" /></Panel>
        <Panel><Metric label="temas de cena" value={rel.data ? String(temas.length) : '…'} hint="nós próprios no grafo (campo=tema)" /></Panel>
        <Panel><Metric label="bases mastigadas" value={rel.data ? String(rel.data.bases) : '…'} hint="1 base narrativa por ciclo — LLM-bound é caro" /></Panel>
      </div>

      {/* ── as relações ── */}
      <Panel title={`🧠 Relações destiladas · ${visiveis.length}${fTema ? ` (tema: ${fTema})` : ''}`}
        actions={fTema ? <button onClick={() => setFTema('')} className="text-[11px] text-[var(--color-accent)]">limpar filtro</button> : undefined}>
        {rel.loading && <Spinner />}
        {rel.error && <ErrorBox message={messageFromError(rel.error)} onRetry={rel.reload} />}
        {rel.data && rel.data.count === 0 && (
          <div className="text-[13px] text-[var(--color-muted)]">
            nada destilado ainda — o L3 espera o censo do L2 passar na base (ele trabalha SOBRE o
            determinístico, nunca às cegas) e mastiga 1 base narrativa por ciclo. O diário 🐿️ (Logs) mostra cada chamada.
          </div>
        )}
        {temas.length > 0 && (
          <div className="mb-2 flex flex-wrap gap-1.5">
            {temas.slice(0, 16).map(([t, n]) => (
              <button key={t} onClick={() => setFTema(fTema === t ? '' : t)}
                className={`rounded-full border px-2.5 py-0.5 text-[11px] ${fTema === t
                  ? 'border-[var(--color-accent)] text-[var(--color-accent)]'
                  : 'border-[var(--color-border)] text-[var(--color-muted)] hover:border-[var(--color-muted)]'}`}>
                {t} · {n}
              </button>
            ))}
          </div>
        )}
        <div className="space-y-1">
          {visiveis.map((r, i) => <LinhaRelacao key={`${r.base}|${r.idx}|${i}`} r={r} onNavegar={navegar} />)}
        </div>
        {rel.data && rel.data.count >= 300 && (
          <div className="mt-2 text-[11px] text-[var(--color-muted)]">mostrando as 300 mais recentes — restrinja por coleção pra ver mais.</div>
        )}
      </Panel>

      {/* ── Think Navigator centrado no clique ── */}
      {navInicial && (
        <Panel title={`Think Navigator · ${navInicial.valor}`}
          actions={<button onClick={() => setNavInicial(null)} className="text-[11px] text-[var(--color-muted)] hover:text-[var(--color-fg)]">fechar</button>}>
          <ThinkNavigator key={navInicial.norm} colecoes={todas.map((c) => c.collection)} inicial={navInicial} />
        </Panel>
      )}

      <EditorPromptRelacoes onSaved={rel.reload} />
    </div>
  )
}

function LinhaRelacao({ r, onNavegar }: { r: RelacaoItem; onNavegar: (valor: string, coll: string) => void }) {
  const btn = 'rounded border border-transparent px-1 font-medium hover:border-[var(--color-accent)] hover:text-[var(--color-accent)]'
  return (
    <div className="flex flex-wrap items-center gap-1.5 rounded-md border border-[var(--color-border)] px-3 py-1.5 text-[13px]">
      <Sparkles size={12} className="shrink-0 text-[var(--color-accent)]" />
      <button className={btn} onClick={() => onNavegar(r.dado.a, r.collection)} title="abrir no Think Navigator">{r.dado.a}</button>
      <span className="rounded-full bg-[var(--color-panel-2)] px-2 py-0.5 text-[11px] italic text-[var(--color-muted)]">{r.dado.rel}</span>
      <button className={btn} onClick={() => onNavegar(r.dado.b, r.collection)} title="abrir no Think Navigator">{r.dado.b}</button>
      {r.dado.tema && <span className="text-[11px] text-[var(--color-muted)]">· tema: {r.dado.tema}</span>}
      <div className="grow" />
      <span className="font-mono text-[10px] text-[var(--color-muted)]" title="base · chunk da cena">
        {r.collection}/{r.base} · cena {String(r.idx)}
      </span>
    </div>
  )
}

// editor focado do template "relacoes" — editar re-mastiga (o checkpoint inclui o hash do prompt)
function EditorPromptRelacoes({ onSaved }: { onSaved: () => void }) {
  const pr = useAsync(getNidhoggPrompts, [])
  const [aberto, setAberto] = useState(false)
  const [texto, setTexto] = useState<string | null>(null)
  const [busy, setBusy] = useState(false)
  const [erro, setErro] = useState<string | null>(null)

  const tpl = pr.data?.templates['relacoes']
  const valor = texto ?? tpl?.system ?? ''

  async function salvar() {
    if (!valor.trim()) { setErro('prompt vazio') ; return }
    setBusy(true); setErro(null)
    try {
      await saveNidhoggPrompt('relacoes', valor, tpl?.description ?? 'Relações estruturais (L3, 100% LLM).', tpl?.max_tokens)
      setTexto(null); setAberto(false); pr.reload(); onSaved()
    } catch (e) { setErro(messageFromError(e)) }
    finally { setBusy(false) }
  }

  return (
    <Panel title="Prompt do destilador (template “relacoes”)"
      actions={
        <button onClick={() => setAberto(!aberto)}
          className="flex items-center gap-1 text-[11px] text-[var(--color-accent)]">
          <Pencil size={11} /> {aberto ? 'fechar' : 'editar'}
        </button>
      }>
      {pr.loading && <Spinner />}
      {pr.error && <ErrorBox message={messageFromError(pr.error)} onRetry={pr.reload} />}
      {!aberto && tpl && (
        <div className="text-[12px] text-[var(--color-muted)]">
          {tpl.description} <span className="text-[10px]">· atualizado {tpl.updated}</span>
        </div>
      )}
      {aberto && (
        <div className="space-y-2">
          <textarea value={valor} onChange={(e) => setTexto(e.target.value)} rows={7}
            className="w-full resize-y rounded-md border border-[var(--color-border)] bg-[var(--color-panel-2)] p-2.5 font-mono text-[12px] outline-none focus:border-[var(--color-accent)]" />
          {erro && <div className="text-[12px] text-[var(--color-crit)]">{erro}</div>}
          <div className="flex items-center gap-2">
            <button onClick={salvar} disabled={busy}
              className="rounded-md bg-[var(--color-accent)] px-4 py-1.5 text-[13px] font-medium text-[var(--color-accent-fg)] disabled:opacity-50">
              {busy ? 'salvando…' : 'salvar prompt'}
            </button>
            <span className="text-[11px] text-[var(--color-muted)]">
              salvar muda o checkpoint (hash do prompt) — o worm re-mastiga TODAS as bases narrativas nos próximos ciclos.
            </span>
          </div>
        </div>
      )}
    </Panel>
  )
}
