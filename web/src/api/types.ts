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
  count?: number
  drivers: Driver[]
}
export interface Driver {
  name: string
  language: string
  description: string
  extensions: string[]
  syllables: number
  keywords?: number
}

// GET /ingestors — drivers de ingestão (scripts do ingestors_dir)
export interface IngestorsResponse {
  ingestors_dir: string
  count: number
  ingestors: { name: string; bytes: number; description: string }[]
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

// GET /stats do ragd — telemetria do daemon (tela Serviços)
export interface StatsResponse {
  version: string
  uptime_secs: number
  collections: number
  bases: number
  chunks: number
  drivers: number
  dicts_active: number
  word_syn_entries: number
  ragfiles_dir: string
  collections_detail: { collection: string; bases: number; chunks: number }[]
  mem: {
    storage: string
    rss_mb: number
    sys_total_mb: number
    sys_avail_mb: number
    est_text_mb?: number
    est_vec_mb?: number
    est_words_mb?: number
  }
}

// GET /config do ragd — configuração do daemon (chaves mascaradas; tela Configuração)
export interface ConfigResponse {
  storage: string
  config_path: string
  drivers_dir: string
  ingestors_dir: string
  ragfiles_dir: string
  max_upload_mb: number
  max_bases: number
  max_chunks_per_base: number
  session_ttl: number
  dev_mode: boolean
  anthropic_key_set: boolean
  anthropic_key_masked: string
  openai_key_set: boolean
  openai_key_masked: string
  active_provider: string
  local_url: string
  nidhogg_url: string
  cache_dir: string
  expansions_entries: number
  thesaurus_dir: string
  dicts_active: number
  word_syn_entries: number
}
export interface SetConfigResponse {
  ok: boolean
  notes: string[]
  reloaded: boolean
  config: ConfigResponse
}

// GET /api/nidhogg — status completo + catálogo de níveis (tela Serviços controla via POST)
export interface NidhoggLevel { n: number; name: string; ia: boolean; desc: string; future?: boolean }
export interface NidhoggStatus {
  module: string
  version: string
  uptime_secs: number
  on: boolean
  level: number
  level_name: string
  levels: NidhoggLevel[]
  needs_ia: boolean
  cadence_secs: number
  cycle_running: boolean
  last_cycle?: string | null
  ragd_online: boolean
}

// ── Nidhogg: leituras da visão geral ──
export interface NidhoggCollection {
  collection: string
  bases: number
  chunks: number | null
  enabled: boolean
  saturation: number
  updated: string
  has_knowledge: boolean
}
export interface NidhoggClassesSummary {
  collection: string
  count: number
  naturezas: Record<string, number>
  tipos: Record<string, number>
  bases: { collection: string; name: string; natureza: string; tipo: string; forma?: string; csv: number; origem: string; confianca: number; classified_at: string }[]
}
export interface NidhoggEntitiesSummary {
  count: number
  nqi_global: number
  por_base: { collection: string; base: string; tipo: string; modo: string; nqi: number; c: number }[]
  por_tipo: { tipo: string; modo: string; nqi: number; c: number; bases: number }[]
}
export interface NidhoggRejeitados {
  count: number
  por_motivo: Record<string, number>
  rejeitados: { collection: string; base: string; natureza: string; tipo: string; motivo: string; nqi: number | null }[]
}

// ── Nidhogg L2: KnowledgeTree (nós de valor → ramos por tipo → registros) ──
export interface TreeItem { base: string; campo: string; idx: number; nqi: number }
export interface TreeRamo { tipo: string; n: number; itens: TreeItem[] }
export interface TreeNode {
  valor: string
  valor_norm: string
  registros: number
  bases: number
  ramos: TreeRamo[]
  co: { valor: string; valor_norm: string; n: number }[] // co-assuntos (mesmo registro) — a profundidade
}
export interface TreeResponse { collection: string; count: number; nodes: TreeNode[]; note?: string }

// [Think Navigator] expansão de UM nó do mindmap
export interface NavNode {
  found: boolean
  valor: string
  valor_norm: string
  registros: number | string
  bases: number | string
  co: { valor: string; valor_norm: string; n: number | string; bases: number | string }[]
  facetas?: { tipo: string; campo: string; n: number | string; bases: number | string }[]
}

// ── Nidhogg L2: cadastro de Dimensões (eixos declarados de navegação/exigência) ──
export interface Dimensao { nome: string; descricao?: string; campos: string[]; tipos: string[] }
export interface DimValorItem {
  valor: string
  valor_norm: string
  registros: number | string
  bases: number | string
  tipos: number | string
}
export interface DimValoresResponse { nome: string; count: number; valores: DimValorItem[] }
export interface DimGap { nome: string; alvo: number; cobertos: number; gaps: string[]; nota: string }

// ── Nidhogg L3: relações destiladas pelo LLM (tipo="relacao" no dump) ──
export interface RelacaoItem {
  collection: string
  base: string
  idx: number | string            // chunk da cena
  dado: { a: string; rel: string; b: string; tema?: string }
  nqi: number
  prov: { via: string; chunk?: number; presentes?: number } | null
  extracted_at: string
}
export interface NidhoggRelacoes { count: number; bases: number; relacoes: RelacaoItem[]; note?: string }

// ── Nidhogg L4: perguntas cadastradas + timeline de respostas ──
export type TipoResposta = 'tabular' | 'oneshot' | 'vivo'
export interface Pergunta {
  nome: string
  texto: string
  tipo: TipoResposta
  escopo: string            // coleção ou '*'
  ativa: boolean
  pai?: string              // recursão declarada: filha de qual pergunta
}
export interface RespostaTabular { colunas: string[]; linhas: string[][]; nota?: string }
export interface EtapaResposta {
  seq: number | string
  tipo: TipoResposta
  resposta: string          // texto, ou JSON de RespostaTabular quando tipo=tabular
  mudou: string             // o que mudou vs a etapa anterior
  fontes: { base: string; trecho?: string }[]
  proximas: string[]        // dimensões não exploradas propostas pela IA
  ms: number | string
  at: string
}
export interface TimelineResposta { pergunta: string; count: number; etapas: EtapaResposta[] }
export interface RespondeuAgora {
  ok: boolean
  pergunta: string
  nova_etapa: boolean
  seq?: number
  mudou?: string
  note?: string
  ms?: number
}

// ── Diário de mastigação do LLM (llm-ledger.jsonl do nidhoggd) ──
export interface LlmLedgerEntry {
  ts: string
  tag: string        // classificador | modelador | extrator
  ctx: string        // "molde-dirigido demo/base tipo=contrato"
  ms: number
  ok: boolean
  finish: string     // stop | length (cortado no teto)
  system: string; system_len: number
  user: string; user_len: number
  resposta: string; resposta_len: number
}
export interface LlmLedgerResponse { file: string; entries: LlmLedgerEntry[] }

// ── Nidhogg L1: doctypes, prompts e moldes ──
export interface NidhoggDoctypes { naturezas: string[]; tipos: string[] }
export interface PromptTemplate { description: string; system: string; updated: string; max_tokens?: number }
export interface NidhoggPrompts { templates: Record<string, PromptTemplate> }
export interface MoldeTemplate {
  schema?: string[]
  regras?: string
  cobertura?: number
  origem?: string
  created_at?: string
  version?: number
}

// ── Nidhogg L0: conhecimento minerado (knowledge.json por coleção) ──
export interface SalientRoot { dim: number; syllable: string; uidf: number; df: number; freq: number }
export interface KnowledgeItem {
  type: string // 'RootIndex' | 'CorpusDict' | …
  level: number
  content: {
    // RootIndex
    bases_count?: number
    total_chunks?: number
    unified_vocab_size?: number
    salient_roots?: SalientRoot[]
    // CorpusDict
    shared_vocab?: number
    unique_vocab?: number
    bases?: { name: string; corpus: string; n_chunks: number; vocab_size: number }[]
  }
}
export interface KnowledgeResponse {
  collection: string
  enabled: boolean
  source_hash: string
  saturation: number
  updated: string
  provenance?: {
    digestion_id: string
    at: string
    via: string
    inputs: { bases: number; total_chunks: number; source_hash: string }
  }
  knowledge: KnowledgeItem[]
}
export interface CacheDigestResponse {
  type: string
  level: number
  scope: string
  updated: string
  content: {
    n_queries: number
    n_variants_total: number
    avg_variants: number
    entries: { query: string; variants: string[]; n_variants: number }[]
  }
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
