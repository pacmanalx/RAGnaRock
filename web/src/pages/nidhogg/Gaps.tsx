import { useMemo, useState } from 'react'
import { Hammer, Lightbulb, Tag, X } from 'lucide-react'
import { useAsync } from '@/hooks/useAsync'
import {
  getNidhoggRejeitados, getNidhoggTemplates, getNidhoggClasses, getNidhoggDoctypes,
  postReclass, postMoldeDirigido,
} from '@/api/ragnarock'
import type { MoldeTemplate } from '@/api/types'
import { messageFromError } from '@/api/client'
import { Panel, Spinner, ErrorBox } from '@/components/ui'

// L4 · Gaps & Propostas — a casa da camada PROPOSITIVA (régua 13/ago: L3 = estrutural-LLM;
// L4 = propõe sobre parâmetros do usuário, com recursão e pesquisa externa — o desenho dela
// é discussão prévia com o Pacman, aqui vive só a semente). Enquanto ela não nasce, o
// cockpit de destrave mora aqui: o que a IA não deu conta sozinha (classificação rejeitada,
// molde reprovado) com as alavancas RE-TIPAR (origem=humano, sticky) e MOLDE DIRIGIDO.
// Doutrina: gap de DIMENSÃO é navegação/exigência DETERMINÍSTICA → mora no L2 (📐 Dimensões).

export type AcaoDirigido = { tipo: string; collection?: string; base?: string; instrucao?: string }
type AcaoRetipar = { collection: string; base: string; tipoAtual: string }

export function NidhoggGaps() {
  const rej = useAsync(getNidhoggRejeitados, [])
  const tpls = useAsync(getNidhoggTemplates, [])
  const cls = useAsync(getNidhoggClasses, [])
  const dts = useAsync(getNidhoggDoctypes, [])

  const [dirigido, setDirigido] = useState<AcaoDirigido | null>(null)
  const [retipar, setRetipar] = useState<AcaoRetipar | null>(null)

  const bases = cls.data?.bases ?? []
  const reprovados = useMemo(() => {
    const t = tpls.data?.templates ?? {}
    return Object.entries(t).filter(([, v]) => v.origem === 'reprovado')
  }, [tpls.data])

  function recarregarTudo() { rej.reload(); tpls.reload(); cls.reload() }

  const nFilas = (rej.data?.count ?? 0) + reprovados.length

  return (
    <div className="space-y-4">
      <div className="flex flex-wrap items-center gap-3">
        <h1 className="text-lg font-semibold">L4 · Gaps &amp; Propostas</h1>
        <span className="text-[12px] text-[var(--color-muted)]">
          a camada propositiva (por vir) + o cockpit de destrave — onde a IA falhou e onde você a re-dirige
        </span>
        <div className="grow" />
        <span className="rounded-full border border-[var(--color-border)] px-3 py-1 text-[12px] tabular-nums text-[var(--color-muted)]">
          {nFilas} item(ns) nas filas
        </span>
      </div>

      {/* ── fila 1: rejeitados de classificação ── */}
      <Panel title={`🚫 Rejeitados de classificação · ${rej.data?.count ?? '…'}`}>
        {rej.loading && <Spinner />}
        {rej.error && <ErrorBox message={messageFromError(rej.error)} onRetry={rej.reload} />}
        {rej.data && rej.data.count === 0 && (
          <div className="text-[13px] text-[var(--color-muted)]">nenhuma base rejeitada — o classificador deu conta de tudo.</div>
        )}
        <div className="space-y-1.5">
          {(rej.data?.rejeitados ?? []).map((r) => (
            <div key={`${r.collection}/${r.base}`} className="flex flex-wrap items-center gap-2 rounded-md border border-[var(--color-border)] px-3 py-2">
              <span className="font-mono text-[11px] text-[var(--color-muted)]">{r.collection}/</span>
              <span className="grow truncate text-[13px]">{r.base}</span>
              <span className="rounded-full bg-[var(--color-warn)]/10 px-2 py-0.5 text-[10px] text-[var(--color-warn)]" title={`classificado como ${r.natureza}/${r.tipo}`}>
                {r.motivo} · {r.tipo}
              </span>
              <button onClick={() => setRetipar({ collection: r.collection, base: r.base, tipoAtual: r.tipo })}
                className="flex items-center gap-1 rounded-md border border-[var(--color-border)] px-2.5 py-1 text-[11px] hover:border-[var(--color-accent)] hover:text-[var(--color-accent)]">
                <Tag size={11} /> re-tipar
              </button>
              <button onClick={() => setDirigido({ tipo: r.tipo, collection: r.collection, base: r.base })}
                className="flex items-center gap-1 rounded-md border border-[var(--color-border)] px-2.5 py-1 text-[11px] hover:border-[var(--color-accent)] hover:text-[var(--color-accent)]">
                <Hammer size={11} /> molde dirigido
              </button>
            </div>
          ))}
        </div>
        <div className="mt-2 text-[11px] text-[var(--color-muted)]">
          re-tipagem grava origem=humano (sticky — o classificador nunca reverte) e purga extração antiga incompatível.
        </div>
      </Panel>

      {/* ── fila 2: moldes reprovados pelo L1 ── */}
      <Panel title={`🚫 Moldes reprovados · ${reprovados.length}`}>
        {tpls.loading && <Spinner />}
        {tpls.error && <ErrorBox message={messageFromError(tpls.error)} onRetry={tpls.reload} />}
        {!tpls.loading && reprovados.length === 0 && (
          <div className="text-[13px] text-[var(--color-muted)]">nenhum molde reprovado — todo tipo na fila do L1 ganhou molde.</div>
        )}
        <div className="space-y-1.5">
          {reprovados.map(([key, t]: [string, MoldeTemplate]) => {
            const tipoPuro = key.split('@')[0]
            const candidatas = bases.filter((b) => b.tipo === tipoPuro)
            return (
              <div key={key} className="flex flex-wrap items-center gap-2 rounded-md border border-[var(--color-crit)]/40 px-3 py-2">
                <span className="font-mono text-[13px] font-semibold">{key}</span>
                <span className="text-[11px] text-[var(--color-muted)]">
                  o L1 tentou e reprovou (cobertura {((t.cobertura ?? 0) * 100).toFixed(0)}%) — sem re-try automático
                </span>
                <div className="grow" />
                <span className="text-[11px] text-[var(--color-muted)]">{candidatas.length} base(s) do tipo</span>
                <button
                  onClick={() => setDirigido({ tipo: tipoPuro, collection: candidatas[0]?.collection, base: candidatas[0]?.name })}
                  className="flex items-center gap-1 rounded-md border border-[var(--color-border)] px-2.5 py-1 text-[11px] hover:border-[var(--color-accent)] hover:text-[var(--color-accent)]">
                  <Hammer size={11} /> molde dirigido
                </button>
              </div>
            )
          })}
        </div>
        <div className="mt-2 text-[11px] text-[var(--color-muted)]">
          molde dirigido sobrescreve a reprovação (origem=humano) e o L0 volta a extrair o tipo no próximo ciclo.
          Se a base está mal classificada (prosa virando "extrato"), o caminho certo é re-tipar na fila de cima.
        </div>
      </Panel>

      {/* ── a camada propositiva (por vir) ── */}
      <Panel title="💡 Perguntas não-perguntadas (propositivo)">
        <div className="flex items-start gap-2 text-[13px] text-[var(--color-muted)]">
          <Lightbulb size={15} className="mt-0.5 shrink-0 text-[var(--color-warn)]" />
          <div>
            A camada que PROPÕE — "o CNPJ X aparece em 40 comprovantes mas não tem contrato no corpus;
            falta o contrato ou falta ingerir?" — trabalha sobre <b>parâmetros que você declara</b>, com
            recursão e pesquisa externa, aumentando o knowledge existente. Nasce em cima destas filas,
            do grafo (L2 determinístico + L3 🧠) e dos gaps de dimensão (📐). Em desenho.
          </div>
        </div>
      </Panel>

      {retipar && (
        <ModalRetipar acao={retipar} tipos={dts.data?.tipos ?? []}
          onClose={() => setRetipar(null)} onDone={() => { setRetipar(null); recarregarTudo() }} />
      )}
      {dirigido && (
        <ModalDirigido acao={dirigido} bases={bases}
          onClose={() => setDirigido(null)} onDone={() => { setDirigido(null); recarregarTudo() }} />
      )}
    </div>
  )
}

// ── modal re-tipar: select do tipo certo → POST /reclass (origem=humano) ──
function ModalRetipar({ acao, tipos, onClose, onDone }: {
  acao: AcaoRetipar; tipos: string[]; onClose: () => void; onDone: () => void
}) {
  const [tipo, setTipo] = useState('')
  const [busy, setBusy] = useState(false)
  const [erro, setErro] = useState<string | null>(null)
  const [nota, setNota] = useState<string | null>(null)

  async function aplicar() {
    if (!tipo) { setErro('escolha o tipo'); return }
    setBusy(true); setErro(null)
    try {
      const r = await postReclass(acao.collection, acao.base, tipo)
      setNota(`re-tipada pra ${r.tipo} (${r.natureza}) — ${r.nota}${r.purgadas ? ` · ${r.purgadas} extração(ões) antiga(s) purgada(s)` : ''}`)
    } catch (e) { setErro(messageFromError(e)); setBusy(false) }
  }

  return (
    <Modal titulo={`Re-tipar · ${acao.base}`} onClose={onClose}>
      {nota ? (
        <div className="space-y-3">
          <div className="rounded-md border border-[var(--color-ok)]/40 bg-[var(--color-ok)]/10 px-3 py-2 text-[13px]">{nota}</div>
          <button onClick={onDone} className="rounded-md bg-[var(--color-accent)] px-4 py-1.5 text-[13px] font-medium text-[var(--color-accent-fg)]">fechar</button>
        </div>
      ) : (
        <div className="space-y-3">
          <div className="text-[12px] text-[var(--color-muted)]">
            hoje: <b>{acao.tipoAtual}</b> (pelo classificador). Escolha o tipo correto — a decisão é
            <b> sticky</b> (origem=humano): o LLM nunca mais reverte.
          </div>
          <select value={tipo} onChange={(e) => setTipo(e.target.value)}
            className="w-full rounded-md border border-[var(--color-border)] bg-[var(--color-panel-2)] px-3 py-2 text-[13px] outline-none">
            <option value="">— tipo correto —</option>
            {tipos.map((t) => <option key={t} value={t}>{t}</option>)}
          </select>
          {erro && <div className="text-[12px] text-[var(--color-crit)]">{erro}</div>}
          <button onClick={aplicar} disabled={busy}
            className="rounded-md bg-[var(--color-accent)] px-4 py-1.5 text-[13px] font-medium text-[var(--color-accent-fg)] disabled:opacity-50">
            {busy ? 'aplicando…' : 'aplicar re-tipagem'}
          </button>
        </div>
      )}
    </Modal>
  )
}

// ── modal molde dirigido: instrução humana → L1 cria o molde SEM gate (iterável) ──
// exportado: o L2 (gaps de dimensão em 📐 Dimensões) abre o mesmo modal pré-preenchido
export function ModalDirigido({ acao, bases, onClose, onDone }: {
  acao: AcaoDirigido
  bases: { collection: string; name: string; tipo: string }[]
  onClose: () => void; onDone: () => void
}) {
  const candidatas = bases.filter((b) => b.tipo === acao.tipo)
  const [amostra, setAmostra] = useState(acao.base ? `${acao.collection}|${acao.base}` : (candidatas[0] ? `${candidatas[0].collection}|${candidatas[0].name}` : ''))
  const [instrucao, setInstrucao] = useState(acao.instrucao ?? '')
  const [busy, setBusy] = useState(false)
  const [erro, setErro] = useState<string | null>(null)
  const [res, setRes] = useState<{ campos: number; cobertura: number; amostra: Record<string, string> } | null>(null)

  async function criar() {
    const [coll, base] = amostra.split('|')
    if (!coll || !base) { setErro('escolha a base amostra'); return }
    if (!instrucao.trim()) { setErro('diga o que extrair (é o "dirigido")'); return }
    setBusy(true); setErro(null)
    try {
      const r = await postMoldeDirigido(acao.tipo, coll, base, instrucao.trim())
      setRes({ campos: r.campos, cobertura: r.cobertura, amostra: r.amostra })
    } catch (e) { setErro(messageFromError(e)) }
    finally { setBusy(false) }
  }

  return (
    <Modal titulo={`Molde dirigido · tipo ${acao.tipo}`} onClose={onClose}>
      {res ? (
        <div className="space-y-3">
          <div className="rounded-md border border-[var(--color-ok)]/40 bg-[var(--color-ok)]/10 px-3 py-2 text-[13px]">
            molde criado: <b>{res.campos} campo(s)</b> · cobertura {(res.cobertura * 100).toFixed(0)}% na amostra —
            o L0 extrai o tipo inteiro no próximo ciclo.
          </div>
          {Object.keys(res.amostra).length > 0 && (
            <div>
              <div className="mb-1 text-[10px] font-semibold uppercase tracking-wide text-[var(--color-muted)]">extração de prova (a amostra pelo molde novo)</div>
              <pre className="max-h-[220px] overflow-auto rounded-md bg-[var(--color-panel-2)] p-2.5 font-mono text-[11px]">{JSON.stringify(res.amostra, null, 2)}</pre>
            </div>
          )}
          <div className="text-[11px] text-[var(--color-muted)]">cobertura baixa? refine a instrução e crie de novo — cada versão substitui a anterior.</div>
          <div className="flex gap-2">
            <button onClick={() => setRes(null)} className="rounded-md border border-[var(--color-border)] px-4 py-1.5 text-[13px]">refinar instrução</button>
            <button onClick={onDone} className="rounded-md bg-[var(--color-accent)] px-4 py-1.5 text-[13px] font-medium text-[var(--color-accent-fg)]">concluir</button>
          </div>
        </div>
      ) : (
        <div className="space-y-3">
          <label className="block">
            <span className="text-[11px] text-[var(--color-muted)]">base amostra (um exemplar do tipo — o L1 aprende nela)</span>
            <select value={amostra} onChange={(e) => setAmostra(e.target.value)}
              className="mt-0.5 w-full rounded-md border border-[var(--color-border)] bg-[var(--color-panel-2)] px-3 py-2 text-[13px] outline-none">
              {acao.base && !candidatas.some((b) => b.collection === acao.collection && b.name === acao.base) && (
                <option value={`${acao.collection}|${acao.base}`}>{acao.collection}/{acao.base}</option>
              )}
              {candidatas.map((b) => (
                <option key={`${b.collection}|${b.name}`} value={`${b.collection}|${b.name}`}>{b.collection}/{b.name}</option>
              ))}
            </select>
          </label>
          <label className="block">
            <span className="text-[11px] text-[var(--color-muted)]">instrução — o que extrair (você dirige; sem gate de cobertura)</span>
            <textarea value={instrucao} onChange={(e) => setInstrucao(e.target.value)} rows={3}
              placeholder="ex: extraia contratante, contratada, os CNPJs e o valor mensal"
              className="mt-0.5 w-full resize-y rounded-md border border-[var(--color-border)] bg-[var(--color-panel-2)] px-3 py-2 text-[13px] outline-none focus:border-[var(--color-accent)]" />
          </label>
          {erro && <div className="text-[12px] text-[var(--color-crit)]">{erro}</div>}
          <button onClick={criar} disabled={busy}
            className="rounded-md bg-[var(--color-accent)] px-4 py-1.5 text-[13px] font-medium text-[var(--color-accent-fg)] disabled:opacity-50">
            {busy ? 'o L1 está criando o molde… (~1-3min, o diário 🐿️ registra)' : 'criar molde dirigido'}
          </button>
        </div>
      )}
    </Modal>
  )
}

function Modal({ titulo, children, onClose }: { titulo: string; children: React.ReactNode; onClose: () => void }) {
  return (
    <div className="fixed inset-0 z-40 flex items-center justify-center bg-black/50 p-4" onClick={onClose}>
      <div className="w-full max-w-[560px] rounded-lg border border-[var(--color-border)] bg-[var(--color-panel)] p-4 shadow-xl" onClick={(e) => e.stopPropagation()}>
        <div className="mb-3 flex items-center gap-2">
          <h2 className="grow truncate text-[15px] font-semibold">{titulo}</h2>
          <button onClick={onClose} className="text-[var(--color-muted)] hover:text-[var(--color-fg)]"><X size={16} /></button>
        </div>
        {children}
      </div>
    </div>
  )
}
