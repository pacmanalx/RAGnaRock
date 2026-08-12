import { useEffect, useState, useCallback } from 'react'
import { messageFromError } from '@/api/client'

// Data fetching no molde Innova: hook customizado com useState/useEffect (sem react-query).
// Devolve { data, loading, error, reload } — o componente só renderiza.
export function useAsync<T>(fn: () => Promise<T>, deps: unknown[] = []) {
  const [data, setData] = useState<T | null>(null)
  const [loading, setLoading] = useState(true)
  const [error, setError] = useState<string | null>(null)

  const run = useCallback(() => {
    let alive = true
    setLoading(true)
    setError(null)
    fn()
      .then((d) => { if (alive) setData(d) })
      .catch((e) => { if (alive) setError(messageFromError(e)) })
      .finally(() => { if (alive) setLoading(false) })
    return () => { alive = false }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, deps)

  useEffect(run, [run])
  return { data, loading, error, reload: run }
}
