import type { ReactNode } from 'react'

// Primitivos mínimos (cockpit denso). Serão trocados por Shadcn/design-system
// quando a linguagem visual for definida — por ora, o suficiente pra modelar.

export function Panel({ title, actions, children, className = '' }: {
  title?: ReactNode; actions?: ReactNode; children: ReactNode; className?: string
}) {
  return (
    <section className={`rounded-lg border border-[var(--color-border)] bg-[var(--color-panel)] ${className}`}>
      {title && (
        <header className="flex items-center justify-between border-b border-[var(--color-border)] px-4 py-2.5">
          <h2 className="text-[13px] font-semibold tracking-wide text-[var(--color-fg)]">{title}</h2>
          {actions}
        </header>
      )}
      <div className="p-4">{children}</div>
    </section>
  )
}

export function Metric({ label, value, hint, tone = 'fg' }: {
  label: string; value: ReactNode; hint?: string; tone?: 'fg' | 'ok' | 'warn' | 'crit' | 'accent'
}) {
  const color = `var(--color-${tone === 'fg' ? 'fg' : tone})`
  return (
    <div className="rounded-md border border-[var(--color-border)] bg-[var(--color-panel-2)] px-4 py-3">
      <div className="text-[11px] uppercase tracking-wider text-[var(--color-muted)]">{label}</div>
      <div className="mt-1 text-2xl font-semibold tabular-nums" style={{ color }}>{value}</div>
      {hint && <div className="mt-0.5 text-[11px] text-[var(--color-muted)]">{hint}</div>}
    </div>
  )
}

export function Dot({ on }: { on: boolean }) {
  return <span className="inline-block h-2 w-2 rounded-full" style={{ background: on ? 'var(--color-ok)' : 'var(--color-crit)' }} />
}

export function Spinner({ label = 'carregando…' }: { label?: string }) {
  return <div className="py-8 text-center text-[13px] text-[var(--color-muted)]">{label}</div>
}

export function ErrorBox({ message, onRetry }: { message: string; onRetry?: () => void }) {
  return (
    <div className="rounded-md border border-[var(--color-crit)]/40 bg-[var(--color-crit)]/10 px-4 py-3 text-[13px]">
      <span className="text-[var(--color-crit)]">falha:</span> {message}
      {onRetry && (
        <button onClick={onRetry} className="ml-3 rounded border border-[var(--color-border)] px-2 py-0.5 text-[12px] hover:bg-[var(--color-panel-2)]">
          tentar de novo
        </button>
      )}
    </div>
  )
}
