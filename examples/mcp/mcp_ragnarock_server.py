#!/usr/bin/env python3
"""
mcp_ragnarock_server — a reference MCP server that exposes a running RAGnaRock
daemon (`ragd`) as native tools to any MCP client (opencode, Claude Code, Kimi
for Code, …).

This shell does ONE thing: translate MCP stdio calls into RAGnaRock's HTTP JSON
API. No search logic lives here — `ragd` does all the work (syllable tokenizer,
tf-idf cosine recall, phonetic matched-filter rerank). It's a thin, dependency-
light bridge you can copy and adapt.

Tools exposed:
    ragnarock_search(query, base, collection, k, rerank, recall_n, phonetic, expand)
    ragnarock_chunk(base, id, collection, before, after)
    ragnarock_list(match, collection)
    ragnarock_ingest(path, collection, name, chunk, driver)
    ragnarock_ingest_text(text, name, collection, append, chunk, filename)

Backend:
    Point RAGD_URL at your ragd. Default: http://127.0.0.1:11499
    Start ragd from the repo root so it auto-loads ragfiles/:
        cd ragd && cargo build --release && ../target/release/ragd   # or ./target/release/ragd

Env vars:
    RAGD_URL      — base URL of the ragd JSON API (default http://127.0.0.1:11499)
    RAGD_TIMEOUT  — request timeout in seconds (default 30)

Requires:  pip install mcp   (the official Python MCP SDK; provides fastmcp)
"""
import json
import os
import socket
import urllib.parse
import urllib.request
import urllib.error

from mcp.server.fastmcp import FastMCP

# On macOS, mDNS (*.local) hosts resolve AAAA (IPv6) before A (IPv4), which can
# stall urllib if RAGD_URL points at a *.local name. Prefer IPv4 to avoid it.
# Harmless on other platforms; drop this block if you don't use *.local hosts.
_orig_getaddrinfo = socket.getaddrinfo
def _ipv4_only_getaddrinfo(*args, **kwargs):
    return [r for r in _orig_getaddrinfo(*args, **kwargs) if r[0] == socket.AF_INET]
socket.getaddrinfo = _ipv4_only_getaddrinfo

RAGD_URL = os.environ.get("RAGD_URL", "http://127.0.0.1:11499").rstrip("/")
TIMEOUT = int(os.environ.get("RAGD_TIMEOUT", "30"))

mcp = FastMCP("ragnarock")


def _post_json(path: str, payload: dict) -> dict:
    data = json.dumps(payload).encode("utf-8")
    req = urllib.request.Request(
        RAGD_URL + path, data=data,
        headers={"Content-Type": "application/json"},
        method="POST",
    )
    with urllib.request.urlopen(req, timeout=TIMEOUT) as resp:
        return json.loads(resp.read())


def _get_json(path: str) -> dict:
    req = urllib.request.Request(RAGD_URL + path, method="GET")
    with urllib.request.urlopen(req, timeout=TIMEOUT) as resp:
        return json.loads(resp.read())


def _err(e: Exception) -> str:
    """Readable error instead of a stack trace — tells 'ragd is down' apart from a query error."""
    if isinstance(e, urllib.error.URLError) and not isinstance(e, urllib.error.HTTPError):
        return json.dumps({
            "error": "ragd unreachable",
            "ragd_url": RAGD_URL,
            "detail": str(getattr(e, "reason", e)),
            "hint": "Start ragd (from the repo root): ./target/release/ragd",
        }, ensure_ascii=False)
    if isinstance(e, urllib.error.HTTPError):
        body = ""
        try:
            body = e.read().decode("utf-8", "replace")
        except Exception:
            pass
        return json.dumps({"error": f"HTTP {e.code}", "ragd_url": RAGD_URL,
                           "detail": body or e.reason}, ensure_ascii=False)
    return json.dumps({"error": type(e).__name__, "detail": str(e)}, ensure_ascii=False)


@mcp.tool()
def ragnarock_search(query: str, base: str = "*", collection: str = "",
                     k: int = 5, rerank: bool = True, recall_n: int = 20,
                     phonetic: bool = False, expand: bool = False) -> str:
    """Syllabic semantic search over RAGnaRock (tf-idf cosine recall + phonetic matched filter).
    Defaults to PRECISE search (no expansion) — best for LOOKUP (finding a file/identifier/name).

    HOW TO SEARCH WELL (the tokenizer is the PORTUGUESE syllable — follow this or results degrade):
      - Write the query in **Portuguese**. Do NOT translate or add English terms: words like
        "evaluation/bulletin" become OOV syllables (e-va-lua-tion...) that pollute the vector and
        sink the match. Prefer the Portuguese phrasing of what you're looking for.
      - **Scope the `collection`** whenever you know the domain. Without a scope the search spans
        every collection (code, docs, session memory...) and noise creeps in. Use `ragnarock_list`
        to see what's loaded.
      - Use **distinctive terms** (domain names, identifiers), not generic words (which are common
        syllables with low idf and won't discriminate).
      - Check `query_syllables` in the response: a strange syllable means your query has noise
        (English/typo) — rephrase.

    query      — query text (Portuguese, distinctive terms).
    base       — "name" exact, "pref*" prefix, or "*" (all) within the scope.
    collection — "" searches ALL; set it to restrict to one collection.
    k          — number of hits in the final result.
    rerank     — apply the phonetic matched filter (stage 2). Turn off to see raw recall.
    recall_n   — candidates from stage 1 before the rerank.
    phonetic   — use phonetic soundex on the query.
    expand     — False (default): PURE precise search — use for LOOKUP (exact file/identifier/name).
                 True: EXPANDED (synonyms via dictionary→cache→AI) — use ONLY for VAGUE/CONCEPTUAL
                 questions; common synonyms can dominate and pollute precise lookups. Falls back to
                 plain /search if expansion fails.

    Returns COMPACT {query, query_syllables, searched_bases (count), hits:[{collection, base, rank,
    matchpoint, mf, span, cos, chunk, start, snippet}]}. (ragd's bulky per-base `searched`/`scope`
    diagnostics are dropped here so they don't blow the agent's context.)
    """
    payload = {"base": base, "query": query, "k": k, "rerank": rerank,
               "recall_n": recall_n, "phonetic": phonetic}
    if collection:
        payload["collection"] = collection
    try:
        # expand=True uses /search_expand (dictionary→cache→AI cascade: expands the query by
        # synonyms before searching = better recall). Falls back to plain /search if expansion
        # errors (e.g. dict+cache miss with no AI provider configured → HTTP 400).
        if expand:
            try:
                data = _post_json("/search_expand", payload)
            except urllib.error.HTTPError:
                data = _post_json("/search", payload)
        else:
            data = _post_json("/search", payload)
        # Compact the response: `hits` is what matters (a few KB). ragd also returns `searched`
        # (one entry per base in scope — can be hundreds) and `scope` (the base list), which can
        # reach ~100s of KB on a large scope and overflow the agent context. Keep hits + syllables
        # + a base count + the small, useful expansion metadata (which synonyms were used).
        if isinstance(data, dict) and "hits" in data:
            compact = {
                "query": data.get("query"),
                "query_syllables": data.get("query_syllables"),
                "searched_bases": len(data.get("searched", [])),
                "hits": data.get("hits", []),
            }
            for f in ("expansions", "source", "dropped", "absent"):
                if f in data:
                    compact[f] = data[f]
            data = compact
        return json.dumps(data, ensure_ascii=False)
    except Exception as e:
        return _err(e)


@mcp.tool()
def ragnarock_chunk(base: str, id: int, collection: str = "default",
                    before: int = 0, after: int = 0) -> str:
    """Fetch the WHOLE chunk(s) of a base by id — to read the full text/code behind a hit (the
    search `snippet` is only an excerpt). Use the `chunk` (id), `base` and `collection` that came
    in the ragnarock_search hit. Don't curl or look for the file on disk: ragd may be remote, and
    this tool is the correct way to read the content.

    base       — the hit's base name.
    id         — the chunk id (the hit's `chunk` field).
    collection — the hit's collection. Default "default".
    before     — number of chunks BEFORE to include for context. Default 0.
    after      — number of chunks AFTER to include for context. Default 0.

    Returns {collection, base, corpus (file name), n_chunks, chunks:[{id, start, len, text}]}.
    """
    payload = {"base": base, "collection": collection, "id": id,
               "before": before, "after": after}
    try:
        return json.dumps(_post_json("/chunk", payload), ensure_ascii=False)
    except Exception as e:
        return _err(e)


@mcp.tool()
def ragnarock_list(match: str = "*", collection: str = "") -> str:
    """List the bases loaded in RAGnaRock plus a summary of collections.

    match      — wildcard on the base name ("sd*", "*"). Default "*".
    collection — "" lists all; "X" restricts to that collection.

    Returns {bases:{collection,match,count,bases:[...]}, collections:{count,total_bases,
    collections:[...]}}.
    """
    q = {}
    if collection:
        q["collection"] = collection
    if match and match != "*":
        q["match"] = match
    qs = ("?" + urllib.parse.urlencode(q)) if q else ""
    try:
        out = {"bases": _get_json("/bases" + qs)}
        out["collections"] = _get_json("/collections")
        return json.dumps(out, ensure_ascii=False)
    except Exception as e:
        return _err(e)


@mcp.tool()
def ragnarock_ingest(path: str, collection: str = "default", name: str = "",
                     chunk: int = 2048, driver: str = "") -> str:
    """Ingest a raw file into RAGnaRock (syllable-tokenize and index it).

    path       — file path (relative to ragd's working dir, or absolute).
    collection — target collection (default "default"). Stored under ragfiles/<collection>/.
    name       — base name; empty derives it from the path.
    chunk      — chunk size in bytes (default 2048).
    driver     — language driver for source code (e.g. "Python"); empty = auto by extension.

    Returns {ok, collection, name, corpus, n_chunks, bases, saved_to}.
    """
    payload = {"path": path, "collection": collection, "chunk": chunk}
    if name:
        payload["name"] = name
    if driver:
        payload["driver"] = driver
    try:
        return json.dumps(_post_json("/ingest_file", payload), ensure_ascii=False)
    except Exception as e:
        return _err(e)


@mcp.tool()
def ragnarock_ingest_text(text: str, name: str, collection: str = "default",
                          append: bool = False, chunk: int = 2048,
                          filename: str = "") -> str:
    """Ingest RAW TEXT into RAGnaRock (no file) — tokenized and indexed server-side.

    TWO MODES:
    - append=False (default) → OVERWRITE: re-ingesting the same (collection, name) replaces the
      whole base. Use for an occasional full re-sync.
    - append=True → INCREMENTAL: accumulates the text into the existing base without losing what
      it had (idf/norm recomputed globally). Creates the base if it doesn't exist. This is the
      mode for a rolling memory: each turn/flush sends only the NEW content under a stable `name`.

    text       — content to index (new turn in append mode; full text in overwrite).
    name       — base name (keep it stable to append to the same base).
    collection — target collection (default "default").
    append     — True accumulates; False (default) overwrites.
    chunk      — chunk size in bytes (default 2048; ignored on append, inherits the base's).
    filename   — corpus label / extension for driver selection (empty → "<name>.txt", PT tokenizer).

    Under the hood: POST /ingest_upload (raw body, ?append=). Returns
    {ok, collection, name, n_chunks, bytes, appended, bases, saved_to, via:"raw"}.
    """
    fn = filename or f"{name}.txt"
    qs = urllib.parse.urlencode({"collection": collection, "filename": fn,
                                 "name": name, "chunk": chunk,
                                 "append": "true" if append else "false"})
    try:
        req = urllib.request.Request(
            RAGD_URL + "/ingest_upload?" + qs,
            data=text.encode("utf-8"),
            headers={"Content-Type": "text/plain; charset=utf-8"},
            method="POST",
        )
        with urllib.request.urlopen(req, timeout=TIMEOUT) as resp:
            return json.dumps(json.loads(resp.read()), ensure_ascii=False)
    except Exception as e:
        return _err(e)


if __name__ == "__main__":
    mcp.run()
