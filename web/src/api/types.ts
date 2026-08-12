// Tipos das respostas dos backends. Feitos à mão por ora (os JSONs do ragd são poucos e
// conhecidos); se um dia o ragd expuser um contrato, geramos daqui como o Innova faz.

export interface Health {
  status: string
  bases: number
  collections: number
  drivers: number
}

export interface NidhoggHealth {
  status: string
  module: string
  version: string
  on: boolean
  level: string
}

export interface CollectionsResponse {
  count: number
  total_bases: number
  collections: CollectionRow[]
}
export interface CollectionRow {
  collection: string
  bases: number
}

export interface DriversResponse {
  drivers: Driver[]
}
export interface Driver {
  name: string
  language: string
  description: string
  extensions: string[]
  syllables: number
}

export interface ThesaurusResponse {
  thesaurus_dir: string
  count: number
  active: number
  dicts: Dict[]
}
export interface Dict {
  code: string
  active: boolean
  entries: number
  source: string
  source_url?: string
  license?: string
  kind?: string
}

// POST /histogram — visualização do hit #1: matched filter + embedding × query (tela Performance)
export interface MfTerm {
  term: string
  k: number
  peak: number
  peak_pos: number
  points: [number, number][] // [posição na sequência, fração que casa 0..1]
}
export interface ChunkDim { dim: number; c: number }
export interface QueryDim { dim: number; c: number; syl: string; hit: boolean } // hit = dim também no chunk (converge no cosseno)
export interface HistogramResponse {
  found: boolean
  collection?: string
  base?: string
  chunk_id?: number
  coverage?: number
  cos?: number
  query_syllables?: string
  vocab_size?: number
  seq_len?: number
  mf?: MfTerm[]
  chunk?: ChunkDim[]
  query?: QueryDim[]
  query_oov?: number
}

export interface ChunkData {
  id: number
  start: number
  len: number
  tokens: number
  oov: number
  norm: number
  text: string | null
}
export interface ChunkResponse {
  corpus?: string
  n_chunks?: number
  chunks: ChunkData[]
}

export interface SearchResponse {
  query: string
  query_syllables?: string
  hits: Hit[]
}
export interface Hit {
  collection: string
  base: string
  corpus: string
  matchpoint: number
  coverage: number
  span: number
  cos: number
  chunk: number
  start: number
  snippet: string
  rank: number
  via?: string // busca expandida: qual variante casou ('original' | sinônimo | 'literal_fallback')
}

// /search_expand — busca semântica (cascata 📚 dicionários → 📖 cache → 🧠 IA; two_phase=false força a inferência)
export interface SearchExpandResponse extends SearchResponse {
  source?: string // 'phase1' | 'dict' | 'cache' | 'llm' | 'literal' | 'literal_fallback'
  provider?: string
  recall?: string
  expansions?: string[]
  dropped?: string[] // sinônimos fora do corpus deste escopo (não buscados)
  absent?: boolean // nada da query ancora no corpus
  did_you_mean?: string[]
  needles?: string[]
}
