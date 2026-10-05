> 🌐 **Language.** English (main version) · 🇧🇷 Versão em português: **[JSONCONTRACT.pt-BR.md](JSONCONTRACT.pt-BR.md)**
> *(the pt-BR version is a translation kept in sync).*

# RAGnaRock — JSON API Contract

**Formal** reference for the HTTP/JSON APIs of the three daemons, written from the code (`ragd/src/main.rs`,
`ragd/src/auth.rs`, `nidhoggd/src/main.rs`). For **runnable examples** (`curl -d @file.json`), see
[`ragd/json_samples/`](ragd/json_samples/) — this document is the specification; that one is the tutorial.

| Daemon | Port | Role |
|---|---|---|
| [`ragd`](#1-ragd--data-api-11499) | **11499** | Engine: search, ingestion, discovery, administration |
| [ValHalla](#2-valhalla--console-11498) | **11498** | Web console (React) served by `ragd`, proxying the API and `nidhoggd` |
| [`nidhoggd`](#3-nidhoggd--intelligence-11497) | **11497** | Intelligence layer (levels 0–4, ClickHouse) |

## Conventions

- **Transport:** HTTP/1.1, `application/json` body — except `/ingest_upload`, `/ingest_any` and `/transcribe`,
  which take multipart (field `file`) or a raw body with metadata in the query string.
- **Collections:** every base belongs to a `collection`; without `collection` in a POST → `"default"`.
  On disk: `ragfiles/<collection>/<name>-tokenized.json`.
- **Base pattern** (in `/search`, `/bases`): `"sda"` (exact) · `"sd*"` (prefix) · `"*"` (all);
  case-insensitive, NFC-normalized.
- **Errors:** HTTP 4xx/5xx with body `{ "error": "<message>" }`.
- **Authentication:** JWT HS256 (§1.1). **Only administrative routes require a token**; data routes
  (search, ingestion, removal, `/chunk`) are open, and `nidhoggd` has no authentication. Expose the ports only
  on a trusted network.

---

## 1. `ragd` — data API (11499)

### 1.1 Authentication

`POST /login` returns an `access` token (15 min) and a `refresh` token (TTL `session_ttl`, default 12 h).
Protected routes take `Authorization: Bearer <access>`: 401 without a token or with an invalid one, 403 without
the capability. Users and profiles live in `auth_file` (default `ragnarock-auth.json`, PBKDF2 passwords); the
first run creates `admin/admin` and the profiles `admin`, `operador`, `leitor`, `auditor`. Capabilities:
`buscar`, `ingerir`, `apagar`, `nidhogg.ver`, `nidhogg.operar`, `admin.config`, `admin.usuarios`,
`admin.servicos`, `*` (today the server enforces only the `admin.*` ones; the others drive the UI).

| Method | Route | Requires | Request | Response |
|---|---|---|---|---|
| POST | `/login` | — | `{login, password}` | `{access, refresh, expires_in:900, usuario:{login, nome, perfil, caps, colls}}` · 401 |
| POST | `/refresh` | — | `{refresh}` | `{access, expires_in}` · 401 |
| GET | `/auth/caps` | — | — | `{caps:[…]}` |
| GET | `/auth/me` | token | — | `{usuario:{login, nome, perfil, caps, colls}, exp}` |
| POST | `/auth/password` | token | `{atual, nova}` (≥ 6 chars) | `{ok, login}` · 401 if `atual` is wrong |
| GET | `/auth/perfis` | `admin.usuarios` | — | `{perfis}` |
| POST | `/auth/perfis` | `admin.usuarios` | `{nome, desc, caps[], colls[]}` (`colls` default `["*"]`) | `{ok, perfil}` · 400 unknown capability |
| DELETE | `/auth/perfis/{nome}` | `admin.usuarios` | — | `{ok, removed}` · 409 in use · 404 |
| GET | `/auth/usuarios` | `admin.usuarios` | — | `{usuarios:[{login, nome, perfil, ativo}]}` |
| POST | `/auth/usuarios` | `admin.usuarios` | `{login, nome, perfil, ativo=true, password}` (password required on create) | `{ok, usuario}` · 409 last active admin |
| DELETE | `/auth/usuarios/{login}` | `admin.usuarios` | — | `{ok, removed}` · 409 last admin · 404 |

### 1.2 Discovery

| Method | Route | Request | Response |
|---|---|---|---|
| GET | `/health` | — | `{status, bases, collections, drivers}` |
| GET | `/bases` | `?collection=` (default all) `&match=` (default `*`) | `{collection, match, count, bases:[{collection, name, n_chunks, vocab_size, corpus, generator, has_text}]}` |
| GET | `/bases/{collection}/{name}` | — | `{collection, name, corpus, generator, n_chunks, vocab_size, vocab_used, has_text, mtime}` · 404 |
| GET | `/collections` | — | `{count, total_bases, collections:[{collection, bases}]}` |
| GET | `/profile` | see below | lexical profile of a base or of the collection |
| GET | `/stats` | — | `{version, uptime_secs, collections, bases, chunks, drivers, dicts_active, word_syn_entries, ragfiles_dir, collections_detail:[{collection, bases, chunks}], mem}` |
| GET | `/expansions` | — | `{count, expansions:{<normalized query>:[variants]}}` (expansion cache) |
| GET | `/drivers` | `?match=` (prefix on language) | `{drivers_dir, match, count, drivers:[{name, language, description, extensions, syllables, keywords, vocab_size, header}]}` |
| GET | `/drivers_out` | — | same shape, listing `<drivers_dir>.out` (uninstalled) |
| GET | `/ingestors` | — | `{ingestors_dir, count, ingestors:[{name, bytes, description}]}` |
| GET | `/interpret` | `?file=` or `?ext=` | `{file?, extension, drivers_dir, drivers_scanned, matched, driver, language, fallback?}` · 400 with neither |
| GET | `/thesaurus` | — | `{thesaurus_dir, count, active, dicts:[{code, active, entries, source, source_url, license, kind, lang_query, lang_target, size_bytes}]}` |

**`GET /profile`** — `?collection=` required (400 without it).
- With `&base=`: `?top=20` → `{scope:"base", collection, base, corpus, n_chunks, vocab_size, vocab_used, has_text, mtime, top_idf:[{dim, syllable, idf}]}`.
- Without `base` (collection): `?top=20&rank=uidf|idffreq&min_freq=0&vectors=1` → `{scope:"collection", collection, bases, chunks, unified_vocab_size, shared_vocab, unique_vocab, rank, min_freq, top_uidf:[{dim, syllable, uidf, df, freq}]}`;
  `vectors` adds `base_vectors:[{name, corpus, n_chunks, dims_used, coverage, unique_dims, shared_dims, vec[]}]`.
- 404 unknown base or collection.

### 1.3 Search — `POST /search`

Single search route (#39), with **opt-in** stages: the conservative default is precise lexical lookup.

**Request:**
```jsonc
{
  "base": "*",              // required — exact | "pref*" | "*"
  "query": "Frodo Bolseiro",// required
  "collection": "livros",   // optional; absent or "*" = all
  "k": 5,                   // results after the merge (default 5)
  "rerank": true,           // stage 2 (coverage/proximity); false = recall only (default true)
  "recall_n": 20,           // recall candidates per base sent to rerank (default 20)
  "unified": true,          // collection-unified vocab+idf (#8); default true when the scope has a
                            // collection with >1 base; false forces each base's local idf
  "phonetic": false,        // match by SOUND (SOUNDEX): "Aslan" finds "Aslam"
  "literal_fallback": true, // alphanumeric needles WITH a digit (OE-6016, M31May-23h28): literal grep,
                            // exact matches go first (#38); false turns it off
  "expand": false           // true = dictionary → cache → AI cascade (same engine as /search_expand)
}
```
**Response** (`expand: false`):
```jsonc
{
  "via": ["silabico", "literal_fallback"],   // effective stages: silabico | phonetic | literal_fallback
  "query": "M31May-23h28",
  "query_syllables": "m-may-h",
  "scope": ["sessoes/marcadores", "livros/EN_aesop_fables", "…"],  // bases searched ("collection/base")
  "searched": [ { "collection":"livros", "base":"EN_aesop_fables", "n_chunks":133, "n_converge":127,
                  "dims":4, "oov":0, "ms_recall":1.2, "ms_rerank":22.3 } ],
  "needles": ["m31may-23h28"],               // only when the query has an alphanumeric token with a digit
  "hits": [
    { "rank":1, "collection":"sessoes", "base":"marcadores", "corpus":"marcador.txt",
      "matchpoint":1.0, "coverage":1.0, "span":1, "cos":0, "chunk":0, "start":0,
      "snippet":"…O marcador M31May-23h28 indica…", "needles_matched":["m31may-23h28"],
      "via":"literal_fallback" },                       // literal hit
    { "rank":2, "collection":"livros", "base":"…", "corpus":"…", "path":"…",
      "matchpoint":0.8, "coverage":0.8, "span":2, "cos":0.2664, "chunk":28, "start":57193,
      "snippet":"…«Frodo» «Bolseiro»…", "recency":"1.000" }   // syllabic hit
  ]
}
```
- Order: `coverage` ↓ · `span` ↑ · `cos` ↓ · recency (tie-break only); literal hits first, no repeated chunk.
- `coverage`/`span` exist only with `rerank`. `recency` is a string, display only. Literal hits have `cos` 0.
- With `expand: true`, the response is the `/search_expand` one (below), with `via`.
- 404 when no base matches the scope. Search is **deterministic**: same query, same response (#56).

### 1.4 Search with expansion — `POST /search_expand`

Preset of `/search` with `expand: true` (compatible with the old contract).
**Request:** `{query, base="*", collection?, k=8, phonetic=false, two_phase=true, literal_fallback=true}`.

Pipeline: (1) literal first — if the query has needles and the grep finds them, it stops; (2) **two-phase** —
runs the original search and, if the top hit covers the whole query (≥ 0.999) with ≥ min(k,2) hits, returns
without expanding (`two_phase: false` forces the cascade); (3) cascade active dictionaries → cache
(`cache/expansions.json`) → AI (written to the cache); variants are merged and rescored against the original query.

| `source` | `via` | When |
|---|---|---|
| `literal` | `["literal"]` | needles found by grep, before anything else |
| `phase1` | `["silabico"]` | original search already strong (`recall:"strong"`) |
| `dict` · `cache` · `llm` | `["silabico", <source>]` | expansion used |
| `literal_fallback` | `["silabico","literal_fallback"]` | no dictionary/cache/AI, but the literal found it |

**Response (normal):** `{via, query, provider, source, expansions:[…], absent:false, dropped:[…], hits}` —
hits carry the syllabic fields + `var_cov`; the hit's `via` = `"original"` or the variant that brought it.
**Absent** (neither the query nor any variant anchors in the vocabulary): `{…, absent:true, dropped, reason, did_you_mean, hits:[]}`.
**Errors:** 400 with no dictionary, cache or AI provider (and the literal found nothing) · 502 AI failure.
These responses carry no `query_syllables`, `scope` or `searched`.

### 1.5 Chunks and diagnostics

| Method | Route | Request | Response |
|---|---|---|---|
| POST | `/chunk` | `{base, collection?, id, before=0, after=0}` or `{base, collection?, ids:[…]}` | `{collection, base, corpus, n_chunks, chunks:[{id, start, len, tokens, oov, norm, text}]}` · 404 |
| POST | `/histogram` | same body as `/search` (uses hit #1) | `{found:true, collection, base, chunk_id, coverage, cos, query_syllables, vocab_size, query, query_oov, chunk, seq_len, mf}` or `{found:false, query_syllables}` |

### 1.6 Ingestion

All of them prepare the base **outside the lock** (driver, tokenization, writing) and only insert it at the end
(#37): searches don't wait for ingestion. Success: `{ok, collection, name, n_chunks, bases, …}`. **413** when
`max_bases` is reached (new bases only) or the base exceeds `max_chunks_per_base` (0 = no cap).

| Method | Route | Request | Extra response fields |
|---|---|---|---|
| POST | `/ingest` | (a) `{name, path}` tokenized JSON · (b) `{name, data:{meta, idf, chunks}}` · (c) `{name, path, raw:true, chunk=2048, driver?, with_text=true, max_chunks=0}` raw file; + `collection` | `raw, saved_to?` |
| POST | `/ingest_file` | `{path, name?, collection?, chunk?, driver?, with_text?, max_chunks?}` (file on the daemon's machine; `name` derived from the path) | `corpus, saved_to` |
| POST | `/ingest_upload` | multipart (`file` + fields) or raw body + `?filename=&name=&collection=&chunk=&max_chunks=&with_text=&driver=&append=` | `filename, corpus, bytes, appended, driver, saved_to, via` (`multipart`/`raw`) |
| POST | `/ingest_any` | like `/ingest_upload`, but first runs the ingestion driver `ingestors/<kind>.py` (pdf, xlsx, xls, docx, doc, pptx, csv, audio — by MIME or extension; 120 s) | same; `driver` = script used |

- `chunk` = characters per chunk (2048) · `driver` = explicit `.drv` (omitted = by extension, PTBR fallback)
  · `with_text` = keep the text (true) · `max_chunks` = 0 all.
- `append=true` (uploads only): appends to the existing base (recomputes only `idf` and `norm`).
- Upload accepts UTF-8 only (binary → 400, except via `/ingest_any`); above `--max-upload` → 413.

**`POST /transcribe`** — audio in, text out (does not ingest or keep the audio). Same input as the upload; runs
outside the lock. Response `{ok, text, chars, filename, bytes, ms, driver}` · 400 empty body · 413 · 415 no
driver · 422 no text · 500 failure or timeout (`transcribe_timeout_s`, default 900 s).

### 1.7 Removal

| Method | Route | Request | Response |
|---|---|---|---|
| DELETE | `/bases/{name}` | `?collection=` (default `default`) `&purge=1` (also deletes the JSON and `.textblob`) | `{ok, removed, collection, purged, bases}` · 404 |
| DELETE | `/collections/{name}` | `?purge=1` (deletes `ragfiles/<name>/`) | `{ok, collection, bases_removed, purged, bases, collections}` · 404 |

Without `purge`, the base comes back on the next boot (autoload rereads the disk).

### 1.8 Administration

| Method | Route | Requires | Request | Response |
|---|---|---|---|---|
| GET | `/config` | `admin.config` | — | `{storage, config_path, drivers_dir, ingestors_dir, ragfiles_dir, max_upload_mb, max_bases, max_chunks_per_base, session_ttl, dev_mode, anthropic_key_set, anthropic_key_masked, openai_key_set, openai_key_masked, active_provider, local_url, nidhogg_url, cache_dir, expansions_entries, thesaurus_dir, dicts_active, word_syn_entries}` |
| POST | `/config` | `admin.config` | any of `storage` (memory\|hybrid\|disk — reloads the bases), `active_provider` (none\|anthropic\|openai\|local), `anthropic_key`, `openai_key`, `clear_anthropic`, `clear_openai`, `local_url`, `session_ttl` (≥ 60) | `{ok, notes[], reloaded, config:{…}}` · 400 |
| POST | `/config/test_provider` | `admin.config` | `{provider: anthropic\|openai}` | `{provider, ok, message}` |
| POST | `/driver_move` | `admin.config` | `{file:"x.drv", action: install\|uninstall}` | `{ok, file, action, installed}` · 400 · 404 |
| POST | `/thesaurus_toggle` | `admin.config` | `{code, action: enable\|disable}` | `{ok, code, action, active, word_entries}` · 404 |
| GET | `/logs` | `admin.servicos` | `?n=300` (max 5000) | `{file, log}` |

---

## 2. ValHalla — console (11498)

Web console (React, `web/dist`) served by `ragd` itself on `dash_port` (default 11498), with SPA fallback to
`index.html`. **It has no data API of its own:** it proxies over HTTP, forwarding the `Authorization` header:

- `/api/*` → `ragd` on the API port, without the `/api` prefix (e.g. `/api/search` → `:11499/search`);
- `/nidhogg-api/*` → `nidhogg_url` without the prefix (e.g. `/nidhogg-api/api/nidhogg/tree` → `:11497/api/nidhogg/tree`);
- proxy failure → 502; the proxy cuts at 120 s.

Login is the §1.1 JWT (`/api/login`), kept by the UI. The search tab uses `POST /search` (`expand: true` in
semantic mode) and the ingestion tab uses the §1.6 uploads.

> Known proxy limitations: it re-sends the body as `application/json` (a multipart upload through 11498 loses
> its boundary — use a raw body with `?filename=`, or 11499 directly) and the 120 s cut can interrupt a long `/transcribe`.

---

## 3. `nidhoggd` — intelligence (11497)

Module daemon: reads the corpus **always through the `ragd` API** (§1), never from disk, and stores what it
learns in ClickHouse (`store = clickhouse`; `sqlite` is the fallback — routes that depend on ClickHouse return
empty with a `note`, or 400). **No authentication**; CORS only with `cors_origin` set.

Levels: `minerador` (0, no AI) · `consciente` (1, classifies) · `estrutural` (2, entities and graph) ·
`estrutural-llm` (3, AI relations) · `propositivo` (4, questions and answers).

### 3.1 Status and control

| Method | Route | Request | Response |
|---|---|---|---|
| GET | `/health` | — | `{status, module, version, on, level, llm_online, llm_tag}` |
| GET | `/api/nidhogg` | — | `{module, version, uptime_secs, on, level, level_name, levels:[{n, name, ia, desc}], needs_ia, cadence_secs, dir, collections_known, cycle_running, last_cycle, ragd_api, ragd_online, llm_online, llm_tag, llm_url, llm_auth, llm_erro, llm_checked, ragd}` |
| POST | `/api/nidhogg` | `{on?, level? (name or 0–4), cadence? (≥ 10 s)}` (written to the cfg) | same as GET |
| GET | `/api/nidhogg/collections` | — | `{collections:[{collection, bases, chunks, enabled, saturation, updated, has_knowledge}]}` |
| POST | `/api/nidhogg/collection` | `{collection, enabled}` | `{ok, collection, enabled}` |
| POST | `/api/nidhogg/run` | — | 202 `{ok, started:true, note}` (forced cycle, async) · 200 `{ok, started:false, reason}` if one is running |

### 3.2 Knowledge and prompts

| Method | Route | Request | Response |
|---|---|---|---|
| GET | `/api/nidhogg/knowledge` | `?collection=&type=&level=` | the collection's `knowledge.json` (L0 pillars, provenance, `l0_diff`…), or `{collections:[…]}` without `collection` |
| GET | `/api/nidhogg/cachedigest` | — | `{type:"CacheDigest", level, scope, updated, content:{n_queries, n_variants_total, avg_variants, entries, note?}}` |
| GET | `/api/nidhogg/prompts` | — | the `prompts.json` library (with `templates{}`) |
| POST | `/api/nidhogg/prompts/template` | `{name, system (≤ 6000), description, max_tokens (64–4000)}` | `{ok, template}` |
| GET | `/api/nidhogg/llm_ledger` | `?n=30` (max 200) | `{file, entries:[{ts, tag, coll, ctx, meta, ms, ok, finish, system, system_len, user, user_len, resposta, resposta_len}]}` (AI ledger; texts truncated) |

### 3.3 Classification, extraction and graph

| Method | Route | Request | Response |
|---|---|---|---|
| GET | `/api/nidhogg/doctypes` | — | `{naturezas, tipos}` |
| POST | `/api/nidhogg/doctypes` | `{naturezas[], tipos[]}` (non-empty) | `{ok, naturezas, tipos}` (counts) |
| GET | `/api/nidhogg/doctypes/uso` | — | `{uso}` |
| GET | `/api/nidhogg/classes` | `?collection=` | `{collection, count, naturezas, tipos, bases}` |
| POST | `/api/nidhogg/reclass` | `{collection, base, tipo}` | `{ok, collection, base, tipo, natureza, csv, extraivel, nota, purgadas}` (origin `humano`; AI never overwrites it) · 400 unknown tipo |
| POST | `/api/nidhogg/relink` | `{collection, de, para}` | `{ok, collection, de, para, classes, entidades}` (moves class, entities and nodes of a renamed base) · 409 `de` still exists or cycle running · 404 `para` missing · 502 |
| POST | `/api/nidhogg/molde` | `{tipo, instrucao, collection, base}` | `{ok, tipo, campos, cobertura, amostra}` (directed extraction template) · 404 · 502 |
| GET | `/api/nidhogg/templates` | — | `{templates}` (templates per tipo) |
| GET | `/api/nidhogg/rejeitados` | — | `{count, por_motivo, rejeitados}` |
| GET | `/api/nidhogg/entities` | `?collection=&base=` | `{count, nqi_global, por_base, por_tipo, amostra}` |
| GET | `/api/nidhogg/relacoes` | `?collection=&n=200` (max 1000) | `{count, bases, relacoes}` |
| GET | `/api/nidhogg/tree` | `?collection=` (required) `&q=` | `{collection, count, nodes}` |
| GET | `/api/nidhogg/suggest` | `?q=` (required) `&collection=*` | `{collection, count, nodes}` |
| GET | `/api/nidhogg/node` | `?norm=` (required) `&collection=` | `{found, valor, valor_norm, registros, bases, co, facetas}` |

### 3.4 Dimensions

| Method | Route | Request | Response |
|---|---|---|---|
| GET | `/api/nidhogg/dimensoes` | — | `{dimensoes:[{nome, descricao, campos[], tipos[]}]}` |
| POST | `/api/nidhogg/dimensoes` | `{dimensoes[], substituir_tudo?}` | `{ok, dimensoes}` · 409 `{error, sumiriam, dica}` if axes would vanish without `substituir_tudo` |
| POST | `/api/nidhogg/dimensoes/upsert` | `{dimensao}` | `{ok, criou, dimensoes}` |
| POST | `/api/nidhogg/dimensoes/remover` | `{nome}` | `{ok, nome, dimensoes}` · 404 |
| GET | `/api/nidhogg/dimensao/valores` | `?nome=` (required) `&collection=&q=` | `{count, valores, nome}` · 404 |
| GET | `/api/nidhogg/dimensoes/gaps` | `?collection=` | `{collection, gaps:[{nome, alvo, cobertos, gaps[], nota}]}` |

### 3.5 Questions and answers (L4)

| Method | Route | Request | Response |
|---|---|---|---|
| GET | `/api/nidhogg/perguntas` | — | `{perguntas:[{nome, texto, tipo (tabular\|oneshot\|vivo), escopo, ativa, pai}]}` |
| POST | `/api/nidhogg/perguntas` | `{perguntas[], substituir_tudo?}` | `{ok, perguntas}` · 409 `{sumiriam}` |
| POST | `/api/nidhogg/perguntas/upsert` | `{pergunta}` | `{ok, criou, pergunta, perguntas}` |
| POST | `/api/nidhogg/perguntas/remover` | `{nome, purgar?}` | `{ok, nome, etapas_apagadas, perguntas}` · 404 |
| POST | `/api/nidhogg/perguntar` | `{pergunta}` (name) | `{ok, pergunta, nova_etapa, …}` (answers now: analyst + comparator) · 404 · 502 |
| GET | `/api/nidhogg/respostas` | `?pergunta=` (required) | `{pergunta, count, etapas}` |
| POST | `/api/nidhogg/respostas/limpar` | `{pergunta}` | `{ok, pergunta, etapas_apagadas}` |

### 3.6 Command-line modes

- `nidhoggd --laya-check <version> [--laya-ort-lib <libonnxruntime.so>]` — Laya gatekeeper parity (#53) against
  the version's `paridade.json` (exit 0 = ok, 1 = mismatch, 2 = error).
- `nidhoggd --classify-list <file.json>` — classifies `[[collection, base], …]` and prints JSONL
  `{coll, base, natureza, tipo, csv, nat_llm}`, without the daemon.

> Configuration (`nidhogg.cfg`): see `ARCHITECTURE.md` §5 (`llm_*`, `llm_ledger*`, `prune_grace_h`, `laya_*`).

---

> Source of truth: the code in `ragd/src/` and `nidhoggd/src/`. This document was reconciled with the code on
> 2026-10-05 (#39).
