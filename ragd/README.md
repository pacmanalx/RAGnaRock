# ragd — o daemon RAGnaRock 🤘

Server HTTP que segura **N bases RAG em memória** e atende **busca/ingestão via JSON**.
Reusa uma **cópia própria da `sylkit`** (em `src/`) — o `rust_concept` fica congelado como
PoC; aqui a lib pode evoluir livremente.

## Build & run

```bash
cd ragd
cargo build --release
# sobe na porta 11499, pré-carregando uma base:
./target/release/ragd --preload sda=../ragfiles/sda-tokenized.json
# (rodando da raiz do projeto, o path fica ragfiles/sda-tokenized.json)
```

Opções: `--port N` (default **11499**), `--drivers-dir <path>` (default `drivers`),
`--ragfiles-dir <path>` (default `ragfiles` — onde `/ingest_file` grava os JSON tokenizados),
`--max-upload <bytes>` (default 1 GB — limite do `POST /ingest_upload`),
`--preload nome=caminho.json` (repetível), `--help`.

## Rotas (HTTP JSON)

| método | rota | body / query | resposta |
|---|---|---|---|
| `GET` | `/health` | — | `{status, bases, drivers}` |
| `GET` | `/bases` | `?collection=X` (default todas) e/ou `?match=sd*` (wildcard no nome) | `{collection, match, count, bases:[{collection,name,n_chunks,vocab_size,corpus,generator,has_text}]}` |
| `GET` | `/collections` | — | `{count, total_bases, collections:[{collection, bases}]}` |
| `GET` | `/drivers` | `?match=ASP*` (wildcard, opcional → default todos) | `{drivers_dir, match, count, drivers:[{name,language,description,extensions,syllables,keywords,vocab_size,header}]}` |
| `GET` | `/interpret` | `?file=foo.py` **ou** `?ext=.py` | `{file?, extension, drivers_scanned, matched, driver, language, fallback?}` |
| `POST` | `/ingest` | `{name, collection?, path}` (JSON tokenizado) **ou** `{name, collection?, data:<base>}` **ou** `{name, collection?, path, raw:true, chunk?, driver?, …}` (bruto — grava em `ragfiles/<collection>/`) | `{ok, collection, name, n_chunks, bases, raw, saved_to?}` |
| `POST` | `/ingest_file` | `{path, collection?, name?, chunk?, driver?, …}` — sem `name` deriva do path. **Grava em `ragfiles/<collection>/<name>-tokenized.json`** | `{ok, collection, name, corpus, n_chunks, bases, saved_to}` |
| `POST` | `/ingest_upload` | multipart (campos `file` + `collection?`/`name?`/…) ‖ raw body + query (`?collection=&filename=&name=&chunk=`). Limite via `--max-upload` | `{ok, collection, name, filename, corpus, n_chunks, bytes, bases, saved_to, via}` |
| `POST` | `/search` | `{base, query, collection?, k?, rerank?, recall_n?, phonetic?}` — sem `collection`, busca em **todas**; com `collection:"X"`, restringe; `base` aceita `nome`/`pref*`/`*` dentro do escopo | `{query, query_syllables, scope:["coll/base",…], searched, hits:[{collection,base,…}]}` |
| `POST` | `/chunk` | `{base, collection?, id, before?, after?}` ou `{base, collection?, ids:[…]}` — `collection` default `"default"` | `{collection, base, chunks:[…]}` |
| `DELETE` | `/bases/{nome}` | `?collection=X` (default `"default"`) | `{ok, removed, collection, bases}` |

## Exemplos

```bash
H=http://localhost:11499
curl -s "$H/bases"                                                        # todas as bases (todas as coleções)
curl -s "$H/bases?collection=innova"                                      # só da coleção 'innova'
curl -s "$H/bases?collection=innova&match=foo*"                           # coleção + wildcard no nome
curl -s "$H/collections"                                                  # quais coleções existem + contagem
curl -s "$H/drivers"                                                      # 33 drivers de linguagem instalados
curl -s "$H/drivers?match=ASP*"                                           # ASPClassic, ASPRazor, ASPWebForms
curl -s "$H/interpret?file=foo.py"                                        # router por ext -> Python
curl -s -X POST $H/search -d '{"base":"sda","query":"Frodo Bolseiro","k":3}'                        # busca global (todas as coleções)
curl -s -X POST $H/search -d '{"collection":"innova","base":"*","query":"caixa","k":5}'             # restrita à coleção 'innova'
curl -s -X POST $H/search -d '{"base":"*","query":"anel","k":5}'                                    # global; base wildcard pega tudo
curl -s -X POST $H/chunk  -d '{"base":"sda","id":87,"before":1,"after":1}'  # contexto (86→87→88)
curl -s -X POST $H/ingest -d '{"name":"sda","path":"ragfiles/default/sda-tokenized.json"}'                                  # JSON tokenizado
curl -s -X POST $H/ingest_file -d '{"path":"logic_path/03_histogram.py","collection":"innova"}'                              # arquivo bruto, coleção custom
curl -s -X POST -F "file=@local.py" -F "name=foo" -F "collection=eduxe" $H/ingest_upload                                     # upload p/ coleção 'eduxe'
curl -s -X POST --data-binary @local.py "$H/ingest_upload?filename=local.py&name=foo&collection=eduxe"                       # idem, raw body
curl -s -X DELETE "$H/bases/sda?collection=default"
```

**Wildcard na base** (`/search` e `/bases?match=`): `"sda"` exata, `"sd*"` prefixo, `"*"` todas.
No `/search` é **scatter-gather** — busca em cada base que casa e faz merge dos hits por
`matchpoint` global; cada hit traz `base, rank, matchpoint, mf, span, cos, chunk, start, snippet`.
O `/chunk` traz o(s) chunk(s) inteiro(s) por id (com `before`/`after`) pra montar contexto.
Os números batem com o `search_rag` da PoC (recall cosseno + rerank matched filter).

📄 **Contrato JSON completo + exemplos prontos pra `curl -d @`:** veja `json_samples/`
(README detalhado + `ingest/search/chunk*.json` + `*.example.json`).

## Próximo passo

Casca **MCP** (`rag_search` / `rag_ingest` / `rag_list`) falando HTTP com este daemon —
aí o Claude (e qualquer agente) busca nas bases carregadas.

## Estrutura

```
ragd/
├── Cargo.toml          (deps: serde, serde_json[preserve_order], tiny_http)
├── json_samples/       # contrato JSON: README + exemplos de request/response
└── src/
    ├── main.rs         # o server (rotas, JSON)
    ├── rag.rs          # RagBase + recall + rerank + snippet (evoluível)
    └── tokenizer.rs · vocab.rs · vector.rs · chunk.rs · index.rs   (cópia própria da sylkit)
```
