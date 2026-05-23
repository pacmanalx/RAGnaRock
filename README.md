> 🌐 **Language.** This is the English version · 🇧🇷 Versão em português: **[README.pt-BR.md](README.pt-BR.md)**
>
> 📖 **Docs note.** This README is in English. The rest of the project — `ARCHITECTURE.md`, `JSONCONTRACT.md`,
> in-code comments and the didactic `logic_path/` track — is currently in **Portuguese (pt-BR)**. An English
> translation is planned.

# 🪨 RAGnaRock

**RAG on the rock.** A RAG (Retrieval-Augmented Generation) built **from scratch, with no neural
network** — just counting and linear algebra — using the **syllable** as its token. No GPU, no embeddings
that age, no black box. Runs on any hardware and is inspectable with the naked eye.

> **The name has layers.** `RAG` + `Ragnarök` + `Rock` (rock'n'roll) + **Rock = stone**: a RAG on a
> **solid foundation**. Built on rock, not on sand (Matthew 7:24-27) — while "SOTA" RAGs need GPUs and
> embeddings that go stale, this one stands on modest hardware — a Raspberry Pi, for example.

---

## Why it exists

Most RAGs depend on a GPU, gigabyte-sized embedding models and heavy vector databases — which
**shuts out anyone without good hardware**. RAGnaRock goes the opposite way:

- **Runs anywhere** — it's a small binary (Rust, ~2 MB). CPU + RAM, no GPU. Works on modest hardware
  (**for example**, a Raspberry Pi 3 or a 2012 Optiplex) and also on Mac, Windows and Linux.
- **Inspectable** — the "embedding" is a histogram of syllables in readable JSON; you can see the vector,
  the `idf`, the search tree. No magic.
- **Teachable** — it ships with a didactic track (`logic_path/`) that rebuilds every RAG principle,
  step by step, from zero.
- **Doesn't age** — no embeddings to reprocess when the model changes; ingestion is incremental.

> **Mission:** popularize the RAG concept — accessible, transparent, runnable by anyone.
> The goal is **not** to be SOTA; it's to raise the floor and to be teachable.

---

## How it works (in 30 seconds)

1. **Token = syllable.** Text is split into syllables (`ca`, `sa`, `tra`, `gan`...).
2. **Embedding = sparse histogram** (bag of syllables) — the syllable counts of each chunk.
3. **Search, stage 1 (recall):** **tf-idf** cosine between the query and the chunks.
4. **Search, stage 2 (rerank):** *matched filter* with phonetic soundex — promotes chunks where the
   query words appear **in sequence and close together**, not merely present.
5. **Optional — query expansion:** expands the query with synonyms (dictionary → cache → AI) before
   searching, gaining recall without hurting latency.

Everything is readable text: you can open a base's JSON and understand exactly what was indexed.

---

## Architecture

The same library (`sylkit`) exists in three incarnations that produce **field-by-field identical** results:

| Component | What it is | Status |
|---|---|---|
| `python_concept/` | Reference PoC in Python (stdlib only) — the algorithm's "ground truth". | ✅ done |
| `rust_concept/`   | **Frozen** Rust port, validated identical to Python (acts as a test). | ✅ done |
| `ragd/`           | **Production daemon** (Rust): holds N bases in memory, search/ingestion via **HTTP JSON API**. This is where development happens. | ✅ done |
| **ValHalla**      | Web console for `ragd` (overview, search, ingestion, performance, drivers, logs). | ✅ done |
| `nidhoggd/`       | **Níðhöggr** — the **intelligence** layer (experimental): the *worm* that digests knowledge. See below. | 🚧 partial |
| **MCP**           | Shell that plugs RAGnaRock into AI agents as a tool (opencode, Claude, etc.). | 🚧 partial |
| `drivers/`        | Language drivers — tokenize **source code** (syllables + per-language keywords). | ✅ done |
| `thesaurus/`      | Multilingual + cross-lingual dictionaries (for query expansion). | ✅ done |
| `logic_path/`     | Didactic track **00 → 10** (frozen memorial) teaching every RAG principle. | ✅ done |

> 📐 **Full specification** (the three daemons in detail, JSON contracts, memory/disk strategies,
> concurrency, failure modes and roadmap): **[`ARCHITECTURE.md`](ARCHITECTURE.md)** *(in Portuguese)*.

---

## 🐉 Nidhogg — the intelligence layer (experimental)

In Norse myth, **Níðhöggr** is the serpent that gnaws the roots of Yggdrasil. In RAGnaRock, `nidhoggd`
is a (benevolent) *worm* that **digests the knowledge** of the collections and distills it into insight
that **survives the deletion of the collection**. It's the project's **analytical layer**: **autonomous**
(`ragd` never consumes it; the reader is the human), with four levels — from the **AI-free** index
(level 0, where the RAG self-organizes) to the propositive **living document** (level 3, with AI).

> **Status: 🚧 partial.** Skeleton ready (separate process on port **11497**, API, level/cadence dials);
> the per-level intelligence (1–3, **opt-in** and AI-powered) is under development. The full design
> (level hierarchy, versioned artifacts, acceptance gate, AI graph, per-level prompt and open questions)
> lives in [`ARCHITECTURE.md` §5](ARCHITECTURE.md#5-nidhoggd--níðhöggr--camada-de-inteligência-11497-parcial)
> *(in Portuguese)*.

---

## Build & run

```bash
# Production daemon (default port 11499). Run from the repo root to auto-load the bases in ragfiles/.
cd ragd && cargo build --release
./target/release/ragd

# Ingest a raw file and search (Rust PoC)
cd rust_concept && cargo build --release
./rust_concept/target/release/embed_gen my_corpus.txt --chunk 2048
./rust_concept/target/release/search_rag my_corpus-tokenized.json "my query" -k 5

# Python PoC (stdlib only)
python3 python_concept/embed_gen.py my_corpus.txt --chunk 2048
python3 python_concept/search_rag.py my_corpus-tokenized.json "my query" -k 5
```

> Project convention: **every script/binary invoked with no arguments prints help** — it never runs
> with silent defaults.

---

## Daemon API (HTTP JSON)

`GET /health · /bases · /collections · /drivers · /interpret` ·
`POST /ingest · /ingest_file · /ingest_upload · /search · /search_expand · /chunk` ·
`DELETE /bases/{name}`.

- Bases are organized as `collection/name`; search is **scatter-gather** with wildcards
  (`"sd*"`, `"*"`) and merge by relevance.
- Each hit carries `collection, base, corpus` (file name), `path`, `chunk`, `matchpoint`,
  `snippet`, etc. — so the AI goes **straight to the file**.

Formal contract for the 3 APIs (ragd, ValHalla, nidhoggd): **[`JSONCONTRACT.md`](JSONCONTRACT.md)**.
Runnable `curl` examples: **[`ragd/json_samples/`](ragd/json_samples/)**.

---

## Status

**New** project (born May 2026), under active development — treat it as **alpha**. The core works
end to end: syllabic search (recall + rerank), daemon with API, ValHalla console, code drivers,
query expansion, incremental ingestion and MCP integration.

**On the radar:** repository ingestion with per-file update, more importers (PDF/DOCX/XLSX),
concurrency (N searches in parallel) and a double-click Windows build. There is **no** formal test
suite or CI yet — fidelity is checked manually (generate on one side, read/search on the other; the
fields match exactly).

---

## License

Code under **[MIT](LICENSE.md)** — use, copy, modify and distribute freely.

> ⚠️ **Third-party data:** any bundled dictionaries/thesauri may derive from sources with their own
> licenses (e.g. the Portuguese thesaurus). Check the data license before redistributing — MIT covers
> the **code**, not necessarily the seed data.

---

## Author

**Alexandre Pereira** — personal, open-source project.

*Built on the rock. 🤘*
