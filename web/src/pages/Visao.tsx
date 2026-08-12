import { useAsync } from '@/hooks/useAsync'
import { getHealth, getCollections } from '@/api/ragnarock'
import { Panel, Metric, Spinner, ErrorBox } from '@/components/ui'

export function Visao() {
  const health = useAsync(getHealth, [])
  const cols = useAsync(getCollections, [])

  return (
    <div className="mx-auto max-w-6xl space-y-5">
      <h1 className="text-lg font-semibold">Visão geral</h1>

      {health.error && <ErrorBox message={health.error} onRetry={health.reload} />}
      {health.loading ? <Spinner /> : health.data && (
        <div className="grid grid-cols-2 gap-3 sm:grid-cols-4">
          <Metric label="Status" value={health.data.status.toUpperCase()} tone="ok" />
          <Metric label="Bases" value={health.data.bases.toLocaleString('pt-BR')} tone="accent" />
          <Metric label="Coleções" value={health.data.collections} />
          <Metric label="Drivers" value={health.data.drivers} />
        </div>
      )}

      <Panel title="Coleções">
        {cols.error && <ErrorBox message={cols.error} onRetry={cols.reload} />}
        {cols.loading ? <Spinner /> : cols.data && (
          <table className="w-full text-[13px]">
            <thead>
              <tr className="border-b border-[var(--color-border)] text-left text-[11px] uppercase tracking-wider text-[var(--color-muted)]">
                <th className="pb-2 font-medium">Coleção</th>
                <th className="pb-2 text-right font-medium">Bases</th>
                <th className="pb-2 pl-4 font-medium">Proporção</th>
              </tr>
            </thead>
            <tbody>
              {cols.data.collections.map((c) => {
                const max = Math.max(...cols.data!.collections.map((x) => x.bases))
                return (
                  <tr key={c.collection} className="border-b border-[var(--color-border)]/50">
                    <td className="py-2 font-medium">{c.collection}</td>
                    <td className="py-2 text-right tabular-nums">{c.bases.toLocaleString('pt-BR')}</td>
                    <td className="py-2 pl-4">
                      <div className="h-2 w-full rounded bg-[var(--color-panel-2)]">
                        <div className="h-2 rounded bg-[var(--color-accent)]" style={{ width: `${(c.bases / max) * 100}%` }} />
                      </div>
                    </td>
                  </tr>
                )
              })}
            </tbody>
          </table>
        )}
      </Panel>
    </div>
  )
}
