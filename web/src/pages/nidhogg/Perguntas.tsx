import { useEffect, useState } from 'react'
import { HelpCircle, Play, Plus, Table2, Timer, Trash2, Zap } from 'lucide-react'
import { useAsync } from '@/hooks/useAsync'
import { getPerguntas, savePerguntas, getTimeline, perguntarAgora, getNidhoggCollections } from '@/api/ragnarock'
import type { Pergunta, TipoResposta, EtapaResposta, RespostaTabular } from '@/api/types'
import { messageFromError } from '@/api/client'
import { Panel, Spinner, ErrorBox } from '@/components/ui'

// L4 · Perguntas — a camada PROPOSITIVA. Você cadastra a questão direta ("quanto faturamos
// este mês?"); o sistema monta o contexto por REGRA (agregados do dump + registros + trechos
// do corpus pelo RAG) e o LLM responde. É "determinística do ponto de inferência": o QUE vai
// pro modelo é decidido por regra, só a resposta é inferida.
// A TIMELINE é o histórico de MUDANÇAS DE PERSPECTIVA: o worm re-responde todo ciclo e um
// comparador decide se virou etapa nova — repetição não polui a linha.

const TIPOS: { v: TipoResposta; label: string; hint: string; icon: typeof Table2 }[] = [
  { v: 'tabular', label: 'tabular cumulativa', hint: 'a resposta é uma tabela que soma/conta sobre o dump acumulado', icon: Table2 },
  { v: 'oneshot', label: 'one shot', hint: 'fato que não muda: responde uma vez e congela', icon: Zap },
  { v: 'vivo', label: 'dado vivo', hint: 're-responde a cada ciclo; cada mudança de perspectiva vira etapa na timeline', icon: Timer },
]

const inputCls = 'w-full rounded-md border border-[var(--color-border)] bg-[var(--color-panel-2)] px-3 py-2 text-[13px] outline-none focus:border-[var(--color-accent)]'

export function PerguntasPanel() {
  const ps = useAsync(getPerguntas, [])
  const colls = useAsync(getNidhoggCollections, [])
  const [sel, setSel] = useState<string | null>(null)
  const [nova, setNova] = useState<Pergunta | null>(null)
  const [salvando, setSalvando] = useState(false)
  const [erro, setErro] = useState<string | null>(null)

  const lista = ps.data?.perguntas ?? []
  const ativa = lista.find((p) => p.nome === sel) ?? null

  useEffect(() => { if (!sel && lista.length > 0) setSel(lista[0].nome) }, [lista, sel])

  async function persistir(novaLista: Pergunta[]) {
    setSalvando(true); setErro(null)
    try { await savePerguntas(novaLista); setNova(null); ps.reload() }
    catch (e) { setErro(messageFromError(e)) }
    finally { setSalvando(false) }
  }

  function excluir(nome: string) {
    if (!confirm(`Excluir a pergunta "${nome}"? (a timeline de respostas já gravada permanece no dump)`)) return
    if (sel === nome) setSel(null)
    void persistir(lista.filter((p) => p.nome !== nome))
  }

  function alternarAtiva(p: Pergunta) {
    void persistir(lista.map((x) => (x.nome === p.nome ? { ...x, ativa: !x.ativa } : x)))
  }

  return (
    <div className="grid gap-4 lg:grid-cols-[340px_1fr]">
      {/* ── coluna do cadastro ── */}
      <div className="space-y-3">
        <Panel title="Questões cadastradas"
          actions={
            <button onClick={() => setNova({ nome: '', texto: '', tipo: 'vivo', escopo: '*', ativa: true })}
              className="flex items-center gap-1 text-[11px] text-[var(--color-accent)]">
              <Plus size={12} /> nova
            </button>
          }>
          {ps.loading && <Spinner />}
          {ps.error && <ErrorBox message={messageFromError(ps.error)} onRetry={ps.reload} />}
          {!ps.loading && lista.length === 0 && !nova && (
            <div className="text-[13px] text-[var(--color-muted)]">
              nenhuma questão ainda. Cadastre a primeira — ex.: <i>"quanto faturamos este mês?"</i> ou
              <i> "qual o ROI mensurável do contrato X?"</i>
            </div>
          )}
          <div className="space-y-1.5">
            {lista.map((p) => {
              const T = TIPOS.find((t) => t.v === p.tipo) ?? TIPOS[2]
              return (
                <div key={p.nome}
                  className={`rounded-md border px-3 py-2 ${sel === p.nome
                    ? 'border-[var(--color-accent)] bg-[var(--color-accent)]/5'
                    : 'border-[var(--color-border)] hover:border-[var(--color-muted)]'}`}>
                  <button onClick={() => setSel(p.nome)} className="w-full text-left">
                    <div className="flex items-center gap-1.5">
                      <T.icon size={12} className="shrink-0 text-[var(--color-accent)]" />
                      <span className="grow truncate text-[13px] font-medium">{p.nome}</span>
                      {!p.ativa && <span className="rounded bg-[var(--color-panel-2)] px-1.5 text-[10px] text-[var(--color-muted)]">pausada</span>}
                    </div>
                    <div className="mt-0.5 line-clamp-2 text-[11px] text-[var(--color-muted)]">{p.texto}</div>
                  </button>
                  <div className="mt-1.5 flex items-center gap-2 text-[10px] text-[var(--color-muted)]">
                    <span>{T.label}</span>
                    <span>· escopo {p.escopo === '*' ? 'todas' : p.escopo}</span>
                    {p.pai && <span title={`desdobrada de "${p.pai}"`}>· ↳ {p.pai}</span>}
                    <div className="grow" />
                    <button onClick={() => alternarAtiva(p)} className="hover:text-[var(--color-fg)]">
                      {p.ativa ? 'pausar' : 'ativar'}
                    </button>
                    <button onClick={() => excluir(p.nome)} className="hover:text-[var(--color-crit)]"><Trash2 size={11} /></button>
                  </div>
                </div>
              )
            })}
          </div>
          {erro && <div className="mt-2 text-[12px] text-[var(--color-crit)]">{erro}</div>}
        </Panel>

        {nova && (
          <EditorPergunta
            valor={nova} setValor={setNova} salvando={salvando}
            colecoes={(colls.data?.collections ?? []).map((c) => c.collection)}
            onCancel={() => setNova(null)}
            onSave={() => {
              if (lista.some((p) => p.nome === nova.nome.trim())) { setErro('já existe pergunta com esse nome'); return }
              void persistir([...lista, { ...nova, nome: nova.nome.trim() }])
            }}
          />
        )}
      </div>

      {/* ── coluna da resposta + timeline ── */}
      {ativa
        ? <Respostas pergunta={ativa} onDesdobrar={(texto) => {
            setNova({ nome: '', texto, tipo: 'vivo', escopo: ativa.escopo, ativa: true, pai: ativa.nome })
          }} />
        : <Panel title="Resposta"><div className="text-[13px] text-[var(--color-muted)]">selecione uma questão à esquerda.</div></Panel>}
    </div>
  )
}

function EditorPergunta({ valor, setValor, onSave, onCancel, salvando, colecoes }: {
  valor: Pergunta; setValor: (p: Pergunta) => void
  onSave: () => void; onCancel: () => void; salvando: boolean; colecoes: string[]
}) {
  return (
    <Panel title="Nova questão">
      <div className="space-y-2.5">
        <label className="block">
          <span className="text-[11px] text-[var(--color-muted)]">nome curto (identifica a questão e a timeline dela)</span>
          <input value={valor.nome} onChange={(e) => setValor({ ...valor, nome: e.target.value })}
            placeholder="faturamento-mes" className={`mt-0.5 ${inputCls}`} />
        </label>
        <label className="block">
          <span className="text-[11px] text-[var(--color-muted)]">a pergunta, em português direto</span>
          <textarea value={valor.texto} onChange={(e) => setValor({ ...valor, texto: e.target.value })} rows={2}
            placeholder="quanto faturamos este mês?" className={`mt-0.5 resize-y ${inputCls}`} />
        </label>
        <div>
          <span className="text-[11px] text-[var(--color-muted)]">tipo de resposta</span>
          <div className="mt-1 grid gap-1.5">
            {TIPOS.map((t) => (
              <button key={t.v} onClick={() => setValor({ ...valor, tipo: t.v })} title={t.hint}
                className={`flex items-center gap-2 rounded-md border px-2.5 py-1.5 text-left text-[12px] ${valor.tipo === t.v
                  ? 'border-[var(--color-accent)] bg-[var(--color-accent)]/10'
                  : 'border-[var(--color-border)] hover:border-[var(--color-muted)]'}`}>
                <t.icon size={13} className="shrink-0 text-[var(--color-accent)]" />
                <span className="font-medium">{t.label}</span>
                <span className="truncate text-[10px] text-[var(--color-muted)]">{t.hint}</span>
              </button>
            ))}
          </div>
        </div>
        <label className="block">
          <span className="text-[11px] text-[var(--color-muted)]">escopo (onde procurar a resposta)</span>
          <select value={valor.escopo} onChange={(e) => setValor({ ...valor, escopo: e.target.value })} className={`mt-0.5 ${inputCls}`}>
            <option value="*">todas as coleções</option>
            {colecoes.map((c) => <option key={c} value={c}>{c}</option>)}
          </select>
        </label>
        {valor.pai && (
          <div className="rounded-md border border-[var(--color-border)] px-2.5 py-1.5 text-[11px] text-[var(--color-muted)]">
            desdobramento de <b>{valor.pai}</b> — a dimensão que a resposta anterior não cobria.
          </div>
        )}
        <div className="flex gap-2">
          <button onClick={onSave} disabled={salvando || !valor.nome.trim() || !valor.texto.trim()}
            className="rounded-md bg-[var(--color-accent)] px-4 py-1.5 text-[13px] font-medium text-[var(--color-accent-fg)] disabled:opacity-50">
            {salvando ? 'salvando…' : 'cadastrar'}
          </button>
          <button onClick={onCancel} className="rounded-md border border-[var(--color-border)] px-4 py-1.5 text-[13px]">cancelar</button>
        </div>
      </div>
    </Panel>
  )
}

// ── a resposta corrente (cabeça da timeline) + as etapas anteriores ──
function Respostas({ pergunta, onDesdobrar }: { pergunta: Pergunta; onDesdobrar: (texto: string) => void }) {
  const tl = useAsync(() => getTimeline(pergunta.nome), [pergunta.nome])
  const [busy, setBusy] = useState(false)
  const [nota, setNota] = useState<string | null>(null)
  const [erro, setErro] = useState<string | null>(null)

  const etapas = tl.data?.etapas ?? []
  const cabeca = etapas.length > 0 ? etapas[etapas.length - 1] : null
  const anteriores = etapas.slice(0, -1).reverse()

  async function responder() {
    setBusy(true); setErro(null); setNota(null)
    try {
      const r = await perguntarAgora(pergunta.nome)
      setNota(r.nova_etapa ? `etapa ${r.seq} gravada — ${r.mudou}` : (r.note ?? 'sem mudança de perspectiva'))
      tl.reload()
    } catch (e) { setErro(messageFromError(e)) }
    finally { setBusy(false) }
  }

  return (
    <div className="space-y-3">
      <Panel title={`❓ ${pergunta.texto}`}
        actions={
          <button onClick={responder} disabled={busy}
            className="flex items-center gap-1 rounded-md bg-[var(--color-accent)] px-3 py-1 text-[11px] font-medium text-[var(--color-accent-fg)] disabled:opacity-50">
            <Play size={11} /> {busy ? 'analisando…' : 'responder agora'}
          </button>
        }>
        {busy && <div className="mb-2 text-[12px] text-[var(--color-muted)]">montando o contexto e chamando o analista — leva de 1 a 4 min; o diário 🐿️ registra cada passo.</div>}
        {nota && <div className="mb-2 rounded-md border border-[var(--color-ok)]/40 bg-[var(--color-ok)]/10 px-3 py-2 text-[12px]">{nota}</div>}
        {erro && <div className="mb-2 text-[12px] text-[var(--color-crit)]">{erro}</div>}
        {tl.loading && <Spinner />}
        {tl.error && <ErrorBox message={messageFromError(tl.error)} onRetry={tl.reload} />}
        {!tl.loading && !cabeca && (
          <div className="text-[13px] text-[var(--color-muted)]">
            ainda sem resposta. O worm responde no próximo ciclo (nível 4 ligado) — ou clique em <b>responder agora</b>.
          </div>
        )}
        {cabeca && <CorpoResposta etapa={cabeca} onDesdobrar={onDesdobrar} />}
      </Panel>

      {anteriores.length > 0 && (
        <Panel title={`🕐 Timeline · ${etapas.length} etapa(s) — o mesmo resultado sob a perspectiva de cada ciclo`}>
          <div className="space-y-2">
            {anteriores.map((e) => (
              <details key={String(e.seq)} className="rounded-md border border-[var(--color-border)] px-3 py-2">
                <summary className="cursor-pointer text-[12px]">
                  <span className="font-mono text-[10px] text-[var(--color-muted)]">#{String(e.seq)} · {e.at}</span>
                  <span className="ml-2">{e.mudou}</span>
                </summary>
                <div className="mt-2"><CorpoResposta etapa={e} onDesdobrar={onDesdobrar} compacto /></div>
              </details>
            ))}
          </div>
          <div className="mt-2 text-[11px] text-[var(--color-muted)]">
            cada etapa é uma MUDANÇA de perspectiva — o worm re-responde todo ciclo, mas o comparador só grava quando o entendimento muda.
          </div>
        </Panel>
      )}
    </div>
  )
}

function CorpoResposta({ etapa, onDesdobrar, compacto }: {
  etapa: EtapaResposta; onDesdobrar: (texto: string) => void; compacto?: boolean
}) {
  let tabela: RespostaTabular | null = null
  if (etapa.tipo === 'tabular') {
    try { tabela = JSON.parse(etapa.resposta) as RespostaTabular } catch { tabela = null }
  }
  return (
    <div className="space-y-2.5">
      {tabela ? (
        <div className="overflow-x-auto">
          <table className="w-full text-[12px]">
            <thead>
              <tr className="border-b border-[var(--color-border)] text-left text-[10px] uppercase tracking-wide text-[var(--color-muted)]">
                {tabela.colunas.map((c) => <th key={c} className="py-1 pr-3 font-medium">{c}</th>)}
              </tr>
            </thead>
            <tbody>
              {tabela.linhas.map((l, i) => (
                <tr key={i} className="border-b border-[var(--color-border)]/50">
                  {l.map((cel, j) => <td key={j} className="py-1 pr-3 tabular-nums">{cel}</td>)}
                </tr>
              ))}
            </tbody>
          </table>
          {tabela.nota && <div className="mt-1.5 text-[11px] text-[var(--color-muted)]">{tabela.nota}</div>}
        </div>
      ) : (
        <div className="whitespace-pre-wrap text-[13px] leading-relaxed">{etapa.resposta}</div>
      )}

      {etapa.fontes.length > 0 && (
        <div className="flex flex-wrap items-center gap-1.5">
          <span className="text-[10px] uppercase tracking-wide text-[var(--color-muted)]">fontes:</span>
          {etapa.fontes.map((f, i) => (
            <span key={i} title={f.trecho} className="rounded-full border border-[var(--color-border)] px-2 py-0.5 font-mono text-[10px] text-[var(--color-muted)]">
              {f.base}
            </span>
          ))}
        </div>
      )}

      {!compacto && etapa.proximas.length > 0 && (
        <div>
          <div className="mb-1 text-[10px] uppercase tracking-wide text-[var(--color-muted)]">
            💡 dimensões não exploradas — clique pra desdobrar numa questão-filha
          </div>
          <div className="flex flex-wrap gap-1.5">
            {etapa.proximas.map((p, i) => (
              <button key={i} onClick={() => onDesdobrar(p)}
                className="flex items-center gap-1 rounded-full border border-[var(--color-border)] px-2.5 py-1 text-left text-[11px] hover:border-[var(--color-accent)] hover:text-[var(--color-accent)]">
                <HelpCircle size={11} className="shrink-0" /> {p}
              </button>
            ))}
          </div>
        </div>
      )}
    </div>
  )
}
