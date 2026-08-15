import { Outlet } from 'react-router-dom'
import { AlertOctagon, RefreshCw } from 'lucide-react'
import { useAsync } from '@/hooks/useAsync'
import { getNidhoggStatus } from '@/api/ragnarock'

// Casca de TODAS as telas do Nidhogg (Visão, L0…L4).
//
// Por que existe: do nível 1 pra cima o worm depende do modelo. Quando o endpoint cai, ele
// simplesmente PARA de produzir — e antes disso a tela continuava verde, mostrando o
// acumulado antigo como se estivesse tudo bem. Falha silenciosa é a pior espécie: o operador
// só descobre horas depois, ao estranhar que nada avança.
//
// O aviso é grande e vermelho de propósito, e some sozinho quando o endpoint volta (o
// keepalive do daemon sonda a cada 15s).

export function NidhoggLayout() {
  const st = useAsync(getNidhoggStatus, [])
  const s = st.data
  // enquanto carrega, não acusa nada — não assustar por latência de rede
  const fora = !!s && s.needs_ia && s.llm_online === false

  return (
    <div className="space-y-5">
      {fora && <LlmForaDoAr status={s} onRecheck={() => st.reload()} />}
      <Outlet />
    </div>
  )
}

function LlmForaDoAr({ status, onRecheck }: {
  status: { llm_tag?: string; llm_url?: string; llm_erro?: string; llm_checked?: string; level_name?: string }
  onRecheck: () => void
}) {
  return (
    <div
      role="alert"
      className="rounded-lg border-2 border-[var(--color-crit)] bg-[var(--color-crit)]/10 px-6 py-5"
    >
      <div className="flex items-start gap-4">
        <AlertOctagon size={40} className="shrink-0 text-[var(--color-crit)]" />
        <div className="min-w-0 flex-1">
          <h2 className="text-[20px] font-bold text-[var(--color-crit)]">
            O modelo de IA está fora do ar
          </h2>
          <p className="mt-1 text-[14px]">
            O Nidhogg está no nível <b>{status.level_name}</b>, que depende do modelo para
            classificar, extrair e destilar. <b>Nada novo será produzido</b> enquanto o
            endpoint não responder — o que você vê nas telas é o conhecimento já acumulado.
          </p>

          <div className="mt-4 grid gap-x-6 gap-y-1.5 text-[13px] sm:grid-cols-[auto_1fr]">
            <span className="text-[var(--color-muted)]">modelo configurado</span>
            <code className="font-medium">{status.llm_tag || '—'}</code>
            <span className="text-[var(--color-muted)]">endpoint</span>
            <code className="break-all">{status.llm_url || '—'}</code>
            {status.llm_erro && (<>
              <span className="text-[var(--color-muted)]">motivo</span>
              <span className="text-[var(--color-crit)]">{status.llm_erro}</span>
            </>)}
            {status.llm_checked && (<>
              <span className="text-[var(--color-muted)]">última sondagem</span>
              <span className="tabular-nums">{status.llm_checked}</span>
            </>)}
          </div>

          <p className="mt-4 text-[12px] text-[var(--color-muted)]">
            Onde mexer: <b>Admin → Configuração</b> (endpoint e provedores) ou
            <b> Admin → Serviços</b> (estado dos daemons). O daemon re-sonda a cada 15s e este
            aviso some sozinho quando o modelo voltar.
          </p>

          <button
            onClick={onRecheck}
            className="mt-3 flex items-center gap-1.5 rounded-md border border-[var(--color-crit)] px-3 py-1.5 text-[12px] font-medium text-[var(--color-crit)] hover:bg-[var(--color-crit)]/10"
          >
            <RefreshCw size={13} /> verificar agora
          </button>
        </div>
      </div>
    </div>
  )
}
