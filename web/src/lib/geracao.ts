// Geração — a linguagem do RAGnaRock (superset ANSI SQL / GraphQL).
// Módulo PURO (sem React): é a semente da spec do parser do motor (#35).
// Nesta fase os comandos são definição de linguagem: nada executa.

export type Modo = 'lexico' | 'semantico' | 'inferir'
export type Lang = 'sql' | 'graphql'
export interface FormState {
  q: string
  modo: Modo
  coll: string
  base: string
  k: number
  phonetic: boolean
}

const sqlStr = (s: string) => `'${s.replace(/'/g, "''")}'`
const sqlId = (s: string) => (/^[a-zA-Z_][a-zA-Z0-9_]*$/.test(s) ? s : `"${s.replace(/"/g, '""')}"`)
const gqlStr = (s: string) => JSON.stringify(s)
export const MODE_SQL: Record<Modo, string> = { lexico: 'LEXICAL', semantico: 'SEMANTIC', inferir: 'INFER' }

export function genSelect(lang: Lang, f: FormState): string {
  const scope = `${f.coll ? sqlId(f.coll) : '*'}.${f.base.trim() && f.base.trim() !== '*' ? sqlId(f.base.trim()) : '*'}`
  if (lang === 'sql')
    return [
      `SELECT rank, collection, base, chunk, cov, span, cos, snippet`,
      `  FROM ${scope}`,
      ` WHERE MATCH(${sqlStr(f.q.trim() || '…')})`,
      `  WITH MODE = ${MODE_SQL[f.modo]}, PHONETIC = ${f.phonetic ? 'ON' : 'OFF'}`,
      ` LIMIT ${f.k};`,
    ].join('\n')
  return [
    `query {`,
    `  search(query: ${gqlStr(f.q.trim() || '…')}, mode: ${MODE_SQL[f.modo]},`,
    `         collection: ${f.coll ? gqlStr(f.coll) : 'null'}, base: ${gqlStr(f.base.trim() || '*')},`,
    `         phonetic: ${f.phonetic}, k: ${f.k}) {`,
    `    rank collection base chunk cov span cos snippet`,
    `  }`,
    `}`,
  ].join('\n')
}

export function genDeleteChunk(lang: Lang, coll: string, base: string, chunk: number): string {
  if (lang === 'sql') return `DELETE FROM ${sqlId(coll)}.${sqlId(base)} WHERE chunk = ${chunk};`
  return `mutation { deleteChunk(collection: ${gqlStr(coll)}, base: ${gqlStr(base)}, chunk: ${chunk}) { ok } }`
}

export function genDeleteBase(lang: Lang, coll: string, base: string): string {
  // sem WHERE = a base inteira; PURGE = apaga também o JSON do disco (contrato do DELETE /bases?purge=1)
  if (lang === 'sql') return `DELETE FROM ${sqlId(coll)}.${sqlId(base)} PURGE;`
  return `mutation { deleteBase(collection: ${gqlStr(coll)}, base: ${gqlStr(base)}, purge: true) { ok removed } }`
}
