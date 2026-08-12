import { Panel } from '@/components/ui'

// "Configurar tudo pra o RAGnaRock e o Nidhogg rodar" — mapa dos campos de config reais
// (ragnarock.cfg + nidhogg). Estrutura da modelagem; leitura/escrita real entra com o
// GET/POST /api/config sob auth. Read-only por ora.
type Field = { key: string; value: string; hint?: string }
type Group = { title: string; fields: Field[] }

const RAGD: Group[] = [
  { title: 'Rede & processo', fields: [
    { key: 'api_port', value: '11499' },
    { key: 'dash_port', value: '11498' },
    { key: 'workers', value: '24', hint: '0 = auto (nº CPUs, 2..16)' },
    { key: 'storage', value: 'memory', hint: 'memory | hybrid' },
    { key: 'max_upload', value: '1 GB' },
  ]},
  { title: 'Caminhos', fields: [
    { key: 'drivers_dir', value: '/opt/ragnarock/drivers' },
    { key: 'ingestors_dir', value: '/opt/ragnarock/ingestors' },
    { key: 'ragfiles_dir', value: '/dados/ragnarock/ragfiles' },
  ]},
  { title: 'IA (query expansion)', fields: [
    { key: 'active_provider', value: 'none', hint: 'none | anthropic | openai | local' },
    { key: 'local_url', value: 'http://127.0.0.1:8080/v1/chat/completions' },
  ]},
  { title: 'Sessão / admin', fields: [
    { key: 'session_ttl', value: '12h' },
    { key: 'admin_user', value: 'admin' },
  ]},
]

const NIDHOGG: Group[] = [
  { title: 'Motor', fields: [
    { key: 'port', value: '11497' },
    { key: 'nivel', value: 'consciente', hint: 'minerador (L0) | consciente (L1) | …' },
    { key: 'llm_url', value: 'http://127.0.0.1:8080', hint: 'llama local — corpus sensível não vaza' },
  ]},
  { title: 'Coleções processadas', fields: [
    { key: 'real', value: 'habilitada · IA local' },
    { key: 'simulacao', value: 'habilitada' },
    { key: 'livros', value: 'desabilitada' },
  ]},
]

function ConfigCard({ groups }: { groups: Group[] }) {
  return (
    <div className="space-y-3">
      {groups.map((g) => (
        <Panel key={g.title} title={g.title}>
          <div className="space-y-2">
            {g.fields.map((f) => (
              <div key={f.key} className="grid grid-cols-[180px_1fr] items-center gap-3">
                <label className="text-[12px] text-[var(--color-muted)]">{f.key}</label>
                <div>
                  <input
                    defaultValue={f.value}
                    readOnly
                    className="w-full rounded border border-[var(--color-border)] bg-[var(--color-panel-2)] px-2.5 py-1.5 text-[13px] text-[var(--color-fg)] outline-none"
                  />
                  {f.hint && <div className="mt-0.5 text-[10px] text-[var(--color-muted)]">{f.hint}</div>}
                </div>
              </div>
            ))}
          </div>
        </Panel>
      ))}
    </div>
  )
}

export function Config() {
  return (
    <div className="space-y-5">
      <h1 className="text-lg font-semibold">Configuração</h1>
      <div className="grid gap-5 lg:grid-cols-2">
        <div className="space-y-3">
          <h2 className="text-[13px] font-semibold uppercase tracking-wider text-[var(--color-accent)]">RAGnaRock (ragd)</h2>
          <ConfigCard groups={RAGD} />
        </div>
        <div className="space-y-3">
          <h2 className="text-[13px] font-semibold uppercase tracking-wider text-[var(--color-accent)]">Nidhogg (nidhoggd)</h2>
          <ConfigCard groups={NIDHOGG} />
        </div>
      </div>
      <div className="text-[11px] text-[var(--color-muted)]">
        Campos read-only na modelagem — edição/salvar entra com o <code>GET/POST /api/config</code> sob auth (perfil admin).
      </div>
    </div>
  )
}
