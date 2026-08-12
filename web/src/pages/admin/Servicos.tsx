import { useAsync } from '@/hooks/useAsync'
import { getHealth, getNidhoggHealth } from '@/api/ragnarock'
import { Panel, Dot, Spinner } from '@/components/ui'

// Painel de serviços do servidor: status ao vivo (real) + ações de restart (a plugar quando
// houver auth/endpoint — hoje o ragd tem POST /api/restart sob sessão; nidhoggd via systemctl).
export function Servicos() {
  const ragd = useAsync(getHealth, [])
  const nid = useAsync(getNidhoggHealth, [])

  const rows = [
    { nome: 'ragd', desc: 'Motor de busca + API (:11499) + ValHalla (:11498)', up: !!ragd.data && !ragd.error, detail: ragd.data ? `${ragd.data.bases} bases · ${ragd.data.drivers} drivers` : ragd.error ?? '' },
    { nome: 'nidhoggd', desc: 'Camada de inteligência (:11497)', up: !!nid.data?.on, detail: nid.data ? `v${nid.data.version} · nível ${nid.data.level}` : nid.error ?? '' },
  ]

  return (
    <div className="mx-auto max-w-4xl space-y-5">
      <h1 className="text-lg font-semibold">Serviços do servidor</h1>
      {(ragd.loading || nid.loading) && <Spinner />}
      <Panel title="Daemons">
        <table className="w-full text-[13px]">
          <thead>
            <tr className="border-b border-[var(--color-border)] text-left text-[11px] uppercase tracking-wider text-[var(--color-muted)]">
              <th className="pb-2 font-medium">Serviço</th>
              <th className="pb-2 font-medium">Estado</th>
              <th className="pb-2 font-medium">Detalhe</th>
              <th className="pb-2 text-right font-medium">Ações</th>
            </tr>
          </thead>
          <tbody>
            {rows.map((r) => (
              <tr key={r.nome} className="border-b border-[var(--color-border)]/50">
                <td className="py-2.5">
                  <div className="font-semibold">{r.nome}</div>
                  <div className="text-[11px] text-[var(--color-muted)]">{r.desc}</div>
                </td>
                <td className="py-2.5">
                  <span className="flex items-center gap-1.5"><Dot on={r.up} /> {r.up ? 'ativo' : 'fora'}</span>
                </td>
                <td className="py-2.5 text-[12px] text-[var(--color-muted)]">{r.detail}</td>
                <td className="py-2.5 text-right">
                  <button disabled className="cursor-not-allowed rounded border border-[var(--color-border)] px-2.5 py-1 text-[12px] text-[var(--color-muted)] opacity-60" title="a plugar com auth">
                    restart
                  </button>
                </td>
              </tr>
            ))}
          </tbody>
        </table>
        <div className="mt-3 text-[11px] text-[var(--color-muted)]">
          Restart via <code>systemctl</code> no servidor — habilita quando o JWT/auth entrar (ação sensível, exige perfil admin).
        </div>
      </Panel>
    </div>
  )
}
