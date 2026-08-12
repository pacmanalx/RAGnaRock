import { useState } from 'react'
import { NavLink, Outlet } from 'react-router-dom'
import {
  PanelLeftClose, PanelLeftOpen, Eye, Search, Upload, Puzzle, Activity, ScrollText,
  Brain, Boxes, ListTree, Network, Lightbulb, Users, ShieldCheck, ServerCog, SlidersHorizontal,
  ChevronDown, LogOut, Sun, Moon, KeyRound, type LucideIcon,
} from 'lucide-react'
import { changePassword } from '@/api/auth'
import { messageFromError } from '@/api/client'
import { useAsync } from '@/hooks/useAsync'
import { getNidhoggHealth } from '@/api/ragnarock'
import { useUiStore } from '@/store/uiStore'
import { useAuthStore } from '@/store/authStore'
import { useThemeStore } from '@/store/themeStore'
import { Dot } from '@/components/ui'

type Item = { to: string; label: string; icon: LucideIcon; end?: boolean }
const NAV: { section: string; items: Item[] }[] = [
  { section: 'RAGnaRock', items: [
    { to: '/', label: 'Visão', icon: Eye, end: true },
    { to: '/comando', label: 'Comando', icon: Search },
    { to: '/ingestao', label: 'Ingestão', icon: Upload },
    { to: '/performance', label: 'Performance', icon: Activity },
  ]},
  { section: 'Nidhogg', items: [
    { to: '/nidhogg', label: 'Visão geral', icon: Brain },
    { to: '/nidhogg/miner', label: 'L0 · Minerador', icon: Boxes },
    { to: '/nidhogg/summary', label: 'L1 · Summary', icon: ListTree },
    { to: '/nidhogg/tree', label: 'L2 · KnowledgeTree', icon: Network },
    { to: '/nidhogg/gaps', label: 'L3 · Gaps & Propostas', icon: Lightbulb },
  ]},
  { section: 'Admin', items: [
    { to: '/admin/servicos', label: 'Serviços', icon: ServerCog },
    { to: '/admin/config', label: 'Configuração', icon: SlidersHorizontal },
    { to: '/admin/perfis', label: 'Perfis', icon: ShieldCheck },
    { to: '/admin/usuarios', label: 'Usuários', icon: Users },
    { to: '/admin/drivers', label: 'Drivers', icon: Puzzle },
    { to: '/admin/logs', label: 'Logs', icon: ScrollText },
  ]},
]

export function AppLayout() {
  const { data: nh } = useAsync(getNidhoggHealth, [])
  const collapsed = useUiStore((s) => s.sidebarCollapsed)
  const toggle = useUiStore((s) => s.toggleSidebar)
  const usuario = useAuthStore((s) => s.usuario)
  const logout = useAuthStore((s) => s.logout)
  const theme = useThemeStore((s) => s.theme)
  const toggleTheme = useThemeStore((s) => s.toggle)
  const [userMenu, setUserMenu] = useState(false)
  const [pwOpen, setPwOpen] = useState(false)

  return (
    <div
      className="grid h-full grid-rows-[48px_1fr]"
      style={{ gridTemplateColumns: `${collapsed ? 56 : 240}px 1fr` }}
    >
      {/* topbar */}
      <header className="col-span-2 flex items-center justify-between border-b border-[var(--color-border)] bg-[var(--color-panel)] px-3">
        <div className="flex items-center gap-2">
          <button onClick={toggle} title="colapsar barra" className="rounded p-1.5 text-[var(--color-muted)] hover:bg-[var(--color-panel-2)] hover:text-[var(--color-fg)]">
            {collapsed ? <PanelLeftOpen size={18} /> : <PanelLeftClose size={18} />}
          </button>
          <span className="text-[15px] font-bold tracking-tight">⚔ ValHalla</span>
        </div>

        <div className="flex items-center gap-4 text-[12px] text-[var(--color-muted)]">
          <span className="flex items-center gap-1.5"><Dot on={!!nh?.on} /> Nidhogg {nh ? nh.level : '—'}</span>
          {/* usuário logado (JWT depois) */}
          <div className="relative">
            <button
              onClick={() => setUserMenu((v) => !v)}
              className="flex items-center gap-2 rounded px-2 py-1 hover:bg-[var(--color-panel-2)]"
            >
              <span className="flex h-6 w-6 items-center justify-center rounded-full bg-[var(--color-accent)] text-[11px] font-bold text-[var(--color-accent-fg)]">
                {(usuario?.nome ?? '?').slice(0, 1).toUpperCase()}
              </span>
              <span className="text-[13px] text-[var(--color-fg)]">{usuario?.nome ?? 'não logado'}</span>
              <ChevronDown size={14} />
            </button>
            {userMenu && (
              <div className="absolute right-0 top-9 z-10 w-48 rounded-md border border-[var(--color-border)] bg-[var(--color-panel-2)] py-1 shadow-xl">
                <div className="px-3 py-1.5 text-[11px] text-[var(--color-muted)]">
                  {usuario?.login} · <span className="text-[var(--color-accent)]">{usuario?.perfil}</span>
                </div>
                <div className="my-1 border-t border-[var(--color-border)]" />
                {/* tema light/dark */}
                <div className="px-3 py-1 text-[10px] uppercase tracking-wider text-[var(--color-muted)]">Tema</div>
                <button onClick={toggleTheme} className="flex w-full items-center justify-between px-3 py-1.5 text-[13px] hover:bg-[var(--color-panel)]">
                  <span className="flex items-center gap-2">
                    {theme === 'dark' ? <Moon size={14} /> : <Sun size={14} />}
                    {theme === 'dark' ? 'Escuro' : 'Claro'}
                  </span>
                  <span className="text-[11px] text-[var(--color-accent)]">trocar</span>
                </button>
                <div className="my-1 border-t border-[var(--color-border)]" />
                <button onClick={() => { setPwOpen(true); setUserMenu(false) }} className="flex w-full items-center gap-2 px-3 py-1.5 text-[13px] hover:bg-[var(--color-panel)]">
                  <KeyRound size={14} /> Trocar senha
                </button>
                <button onClick={() => { logout(); setUserMenu(false) }} className="flex w-full items-center gap-2 px-3 py-1.5 text-[13px] hover:bg-[var(--color-panel)]">
                  <LogOut size={14} /> Sair
                </button>
              </div>
            )}
          </div>
        </div>
      </header>

      {pwOpen && <TrocarSenhaModal onClose={() => setPwOpen(false)} />}

      {/* nav lateral colapsável */}
      <nav className="row-start-2 overflow-y-auto border-r border-[var(--color-border)] bg-[var(--color-panel)] py-3">
        {NAV.map((grp) => (
          <div key={grp.section} className="mb-4">
            {!collapsed && (
              <div className="px-4 pb-1 text-[10px] font-semibold uppercase tracking-wider text-[var(--color-muted)]">{grp.section}</div>
            )}
            {grp.items.map((it) => {
              const Icon = it.icon
              return (
                <NavLink
                  key={it.to}
                  to={it.to}
                  end={it.end}
                  title={collapsed ? it.label : undefined}
                  className={({ isActive }) =>
                    `flex items-center gap-2.5 border-l-2 py-1.5 text-[13px] ${collapsed ? 'justify-center px-0' : 'px-4'} ${
                      isActive
                        ? 'border-[var(--color-accent)] bg-[var(--color-panel-2)] text-[var(--color-fg)]'
                        : 'border-transparent text-[var(--color-muted)] hover:bg-[var(--color-panel-2)] hover:text-[var(--color-fg)]'
                    }`
                  }
                >
                  <Icon size={16} className="shrink-0" />
                  {!collapsed && <span>{it.label}</span>}
                </NavLink>
              )
            })}
          </div>
        ))}
      </nav>

      <main className="row-start-2 overflow-y-auto p-5">
        <Outlet />
      </main>
    </div>
  )
}

// Modal de troca da PRÓPRIA senha (menu do usuário). Exige a senha atual —
// não existe fluxo de "esqueci a senha" (decisão de produto): admin regrava.
function TrocarSenhaModal({ onClose }: { onClose: () => void }) {
  const [atual, setAtual] = useState('')
  const [nova, setNova] = useState('')
  const [confirma, setConfirma] = useState('')
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [ok, setOk] = useState(false)

  async function submit(e: React.FormEvent) {
    e.preventDefault()
    if (busy) return
    if (nova !== confirma) { setError('a confirmação não confere'); return }
    if (nova.length < 6) { setError('a nova senha precisa de ao menos 6 caracteres'); return }
    setBusy(true); setError(null)
    try {
      await changePassword(atual, nova)
      setOk(true)
      setTimeout(onClose, 1200)
    } catch (err) { setError(messageFromError(err)) }
    finally { setBusy(false) }
  }

  const inputCls = 'w-full rounded-md border border-[var(--color-border)] bg-[var(--color-panel-2)] px-3 py-2 text-[13px] outline-none focus:border-[var(--color-accent)]'

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/50" onClick={onClose}>
      <form onSubmit={submit} onClick={(e) => e.stopPropagation()}
        className="w-[320px] space-y-3 rounded-lg border border-[var(--color-border)] bg-[var(--color-panel)] p-6">
        <div className="text-[15px] font-semibold">Trocar senha</div>
        <div>
          <div className="mb-1 text-[11px] uppercase tracking-wide text-[var(--color-muted)]">senha atual</div>
          <input type="password" value={atual} onChange={(e) => setAtual(e.target.value)} autoFocus autoComplete="current-password" className={inputCls} />
        </div>
        <div>
          <div className="mb-1 text-[11px] uppercase tracking-wide text-[var(--color-muted)]">nova senha</div>
          <input type="password" value={nova} onChange={(e) => setNova(e.target.value)} autoComplete="new-password" className={inputCls} />
        </div>
        <div>
          <div className="mb-1 text-[11px] uppercase tracking-wide text-[var(--color-muted)]">confirmar nova senha</div>
          <input type="password" value={confirma} onChange={(e) => setConfirma(e.target.value)} autoComplete="new-password" className={inputCls} />
        </div>
        {error && <div className="text-[12px] text-[var(--color-crit)]">{error}</div>}
        {ok && <div className="text-[12px] text-[var(--color-ok)]">senha trocada ✓</div>}
        <div className="flex gap-2">
          <button disabled={busy || !atual || !nova || !confirma}
            className="rounded-md bg-[var(--color-accent)] px-4 py-2 text-[13px] font-semibold text-[var(--color-accent-fg)] hover:opacity-90 disabled:opacity-50">
            {busy ? 'trocando…' : 'trocar'}
          </button>
          <button type="button" onClick={onClose} className="rounded-md border border-[var(--color-border)] px-4 py-2 text-[13px] hover:bg-[var(--color-panel-2)]">cancelar</button>
        </div>
      </form>
    </div>
  )
}
