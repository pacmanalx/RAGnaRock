import { Panel } from '@/components/ui'

// Abas ainda não modeladas — marcam o mapa da UI (o que vem por aí) sem fingir dado.
export function Placeholder({ title, note }: { title: string; note: string }) {
  return (
    <div className="space-y-5">
      <h1 className="text-lg font-semibold">{title}</h1>
      <Panel>
        <div className="py-10 text-center">
          <div className="text-[13px] text-[var(--color-muted)]">{note}</div>
          <div className="mt-2 text-[11px] text-[var(--color-muted)]">a modelar na fase de UI</div>
        </div>
      </Panel>
    </div>
  )
}
