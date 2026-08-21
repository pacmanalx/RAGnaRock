// Funções de API por domínio (molde Innova: um módulo fino sobre o client).
import { ragd, nidhogg } from './client'
import type {
  Health, NidhoggHealth, CollectionsResponse, DriversResponse, SearchResponse, SearchExpandResponse,
  ThesaurusResponse, ChunkResponse, HistogramResponse, StatsResponse, NidhoggStatus,
  ConfigResponse, SetConfigResponse, IngestorsResponse,
  NidhoggCollection, NidhoggClassesSummary, NidhoggEntitiesSummary, NidhoggRejeitados,
  KnowledgeResponse, CacheDigestResponse, NidhoggDoctypes, NidhoggPrompts, MoldeTemplate,
  TreeResponse, NavNode, Dimensao, DimValoresResponse, DimGap, LlmLedgerResponse,
  NidhoggRelacoes, Pergunta, TimelineResposta, RespondeuAgora,
} from './types'

export const getHealth = () => ragd.get<Health>('/health')
export const getCollections = () => ragd.get<CollectionsResponse>('/collections')
export const getDrivers = () => ragd.get<DriversResponse>('/drivers')
export const getDriversOut = () => ragd.get<DriversResponse>('/drivers_out')
export const getIngestors = () => ragd.get<IngestorsResponse>('/ingestors')
// instala/desinstala driver de linguagem (move drivers ↔ drivers.out; guard admin.config)
export const moveDriver = (file: string, action: 'install' | 'uninstall') =>
  ragd.post<{ ok: boolean; installed: number }>('/driver_move', { file, action })
// liga/desliga dicionário (inuse.flag) — reflete na busca expandida na hora
export const toggleDict = (code: string, enable: boolean) =>
  ragd.post<{ ok: boolean; active: number; word_entries: number }>('/thesaurus_toggle', { code, action: enable ? 'enable' : 'disable' })
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

// ── tela Serviços: telemetria + controle dos daemons ──
export const getStats = () => ragd.get<StatsResponse>('/stats')
export const getNidhoggStatus = () => nidhogg.get<NidhoggStatus>('/api/nidhogg')
export const setNidhogg = (cfg: { on?: boolean; level?: number; cadence?: number }) =>
  nidhogg.post<NidhoggStatus>('/api/nidhogg', { ...cfg })
export const runNidhoggCycle = () => nidhogg.post<{ ok: boolean; started?: boolean; note?: string }>('/api/nidhogg/run')

// ── Nidhogg: visão geral (leituras + habilitar coleção) ──
export const getNidhoggCollections = () => nidhogg.get<{ collections: NidhoggCollection[] }>('/api/nidhogg/collections')
export const getNidhoggClasses = (coll?: string) =>
  nidhogg.get<NidhoggClassesSummary>(`/api/nidhogg/classes${coll ? `?collection=${encodeURIComponent(coll)}` : ''}`)
export const getNidhoggEntities = () => nidhogg.get<NidhoggEntitiesSummary>('/api/nidhogg/entities')
export const getNidhoggTemplates = () => nidhogg.get<{ templates: Record<string, MoldeTemplate> }>('/api/nidhogg/templates')
// [L2] a árvore de assuntos: nós de valor ligando registros do dump (?q= busca)
export const getNidhoggTree = (collection: string, q = '') =>
  nidhogg.get<TreeResponse>(`/api/nidhogg/tree?collection=${encodeURIComponent(collection)}${q ? `&q=${encodeURIComponent(q)}` : ''}`)
// [Think Navigator] sugestões leves de tema (sem ramos/co — responde em ms)
export const getNavSuggest = (collection: string, q: string) =>
  nidhogg.get<{ nodes: { valor: string; valor_norm: string }[] }>(
    `/api/nidhogg/suggest?collection=${encodeURIComponent(collection)}&q=${encodeURIComponent(q)}`)
// [Think Navigator] expande um nó do mindmap (relacionados por co-ocorrência)
export const getNavNode = (collection: string, norm: string) =>
  nidhogg.get<NavNode>(`/api/nidhogg/node?collection=${encodeURIComponent(collection)}&norm=${encodeURIComponent(norm)}`)
// [L3 · Estrutural LLM] relações destiladas pelo LLM nas cenas densas (tipo="relacao" no dump)
export const getNidhoggRelacoes = (collection?: string, n = 300) =>
  nidhogg.get<NidhoggRelacoes>(
    `/api/nidhogg/relacoes?n=${n}${collection && collection !== '*' ? `&collection=${encodeURIComponent(collection)}` : ''}`)
// ── [L4 · Perguntas] cadastro de questões diretas + timeline das respostas ──
export const getPerguntas = () => nidhogg.get<{ perguntas: Pergunta[] }>('/api/nidhogg/perguntas')
// GRANULAR de propósito: cada gesto mexe só na SUA questão. O antigo replace-all (mandar a
// lista inteira do navegador a cada clique) apagou 3 questões em 21/ago — uma aba velha ou um
// clique errado levavam o cadastro junto. Aquela rota continua existindo, mas hoje exige
// `substituir_tudo` pra remover em massa, e a tela não a usa mais.
export const upsertPergunta = (pergunta: Pergunta) =>
  nidhogg.post<{ ok: boolean; criou: boolean; perguntas: Pergunta[] }>('/api/nidhogg/perguntas/upsert', { pergunta })
export const removerPergunta = (nome: string, purgar: boolean) =>
  nidhogg.post<{ ok: boolean; nome: string; etapas_apagadas: number; perguntas: Pergunta[] }>(
    '/api/nidhogg/perguntas/remover', { nome, purgar })
export const getTimeline = (pergunta: string) =>
  nidhogg.get<TimelineResposta>(`/api/nidhogg/respostas?pergunta=${encodeURIComponent(pergunta)}`)
// responde AGORA (não espera o ciclo) — LENTO: monta contexto, analista e comparador
export const perguntarAgora = (pergunta: string) =>
  nidhogg.post<RespondeuAgora>('/api/nidhogg/perguntar', { pergunta })

// apaga a TIMELINE de uma questão (o cadastro fica). A pergunta volta a responder do zero —
// inclusive as one-shot, que só congelam enquanto existe etapa anterior.
export const limparRespostas = (pergunta: string) =>
  nidhogg.post<{ ok: boolean; pergunta: string; etapas_apagadas: number }>('/api/nidhogg/respostas/limpar', { pergunta })

// ── L4 · cockpit de destrave: re-tipar (origem=humano, sticky) + molde dirigido ──
export const postReclass = (collection: string, base: string, tipo: string) =>
  nidhogg.post<{ ok: boolean; tipo: string; natureza: string; csv: boolean; extraivel: boolean; nota: string; purgadas: number }>(
    '/api/nidhogg/reclass', { collection, base, tipo })
// molde dirigido: o humano diz O QUE extrair; sem gate de cobertura; iterável (version). LENTO (~1-3min).
export const postMoldeDirigido = (tipo: string, collection: string, base: string, instrucao: string) =>
  nidhogg.post<{ ok: boolean; tipo: string; campos: number; cobertura: number; amostra: Record<string, string> }>(
    '/api/nidhogg/molde', { tipo, collection, base, instrucao })

// [L2 · Dimensões] eixos declarados: o humano diz o que importa, a navegação pivota por eles
export const getDimensoes = () => nidhogg.get<{ dimensoes: Dimensao[] }>('/api/nidhogg/dimensoes')
export const saveDimensoes = (dimensoes: Dimensao[]) =>
  nidhogg.post<{ ok: boolean }>('/api/nidhogg/dimensoes', { dimensoes })
export const getDimensaoValores = (nome: string, collection: string, q = '') =>
  nidhogg.get<DimValoresResponse>(
    `/api/nidhogg/dimensao/valores?nome=${encodeURIComponent(nome)}&collection=${encodeURIComponent(collection)}${q ? `&q=${encodeURIComponent(q)}` : ''}`)
export const getDimensoesGaps = (collection: string) =>
  nidhogg.get<{ collection: string; gaps: DimGap[] }>(
    `/api/nidhogg/dimensoes/gaps?collection=${encodeURIComponent(collection)}`)
// vocabulário EDITÁVEL do classificador (Fase 1) — editar reclassifica no próximo ciclo
export const getNidhoggDoctypes = () => nidhogg.get<NidhoggDoctypes>('/api/nidhogg/doctypes')
export const setNidhoggDoctypes = (naturezas: string[], tipos: string[]) =>
  nidhogg.post<{ ok: boolean }>('/api/nidhogg/doctypes', { naturezas, tipos })
// biblioteca de prompts nomeados (o que/como cada nível extrai)
export const getNidhoggPrompts = () => nidhogg.get<NidhoggPrompts>('/api/nidhogg/prompts')
export const saveNidhoggPrompt = (name: string, system: string, description: string, max_tokens?: number) =>
  nidhogg.post<{ ok: boolean }>('/api/nidhogg/prompts/template', { name, system, description, ...(max_tokens ? { max_tokens } : {}) })
export const getNidhoggRejeitados = () => nidhogg.get<NidhoggRejeitados>('/api/nidhogg/rejeitados')
// conhecimento minerado do L0 (RootIndex + CorpusDict) de UMA coleção
export const getNidhoggKnowledge = (collection: string) =>
  nidhogg.get<KnowledgeResponse>(`/api/nidhogg/knowledge?collection=${encodeURIComponent(collection)}`)
// digest GLOBAL do cache de expansão do ragd (pilar do L0)
export const getNidhoggCacheDigest = () => nidhogg.get<CacheDigestResponse>('/api/nidhogg/cachedigest')
// liga/desliga o ACESSO do worm a uma coleção (não re-mastiga a mesma N vezes)
export const toggleNidhoggCollection = (collection: string, enabled: boolean) =>
  nidhogg.post<{ ok: boolean }>('/api/nidhogg/collection', { collection, enabled })

// ── tela Logs (guard admin.servicos no backend) ──
export const getLogs = (n = 300) => ragd.get<{ file: string; log: string }>(`/logs?n=${n}`)
// diário de mastigação do LLM (cauda do llm-ledger.jsonl; mais recente primeiro)
export const getLlmLedger = (n = 30) => nidhogg.get<LlmLedgerResponse>(`/api/nidhogg/llm_ledger?n=${n}`)

// ── tela Configuração (guard admin.config no backend) ──
export const getConfig = () => ragd.get<ConfigResponse>('/config')
export const setConfig = (patch: Record<string, unknown>) => ragd.post<SetConfigResponse>('/config', patch)
export const testProvider = (provider: string) =>
  ragd.post<{ provider: string; ok: boolean; message: string }>('/config/test_provider', { provider })

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
