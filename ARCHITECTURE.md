> 🌐 **Language.** English version · 🇧🇷 Versão em português: **[ARCHITECTURE.pt-BR.md](ARCHITECTURE.pt-BR.md)**
> *(the pt-BR version is the canonical source; this English version is a translation kept in sync).*

# RAGnaRock — Architecture & Specification

> **Implementation North Star.** This document describes the **whole** solution — including what hasn't
> been built yet. It is the reference for evolving the project without losing coherence.
>
> Status marker on each item: **[DONE]** · **[PARTIAL]** (skeleton/stub) · **[FUTURE]** (planned).
>
> **Filter for every decision** (applies to any line below): *does this keep RAGnaRock simple,
> transparent, running on any hardware and teachable?* If it requires a black box, a GPU, or complexity
> that scares away the beginner → it's **optional/opt-in or stays out**, however good it is technically.

---

## 1. Overview & invariants

RAGnaRock is a RAG **with no neural network**: token = **syllable** (PT), embedding = **sparse histogram**
(bag of syllables), search = **tf-idf cosine** (recall) + **phonetic matched filter** (rerank).
Everything is inspectable (readable JSON), runs on **CPU + RAM, no GPU**.

**Three daemons**, independent processes that talk over **HTTP JSON**:

| Daemon | Port | Role | Status |
|---|---|---|---|
| `ragd` | **11499** (API) | Engine: holds N bases in RAM, search/ingestion | [DONE] |
| **ValHalla** (in `ragd`) | **11498** (console) | Supervisory web console | [DONE] |
| `nidhoggd` (Níðhöggr) | **11497** (modules) | **Intelligence** layer: distills knowledge | [PARTIAL] |

**Invariants (don't break):**
1. **JSON is the contract and the persistence.** Each base is a readable JSON on disk; RAM is just a
   rebuildable cache. Kill `ragd` → start it again → it reloads from `ragfiles/`. (Answers "do I lose data
   on a crash?": no — what's on disk is the truth; ingestion writes the JSON **before** loading into RAM.)
2. **Same key order in the JSON** (serde `preserve_order`) — guarantees byte-for-byte equivalence across
   the three incarnations of the library (`python_concept`, `rust_concept`, `ragd`).
3. **`nidhoggd` always reads the corpus through the `ragd` API, never from disk** — independent of where the data lives.
4. **Intelligence (AI) is always opt-in and starts off.** The RAG core depends on no AI whatsoever.

---

## 2. Data model

A **base** = `{ meta, idf, chunks }`, persisted at `ragfiles/<collection>/<name>-tokenized.json`.

```jsonc
{
  "meta": {
    "corpus": "MyController.cs",                    // file name (with extension)
    "source_file": "<upload:...>",                  // origin (path or upload label)
    "bytes": 12345, "chunk_size": 2048, "n_chunks": 117,
    "vocab_size": 1956, "vocab_used": 312,
    "tokens_total": 9001, "oov_total": 42, "coverage": 0.9953,
    "with_text": true,                              // do chunks keep the text?
    "generator": "ragd-ingest", "tokens_file": "tokens_CSharp_PTBR.drv",
    "language": "CSharp", "matched_by_ext": true,
    "built_at": "2026-05-23T...", "vocab": ["ca","sa",...]   // driver vocabulary (fixed order)
  },
  "idf": { "<dim>": 0.693147, ... },                // idf per dimension (syllable)
  "chunks": [
    {
      "id": 0, "start": 0, "len": 2034,             // offset/len in chars within the corpus
      "tokens": 410, "oov": 3,
      "vec": { "<dim>": <count>, ... },             // sparse tf (syllable histogram)
      "norm": 16.374664,                            // L2 norm of the tf-idf vector (for cosine)
      "text": "...",                                // the chunk's text (if with_text)
      "words": [["fro","do"],["bol","sei","ro"]]    // [DONE, in RAM] syllables per word (rerank cache)
    }
  ]
}
```

- **Smoothed `idf`:** `idf(dim) = ln((N + 1) / df)` where `N` = number of chunks, `df` = number of chunks
  that contain the dim. The `+1` avoids collapsing to 0 in a **single-chunk** base (with `ln(N/df)`,
  df=N=1 → idf=0 → null vector → invisible base). [DONE]
- **`vec` is raw tf** (independent per chunk). Only `idf` (global to the base) and `norm` (per chunk) depend
  on the whole corpus → that's why append recomputes only those two. [DONE]
- **`words`** (syllables per word) is not serialized: it's derivable from `text` and cached in RAM in
  `memory` mode; in `hybrid` mode it's recomputed on demand during rerank. [DONE]

### 2.1 Syllabification — the token's algorithm [DONE]

The token is the **syllable**, produced by a deterministic PT-BR syllabifier in `ragd/src/tokenizer.rs`
(`syllabify`, `syllable_seq`, `normalize`). Effective rules:

- **Vowels/semivowels:** vowels `a e i o u` (+ accented ones); weak `i u` form a **diphthong**; accented
  `í ú` **break** the diphthong (force a hiatus).
- **Nuclei:** strong vowel + strong → **hiatus** (split: "co-a-lha"); weak + strong or vice versa →
  **diphthong** (join: "pou-so", "coi-sa").
- **Onsets:** digraphs `ch lh nh` = **one** sound; `qu`/`gu` + high vowel → silent `u`; mute+liquid clusters
  (`bl br cl cr dl dr fl fr gl gr pl pr tl tr vl vr`) stay together in the onset.
- **Coda × onset:** a single consonant between nuclei becomes the coda of the previous one + the onset of
  the next; in a cluster, the **last two** go to the next onset **if** they form a valid cluster, otherwise
  only the last one.
- **Normalization = canonical key:** lowercase → Unicode NFD → strip diacritics. "Narnia"→"narnia",
  "Élrond"→"elrond" — an accent does **not** create a distinct dimension.

> The PoCs `python_concept/` and `rust_concept/` are a **frozen reference** (historical validation); the
> living spec is `ragd`'s. **[FUTURE]** golden cases (words with consensus syllabification) to harden the
> syllabifier against regression.

---

## 3. `ragd` — the production daemon

### 3.1 Process, ports, state, config

- A single Rust process serves **two ports** via `Arc<Mutex<State>>`: **11499** (JSON API) and **11498**
  (ValHalla, separate thread). [DONE]
- `State` = bases in memory (`HashMap<collection, HashMap<name, RagBase>>`), drivers_dir, ragfiles_dir,
  config, console sessions. [DONE]
- **Auto-load on boot:** scans `ragfiles_dir` (each subdir = collection, each `*-tokenized.json` = base). [DONE]
- **Config `ragnarock.cfg`** (keys):

  | key | default | function |
  |---|---|---|
  | `api_port` | 11499 | JSON API port |
  | `dash_port` | 11498 | ValHalla console port |
  | `drivers_dir` | `drivers` | language drivers (`.drv`) |
  | `ragfiles_dir` | `ragfiles` | tokenized bases (auto-load) |
  | `max_upload` | 1 GB | cap for `POST /ingest_upload` |
  | `autoload` | true | load bases on boot |
  | `storage` | `memory` | `memory` (caches tokens) \| `hybrid` (recomputes) |
  | `admin_user`/`admin_pass` | admin/admin | console login — **[FUTURE] change outside dev** |
  | `active_provider` | none | `none`\|`anthropic`\|`openai` (1 active; for query expansion) |
  | `cache_dir` | `cache` | `thesaurus.json` / `expansions.json` |
  | `log_file` | `/tmp/ragd-all.log` | file read by the Logs tab (= launcher redirect) |
  | `log_utc_offset` | -3 | timezone of timestamps |

  > ⚠️ `ragnarock.cfg` holds the providers' **API keys** → it's in `.gitignore`. Version a sanitized
  > `ragnarock.cfg.example`. [FUTURE]

### 3.2 Search pipeline [DONE]

`base.search(query, k, rerank, recall_n, phonetic)` → `(hits, info)`, in two stages:

1. **Recall (sparse tf-idf cosine):** tokenize the query into syllables → tf vector weighted by `idf` →
   cosine against each chunk (iterate the smaller vector; only shared dims count). Take the `recall_n` candidates.
2. **Rerank (proximity phonetic matched filter):** over the candidates, measure the **smallest window**
   that covers a match of each query word (proximity), **ignoring monosyllables** (stopwords), with optional
   soundex (`phonetic`). Score combines coverage + proximity. Returns top-`k`.

- **Phonetic rerank (SOUNDEX — `ragd/src/rag.rs`):** with `phonetic:true`, two terms match when they share
  the **same SOUNDEX code** (consonants mapped 1–6; vowels/`h`/`w` = 0; classic `h`/`w` retention; truncated
  to 4). Applied **only to terms of ≤3 syllables** (names/spelling variants: `"Aslan"`→`"Aslam"`); long terms
  discriminate by their own syllable sequence (avoids false matches like `ressurreição`~`rigorosa`).
  Computed **once per query** (not per candidate). It is a `ragd` feature — it does not exist in the frozen PoCs.
- **Collection-unified recall (`unified:true`, opt-in — [#8]):** instead of each base's local idf, the recall
  runs in a per-collection **unified space** — a `CollectionProfile` (vocab merged from the bases' drivers +
  idf recomputed over the whole collection = "repo idf"), built in memory and cached, auto-invalidated by a
  fingerprint `(n_bases, total_chunks)`. Each chunk's `vec` is remapped local→global on the fly. Lets a query
  match across **files of different languages** (e.g. Python + Rust in the same collection) with a
  discriminative repo idf. Default **off** (per-request flag); the rerank stage is unchanged.
- **Scatter-gather:** `/search` resolves the scope (`collection` + wildcard on `base`: `"sda"`, `"sd*"`,
  `"*"`), searches each matching base (parallelized with rayon when there's >1 base) and **merges by matchpoint**.
- **Hit:** `{ collection, base, corpus, path, chunk, matchpoint, mf, span, cos, start, snippet }` — the
  `path` is reconstructed (`base` decoded `__`→`/` + `corpus`) so the **AI goes straight to the file**. [DONE]
- **Query expansion (`search_expand`):** **dictionary → cache → AI** (active provider) cascade that expands
  the query with synonyms before searching, with a **vocab filter** (only synonyms anchored in the scope's
  corpus) and higher weight on the original term. Exposed on the API (11499) **and** in the console. [DONE]
  - ⚠️ Low-`idf` synonyms (common words) can dominate and pollute precise lookups → the consumer should
    prefer **pure** search (`expand=false`) for identifier/file lookup. **[FUTURE]:** prune low-idf
    synonyms in the expansion.

### 3.3 Ingestion [DONE]

- `POST /ingest` (tokenized JSON, inline base, or raw), `POST /ingest_file` (path), `POST /ingest_upload`
  (multipart **or** raw body + query string — ingests **raw text with no file**).
- **Default = overwrite by name** (`bases.insert(name, base)` — replaces the whole base).
- **Incremental append with chunk-packing** (`append=true`): instead of creating a new chunk, it **fills the
  last chunk up to `chunk_size` and overflows** the excess; only the "tail" (last chunk + new text) is
  re-tokenized, the rest reuses its `vec`; recomputes global `idf` + `norm`. Chunks grow ordered and full →
  with `N>1` the idf starts to discriminate.
- **Persistence:** writes `ragfiles/<collection>/<name>-tokenized.json` **before** loading into RAM.

### 3.4 Memory and disk strategy

| mode | in RAM | trade-off | status |
|---|---|---|---|
| `memory` (default) | `meta`+`idf`+`chunks` **with `words` cached** | faster search, +RAM | [DONE] |
| `hybrid` | same **without `words`** (recomputes only for candidates at rerank) | −66% RAM measured, slightly slower broad search | [DONE] |

- **Durability:** the truth is on disk (`ragfiles/`); RAM is a cache → a crash recovers on boot.
- **`[FUTURE]` mmap/on-disk Qdrant-style:** **not now.** Kimi and Codex converged: the system is
  **CPU-bound on syllabification**, not I/O-bound; mmap adds bug surface (corruption, lock, flush) and
  **betrays "runs anywhere"** (native/FS dependencies). Only consider if **corpus > ~80% of RAM**, and even
  then **opt-in by build/config** (modular), never default.
- **Memory pressure:** the console measures RSS (`/proc/self/statm`) + text/vec/words estimate; measured:
  ~580 bases ≈ 516 MB (`memory`) → 174 MB (`hybrid`). [DONE]

### 3.5 Concurrency

- **Today:** global `Arc<Mutex<State>>` — every operation (read or write) competes for the same lock.
  Measured throughput: ~500 req/s on an M-series Mac, ~65 on a 2-core x86, ~43 on a Raspberry Pi 3 (global search). [DONE]
- **Why it's enough today:** the main use is **ONE AI, sequential** — no real contention. A Mutex works
  well up to dozens of concurrent req/s.
  - ⚠️ **Note (Kimi):** `rayon` parallelizes the scatter-gather, but the global `Mutex` **re-serializes**
    internally — real parallelism only comes with the `RwLock`/per-collection lock below. It's a **[FUTURE]**
    optimization of **the same priority** as on-disk reads in `hybrid`; not urgent with 1 sequential AI.
- **`[FUTURE]` when it becomes multi-agent:**
  - `Mutex<State>` → **`RwLock<State>`**: N **read-only searches** in parallel; `write()` only on
    ingest/delete. (Codex's caveat: rerank in `hybrid` recomputes `words` — but that's a pure read, fits the
    read-lock; it doesn't become a write.)
  - **Per-collection granularity** (lock per collection, not global) → search collection A while ingesting into B.
  - Careful: writer starvation if readers are continuous (use a fair `RwLock`).
  - Codex suggests decoupling ingest×search via **channel/message** (lock-light) — keep that for if the RwLock
    isn't enough; YAGNI before that.

### 3.6 Language drivers [DONE]

- **Source-code** tokenization uses `.drv`: syllables from the `SourceCode` base (PT + code syllables) +
  the language's **reserved keywords**. `tokens_PTBR.drv` and `tokens_SourceCode_PTBR.drv` are the **fixed
  matrix**; the others derive via `tools/gen_drivers.py`. `GET /interpret?file=foo.py` routes extension →
  driver/language.

### 3.7 HTTP contract — routes

> 📐 **Detailed contract** (request/response of each route of the 3 daemons): **[`JSONCONTRACT.md`](JSONCONTRACT.md)**.
> Runnable `curl -d @` examples: `ragd/json_samples/`. Below, the route summary.

**Implemented [DONE]:**

| method | route | function |
|---|---|---|
| GET | `/health` | `{status, bases, collections, drivers}` |
| GET | `/bases` `?collection=&match=` | list bases (with `corpus`, `n_chunks`...) |
| GET | `/collections` | summary per collection |
| GET | `/drivers` `?match=` | list drivers |
| GET | `/interpret` `?file=\|?ext=` | extension → driver |
| POST | `/search` | pure search (recall+rerank) |
| POST | `/search_expand` | search with query expansion |
| POST | `/ingest` · `/ingest_file` · `/ingest_upload` | ingestion (includes `append=true`) |
| POST | `/chunk` | retrieve whole chunk(s) by id (`before`/`after`) |
| DELETE | `/bases/{name}` `?collection=` | remove base |

**To define/missing [FUTURE]:**
- `DELETE /collections/{name}` (remove a whole collection).
- `GET /stats` (public aggregate; today only internal in the console).
- `GET /bases/{coll}/{name}` (metadata of 1 base without searching).
- `GET /profile?collection=&base=` — **lexical profile** (`vocab_used`, `dims`, `top_idf[]`) to feed
  Nidhogg's **level 0** without probing via `/search` (expensive). Found during the review cycle (§5.3).
- Ingestion **by file inside a repo** (base = N files; incremental update by file `sha` — see §6). Today
  base = 1 file.

---

## 4. ValHalla — web console (11498) [DONE]

Supervisory console embedded in `ragd` (HTML in the binary), **cookie session** (login `admin/admin`,
TTL; **[FUTURE]** real password). Tabs:

- **Overview** — collections/bases/chunks/drivers, distribution bars, memory pressure.
- **Search** — form + results; **expand 🧠** toggle (calls `/api/search_expand`) and **phonetic**;
  chunk modal (file + path + chunk N/total).
- **Ingestion** — file/folder upload (`webkitdirectory`), pick a collection, status per file.
- **Performance** — query×chunk histogram, matched filter with convergence point, heatmap.
- **Drivers** — list of languages/keywords.
- **Logs** — tail of `log_file`, auto-refresh, colored lines (the `search_expand` hierarchical tree shows here).
- **Config** — `memory|hybrid` storage, API keys (masked vault, 1 active provider), restart.
- **Dictionaries** — turn thesaurus dicts on/off (toggle by flag, doesn't move the file).
- **Nidhogg** **[FUTURE]** — the "big screen" of the intelligence layer: turn on/off globally and per
  collection, level dial + cadence/window, per-level prompt, the **acceptance gate** toggle + **accept**
  button for each artifact version, and reading the versioned artifacts (living document, knowledge tree).
  **On TURN-ON: mandatory disclaimer** about AI consumption.

> ValHalla **reads and operates** `ragd`; it has no search logic of its own (delegates to the API).

---

## 5. `nidhoggd` / Níðhöggr — intelligence layer (11497) [PARTIAL]

> In the myth, Níðhöggr is the serpent that gnaws the roots of Yggdrasil. Here, the worm gnaws/**digests
> the knowledge** of the RAG's tree and distills it into insight that **survives the deletion of the collection**.

> 💎 **Why Nidhogg matters (positioning — owner's decision).** It is the **analytical layer** — the
> **turning point where the project becomes a product of value ($$$)**. The core (`ragd`) is OSS and runs
> anywhere; Nidhogg is where the **open source subsidizes its users**: it produces **concrete, AI-assisted
> analyses** on any subject (code, books, articles), letting a **consultant / student / company arrive
> well-grounded**. Whoever turns the AI on reaps understanding worth money.

### 5.1 Concept & invariants [DONE: skeleton]

- **Separate** process, a **"module daemon"** (port 11497 will host N modules beyond Nidhogg).
- Reads the corpus **always via the `ragd` API** (never disk) → independent of location.
- **Starts OFF** (levels ≥1 consume AI). Turn on/off **globally** and **per collection** (doesn't re-chew
  the same one N times). A keepalive pings `ragd` every 15s and caches it (status never does a live curl).
- **Two orthogonal dials:** **level** (depth) + **cadence** (seconds between cycles = time budget).

### 5.2 Nature & consumption — Nidhogg is AUTONOMOUS; the reader is HUMAN

> **Decision (owner):** Nidhogg is an **autonomous project**, a **critical analyzer**. `ragd` **NEVER**
> consumes it — decoupled daemons. The value is in the **artifact itself**; it **does not depend** on being
> consumed by another machine. *"It doesn't matter whether anyone will consume it or not"* — the
> **accumulated understanding IS the product** (like a scholar's notebook that grows on its own). This
> answers Codex's critique at the root: the consumer is the **human who reads**, not a system.

- **Consumer = the human**, via ValHalla (and export): opens and **reads** the distilled artifacts.
- **First-class artifacts** (deliverables, not an auxiliary search index):
  - **Living document** (**propositive** level): grows **indefinitely** each cycle. Owner's use case:
    *open it after 15 days and read a deep summary of a work (e.g. The Lord of the Rings), with nuance of
    detail, in styles (modern, archaic…)* — a "companion" that deepens over time.
  - **Knowledge tree / mind map** (**structural** level): navigable, starting from the work — valid for
    **source code, text, book, article**, any ingestion of the base.
- **`GET /api/nidhogg/knowledge?collection=&type=&level=`** serves these artifacts (for ValHalla and export).
- `ragd` **does not read or inject** this into search. If one day an agent wants to use the artifacts as
  context, it reads them via the Nidhogg API — **secondary, optional use**, not the reason to exist.

### 5.3 `source_hash`, diff and incrementality [FUTURE]

Kimi and Codex converged: detect real change **cheaply**, no false positives, and digest **only what changed**.

- **`state_hash` per base** = `hash(base_name, n_chunks, vocab_size, corpus)` — cheap, comes straight from
  `GET /bases` (doesn't read the content). **Never uses path** (renaming the path doesn't change it;
  `base_name` is a stable id).
- **Collection `source_hash`** = hash of the **ordered** list of its bases' `state_hash`.
- **Diff per cycle:** compares the previous checkpoint (`{base → state_hash}`) against the current one →
  **new / changed / removed** bases. Processes only the changed ones; marks orphans (removed); keeps the intact ones.
- **Doesn't re-chew** a collection/base whose `state_hash` equals the last one → saves AI (cadence ≠ rework).

> ⚠️ **Contract gap found (Kimi):** `ragd` **doesn't expose today** `idf`/`dims`/effective vocabulary per
> base in an endpoint — level 0 would have to **probe** via `/search` with syllable probes (expensive).
> **Decision:** add a **profile** endpoint to `ragd` → `GET /profile?collection=&base=` returning
> `{vocab_size, vocab_used, dims, top_idf:[{dim,syllable,idf,df}]}`. Feeds level 0 cheaply. **[FUTURE — new contract in ragd]**

### 5.4 The 4 levels — algorithms and schemas

| level | name | AI? | produces | status |
|---|---|---|---|---|
| 0 | **miner** | no | 3 pillars: RootIndex · CorpusDict · CacheDigest | [PARTIAL] |
| 1 | **conscious** | yes | `Summary` per collection (insight that survives deletion) | [FUTURE] |
| 2 | **structural** | yes | **Knowledge tree / mind map** of the work (`KnowledgeTree`) | [FUTURE] |
| 3 | **propositive** | yes | **Incremental living document** (`LivingDocument`, grows over time) + `Gap`/`Suggestion` | [FUTURE] |

**Level 0 (no AI) — the 3 pillars.** ⚠️ **Honesty (Codex):** level 0 is **navigation / index /
health-check** ("is my collection sound and navigable?"), **not "knowledge"** — don't sell it as such.
Even so it delivers value on its own (base for the AI levels + observability) and costs zero AI.

> 🌱 **The seed of Nidhogg (origin of the idea).** Level 0 is the piece that gives RAGnaRock back
> **collections organized about its own collections** — a **self-organization agent** of the RAG over
> itself. This is where Nidhogg was born. That's why the **0→1 step never has an acceptance gate** (§5.4):
> there's nothing for a human to approve when the RAG is just tidying itself up.

- **RootIndex** — most salient syllables/dims per collection (ranked by `idf × freq`), grouped by root.
  `content:{ bases_count, total_chunks, roots:[{stem, dims, df_chunks, idf_score, bases}], coverage_ratio }`.
- **CorpusDict** — effective vocabulary (dims used, top by `idf`, coverage/`oov` per base, shared vs unique
  dims). `content:{ vocab_size, active_dims, top_idf:[{dim,syllable,idf,df}], shared_dims, unique_dims }`.
- **CacheDigest** — consolidates the query-expansion cache: synonyms seen ≥ N times that map to the **same**
  chunks become equivalence clusters. `content:{ entries:[{canonical, variants, shared_chunk_ids, hit_count}], hit_rate }`.

**Levels 1–3 (AI) — input, sampling and output:**

| level | input to the LLM | sampling | output (`type`) |
|---|---|---|---|
| 1 | chunks **new/changed** since `source_hash` + base meta | up to `MAX_CHUNKS_PER_LEVEL` (~100) spaced + top-N by `idf`; if few, all | `Summary {themes, entities, key_chunks, abstract, chunk_range}` |
| 2 | level-1 `Summary` of the work/collection (metadata — not a sample) | — | `KnowledgeTree {root, nodes[], edges[]}` — navigable mind map (dimension hierarchy/fit is the base) |
| 3 | the work + `KnowledgeTree` + `Summary` + the previous version of the living document | incremental: only what came in since the last cycle | `LivingDocument {sections[], style, version, grows:true}` (deep summary that grows, style variants) + `Gap`/`Suggestion` |

- **The budget is the RUNNER'S DECISION:** cadence = **time** budget per cycle; add a cap of **tokens/cycle**
  for the AI levels. Starts **OFF**, opt-in per collection (§5.1), level 0 covers the no-AI case, and **no
  spending cap by default** — a conscious choice, with a **mandatory disclaimer on turn-on** (§4). The AI
  graph (level 3) multiplies consumption (N AIs × N levels) — it's the $$$ layer par excellence.
- **Rate-limit = the dimension configuration itself (owner's decision):** there's no separate rate-limiter.
  The owner defines **when each dimension fires within the cycle** (e.g. level 1 every cycle, level 3 every
  N cycles) — *that already is* the throughput/cost control. Add the optional tokens/cycle cap on top. Thus
  "consuming a lot of AI" becomes an explicit config choice, not an accident.
- **Incremental:** level 1 processes only new chunks; level 0 reprocesses the whole changed base (it's cheap).
- **HIERARCHICAL order (1→2→3):** the AI levels happen **in sequence** — there's no level 2 without 1, nor 3
  without 2 (the knowledge dimension is hierarchical by nature). The **dial selects the top level**; the
  worker runs `1..N` in order within the cycle. Level 0 (no AI) is always the base.
- **Additive, versioned + acceptance gate (owner's decision):** each dimension's artifact is **versioned** —
  every re-derivation creates a new `version` and archives the previous one as `frozen_version` (§5.7); the
  set of versions **only accumulates** (additive), even when the active body is replaced. Between one
  dimension and the next there's an **optional acceptance gate, switchable PER DIMENSION** (`accept_gate` =
  set of levels with a gate, in ValHalla) — **not** global and **not** per generated item. Turning the gate
  on at dimension N means: *N's artifact only releases dimension N+1 after approval*. **There are only two
  logical gate points: `accept_gate ⊆ {1, 2}`** (controlling 1→2 and 2→3):
  - **0→1 never has a gate** — level 0 is autonomous self-organization of the RAG over itself (the seed of
    Nidhogg, see above); there's nothing to approve.
  - **3 has no gate** — it's the top level, there's no next dimension to release.
  - **1→2 is rarely left on in practice** — dimension 1 emits *far* more artifacts (one `Summary` per
    collection/base); approving them all would be unfeasible. The gate at **2→3** is the palatable one (far fewer artifacts).
  - **dimension without a gate (default):** automatic cascade — the artifact feeds the next dimension in the same cycle.
  - **dimension with gate ON:** the artifact stays `pending` and **only releases the next dimension after
    human acceptance** (button in ValHalla). It's the quality checkpoint — e.g. with a gate on dim. 2, the
    human validates the tree **before** the living document (dim. 3) is born from it. The acceptance is also
    the **utility signal** (closes the feedback loop Kimi pointed out) without `ragd` ever consuming the artifact.
  - **Conscious trade-off (to stress):** turning the gate on **breaks the autonomous cycle** (§5.2) and
    injects **human dependency** — the worm stops and *waits* for the acceptance, no longer running on its
    own. This may be acceptable (I want to review before going deeper) or unacceptable (I want the worm 100%
    autonomous). There's no right answer: that's why it's an **opt-in feature**, an *autonomy × control*
    trade-off decided case by case by whoever operates it — never imposed.
- **Per-level prompt = the TONE (owner's decision):** each AI level (1, 2, 3) has a **configurable prompt**
  (editable in ValHalla / `nidhogg.cfg`: `prompt_consciente`, `prompt_estrutural`, `prompt_propositivo`) —
  it's how you dictate the **tone/style** of each extraction (e.g. modern vs archaic in the `LivingDocument`).
- **Delta cascade — 3 re-derivation modes (owner's decision):** when a lower level changes, the upper one
  re-derives through the `derived_from`/`digestion_id` link, but **does not assume monotonic growth** (Kimi's
  critique: tracking provenance ≠ tracking semantic impact). The artifact grows, **shrinks or is remade**, in
  three modes:
  - **additive** — appends the delta (the work gained content; the `LivingDocument` extends, the tree gains a branch).
  - **structural replacement** — swaps a **whole section / branch**, not just the tip. It's the common case in
    **code**: changing a structural line rewrites the whole *line of reasoning* — *"the rope doesn't just
    grow; sometimes the segment is swapped entirely"*.
  - **full rewrite** — the change invalidates the framing (central themes/entities changed at level 1); the
    artifact is **remade from scratch**.
- **What gets swapped never disappears:** the previous version of the branch/document becomes a
  `frozen_version` (preserves history — §5.7); the **active body** is always the current one. The mode
  trigger: additive change → local; *reframing* detected at level 1 → replacement/rewrite. (Fine impact
  mechanism — a semantic signature per branch to decide local vs. global — is **[FUTURE — implementation]**.)
- **Self-improvement built into the propositive layer (dispenses with "Synthesis"):** dim. 3 **reads the
  previous version of the document and improves it** — refining IS the propositive analysis. So there's **no
  separate consolidation mechanism**: the risk Kimi raised (the living document bloats/repeats/contradicts)
  resolves by construction, inside level 3 itself, every cycle.
- **AI graph in confrontation — exclusive to the propositive layer (owner's decision):** at dim. 3 the final
  artifact **need not come from a single AI**. The operator builds an **inference graph** in ValHalla: nodes =
  available AIs (pluggable providers — Bedrock, Kimi, Codex, local…), edges = **who confronts / feeds whom**,
  in **N levels** of confrontation until *"the artifact that generates the final artifact"*. It's the Side AI
  pattern (generator × critic × arbiter) **institutionalized inside Nidhogg**. The graph's intermediate
  artifacts are **input**; only the root node emits the versioned `LivingDocument`.
  - **Only valid for dim. 3.** Dims 1 and 2 use **direct AI** (1 call per extraction) — multi-AI confrontation
    is a cost only the propositive layer justifies.
  - The **provider** here stops being "pick 1 model" and becomes "orchestrate several in a confrontation DAG"
    (pluggable: Bedrock, model choice, round-robin). Graph config in `nidhogg.cfg`/ValHalla. **[FUTURE]**

### 5.5 Worker cycle, files and resumability [FUTURE]

**Per-collection layout** (in `dir/`), append-only to be resumable:

```
<dir>/
  <coll>.knowledge.jsonl    # 1 knowledge item per line (atomic write, append)
  <coll>.checkpoint.json    # { base_name: state_hash } + last base processed
  <coll>.provenance.jsonl   # 1 digestion per line
  <coll>.config.json        # { enabled, level, cadence_s, last_run, accept_gate:[⊆{1,2}] }
```

**Pseudo-flow of one cycle** (synthesized with Kimi):

```
for each ENABLED collection:
  current_bases = GET /bases?collection=coll
  diff = compare(previous_checkpoint, state_hash(current_bases))   # new/changed/removed
  digestion_id = uuid()
  top_level = config[coll].level                        # dial = top level (0..3)
  level 0 (always): RootIndex + CorpusDict + CacheDigest of the collection → append to .jsonl
  for each CHANGED base: update checkpoint[base] = state_hash
  # AI levels in STRICT sequence, always per collection/work (never cross-collection).
  # the gate is PER DIMENSION: level N only runs if N-1 has no gate, or its version is accepted:
  released(N) = ((N-1) not in accept_gate) or (current_version(N-1).status == "accepted")
  if top_level >= 1 and released(1): Summary of NEW chunks → new version → append (pending|accepted)
  if top_level >= 2 and released(2): KnowledgeTree of the work ← Summary (re-derives the branch) → version → append
  if top_level >= 3 and released(3): LivingDocument (appends delta) + Gap/Suggestion
                                     ← KnowledgeTree + Summary + previous version → version → append
  write provenance(digestion_id, inputs=changed_bases)
  recompute saturation ; mark orphans (removed bases)
```

**Resumability:** if the AI fails midway, the already-written `.jsonl` is valid (atomic append) and the
`checkpoint` points to the last completed base → the next cycle **resumes** from there. Never a blind append:
dedup by `digestion_id`/`derived_from`. The consolidated `<coll>.knowledge.json` (§5.6) is a **view** of the
`.jsonl`+checkpoint (or the `.jsonl` becomes canonical and the `.json` is generated). **[implementation decision]**

### 5.6 Consolidated knowledge schema — `<dir>/<coll>.knowledge.json`

One file **per collection** (today: `{collection, enabled, source_hash, saturation, updated, provenance,
knowledge[]}`). Target shape:

```jsonc
{
  "collection": "my_collection",
  "enabled": true,
  "source_hash": "sha256 of the collection's state at the last digestion",
  "saturation": 0.0,                 // 0..1 — fraction of the knowledge still verifiable (see 5.4)
  "updated": "ISO8601",
  "provenance": [                    // traceability: where EACH digestion came from
    { "digestion_id":"uuid", "ts":"ISO8601", "source_hash":"sha256", "level":1,
      "inputs":["collection:my_collection"], "model":"kimi-for-coding|null", "tokens_in":0, "tokens_out":0 }
  ],
  "knowledge": [                     // the distilled items
    { "type":"RootIndex|CorpusDict|CacheDigest|Summary|KnowledgeTree|LivingDocument|Gap|Suggestion",
      "level":1, "version":1, "created":"ISO8601", "content":{}, "confidence":0.0,
      "derived_from":["digestion_id"], "frozen":false,   // frozen=true when the source dies/changes
      "status":"pending|accepted" }   // acceptance gate (§5.4): pending blocks the next level if accept_gate=on
  ]
}
```

### 5.7 Saturation, provenance, surviving deletion

- **`source_hash` (hash, not name):** each knowledge item points to a hash of the source's state. Renaming/
  deleting the collection **does not invalidate** what was already distilled; it only marks that the source changed.
- **`saturation` = (items still verifiable against a live source) / (total items).** `→1.0` everything
  anchored; `<0.5` a warning of too much **orphaned** knowledge. It naturally decays if collections disappear.
- **Dead source → FROZEN artifact (owner's decision):** when the source disappears/changes, the distillate is
  **never deleted** — surviving deletion is the *feature*. It becomes `frozen:true` with a **freshness seal**
  (source **alive** / **changed** since X / **frozen** at X) so the human reader knows the state. `saturation`
  is just this **freshness indicator**, **never** a pruning trigger.
- **Invariant:** no level ≥1 item is generated without `provenance` (digestion_id + source_hash + model).
- **Cadence ≠ saturation:** the worm doesn't re-chew a saturated collection (`source_hash` equal to the last one) — saves AI.

### 5.8 Module API

> Detailed contract in **[`JSONCONTRACT.md` §3](JSONCONTRACT.md#3-nidhoggd--intelligence-11497-partial)**.

**[DONE]** `GET /health` · `GET /api/nidhogg` (status: level, cadence, ragd keepalive, knowledge) ·
`GET /api/nidhogg/collections` (collections + digestion state) · `POST /api/nidhogg`
(`{on, level, cadence}`) · `POST /api/nidhogg/collection` (`{collection, enabled}`) ·
`POST /api/nidhogg/run` (triggers a cycle — **stub**).

**[FUTURE]** `GET /api/nidhogg/knowledge?collection=&type=&level=` (consumption of the distilled insight — §5.2) ·
`POST /api/nidhogg/accept` (`{collection, type, level, version}` → marks `status:accepted`, releases the next
level when `accept_gate=on` — §5.4).

### 5.9 Open questions (agenda for the next Side AI rounds)

> A **live** section: records only what's **not yet decided**. When a question closes, it **leaves here and
> becomes a decision in the body** — it doesn't stay as a "resolved risk" (that would be redundant). The risks
> from cycles 1–4 (solution-looking-for-a-problem, orphan/stale, cost, framing) were resolved and now live in
> §5.1 (off/opt-in), §5.2 (autonomy/human consumer), §5.4 (hierarchy, gate, graph, budget, per-level prompt)
> and §5.7 (survival by freezing).

- **Semantic impact of the delta cascade** — when a source changes, deciding between re-deriving *just the
  branch* or *rewriting* is, today, a heuristic by *reframing* (§5.4). The fine mechanism (a semantic
  signature per branch to measure propagation) is **[FUTURE — implementation]**.
- *(the next Side AI rounds record here whatever comes up.)*

---

## 6. Cross-cutting strategies & bigger roadmap

- **Repo as a base (not 1-file-per-base) [FUTURE]:** the chunk schema gains `file` + lines; `meta` gains a
  file map with `sha`; `POST /ingest_file {base, file}` recomputes only that file (removes the file's old
  chunks, inserts the new ones, updates `sha`); `POST /sync {base, path}` scans and updates only what changed.
  It's the heart of "code RAG with per-file update".
- **Ingestors triggered by the user's AI [FUTURE]:** the agent triggers ingestion (repo, git diff, specific
  files) via MCP / CLIs.
- **Importers [FUTURE]:** PDF/DOCX/XLSX (client-side extraction vs server-side sidecar).
- **Windows build [FUTURE]:** pure Rust should compile; mind `/dev/urandom` (Windows entropy) and the
  `log_file` default.
- **Deploy [DONE]:** cross-compile (`cargo zigbuild --target {x86_64,aarch64}-unknown-linux-gnu.2.31`) +
  rsync the binary + a launcher that brings up `ragd` + `nidhoggd` detached and redirects stdout to `log_file`.
- **Security [PARTIAL]:** change `admin/admin`; keys only in `cfg` (gitignored); CORS open on 11497 (revisit
  when exposing); console session with TTL (rotate cookie — [FUTURE]).

---

## 7. Appendix — failure modes

**Obvious:** corrupted base JSON (write a `.bak` before overwrite, validate on load) · OOM (use `hybrid`,
[FUTURE] base/chunk caps) · AI down (levels ≥1 degrade to level 0) · ragd down (Nidhogg's keepalive degrades
gracefully, cached status).

**Non-obvious (Kimi/Codex):** divergent syllabification between ingestion and search (same driver/vocab is
mandatory — append inherits the base's driver) · writer starvation in the RwLock (use a fair lock) ·
`source_hash` false positive on rename (manual re-link [FUTURE]) · slow rerank in `hybrid` (acceptable;
measure) · `knowledge.json` growing unbounded (compact provenance [FUTURE]) · level 3 hallucinating
nonexistent gaps (confidence + human audit).

---

## 8. Pending decisions (trigger → action)

| decision | trigger | options |
|---|---|---|
| mmap/on-disk | corpus > ~80% RAM | structured binary vs LMDB; **opt-in by build** |
| `RwLock` + inter-query parallelism | latency under real concurrent load | RwLock; then per-collection lock |
| base = repo (N files) | use it as a serious code RAG | `file`+`sha` in the schema; `/sync` |
| prune low-idf synonyms in expand | expansion polluting lookup | idf filter in the cascade |
| Nidhogg levels 1–3 (AI) | owner's decision (budget + cadence) | start at level 0; rise per roadmap, gated by budget+disclaimer (§5.2/§5.4) — **not** by "waiting for a consumer" |

---

> *Living document — increment each cycle. Curated synthesis from the Kimi (generator) × Codex (critic)
> counterpoint, keeping the helm on the mission: simple, inspectable, runs anywhere.*
