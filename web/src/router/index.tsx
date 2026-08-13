import { createBrowserRouter, Navigate } from 'react-router-dom'
import { AppLayout } from '@/layouts/AppLayout'
import { Login } from '@/pages/Login'
import { useAuthStore } from '@/store/authStore'

// Guard de rota: sem JWT → /login. As caps finas ficam por tela (hasCap).
function RequireAuth({ children }: { children: React.ReactNode }) {
  const ok = useAuthStore((s) => s.isAuthenticated)
  return ok ? <>{children}</> : <Navigate to="/login" replace />
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

// Router central único (molde Innova: rotas explícitas, não file-based).
export const router = createBrowserRouter([
  { path: '/login', element: <Login /> },
  {
    path: '/',
    element: <RequireAuth><AppLayout /></RequireAuth>,
    children: [
      { index: true, element: <Visao /> },
      { path: 'comando', element: <Comando /> },
      { path: 'buscar', element: <Navigate to="/comando" replace /> }, // rota antiga
      { path: 'ingestao', element: <Ingestao /> },
      { path: 'performance', element: <Performance /> },
      { path: 'nidhogg', element: <NidhoggVisao /> },
      { path: 'nidhogg/miner', element: <NidhoggMiner /> },
      { path: 'nidhogg/summary', element: <Placeholder title="L1 · Summary" note="Insights por coleção, com confiança visível." /> },
      { path: 'nidhogg/tree', element: <Placeholder title="L2 · KnowledgeTree" note="Grafo de relações sobre o dado normalizado." /> },
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
