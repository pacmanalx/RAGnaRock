> 🌐 **Language.** English version · 🇧🇷 Versão em português: **[JSONCONTRACT.pt-BR.md](JSONCONTRACT.pt-BR.md)**
> *(the pt-BR version is the canonical source; this English version is a translation kept in sync).*

# RAGnaRock — JSON API Contract

**Formal** reference for the HTTP/JSON APIs of the three daemons. For **runnable examples**
(`curl -d @file.json`), see [`ragd/json_samples/`](ragd/json_samples/) — this document is the
specification; that one is the tutorial.

| Daemon | Port | Role | Status |
|---|---|---|---|
| [`ragd`](#1-ragd--data-api-11499) | **11499** | Engine: search, ingestion, discovery | [DONE] |
| [ValHalla](#2-valhalla--console-11498) | **11498** | Supervisory web console (operates `ragd`/`nidhoggd`) | [DONE] |
| [`nidhoggd`](#3-nidhoggd--intelligence-11497) | **11497** | Intelligence layer (knowledge digestion) | [PARTIAL] |

## Conventions

- **Transport:** HTTP/1.1, `application/json` body (except `/ingest_upload` multipart/raw).
- **Collections:** every base belongs to a `collection`; without `collection` in a POST → `"default"`.
  On disk: `ragfiles/<collection>/<name>-tokenized.json`.
- **Base wildcard** (in `/search`, `/bases`): `"sda"` (exact) · `"sd*"` (prefix) · `"*"` (all).
- **Errors:** HTTP 4xx/5xx with body `{ "error": "<message>" }`. Upload above `--max-upload` → 413.
- **Per-route status:** **[DONE]** implemented · **[FUTURE]** planned (target contract, doesn't respond yet).

---

## 1. `ragd` — data API (11499)

### Discovery

| Method | Route | Request | Response (fields) | Status |
|---|---|---|---|---|
| GET | `/health` | — | `{status, bases, collections, drivers}` | [DONE] |
| GET | `/bases` | `?collection=&match=` | `{match, count, bases:[{name, n_chunks, vocab_size, corpus, generator, has_text}]}` | [DONE] |
| GET | `/collections` | — | `{count, total_bases, collections:[{collection, bases}]}` | [DONE] |
| GET | `/drivers` | `?match=` | `{drivers_dir, match, count, drivers:[{name, language, description, extensions[], syllables, keywords, vocab_size, header}]}` | [DONE] |
| GET | `/interpret` | `?file=` \| `?ext=` | `{file?, extension, drivers_scanned, matched, driver, language, fallback?}` | [DONE] |
| GET | `/thesaurus` | `?match=` | `{thesaurus_dir, count, dicts:[{code, description, entries, origin, license, inuse}]}` | [DONE] |

### Search — `POST /search` [DONE]

**Request:**
```jsonc
{
  "base": "sda",        // required — exact | "pref*" | "*"
  "query": "Frodo Bolseiro",  // required
  "collection": "default",    // optional — restricts the scope
  "k": 5,               // results after the merge (default 5)
  "rerank": true,       // stage 2 (proximity); false = recall only (default true)
  "recall_n": 20,       // recall candidates per base sent to rerank (default 20)
  "phonetic": false     // match by SOUND (SOUNDEX): "Aslan" finds "Aslam" (default false)
}
```
**Response:**
```jsonc
{
  "query": "Frodo Bolseiro",
  "query_syllables": "fro-do-bol-sei-ro",
  "bases": ["sda"],                  // bases actually searched
  "searched": [                      // per-base stats (the "scatter")
    { "base":"sda", "n_chunks":1489, "n_converge":1451, "dims":4, "oov":0, "ms_recall":0.4, "ms_rerank":6.7 }
  ],
  "hits": [                          // ordered by global matchpoint (highest first)
    { "base":"sda", "collection":"default", "rank":1,
      "matchpoint":0.80,  // ordering score (rerank on; otherwise = cosine)
      "mf":1.00,          // matched filter: fraction of the query that is contiguous (0..1)
      "span":2,           // proximity between words (smaller = better)
      "cos":0.2664,       // cosine similarity (stage 1, recall)
      "chunk":28,         // chunk id (use in /chunk)
      "start":57193,      // offset (char) in the corpus
      "snippet":"…«Frodo» «Bolseiro»…" }  // matched terms between «»
  ]
}
```

### Search with expansion — `POST /search_expand` [DONE]

Same shape as `/search`, with synonym expansion (**dictionary → cache → AI** cascade) before searching.
**Request:** `{query, collection?, base?, k?, phonetic?}`.
**Response:** same as `/search` + `{expansions:[...], source:"dict|cache|ia"}`.

### Retrieve chunk(s) — `POST /chunk` [DONE]

Brings the **whole chunk** (text + metadata) by id, to assemble context.
**Request:**
```jsonc
{ "base":"sda", "collection":"default", "id":87, "before":1, "after":1 }   // window
// or: { "base":"sda", "ids":[12,87,200] }                                  // explicit list
```
**Response:**
```jsonc
{ "base":"sda", "chunks":[
  { "id":86, "start":175727, "len":2046, "tokens":710, "oov":145, "norm":12.3, "text":"…" },
  { "id":87, "start":177773, "len":2041, "tokens":711, "oov":150, "norm":11.9, "text":"…" }
]}
```

### Ingestion [DONE]

| Method | Route | Modes | Response |
|---|---|---|---|
| POST | `/ingest` | (a) `{name, path}` tokenized JSON · (b) `{name, data:{meta,idf,chunks}}` inline · (c) `{name, path, raw:true, chunk?, driver?, with_text?, max_chunks?}` raw | `{ok, collection, name, n_chunks, bases, raw, saved_to?}` |
| POST | `/ingest_file` | `{path, collection?, name?, chunk?, driver?, with_text?, max_chunks?}` (file on the daemon's machine) | `{ok, collection, name, corpus, n_chunks, bases, saved_to}` |
| POST | `/ingest_upload` | multipart (field `file`) **or** raw body + querystring (`?filename=&name=&chunk=…`) | `{ok, name, filename, corpus, n_chunks, bytes, bases, saved_to, via}` |

Common optionals: `chunk` (chars/chunk, default 2048), `driver` (explicit `.drv`; omitted = auto by extension
with PTBR fallback), `with_text` (default true), `max_chunks` (0 = all). `append=true` enables incremental
append with chunk-packing (recomputes only `idf`+`norm`). Upload accepts UTF-8 only; binary → 400.

### Removal

| Method | Route | Request | Response | Status |
|---|---|---|---|---|
| DELETE | `/bases/{name}` | `?collection=` (default `default`) | `{ok, removed, collection, bases}` | [DONE] |
| DELETE | `/collections/{name}` | — | `{ok, removed, bases}` | [FUTURE] |

### Planned [FUTURE]

| Method | Route | What for |
|---|---|---|
| GET | `/stats` | public aggregate (today only internal in the console) |
| GET | `/bases/{coll}/{name}` | metadata of 1 base without searching |
| GET | `/profile?collection=&base=` | **lexical profile** `{vocab_size, vocab_used, dims, top_idf:[{dim,syllable,idf,df}]}` — feeds Nidhogg's level 0 without probing via `/search` |

---

## 2. ValHalla — console (11498)

Supervisory web console **embedded in `ragd`** (HTML served by the binary), on `dash_port` (default 11498).
**It has no data API of its own** — it operates `ragd` (same `State`, in-process) and **proxies** the
`nidhoggd` routes. That's why the browser talks only to 11498 (no CORS).

- **Authentication:** cookie session after `admin/admin` login (TTL). **[FUTURE]** real configurable password.
- **Data routes:** the tabs call the same `ragd` routes (§1) — e.g. the Search tab uses `POST /search`
  and `POST /search_expand`; the Ingestion tab uses `POST /ingest_upload`.
- **Nidhogg proxy:** the `/api/nidhogg*` routes (§3) are forwarded to `nidhoggd` (`nidhogg_url`, default
  `http://127.0.0.1:11497`). The proxy runs **outside the `State` lock** (avoids a re-entrancy deadlock).
- **Keepalive:** the `nidhoggd` online/offline status is cached (ping every 15s); the UI degrades
  gracefully if the module is down.

> ValHalla's data contract **is** that of `ragd` (§1) and `nidhoggd` (§3); it introduces no new schema.

---

## 3. `nidhoggd` — intelligence (11497) [PARTIAL]

Module daemon. Reads the corpus **always via the `ragd` API** (§1), never from disk. Today the **skeleton**
responds (status, config, per-collection control); the **intelligence** (levels ≥1) is a stub.

### Implemented [DONE — skeleton]

| Method | Route | Request | Response (fields) |
|---|---|---|---|
| GET | `/health` | — | `{status, module, version, on, level}` |
| GET | `/api/nidhogg` | — | `{module, version, uptime_secs, on, level, level_name, levels, needs_ia, cadence_secs, dir, collections_known, last_cycle, ragd_api, ragd_online, ragd:{…}}` |
| GET | `/api/nidhogg/collections` | — | `{collections:[{collection, bases, chunks, enabled, saturation, updated, has_knowledge}]}` |
| POST | `/api/nidhogg` | `{on:bool, level:"burro\|consciente\|estrutural\|propositivo", cadence:secs}` | same as `GET /api/nidhogg` |
| POST | `/api/nidhogg/collection` | `{collection, enabled:bool}` | `{ok, collection, enabled}` |
| POST | `/api/nidhogg/run` | — | `{ok, note}` (triggers a cycle — **stub**, intelligence still 0) |

### Planned [FUTURE]

| Method | Route | Request | Response |
|---|---|---|---|
| GET | `/api/nidhogg/knowledge` | `?collection=&type=&level=` | `{knowledge:[{type, level, version, created, content, confidence, derived_from[], frozen, status}]}` — serves the distilled artifacts (living document, knowledge tree) |
| POST | `/api/nidhogg/accept` | `{collection, type, level, version}` | `{ok, status:"accepted"}` — marks the artifact as accepted and releases the next level when `accept_gate` requires it |

> Knowledge item schema and states (`pending|accepted`, `frozen`, `version`): see
> [`ARCHITECTURE.md` §5.6](ARCHITECTURE.md#56-consolidated-knowledge-schema--dircollknowledgejson).

---

> Source of truth for the `ragd` contract: the code in `ragd/src/` + the examples in `ragd/json_samples/`.
> Routes marked **[FUTURE]** describe the target contract (implementation North Star), not yet responding.
