// Funções de API por domínio (molde Innova: um módulo fino sobre o client).
import { ragd, nidhogg } from './client'
import type {
  Health, NidhoggHealth, CollectionsResponse, DriversResponse, SearchResponse, ThesaurusResponse,
  ChunkResponse,
} from './types'

export const getHealth = () => ragd.get<Health>('/health')
export const getCollections = () => ragd.get<CollectionsResponse>('/collections')
export const getDrivers = () => ragd.get<DriversResponse>('/drivers')
export const getThesaurus = () => ragd.get<ThesaurusResponse>('/thesaurus')
export const search = (query: string, k = 8) =>
  ragd.post<SearchResponse>('/search', { base: '*', query, k })

// Um chunk (com before/after de contexto). Usado pela modal de inspeção.
export const fetchChunk = (collection: string, base: string, id: number, before = 0, after = 0) =>
  ragd.post<ChunkResponse>('/chunk', { collection, base, id, before, after })

// Documento inteiro numa request (id 0 + after = n_chunks-1) — pro download em .md.
export const fetchDocument = (collection: string, base: string, nChunks: number) =>
  ragd.post<ChunkResponse>('/chunk', { collection, base, id: 0, before: 0, after: Math.max(0, nChunks - 1) })

export const getNidhoggHealth = () => nidhogg.get<NidhoggHealth>('/health')

// [#9] Resultado do POST /ingest_any (upload → driver → base tokenizada).
export interface IngestResult {
  ok: boolean
  collection: string
  name: string
  filename: string
  driver?: string
  n_chunks?: number
  bytes?: number
  appended?: boolean
  error?: string
}

// Sobe UM arquivo cru pro /ingest_any (o proxy da 11498 repassa pra API real).
export function ingestAny(
  file: File,
  opts: { collection: string; name: string; chunk: number },
): Promise<IngestResult> {
  const q = new URLSearchParams({
    collection: opts.collection,
    filename: file.name,
    name: opts.name,
    chunk: String(opts.chunk),
  })
  return file.arrayBuffer().then((buf) =>
    ragd.postRaw<IngestResult>(`/ingest_any?${q}`, buf, file.type || 'application/octet-stream'),
  )
}
