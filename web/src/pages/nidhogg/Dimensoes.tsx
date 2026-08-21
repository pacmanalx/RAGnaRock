import { useEffect, useState } from 'react'
import { Compass, Pencil, Plus, Search, Trash2 } from 'lucide-react'
import { useAsync } from '@/hooks/useAsync'
import { getDimensoes, upsertDimensao, removerDimensao, getDimensaoValores, getDimensoesGaps, getNidhoggClasses } from '@/api/ragnarock'
import type { Dimensao, DimValorItem, DimGap } from '@/api/types'
import { ModalDirigido, type AcaoDirigido } from './Gaps'
import { messageFromError } from '@/api/client'
import { Panel, Spinner, ErrorBox } from '@/components/ui'

// L2 · Cadastro de Dimensões — a ponte L2→L3. O humano DECLARA os eixos que importam
// (padrões de campo como *_cnpj, mencao…); a navegação pivota por eles de forma
// determinística sobre o dump; e onde o corpus não entrega o eixo vira GAP — a demanda
// de mastigação que alimenta o molde dirigido (L1) e as perguntas da L3.

const num = (v: number | string) => (typeof v === 'number' ? v : parseInt(v || '0', 10) || 0)

export function DimensoesPanel({ colecoes, onNavegar }: {
  colecoes: string[]
  // clique num valor → abre o Think Navigator centrado nele
  onNavegar: (valor: string, norm: string, escopo: string) => void
}) {
  const dims = useAsync(getDimensoes, [])
  const classes = useAsync(getNidhoggClasses, [])   // amostras candidatas pro molde dirigido dos gaps
  const [dirigir, setDirigir] = useState<AcaoDirigido | null>(null)
  const [sel, setSel] = useState<string | null>(null)
  const [escopo, setEscopo] = useState('*')
  const [editando, setEditando] = useState<Dimensao | null>(null)
  const [novo, setNovo] = useState(false)
  const [salvando, setSalvando] = useState(false)
  const [erro, setErro] = useState<string | null>(null)

  const lista = dims.data?.dimensoes ?? []
  const ativa = lista.find((d) => d.nome === sel) ?? null

  // um eixo por vez, nunca a lista inteira: o servidor lê o que está no disco e mexe só neste.
  // `anterior` cobre o RENOMEAR: o nome é a chave, então gravar com o nome novo cria um segundo
  // eixo — o antigo precisa sair depois, e só depois que o novo já está no disco.
  async function salvar(d: Dimensao, anterior?: string) {
    setSalvando(true); setErro(null)
    try {
      await upsertDimensao(d)
      if (anterior && anterior !== d.nome) await removerDimensao(anterior)
      setEditando(null); setNovo(false)
      if (sel === anterior) setSel(d.nome)
      dims.reload()
    } catch (e) { setErro(messageFromError(e)) }
    finally { setSalvando(false) }
  }

  async function excluir(nome: string) {
    if (!confirm(`Excluir a dimensão "${nome}"? (só o eixo declarado — nenhum dado do dump é tocado)`)) return
    setSalvando(true); setErro(null)
    try {
      await removerDimensao(nome)
      if (sel === nome) setSel(null)
      dims.reload()
    } catch (e) { setErro(messageFromError(e)) }
    finally { setSalvando(false) }
  }

  return (
    <div className="grid gap-4 lg:grid-cols-[320px_1fr]">
      {/* ── coluna dos eixos declarados ── */}
      <div className="space-y-3">
        <Panel title="Eixos declarados">
          {dims.loading && <Spinner />}
          {dims.error && <ErrorBox message={messageFromError(dims.error)} onRetry={dims.reload} />}
          <div className="space-y-2">
            {lista.map((d) => (
              <div key={d.nome}
                className={`group rounded-md border px-3 py-2 transition-colors ${
                  sel === d.nome ? 'border-[var(--color-accent)] bg-[var(--color-accent)]/5' : 'border-[var(--color-border)] hover:border-[var(--color-accent)]/50'
                }`}>
                <button onClick={() => setSel(d.nome)} className="flex w-full items-center gap-2 text-left">
                  <Compass size={14} className="shrink-0 text-[var(--color-accent)]" />
                  <span className="grow truncate text-[13px] font-semibold">{d.nome}</span>
                  <span className="hidden shrink-0 gap-1 group-hover:flex">
                    <Pencil size={12} className="text-[var(--color-muted)] hover:text-[var(--color-fg)]"
                      onClick={(e) => { e.stopPropagation(); setNovo(false); setEditando({ ...d }) }} />
                    <Trash2 size={12} className="text-[var(--color-muted)] hover:text-[var(--color-crit)]"
                      onClick={(e) => { e.stopPropagation(); excluir(d.nome) }} />
                  </span>
                </button>
                {d.descricao && <div className="mt-0.5 text-[11px] text-[var(--color-muted)]">{d.descricao}</div>}
                <div className="mt-1 flex flex-wrap gap-1">
                  {d.campos.map((c) => (
                    <code key={c} className="rounded bg-[var(--color-panel-2)] px-1.5 py-0.5 text-[10px]">{c}</code>
                  ))}
                  {d.tipos.length > 0 && d.tipos.map((t) => (
                    <span key={t} className="rounded-full border border-[var(--color-border)] px-1.5 py-0.5 text-[10px] text-[var(--color-muted)]">tipo: {t}</span>
                  ))}
                </div>
              </div>
            ))}
          </div>
          <button onClick={() => { setEditando({ nome: '', descricao: '', campos: [], tipos: [] }); setNovo(true) }}
            className="mt-3 flex items-center gap-1.5 rounded-md border border-dashed border-[var(--color-border)] px-3 py-1.5 text-[12px] text-[var(--color-muted)] transition-colors hover:border-[var(--color-accent)] hover:text-[var(--color-accent)]">
            <Plus size={13} /> nova dimensão
          </button>
          {erro && <div className="mt-2 text-[12px] text-[var(--color-crit)]">{erro}</div>}
        </Panel>

        <GapsPanel escopo={escopo} recarregarKey={dims.data ? JSON.stringify(lista) : ''} onDirigir={setDirigir} />
        {dirigir && (
          <ModalDirigido acao={dirigir} bases={classes.data?.bases ?? []}
            onClose={() => setDirigir(null)} onDone={() => setDirigir(null)} />
        )}
      </div>

      {/* ── coluna de navegação (valores do eixo) ── */}
      <div className="min-w-0">
        {editando ? (
          <FormDimensao dim={editando} novo={novo} lista={lista} salvando={salvando}
            onCancel={() => { setEditando(null); setNovo(false) }}
            onSave={(d) => { void salvar(d, novo ? undefined : editando.nome) }} />
        ) : ativa ? (
          <ValoresDimensao dim={ativa} escopo={escopo} setEscopo={setEscopo} colecoes={colecoes} onNavegar={onNavegar} />
        ) : (
          <Panel title="Dimensões — eixos que forçam a mastigação">
            <div className="space-y-1.5 text-[13px] text-[var(--color-muted)]">
              <div>Uma dimensão é um <b>eixo declarado por você</b>: um conjunto de padrões de campo (<code>*_cnpj</code>, <code>mencao</code>…) e, opcionalmente, os tipos de documento que DEVEM entregá-lo.</div>
              <div>· <b>navegar</b>: clique num eixo à esquerda → todos os valores dele no dump → clique num valor → o Think Navigator abre a teia de relações dele;</div>
              <div>· <b>exigir</b>: o painel de gaps mostra os tipos do corpus onde o eixo <b>não</b> foi extraído — é a fila do molde dirigido (L1) e das perguntas da L3.</div>
            </div>
          </Panel>
        )}
      </div>
    </div>
  )
}

// ── formulário criar/editar (campos e tipos como texto separado por vírgula) ──
function FormDimensao({ dim, novo, lista, salvando, onSave, onCancel }: {
  dim: Dimensao; novo: boolean; lista: Dimensao[]; salvando: boolean
  onSave: (d: Dimensao) => void; onCancel: () => void
}) {
  const [nome, setNome] = useState(dim.nome)
  const [desc, setDesc] = useState(dim.descricao ?? '')
  const [campos, setCampos] = useState(dim.campos.join(', '))
  const [tipos, setTipos] = useState(dim.tipos.join(', '))
  const [erro, setErro] = useState<string | null>(null)

  function submeter() {
    const n = nome.trim()
    const cs = campos.split(',').map((s) => s.trim()).filter(Boolean)
    if (!n) { setErro('a dimensão precisa de um nome'); return }
    if (novo && lista.some((d) => d.nome === n)) { setErro(`já existe a dimensão "${n}"`); return }
    if (cs.length === 0) { setErro('declare ao menos um padrão de campo (ex.: *_cnpj)'); return }
    const inval = cs.find((c) => !/^[a-zA-Z0-9_*.-]+$/.test(c))
    if (inval) { setErro(`padrão inválido: "${inval}" — use letras, dígitos, _ . - e *`); return }
    onSave({ nome: n, descricao: desc.trim(), campos: cs, tipos: tipos.split(',').map((s) => s.trim()).filter(Boolean) })
  }

  return (
    <Panel title={novo ? 'Nova dimensão' : `Editar · ${dim.nome}`}>
      <div className="max-w-[560px] space-y-3 text-[13px]">
        <label className="block">
          <span className="text-[11px] text-[var(--color-muted)]">nome do eixo</span>
          <input value={nome} onChange={(e) => setNome(e.target.value)} placeholder="CNPJ/CPF"
            className="mt-0.5 w-full rounded-md border border-[var(--color-border)] bg-[var(--color-panel-2)] px-3 py-1.5 outline-none focus:border-[var(--color-accent)]" />
        </label>
        <label className="block">
          <span className="text-[11px] text-[var(--color-muted)]">descrição (o que este eixo liga)</span>
          <input value={desc} onChange={(e) => setDesc(e.target.value)} placeholder="identidade fiscal — liga contratos, comprovantes e cadastros"
            className="mt-0.5 w-full rounded-md border border-[var(--color-border)] bg-[var(--color-panel-2)] px-3 py-1.5 outline-none focus:border-[var(--color-accent)]" />
        </label>
        <label className="block">
          <span className="text-[11px] text-[var(--color-muted)]">padrões de campo (vírgula; <code>*</code> = curinga) — é o que casa contra o <code>campo</code> das extrações</span>
          <input value={campos} onChange={(e) => setCampos(e.target.value)} placeholder="*_cnpj, *_cpf, cnpj, cpf"
            className="mt-0.5 w-full rounded-md border border-[var(--color-border)] bg-[var(--color-panel-2)] px-3 py-1.5 font-mono text-[12px] outline-none focus:border-[var(--color-accent)]" />
        </label>
        <label className="block">
          <span className="text-[11px] text-[var(--color-muted)]">tipos-alvo (opcional; vazio = todo tipo extraível DEVE entregar o eixo — alimenta os gaps)</span>
          <input value={tipos} onChange={(e) => setTipos(e.target.value)} placeholder="contrato, comprovante"
            className="mt-0.5 w-full rounded-md border border-[var(--color-border)] bg-[var(--color-panel-2)] px-3 py-1.5 font-mono text-[12px] outline-none focus:border-[var(--color-accent)]" />
        </label>
        {erro && <div className="text-[12px] text-[var(--color-crit)]">{erro}</div>}
        <div className="flex gap-2">
          <button onClick={submeter} disabled={salvando}
            className="rounded-md bg-[var(--color-accent)] px-4 py-1.5 text-[13px] font-medium text-[var(--color-accent-fg)] disabled:opacity-50">
            {salvando ? 'salvando…' : 'salvar'}
          </button>
          <button onClick={onCancel} className="rounded-md border border-[var(--color-border)] px-4 py-1.5 text-[13px]">cancelar</button>
        </div>
      </div>
    </Panel>
  )
}

// ── valores do eixo: o primeiro clique da corrente de drill-down ──
function ValoresDimensao({ dim, escopo, setEscopo, colecoes, onNavegar }: {
  dim: Dimensao; escopo: string; setEscopo: (c: string) => void; colecoes: string[]
  onNavegar: (valor: string, norm: string, escopo: string) => void
}) {
  const [q, setQ] = useState('')
  const [qAplicado, setQAplicado] = useState('')
  useEffect(() => {
    const id = setTimeout(() => setQAplicado(q.trim()), 350)
    return () => clearTimeout(id)
  }, [q])
  // eixo trocou → limpa a busca
  useEffect(() => { setQ(''); setQAplicado('') }, [dim.nome])

  const vals = useAsync(() => getDimensaoValores(dim.nome, escopo, qAplicado), [dim.nome, escopo, qAplicado])

  return (
    <Panel title={`${dim.nome} · valores no dump`}>
      <div className="mb-3 flex flex-wrap items-center gap-2">
        <select value={escopo} onChange={(e) => setEscopo(e.target.value)}
          className="rounded-md border border-[var(--color-border)] bg-[var(--color-panel-2)] px-2 py-1.5 text-[12px] outline-none">
          <option value="*">todas as coleções</option>
          {colecoes.map((c) => <option key={c} value={c}>{c}</option>)}
        </select>
        <div className="relative">
          <Search size={13} className="absolute left-2.5 top-1/2 -translate-y-1/2 text-[var(--color-muted)]" />
          <input value={q} onChange={(e) => setQ(e.target.value)} placeholder="filtrar valores…"
            className="w-[220px] rounded-md border border-[var(--color-border)] bg-[var(--color-panel-2)] py-1.5 pl-8 pr-3 text-[12px] outline-none focus:border-[var(--color-accent)]" />
        </div>
        <div className="grow" />
        <span className="text-[11px] text-[var(--color-muted)]">
          campos: {dim.campos.map((c) => <code key={c} className="ml-1 rounded bg-[var(--color-panel-2)] px-1 py-0.5 text-[10px]">{c}</code>)}
        </span>
      </div>

      {vals.loading && <Spinner label="pivotando o dump…" />}
      {vals.error && <ErrorBox message={messageFromError(vals.error)} onRetry={vals.reload} />}
      {vals.data && vals.data.valores.length === 0 && (
        <div className="text-[13px] text-[var(--color-muted)]">
          nenhum valor {qAplicado ? 'casa com o filtro' : 'no dump para este eixo'} — o eixo declara,
          mas quem entrega é a extração (L1). Veja os <b>gaps</b> à esquerda: são os tipos que ainda não entregam.
        </div>
      )}
      <div className="grid gap-1 sm:grid-cols-2 xl:grid-cols-3">
        {(vals.data?.valores ?? []).map((v: DimValorItem) => (
          <button key={v.valor_norm}
            onClick={() => onNavegar(v.valor, v.valor_norm, escopo)}
            title="abrir a teia de relações no Think Navigator"
            className="flex items-center gap-2 rounded-md border border-[var(--color-border)] px-3 py-2 text-left transition-colors hover:border-[var(--color-accent)]">
            <span className="grow truncate text-[13px]">{v.valor}</span>
            <span className="shrink-0 text-[10px] tabular-nums text-[var(--color-muted)]">
              {num(v.registros)} reg · {num(v.bases)} doc · {num(v.tipos)} tipo(s)
            </span>
          </button>
        ))}
      </div>
      {vals.data && vals.data.valores.length >= 200 && (
        <div className="mt-2 text-[11px] text-[var(--color-muted)]">mostrando os 200 mais conectados — refine com o filtro.</div>
      )}
      <div className="mt-3 text-[11px] text-[var(--color-muted)]">
        clique num valor → o Think Navigator abre centrado nele (a corrente: eixo → valor → relações → …).
      </div>
    </Panel>
  )
}

// ── gaps: onde o eixo declarado NÃO alcança — é L2 (exigência determinística); o chip abre
// o molde dirigido pré-preenchido (a alavanca que re-dirige a mastigação do L1) ──
function GapsPanel({ escopo, recarregarKey, onDirigir }: {
  escopo: string; recarregarKey: string; onDirigir: (a: AcaoDirigido) => void
}) {
  const gaps = useAsync(() => getDimensoesGaps(escopo), [escopo, recarregarKey])
  const [aberto, setAberto] = useState<string | null>(null)

  return (
    <Panel title="Gaps — onde o eixo não alcança">
      {gaps.loading && <Spinner />}
      {gaps.error && <ErrorBox message={messageFromError(gaps.error)} onRetry={gaps.reload} />}
      <div className="space-y-2">
        {(gaps.data?.gaps ?? []).map((g: DimGap) => {
          const pct = g.alvo > 0 ? Math.round((g.cobertos / g.alvo) * 100) : 100
          const ok = g.gaps.length === 0
          return (
            <div key={g.nome} className="rounded-md border border-[var(--color-border)] px-3 py-2">
              <button onClick={() => setAberto(aberto === g.nome ? null : g.nome)} className="flex w-full items-center gap-2 text-left">
                <span className="grow truncate text-[12px] font-semibold">{g.nome}</span>
                <span className={`shrink-0 text-[11px] tabular-nums ${ok ? 'text-[var(--color-ok)]' : 'text-[var(--color-warn)]'}`}>
                  {g.cobertos}/{g.alvo} tipo(s) · {pct}%
                </span>
              </button>
              <div className="mt-1.5 h-1.5 overflow-hidden rounded-full bg-[var(--color-panel-2)]">
                <div className={`h-full rounded-full ${ok ? 'bg-[var(--color-ok)]' : 'bg-[var(--color-warn)]'}`} style={{ width: `${pct}%` }} />
              </div>
              {aberto === g.nome && !ok && (
                <div className="mt-2 flex flex-wrap gap-1">
                  {g.gaps.map((t) => (
                    <button key={t} title={`${g.nota} — clique pra abrir o molde dirigido pré-preenchido`}
                      onClick={() => onDirigir({ tipo: t, instrucao: `extraia os campos do eixo "${g.nome}" deste tipo de documento` })}
                      className="rounded-full border border-[var(--color-warn)]/40 bg-[var(--color-warn)]/10 px-2 py-0.5 text-[10px] transition-colors hover:border-[var(--color-accent)] hover:text-[var(--color-accent)]">{t} 🔨</button>
                  ))}
                </div>
              )}
              {aberto === g.nome && ok && (
                <div className="mt-1.5 text-[11px] text-[var(--color-muted)]">eixo plenamente alimentado neste escopo.</div>
              )}
            </div>
          )
        })}
      </div>
      <div className="mt-2 text-[11px] text-[var(--color-muted)]">
        tipo sem campo do eixo = <b>gap declarado</b> — candidato a molde dirigido (L1) e pergunta da L3.
      </div>
    </Panel>
  )
}
