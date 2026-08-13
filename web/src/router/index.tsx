import { createBrowserRouter, Navigate } from 'react-router-dom'
import { AppLayout } from '@/layouts/AppLayout'
import { Login } from '@/pages/Login'
import { useAuthStore } from '@/store/authStore'

// Guard de rota: sem JWT → /login. As caps finas ficam por tela (hasCap).
function RequireAuth({ children }: { children: React.ReactNode }) {
  const ok = useAuthStore((s) => s.isAuthenticated)
  return ok ? <>{children}</> : <Navigate to="/login" replace />
}

// Erro de runtime numa tela não pode virar tela branca do router — mostra e oferece recarregar.
function ErroDeTela() {
  return (
    <div className="flex h-full flex-col items-center justify-center gap-3 bg-[var(--color-bg)] p-8 text-center">
      <div className="text-[28px]">⚔💥</div>
      <div className="text-[16px] font-semibold">algo quebrou nesta tela</div>
      <div className="max-w-[420px] text-[13px] text-[var(--color-muted)]">
        o erro ficou registrado no console do navegador (F12) — recarregar resolve a sessão; se repetir, manda o erro pro Claude.
      </div>
      <button onClick={() => window.location.reload()}
        className="rounded-md bg-[var(--color-accent)] px-4 py-2 text-[13px] font-semibold text-[var(--color-accent-fg)] hover:opacity-90">
        recarregar
      </button>
    </div>
  )
}
import { Visao } from '@/pages/Visao'
import { Comando } from '@/pages/Comando'
import { Ingestao } from '@/pages/Ingestao'
import { Performance } from '@/pages/Performance'
import { Placeholder } from '@/pages/Placeholder'
import { Perfis } from '@/pages/admin/Perfis'
import { Usuarios } from '@/pages/admin/Usuarios'
import { Servicos } from '@/pages/admin/Servicos'
import { Config } from '@/pages/admin/Config'
import { Drivers } from '@/pages/admin/Drivers'
import { Logs } from '@/pages/admin/Logs'
import { NidhoggVisao } from '@/pages/nidhogg/Visao'
import { NidhoggMiner } from '@/pages/nidhogg/Miner'
import { NidhoggSummary } from '@/pages/nidhogg/Summary'
import { NidhoggTree } from '@/pages/nidhogg/Tree'

// Router central único (molde Innova: rotas explícitas, não file-based).
export const router = createBrowserRouter([
  { path: '/login', element: <Login />, errorElement: <ErroDeTela /> },
  {
    path: '/',
    element: <RequireAuth><AppLayout /></RequireAuth>,
    errorElement: <ErroDeTela />,
    children: [
      { index: true, element: <Visao /> },
      { path: 'comando', element: <Comando /> },
      { path: 'buscar', element: <Navigate to="/comando" replace /> }, // rota antiga
      { path: 'ingestao', element: <Ingestao /> },
      { path: 'performance', element: <Performance /> },
      { path: 'nidhogg', element: <NidhoggVisao /> },
      { path: 'nidhogg/miner', element: <NidhoggMiner /> },
      { path: 'nidhogg/summary', element: <NidhoggSummary /> },
      { path: 'nidhogg/tree', element: <NidhoggTree /> },
      { path: 'nidhogg/gaps', element: <Placeholder title="L3 · Gaps & Propostas" note="Fila das perguntas não-perguntadas." /> },
      { path: 'admin/perfis', element: <Perfis /> },
      { path: 'admin/usuarios', element: <Usuarios /> },
      { path: 'admin/servicos', element: <Servicos /> },
      { path: 'admin/config', element: <Config /> },
      { path: 'admin/drivers', element: <Drivers /> },
      { path: 'admin/logs', element: <Logs /> },
      { path: '*', element: <Placeholder title="404" note="rota não encontrada" /> },
    ],
  },
])
