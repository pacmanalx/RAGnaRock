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
}
