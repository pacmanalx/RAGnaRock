import { useState } from 'react'
import { useNavigate } from 'react-router-dom'
import { useAuthStore } from '@/store/authStore'
import { messageFromError } from '@/api/client'

// Tela de login — POST /login no ragd, JWT no authStore. Bootstrap: admin/admin (trocar!).
export function Login() {
  const nav = useNavigate()
  const doLogin = useAuthStore((s) => s.login)
  const [login, setLogin] = useState('')
  const [password, setPassword] = useState('')
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)

  async function submit(e: React.FormEvent) {
    e.preventDefault()
    if (!login.trim() || busy) return
    setBusy(true); setError(null)
    try {
      await doLogin(login.trim(), password)
      nav('/', { replace: true })
    } catch (err) { setError(messageFromError(err)) }
    finally { setBusy(false) }
  }

  const inputCls = 'w-full rounded-md border border-[var(--color-border)] bg-[var(--color-panel-2)] px-3 py-2.5 text-[14px] outline-none focus:border-[var(--color-accent)]'

  return (
    <div className="flex h-full items-center justify-center bg-[var(--color-bg)]">
      <form onSubmit={submit} className="w-[340px] space-y-4 rounded-lg border border-[var(--color-border)] bg-[var(--color-panel)] p-8">
        <div className="text-center">
          <div className="text-[22px] font-bold tracking-tight">⚔ ValHalla</div>
          <div className="mt-1 text-[12px] text-[var(--color-muted)]">console do RAGnaRock</div>
        </div>
        <div>
          <div className="mb-1 text-[11px] uppercase tracking-wide text-[var(--color-muted)]">login</div>
          <input value={login} onChange={(e) => setLogin(e.target.value)} autoFocus autoComplete="username" className={inputCls} />
        </div>
        <div>
          <div className="mb-1 text-[11px] uppercase tracking-wide text-[var(--color-muted)]">senha</div>
          <input type="password" value={password} onChange={(e) => setPassword(e.target.value)} autoComplete="current-password" className={inputCls} />
        </div>
        {error && <div className="text-[12px] text-[var(--color-crit)]">{error}</div>}
        <button disabled={busy} className="w-full rounded-md bg-[var(--color-accent)] py-2.5 text-[14px] font-semibold text-[var(--color-accent-fg)] hover:opacity-90 disabled:opacity-50">
          {busy ? 'entrando…' : 'entrar'}
        </button>
      </form>
    </div>
  )
}
