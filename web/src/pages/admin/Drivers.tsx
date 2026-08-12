import { useState } from 'react'
import { useAsync } from '@/hooks/useAsync'
import { getDrivers, getThesaurus } from '@/api/ragnarock'
import { Panel, Spinner, ErrorBox, Dot } from '@/components/ui'

// Os 3 tipos de driver do RAGnaRock: linguagem (.drv, tokeniza código), dicionários
// (thesaurus por-palavra) e ingestores (.py, converte formato na entrada).
type Tab = 'lang' | 'dicts' | 'ingestors'
const TABS: { id: Tab; label: string; hint: string }[] = [
  { id: 'lang', label: 'Linguagem', hint: '.drv — tokeniza código-fonte por linguagem' },
  { id: 'dicts', label: 'Dicionários', hint: 'thesaurus por-palavra (expansão)' },
  { id: 'ingestors', label: 'Ingestores', hint: '.py — converte pdf/docx/xlsx/csv/banco na entrada' },
]

// ingestores ainda não têm endpoint de listagem no ragd — lista conhecida (a plugar).
const INGESTORS = [
  { name: 'csv.py', dep: 'stdlib', out: 'CSV', kinds: '.csv' },
  { name: 'xlsx.py', dep: 'openpyxl', out: 'CSV', kinds: '.xlsx (+ MIME)' },
  { name: 'pdf.py', dep: 'pypdf', out: 'texto', kinds: '.pdf (+ MIME)' },
  { name: 'docx.py', dep: 'python-docx', out: 'texto', kinds: '.docx (+ MIME)' },
  { name: 'mysql.py', dep: 'pymysql', out: 'CSV', kinds: 'receita de rede' },
  { name: 'postgres.py', dep: 'psycopg2', out: 'CSV', kinds: 'receita de rede' },
]

export function Drivers() {
  const [tab, setTab] = useState<Tab>('lang')
  const lang = useAsync(getDrivers, [])
  const dicts = useAsync(getThesaurus, [])

  return (
    <div className="space-y-4">
      <h1 className="text-lg font-semibold">Drivers</h1>

      <div className="flex gap-1 border-b border-[var(--color-border)]">
        {TABS.map((t) => (
          <button
            key={t.id}
            onClick={() => setTab(t.id)}
            className={`border-b-2 px-4 py-2 text-[13px] ${
              tab === t.id
                ? 'border-[var(--color-accent)] text-[var(--color-fg)]'
                : 'border-transparent text-[var(--color-muted)] hover:text-[var(--color-fg)]'
            }`}
          >
            {t.label}
          </button>
        ))}
      </div>
      <div className="text-[11px] text-[var(--color-muted)]">{TABS.find((t) => t.id === tab)!.hint}</div>

      {tab === 'lang' && (
        <Panel title={lang.data ? `${lang.data.drivers.length} drivers de linguagem` : 'Linguagem'}>
          {lang.error && <ErrorBox message={lang.error} onRetry={lang.reload} />}
          {lang.loading ? <Spinner /> : lang.data && (
            <table className="w-full text-[13px]">
              <thead>
                <tr className="border-b border-[var(--color-border)] text-left text-[11px] uppercase tracking-wider text-[var(--color-muted)]">
                  <th className="pb-2 font-medium">Linguagem</th>
                  <th className="pb-2 font-medium">Extensões</th>
                  <th className="pb-2 text-right font-medium">Sílabas</th>
                </tr>
              </thead>
              <tbody>
                {lang.data.drivers.map((d) => (
                  <tr key={d.name} className="border-b border-[var(--color-border)]/50">
                    <td className="py-2 font-medium">{d.language}</td>
                    <td className="py-2 text-[12px] text-[var(--color-muted)]">{d.extensions.join(' ')}</td>
                    <td className="py-2 text-right tabular-nums">{d.syllables.toLocaleString('pt-BR')}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          )}
        </Panel>
      )}

      {tab === 'dicts' && (
        <Panel title={dicts.data ? `${dicts.data.active}/${dicts.data.count} dicionários ativos` : 'Dicionários'}>
          {dicts.error && <ErrorBox message={dicts.error} onRetry={dicts.reload} />}
          {dicts.loading ? <Spinner /> : dicts.data && (
            <table className="w-full text-[13px]">
              <thead>
                <tr className="border-b border-[var(--color-border)] text-left text-[11px] uppercase tracking-wider text-[var(--color-muted)]">
                  <th className="pb-2 font-medium">Código</th>
                  <th className="pb-2 font-medium">Fonte</th>
                  <th className="pb-2 text-right font-medium">Entradas</th>
                  <th className="pb-2 text-right font-medium">Ativo</th>
                </tr>
              </thead>
              <tbody>
                {dicts.data.dicts.map((d) => (
                  <tr key={d.code} className="border-b border-[var(--color-border)]/50">
                    <td className="py-2 font-medium">{d.code}</td>
                    <td className="py-2 text-[12px] text-[var(--color-muted)]">{d.source}</td>
                    <td className="py-2 text-right tabular-nums">{d.entries.toLocaleString('pt-BR')}</td>
                    <td className="py-2 text-right"><Dot on={d.active} /></td>
                  </tr>
                ))}
              </tbody>
            </table>
          )}
        </Panel>
      )}

      {tab === 'ingestors' && (
        <Panel title={`${INGESTORS.length} ingestores`}>
          <table className="w-full text-[13px]">
            <thead>
              <tr className="border-b border-[var(--color-border)] text-left text-[11px] uppercase tracking-wider text-[var(--color-muted)]">
                <th className="pb-2 font-medium">Driver</th>
                <th className="pb-2 font-medium">Dependência</th>
                <th className="pb-2 font-medium">Entrada</th>
                <th className="pb-2 font-medium">Saída</th>
              </tr>
            </thead>
            <tbody>
              {INGESTORS.map((i) => (
                <tr key={i.name} className="border-b border-[var(--color-border)]/50">
                  <td className="py-2 font-medium">{i.name}</td>
                  <td className="py-2 text-[12px] text-[var(--color-muted)]">{i.dep}</td>
                  <td className="py-2 text-[12px]">{i.kinds}</td>
                  <td className="py-2 text-[12px]">{i.out}</td>
                </tr>
              ))}
            </tbody>
          </table>
          <div className="mt-3 text-[11px] text-[var(--color-muted)]">lista fixa — o ragd ainda não expõe <code>/ingestors</code>; a plugar.</div>
        </Panel>
      )}
    </div>
  )
}
