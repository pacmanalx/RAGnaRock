// Funções de API por domínio (molde Innova: um módulo fino sobre o client).
import { ragd, nidhogg } from './client'
import type {
  Health, NidhoggHealth, CollectionsResponse, DriversResponse, SearchResponse, ThesaurusResponse,
} from './types'

export const getHealth = () => ragd.get<Health>('/health')
export const getCollections = () => ragd.get<CollectionsResponse>('/collections')
export const getDrivers = () => ragd.get<DriversResponse>('/drivers')
export const getThesaurus = () => ragd.get<ThesaurusResponse>('/thesaurus')
export const search = (query: string, k = 8) =>
  ragd.post<SearchResponse>('/search', { base: '*', query, k })

export const getNidhoggHealth = () => nidhogg.get<NidhoggHealth>('/health')
