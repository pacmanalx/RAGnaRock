# RAGnaRock as an MCP server — reference example

A thin **MCP** (Model Context Protocol) server that plugs a running RAGnaRock daemon (`ragd`) into any
MCP-capable agent as native tools. It's a stdio↔HTTP bridge — **no search logic lives here**, `ragd`
does all the work. Copy [`mcp_ragnarock_server.py`](mcp_ragnarock_server.py), point it at your `ragd`,
and wire it into your client below.

## Tools

| tool | what it does | ragd endpoint |
|---|---|---|
| `ragnarock_search` | syllabic search (tf-idf recall + phonetic rerank); compact hits | `POST /search` · `/search_expand` |
| `ragnarock_chunk` | fetch whole chunk(s) by id, with `before`/`after` context | `POST /chunk` |
| `ragnarock_list` | list loaded bases + collection summary | `GET /bases` · `/collections` |
| `ragnarock_ingest` | ingest a raw **file** | `POST /ingest_file` |
| `ragnarock_ingest_text` | ingest raw **text** (overwrite or `append`) | `POST /ingest_upload` |

## Prerequisites

1. **A running `ragd`** with some bases loaded:
   ```bash
   cd ragd && cargo build --release && ./target/release/ragd   # run from repo root to auto-load ragfiles/
   curl http://127.0.0.1:11499/health                          # {"status":"ok",...}
   ```
2. **The Python MCP SDK**:
   ```bash
   pip install mcp        # see requirements.txt
   ```
3. Note the **absolute path** to `mcp_ragnarock_server.py` — every client config needs it.

The server reads two env vars: `RAGD_URL` (default `http://127.0.0.1:11499`) and `RAGD_TIMEOUT`
(seconds, default `30`). Set `RAGD_URL` if `ragd` runs on another host/port.

## Wiring it into a client

All three clients launch the **same** stdio server; only the config file/shape differs. Replace
`/ABS/PATH/TO` with your real path.

### opencode — `~/.config/opencode/opencode.json`

```json
{
  "mcp": {
    "ragnarock": {
      "type": "local",
      "command": ["python3", "/ABS/PATH/TO/mcp_ragnarock_server.py"],
      "environment": { "RAGD_URL": "http://127.0.0.1:11499" },
      "enabled": true
    }
  }
}
```

### Claude Code

One-liner (project- or user-scoped):

```bash
claude mcp add ragnarock --env RAGD_URL=http://127.0.0.1:11499 \
  -- python3 /ABS/PATH/TO/mcp_ragnarock_server.py
```

…or equivalently in `.mcp.json` (project) / your Claude config:

```json
{
  "mcpServers": {
    "ragnarock": {
      "command": "python3",
      "args": ["/ABS/PATH/TO/mcp_ragnarock_server.py"],
      "env": { "RAGD_URL": "http://127.0.0.1:11499" }
    }
  }
}
```

### Kimi for Code (Kimi CLI)

Kimi for Code consumes the **standard MCP `mcpServers` shape** — the same server entry as Claude Code:

```json
{
  "mcpServers": {
    "ragnarock": {
      "command": "python3",
      "args": ["/ABS/PATH/TO/mcp_ragnarock_server.py"],
      "env": { "RAGD_URL": "http://127.0.0.1:11499" }
    }
  }
}
```

Place it in Kimi's MCP settings file (check the Kimi CLI docs for the exact location on your install).
The server itself is client-agnostic, so the entry is identical to the one above.

## Searching well (read this — it's not optional)

RAGnaRock tokenizes by the **Portuguese syllable**. To get good hits:

- **Write queries in Portuguese.** English words become out-of-vocabulary syllables that pollute the
  vector and sink the match. Don't pad a query with translations.
- **Scope the `collection`** when you know the domain — an unscoped search spans everything and picks
  up noise. Use `ragnarock_list` to see what's loaded.
- **Use distinctive terms** (names, identifiers), not generic words.
- Inspect `query_syllables` in the result: a weird syllable means your query carries noise — rephrase.
- Keep `expand=False` (default) for precise **lookup**; only set `expand=True` for vague/conceptual
  questions (synonym expansion can dominate and pollute exact lookups).

## Troubleshooting

- **`"error": "ragd unreachable"`** — `ragd` isn't running or `RAGD_URL` is wrong. Check
  `curl $RAGD_URL/health`.
- **Tools don't appear in the client** — verify the absolute path, that `python3` resolves the `mcp`
  package (use the same interpreter/venv where you `pip install`ed it), and restart the client.
- **Stalls on a `*.local` host** — the IPv4-only resolver block at the top of the server handles the
  common macOS mDNS case; remove it if you don't use `*.local` names.
- **Inspect manually** — the MCP Inspector (`npx @modelcontextprotocol/inspector python3
  /ABS/PATH/TO/mcp_ragnarock_server.py`) lists the tools and lets you call them by hand.
