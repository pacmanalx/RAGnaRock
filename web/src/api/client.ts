// Camada de API — molde Innova (fetch nativo tipado, sem axios/react-query), ADAPTADA pros
// DOIS backends do RAGnaRock. `makeClient(baseUrl)` gera um cliente; exportamos um por backend.
// Base URLs vêm de env (VITE_RAGD_URL / VITE_NIDHOGG_URL) — o "desacoplável no futuro".
// Auth (JWT) fica pra depois: por ora só os GET/POST públicos do ragd, sem Authorization.

export class HttpError extends Error {
  status: number
  constructor(status: number, message: string) {
    super(message)
    this.name = 'HttpError'
    this.status = status
  }
}

export function messageFromError(e: unknown): string {
  if (e instanceof HttpError) return `${e.status}: ${e.message}`
  if (e instanceof Error) return e.message
  return String(e)
}

type Json = Record<string, unknown>

// Respostas de API nunca devem cachear (o server já manda no-store; isto reforça contra
// caches teimosos): URL única por request + cache:'no-store' no fetch.
function bust(path: string): string {
  return `${path}${path.includes('?') ? '&' : '?'}_cb=${Date.now()}`
}

function makeClient(baseUrl: string) {
  async function request<T>(method: string, path: string, body?: Json): Promise<T> {
    const res = await fetch(`${baseUrl}${bust(path)}`, {
      method,
      cache: 'no-store',
      headers: body ? { 'Content-Type': 'application/json' } : undefined,
      body: body ? JSON.stringify(body) : undefined,
    })
    const text = await res.text()
    const data = text ? JSON.parse(text) : null
    if (!res.ok) {
      const msg = (data && (data.error as string)) || res.statusText
      throw new HttpError(res.status, msg)
    }
    return data as T
  }
  // POST com corpo BINÁRIO cru (upload de arquivo). O ragd /ingest_any lê os bytes do body e
  // roteia o driver por MIME (contentType) ou pela extensão do ?filename=.
  async function postRaw<T>(path: string, body: ArrayBuffer, contentType: string): Promise<T> {
    const res = await fetch(`${baseUrl}${bust(path)}`, {
      method: 'POST',
      cache: 'no-store',
      headers: { 'Content-Type': contentType || 'application/octet-stream' },
      body,
    })
    const text = await res.text()
    const data = text ? JSON.parse(text) : null
    if (!res.ok) throw new HttpError(res.status, (data && (data.error as string)) || res.statusText)
    return data as T
  }

  return {
    baseUrl,
    get: <T>(path: string) => request<T>('GET', path),
    post: <T>(path: string, body?: Json) => request<T>('POST', path, body),
    postRaw,
  }
}

const RAGD_URL = import.meta.env.VITE_RAGD_URL ?? '/api'
const NIDHOGG_URL = import.meta.env.VITE_NIDHOGG_URL ?? '/nidhogg'

export const ragd = makeClient(RAGD_URL)
export const nidhogg = makeClient(NIDHOGG_URL)
