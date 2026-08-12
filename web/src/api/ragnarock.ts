// Funções de API por domínio (molde Innova: um módulo fino sobre o client).
import { ragd, nidhogg } from './client'
import type {
  Health, NidhoggHealth, CollectionsResponse, DriversResponse, SearchResponse, SearchExpandResponse,
  ThesaurusResponse, ChunkResponse, HistogramResponse,
} from './types'

export const getHealth = () => ragd.get<Health>('/health')
export const getCollections = () => ragd.get<CollectionsResponse>('/collections')
export const getDrivers = () => ragd.get<DriversResponse>('/drivers')
export const getThesaurus = () => ragd.get<ThesaurusResponse>('/thesaurus')
// Opções comuns de escopo da busca (mesmo contrato da aba Buscar do dashboard legado).
export interface SearchOpts {
  collection?: string // vazio = todas
  base?: string // wildcard: 'sda' exata · 'sd*' prefixo · '*' todas
  k?: number
  phonetic?: boolean
}
const searchBody = (query: string, o: SearchOpts) => ({
  base: o.base?.trim() || '*',
  query,
  k: o.k ?? 8,
  phonetic: !!o.phonetic,
  ...(o.collection ? { collection: o.collection } : {}),
})

// Léxico puro (silábico tf-idf + matched filter).
export const search = (query: string, opts: SearchOpts = {}) =>
  ragd.post<SearchResponse>('/search', searchBody(query, opts))

// Histograma do hit #1: matched filter (query deslizando no chunk) + embedding × query.
export const getHistogram = (query: string, opts: SearchOpts = {}) =>
  ragd.post<HistogramResponse>('/histogram', searchBody(query, opts))

// Semântico: expansão em cascata 📚 dicionários → 📖 cache → 🧠 IA. Com forceInfer,
// two_phase=false — pula o atalho "léxico já foi forte" e SEMPRE roda a cascata.
export const searchExpand = (query: string, opts: SearchOpts & { forceInfer?: boolean } = {}) =>
  ragd.post<SearchExpandResponse>('/search_expand', {
    ...searchBody(query, opts),
    ...(opts.forceInfer ? { two_phase: false } : {}),
  })

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
