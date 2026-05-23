> 🌐 **Idioma.** Versão em **português (pt-BR)**. Versão principal em inglês: **[JSONCONTRACT.md](JSONCONTRACT.md)**.

# RAGnaRock — Contrato JSON das APIs

Referência **formal** das APIs HTTP/JSON dos três daemons. Para **exemplos executáveis**
(`curl -d @arquivo.json`), veja [`ragd/json_samples/`](ragd/json_samples/) — este documento
é a especificação; aquele é o tutorial.

| Daemon | Porta | Papel | Status |
|---|---|---|---|
| [`ragd`](#1-ragd--api-de-dados-11499) | **11499** | Motor: busca, ingestão, descoberta | [FEITO] |
| [ValHalla](#2-valhalla--console-11498) | **11498** | Console web supervisório (opera o `ragd`/`nidhoggd`) | [FEITO] |
| [`nidhoggd`](#3-nidhoggd--inteligência-11497) | **11497** | Camada de inteligência (digestão de conhecimento) | [PARCIAL] |

## Convenções

- **Transporte:** HTTP/1.1, corpo `application/json` (exceto `/ingest_upload` multipart/raw).
- **Coleções:** toda base pertence a uma `collection`; sem `collection` num POST → `"default"`.
  No disco: `ragfiles/<collection>/<name>-tokenized.json`.
- **Wildcard de base** (em `/search`, `/bases`): `"sda"` (exata) · `"sd*"` (prefixo) · `"*"` (todas).
- **Erros:** HTTP 4xx/5xx com corpo `{ "error": "<mensagem>" }`. Upload acima de `--max-upload` → 413.
- **Status por rota:** **[FEITO]** implementada · **[FUTURO]** planejada (contrato-alvo, ainda não responde).

---

## 1. `ragd` — API de dados (11499)

### Descoberta

| Método | Rota | Request | Response (campos) | Status |
|---|---|---|---|---|
| GET | `/health` | — | `{status, bases, collections, drivers}` | [FEITO] |
| GET | `/bases` | `?collection=&match=` | `{match, count, bases:[{name, n_chunks, vocab_size, corpus, generator, has_text}]}` | [FEITO] |
| GET | `/collections` | — | `{count, total_bases, collections:[{collection, bases}]}` | [FEITO] |
| GET | `/drivers` | `?match=` | `{drivers_dir, match, count, drivers:[{name, language, description, extensions[], syllables, keywords, vocab_size, header}]}` | [FEITO] |
| GET | `/interpret` | `?file=` \| `?ext=` | `{file?, extension, drivers_scanned, matched, driver, language, fallback?}` | [FEITO] |
| GET | `/thesaurus` | `?match=` | `{thesaurus_dir, count, dicts:[{code, description, entries, origin, license, inuse}]}` | [FEITO] |

### Busca — `POST /search` [FEITO]

**Request:**
```jsonc
{
  "base": "sda",        // obrigatório — exata | "pref*" | "*"
  "query": "Frodo Bolseiro",  // obrigatório
  "collection": "default",    // opcional — restringe o escopo
  "k": 5,               // resultados após o merge (default 5)
  "rerank": true,       // estágio 2 (proximidade); false = só recall (default true)
  "recall_n": 20,       // candidatos do recall por base que vão ao rerank (default 20)
  "phonetic": false     // casa por SOM (SOUNDEX): "Aslan" acha "Aslam" (default false)
}
```
**Response:**
```jsonc
{
  "query": "Frodo Bolseiro",
  "query_syllables": "fro-do-bol-sei-ro",
  "bases": ["sda"],                  // bases efetivamente buscadas
  "searched": [                      // stats por base (o "scatter")
    { "base":"sda", "n_chunks":1489, "n_converge":1451, "dims":4, "oov":0, "ms_recall":0.4, "ms_rerank":6.7 }
  ],
  "hits": [                          // ordenados por matchpoint global (maior primeiro)
    { "base":"sda", "collection":"default", "rank":1,
      "matchpoint":0.80,  // score de ordenação (rerank ligado; senão = cosseno)
      "mf":1.00,          // matched filter: fração da query contígua (0..1)
      "span":2,           // proximidade entre palavras (menor = melhor)
      "cos":0.2664,       // similaridade cosseno (estágio 1, recall)
      "chunk":28,         // id do chunk (use em /chunk)
      "start":57193,      // offset (char) no corpus
      "snippet":"…«Frodo» «Bolseiro»…" }  // termos casados entre «»
  ]
}
```

### Busca com expansão — `POST /search_expand` [FEITO]

Mesma forma do `/search`, com expansão de sinônimos (cascata **dicionário → cache → IA**) antes de buscar.
**Request:** `{query, collection?, base?, k?, phonetic?}`.
**Response:** igual ao `/search` + `{expansions:[...], source:"dict|cache|ia"}`.

### Recuperar chunk(s) — `POST /chunk` [FEITO]

Traz o **chunk inteiro** (texto + metadados) por id, pra montar contexto.
**Request:**
```jsonc
{ "base":"sda", "collection":"default", "id":87, "before":1, "after":1 }   // janela
// ou: { "base":"sda", "ids":[12,87,200] }                                  // lista explícita
```
**Response:**
```jsonc
{ "base":"sda", "chunks":[
  { "id":86, "start":175727, "len":2046, "tokens":710, "oov":145, "norm":12.3, "text":"…" },
  { "id":87, "start":177773, "len":2041, "tokens":711, "oov":150, "norm":11.9, "text":"…" }
]}
```

### Ingestão [FEITO]

| Método | Rota | Modos | Response |
|---|---|---|---|
| POST | `/ingest` | (a) `{name, path}` JSON tokenizado · (b) `{name, data:{meta,idf,chunks}}` embutido · (c) `{name, path, raw:true, chunk?, driver?, with_text?, max_chunks?}` bruto | `{ok, collection, name, n_chunks, bases, raw, saved_to?}` |
| POST | `/ingest_file` | `{path, collection?, name?, chunk?, driver?, with_text?, max_chunks?}` (arquivo na máquina do daemon) | `{ok, collection, name, corpus, n_chunks, bases, saved_to}` |
| POST | `/ingest_upload` | multipart (campo `file`) **ou** raw body + querystring (`?filename=&name=&chunk=…`) | `{ok, name, filename, corpus, n_chunks, bytes, bases, saved_to, via}` |

Opcionais comuns: `chunk` (chars/chunk, default 2048), `driver` (`.drv` explícito; omitido = auto por extensão
com fallback PTBR), `with_text` (default true), `max_chunks` (0 = todos). `append=true` ativa o append
incremental com chunk-packing (recomputa só `idf`+`norm`). Upload só aceita UTF-8; binário → 400.

### Remoção

| Método | Rota | Request | Response | Status |
|---|---|---|---|---|
| DELETE | `/bases/{nome}` | `?collection=` (default `default`) | `{ok, removed, collection, bases}` | [FEITO] |
| DELETE | `/collections/{nome}` | — | `{ok, removed, bases}` | [FUTURO] |

### Planejadas [FUTURO]

| Método | Rota | Para quê |
|---|---|---|
| GET | `/stats` | agregado público (hoje só interno no console) |
| GET | `/bases/{coll}/{name}` | metadados de 1 base sem buscar |
| GET | `/profile?collection=&base=` | **perfil léxico** `{vocab_size, vocab_used, dims, top_idf:[{dim,syllable,idf,df}]}` — alimenta o nível 0 do Nidhogg sem sondar via `/search` |

---

## 2. ValHalla — console (11498)

Console web supervisório **embutido no `ragd`** (HTML servido pelo binário), na porta `dash_port`
(default 11498). **Não tem API de dados própria** — opera o `ragd` (mesma `State`, em processo) e faz
**proxy** das rotas do `nidhoggd`. Por isso o navegador fala só com a 11498 (sem CORS).

- **Autenticação:** sessão por **cookie** após login `admin/admin` (TTL). **[FUTURO]** senha real configurável.
- **Rotas de dados:** as abas chamam as mesmas rotas do `ragd` (§1) — ex.: a aba Buscar usa `POST /search`
  e `POST /search_expand`; a aba Ingestão usa `POST /ingest_upload`.
- **Proxy do Nidhogg:** as rotas `/api/nidhogg*` (§3) são repassadas ao `nidhoggd` (`nidhogg_url`, default
  `http://127.0.0.1:11497`). O proxy roda **fora do lock** da `State` (evita deadlock de re-entrância).
- **Keepalive:** o status online/offline do `nidhoggd` é cacheado (ping a cada 15s); a UI degrada graciosa
  se o módulo estiver fora.

> O contrato de dados do ValHalla **é** o do `ragd` (§1) e o do `nidhoggd` (§3); ele não introduz schema novo.

---

## 3. `nidhoggd` — inteligência (11497) [PARCIAL]

Daemon de módulos. Lê o corpus **sempre pela API do `ragd`** (§1), nunca do disco. Hoje o **esqueleto**
responde (status, config, controle por coleção); a **inteligência** (níveis ≥1) é stub.

### Implementadas [FEITO — esqueleto]

| Método | Rota | Request | Response (campos) |
|---|---|---|---|
| GET | `/health` | — | `{status, module, version, on, level}` |
| GET | `/api/nidhogg` | — | `{module, version, uptime_secs, on, level, level_name, levels, needs_ia, cadence_secs, dir, collections_known, last_cycle, ragd_api, ragd_online, ragd:{…}}` |
| GET | `/api/nidhogg/collections` | — | `{collections:[{collection, bases, chunks, enabled, saturation, updated, has_knowledge}]}` |
| POST | `/api/nidhogg` | `{on:bool, level:"burro\|consciente\|estrutural\|propositivo", cadence:secs}` | idem `GET /api/nidhogg` |
| POST | `/api/nidhogg/collection` | `{collection, enabled:bool}` | `{ok, collection, enabled}` |
| POST | `/api/nidhogg/run` | — | `{ok, note}` (dispara ciclo — **stub**, inteligência ainda 0) |

### Planejadas [FUTURO]

| Método | Rota | Request | Response |
|---|---|---|---|
| GET | `/api/nidhogg/knowledge` | `?collection=&type=&level=` | `{knowledge:[{type, level, version, created, content, confidence, derived_from[], frozen, status}]}` — serve os artefatos destilados (documento vivo, árvore de conhecimento) |
| POST | `/api/nidhogg/accept` | `{collection, type, level, version}` | `{ok, status:"accepted"}` — marca o artefato como aceito e libera o nível seguinte quando `accept_gate` o exige |

> Schema do item de conhecimento e estados (`pending|accepted`, `frozen`, `version`): ver
> [`ARCHITECTURE.pt-BR.md` §5.6](ARCHITECTURE.pt-BR.md#56-schema-do-conhecimento-consolidado--dircollknowledgejson).

---

> Fonte de verdade do contrato do `ragd`: o código em `ragd/src/` + os exemplos em `ragd/json_samples/`.
> Rotas marcadas **[FUTURO]** descrevem o contrato-alvo (norte de implementação), ainda não respondem.
