import { Panel } from '@/components/ui'
import { Plus } from 'lucide-react'

// Cadastro de perfis (papéis) — estrutura da modelagem. Define o que cada perfil pode fazer;
// alimenta o RBAC de rota (guard) quando o JWT entrar.
const PERFIS = [
  { nome: 'admin', desc: 'Controle total: config, serviços, usuários, ingestão, Nidhogg', pode: ['tudo'] },
  { nome: 'operador', desc: 'Opera o RAG: busca, ingestão, acompanha o Nidhogg', pode: ['buscar', 'ingerir', 'ver Nidhogg'] },
  { nome: 'leitor', desc: 'Somente consulta', pode: ['buscar'] },
]

export function Perfis() {
  return (
    <div className="mx-auto max-w-4xl space-y-5">
      <div className="flex items-center justify-between">
        <h1 className="text-lg font-semibold">Perfis</h1>
        <button className="flex items-center gap-1.5 rounded-md bg-[var(--color-accent)] px-3 py-1.5 text-[13px] font-semibold text-[var(--color-accent-fg)] hover:opacity-90">
          <Plus size={15} /> Novo perfil
        </button>
      </div>
      <div className="grid gap-3 sm:grid-cols-3">
        {PERFIS.map((p) => (
          <Panel key={p.nome} title={p.nome}>
            <div className="text-[12px] text-[var(--color-muted)]">{p.desc}</div>
            <div className="mt-3 flex flex-wrap gap-1.5">
              {p.pode.map((c) => (
                <span key={c} className="rounded bg-[var(--color-panel-2)] px-2 py-0.5 text-[11px] text-[var(--color-fg)]">{c}</span>
              ))}
            </div>
          </Panel>
        ))}
      </div>
    </div>
  )
}
