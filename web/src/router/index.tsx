import { createBrowserRouter, Navigate } from 'react-router-dom'
import { AppLayout } from '@/layouts/AppLayout'
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

// Router central único (molde Innova: rotas explícitas, não file-based).
export const router = createBrowserRouter([
  {
    path: '/',
    element: <AppLayout />,
    children: [
      { index: true, element: <Visao /> },
      { path: 'comando', element: <Comando /> },
      { path: 'buscar', element: <Navigate to="/comando" replace /> }, // rota antiga
      { path: 'ingestao', element: <Ingestao /> },
      { path: 'performance', element: <Performance /> },
      { path: 'nidhogg', element: <Placeholder title="Nidhogg — Visão geral" note="Estado da camada de inteligência: coleções, saturação, NQI." /> },
      { path: 'nidhogg/miner', element: <Placeholder title="L0 · Minerador" note="Nível determinístico (zero-IA): agrupa os documentos por forma em clusters e aplica os moldes/templates. Os 3 pilares e a saturação." /> },
      { path: 'nidhogg/summary', element: <Placeholder title="L1 · Summary" note="Insights por coleção, com confiança visível." /> },
      { path: 'nidhogg/tree', element: <Placeholder title="L2 · KnowledgeTree" note="Grafo de relações sobre o dado normalizado." /> },
      { path: 'nidhogg/gaps', element: <Placeholder title="L3 · Gaps & Propostas" note="Fila das perguntas não-perguntadas." /> },
      { path: 'admin/perfis', element: <Perfis /> },
      { path: 'admin/usuarios', element: <Usuarios /> },
      { path: 'admin/servicos', element: <Servicos /> },
      { path: 'admin/config', element: <Config /> },
      { path: 'admin/drivers', element: <Drivers /> },
      { path: 'admin/logs', element: <Placeholder title="Logs" note="Tail ao vivo do daemon (via /api/logs, sob auth)." /> },
      { path: '*', element: <Placeholder title="404" note="rota não encontrada" /> },
    ],
  },
])
