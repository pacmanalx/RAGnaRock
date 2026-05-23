# RAGnaRock — o contrato JSON da API

Como conversar com o `ragd` (daemon na porta **11499**) por HTTP/JSON: **ingestão** de
bases, **pesquisa** e **recuperação de chunks** (pra montar contexto). Todos os exemplos
têm um `.json` ao lado pra usar com `curl -d @`.

> Rode o daemon **e** os `curl` a partir da **raiz do projeto** (`ragnarock/`): o `path`
> de ingestão é resolvido pelo cwd do daemon, e os `@ragd/json_samples/…` pelo cwd do curl.
> ```bash
> ./ragd/target/release/ragd --preload default/sda=ragfiles/default/sda-tokenized.json
> ```
> `H=http://localhost:11499` nos exemplos abaixo.

## Coleções (namespaces)

Toda base pertence a uma **coleção**. Sem `collection` nos POSTs → cai em `"default"`.
No filesystem: `ragfiles/<collection>/<name>-tokenized.json` (subpasta por coleção,
criada automaticamente). Usa pra **escopo** de busca, organização e DELETE seletivo.

- `POST /ingest_file {"path":"...","collection":"innova"}` → base vai pra `innova/`
- `POST /search {"collection":"innova","base":"*","query":"..."}` → busca só em `innova`
- `POST /search {"base":"*","query":"..."}` (sem `collection`) → busca em **todas** as coleções
- `DELETE /bases/foo?collection=innova` → remove só `innova/foo`
- `GET /collections` → lista coleções com contagem
- `GET /bases?collection=innova` → lista bases dessa coleção
- `--preload innova/foo=ragfiles/innova/foo-tokenized.json` → preload com coleção (sem `/` cai em `default`)

Hits de `/search` agora trazem `collection` em cada um — pra você saber de onde veio o resultado.

---

## 1. Ingestão — `POST /ingest`

Sobe uma base RAG pra memória do daemon, sob um **nome**. Três modos:

### 1a. Por caminho de JSON tokenizado (recomendado) — `ingest_by_path.json`
```json
{ "name": "sda", "path": "ragfiles/sda-tokenized.json" }
```
```bash
curl -s -X POST $H/ingest -d @ragd/json_samples/ingest_by_path.json
```

### 1b. Por dados embutidos — `ingest_by_data.json`
A base inteira vai dentro de `data` (mesmo schema do `embed_gen`: `meta`+`idf`+`chunks`).
```bash
curl -s -X POST $H/ingest -d @ragd/json_samples/ingest_by_data.json
```

### 1c. Arquivo BRUTO (`raw:true`) — `ingest_raw.json`
O daemon tokeniza dentro de si **e grava o JSON em `ragfiles/<name>-tokenized.json`**
(persistente — sobrevive a reinício do daemon via `--preload nome=ragfiles/<name>-tokenized.json`).
Escolhe driver pela extensão (auto, com fallback PTBR), ou aceita `driver` explícito. Os
campos `chunk`, `with_text`, `max_chunks` são opcionais.
```json
{ "name": "py01", "path": "logic_path/01_tokenizer_zipf.py", "raw": true, "chunk": 2048 }
```
```bash
curl -s -X POST $H/ingest -d @ragd/json_samples/ingest_raw.json
```
| campo | tipo | default | o que é |
|---|---|---|---|
| `raw` | bool | false | flag que ativa o modo arquivo bruto |
| `driver` | string | auto | nome do `.drv` (ex `tokens_Python_PTBR.drv`); omitido = auto pela ext |
| `chunk` | int | 2048 | tamanho do chunk em chars |
| `with_text` | bool | true | guarda o texto do chunk no JSON |
| `max_chunks` | int | 0 | limita chunks (0 = todos) |

**Resposta:** `{ "ok": true, "name": "sda", "n_chunks": 1489, "bases": 1, "raw": false }`
(no modo `raw:true` vem também `saved_to: "/abs/path/ragfiles/<name>-tokenized.json"`)

---

## 1bis. Ingestão de arquivo bruto — `POST /ingest_file`

Atalho dedicado pra arquivos brutos (mesma máquina interna do `/ingest` com `raw:true`).
Aceita os mesmos opcionais (`chunk`, `driver`, `with_text`, `max_chunks`). Sem `name`,
deriva do path: `logic_path/03_histogram.py` → `logic_path__03_histogram_py`.

### Mínimo — `ingest_file_auto.json`
```json
{ "path": "logic_path/03_histogram.py" }
```
```bash
curl -s -X POST $H/ingest_file -d @ragd/json_samples/ingest_file_auto.json
```

### Completo — `ingest_file_full.json`
```json
{ "path": "logic_path/03_histogram.py", "name": "hist_py", "chunk": 1024,
  "driver": "tokens_Python_PTBR.drv", "with_text": true }
```

**Resposta:** `{ "ok": true, "name": "hist_py", "corpus": "03_histogram.py", "n_chunks": 3, "bases": N, "saved_to": "/abs/path/ragfiles/hist_py-tokenized.json" }`

> **Persistência**: o JSON tokenizado é sempre gravado em `ragfiles/<name>-tokenized.json`
> (configurável via `--ragfiles-dir`). Pra recarregar tudo após reiniciar o daemon, use
> `--preload nome=ragfiles/nome-tokenized.json` (repetível).
>
> **Sobre o driver escolhido**: a base resultante tem `meta.tokens_file`, `meta.language`
> e `meta.matched_by_ext` indicando exatamente qual driver foi usado e se veio por match
> de extensão ou fallback. O arquivo de origem fica em `meta.source_file` (caminho absoluto).

---

## 1ter. Upload de arquivo remoto — `POST /ingest_upload`

Use quando o arquivo **não está na máquina do daemon** (cliente em outro host). O daemon
recebe o conteúdo via HTTP, tokeniza e grava em `ragfiles/<name>-tokenized.json` (mesma
persistência do `/ingest_file`). **Limite**: `--max-upload` (default 1 GB) — acima disso
retorna HTTP 413.

Dois modos via `Content-Type`:

### (a) multipart/form-data
Padrão web — campo `file` carrega o arquivo, demais campos são strings de metadados
(`name`, `filename`, `chunk`, `driver`, `with_text`, `max_chunks`).
```bash
curl -s -X POST \
  -F "file=@logic_path/03_histogram.py" \
  -F "name=hist_uploaded" \
  -F "chunk=1024" \
  $H/ingest_upload
```

### (b) raw body (qualquer outro Content-Type)
Body inteiro é o arquivo; metadados via query string (`?filename=...&name=...&chunk=...`).
```bash
curl -s -X POST --data-binary @logic_path/03_histogram.py \
  "$H/ingest_upload?filename=03_histogram.py&name=hist_uploaded&chunk=1024" \
  -H "Content-Type: application/octet-stream"
```

**Resposta** (`ingest_upload_multipart_response.example.json` / `ingest_upload_raw_response.example.json`):
```json
{
  "ok": true, "name": "hist_uploaded", "filename": "03_histogram.py",
  "corpus": "03_histogram.py", "n_chunks": 3, "bytes": 2769,
  "bases": 1, "saved_to": "/abs/path/ragfiles/hist_uploaded-tokenized.json",
  "via": "multipart"
}
```

| campo | tipo | default | o que é |
|---|---|---|---|
| `file` (multipart) / body raw | bytes | obrigatório | conteúdo UTF-8 do arquivo |
| `filename` | string | `upload.bin` | nome lógico — usado pra escolher driver pela extensão |
| `name` | string | derivado de `filename` | nome da base |
| `chunk` | int | 2048 | chars por chunk |
| `driver` | string | auto | força um `.drv` (ex `tokens_Python_PTBR.drv`); omitido = auto |
| `with_text` | bool | true | grava texto do chunk no JSON |
| `max_chunks` | int | 0 | limita chunks (0 = todos) |

> **Observação**: o arquivo precisa ser UTF-8 (texto, código, markdown, JSON, etc).
> Binários puros (imagens, executáveis) são rejeitados com HTTP 400 — `meta.source_file`
> fica como `<upload:filename>` pra deixar claro que o original não está em disco.

---

## 2. Pesquisa — `POST /search`

### Request — `search.json` / `search_minimal.json` / `search_wildcard.json`
| campo | tipo | default | o que é |
|---|---|---|---|
| `base` | string | **(obrigatório)** | nome exato, **`pref*`** (prefixo) ou **`*`** (todas) |
| `query` | string | **(obrigatório)** | texto da busca |
| `k` | int | 5 | quantos resultados (no total, após o merge) |
| `rerank` | bool | true | liga o estágio 2 (proximidade de termos); `false` = só recall |
| `recall_n` | int | 20 | candidatos do recall por base que vão pro rerank |
| `phonetic` | bool | false | casa por **SOM** (SOUNDEX): `"Aslan"` acha `"Aslam"`, tolera grafia/erro de digitação |

> **Rerank por proximidade de termos** (estágio 2): ignora monossílabos (stopwords), exige
> co-ocorrência dos termos-chave no chunk e ordena por **cobertura → proximidade (span) →
> cosseno**. O casamento é por **fronteira de palavra** (o termo casa uma palavra inteira,
> não cruza espaços). Com `phonetic:true`, um termo casa também quando **soa igual**
> (mesmo código SOUNDEX) — ótimo pra nomes/grafias variantes. Dica: para busca por
> entidade, passe `recall_n` alto (ex: total de chunks da base).

**Wildcard na base** (scatter-gather): busca em todas as bases que casam, junta os hits
e reordena por `matchpoint` global. Cada hit indica de qual `base` veio.
- `"base": "sda"` → exata
- `"base": "sd*"` → todas que começam com `sd`
- `"base": "*"` → todas

```bash
curl -s -X POST $H/search -d @ragd/json_samples/search.json
curl -s -X POST $H/search -d @ragd/json_samples/search_wildcard.json   # base "sd*"
curl -s -X POST $H/search -d @ragd/json_samples/search_phonetic.json   # casa por som: Aslan->Aslam
curl -s -X POST $H/search -d @ragd/json_samples/search_no_rerank.json
```

### Response (ver `search_response.example.json`)
```json
{
  "query": "Frodo Bolseiro",
  "query_syllables": "fro-do-bol-sei-ro",
  "bases": ["sda"],                       // bases efetivamente buscadas
  "searched": [                           // stats por base (o "scatter")
    { "base": "sda", "n_chunks": 1489, "n_converge": 1451,
      "dims": 4, "oov": 0, "ms_recall": 0.4, "ms_rerank": 6.7 }
  ],
  "hits": [
    {
      "base": "sda",        // de qual base veio
      "rank": 1,
      "matchpoint": 0.80,   // score de ordenação (com rerank); sem rerank = cosseno
      "mf": 1.00,           // matched filter: fração da query contígua (0..1)
      "span": 2,            // proximidade entre as palavras (menor = melhor)
      "cos": 0.2664,        // similaridade cosseno (estágio 1)
      "chunk": 28,          // id do chunk (use no /chunk pra pegar o texto inteiro)
      "start": 57193,       // offset (char) no corpus
      "snippet": "…«Frodo» «Bolseiro»…"
    }
  ]
}
```
Ordenação: **maior `matchpoint` primeiro** (global, entre todas as bases buscadas).

---

## 3. Recuperar chunk(s) — `POST /chunk`

Traz o **chunk inteiro** (texto + metadados) por id — pra montar o **contexto completo**
em torno de um hit (chunk anterior, próximo, ou uma lista qualquer).

### Request — `chunk.json` (janela) / `chunk_ids.json` (lista)
| campo | tipo | default | o que é |
|---|---|---|---|
| `base` | string | **(obrigatório)** | nome exato da base (sem wildcard aqui) |
| `id` | int | — | chunk alvo |
| `before` | int | 0 | quantos chunks **antes** do alvo trazer |
| `after` | int | 0 | quantos chunks **depois** do alvo trazer |
| `ids` | int[] | — | lista explícita de ids (alternativa a `id`) |

```bash
# o chunk 87 + o anterior + o próximo (contexto):
curl -s -X POST $H/chunk -d @ragd/json_samples/chunk.json
# uma lista específica:
curl -s -X POST $H/chunk -d @ragd/json_samples/chunk_ids.json
```

### Response (ver `chunk_response.example.json`)
```json
{
  "base": "sda",
  "chunks": [
    { "id": 86, "start": 175727, "len": 2046, "tokens": 710, "oov": 145, "norm": 12.3, "text": "…" },
    { "id": 87, "start": 177773, "len": 2041, "tokens": 711, "oov": 150, "norm": 11.9, "text": "…" },
    { "id": 88, "start": 179813, "len": 2038, "tokens": 705, "oov": 141, "norm": 12.1, "text": "…" }
  ]
}
```
**Padrão de uso:** `/search` acha o chunk relevante (ex: 87) → `/chunk` com `before/after`
expande o contexto pra alimentar um LLM (o "augmented" do RAG).

---

## 4. Listar / descobrir bases — `GET /bases`

Lista as bases carregadas, com **filtro por wildcard** opcional (`?match=`, mesmo padrão
do `/search`: exato, `pref*` ou `*`). Sem `match`, lista todas.

```bash
curl -s "$H/bases"                # todas
curl -s "$H/bases?match=sd*"      # só as que começam com 'sd'
curl -s "$H/bases?match=livros"   # exata
```
**Resposta:**
```json
{
  "match": "sd*", "count": 3,
  "bases": [
    { "name": "sda", "n_chunks": 1489, "vocab_size": 1956,
      "corpus": "sda.txt", "generator": "embed_gen.py", "has_text": true }
  ]
}
```

## 5. Listar drivers instalados — `GET /drivers`

Lista os drivers `.drv` da pasta `drivers/` (configurável via `--drivers-dir`). Cada driver
é um vocabulário fixo (sílabas + keywords reservadas marcadas com `=`, **Jeito B**) usado
na ingestão para tokenizar um corpus de uma linguagem específica. Cada `.drv` declara no
cabeçalho a **descricao** e as **extensoes** que cobre — base do interpretador (rota 6).
Mesma sintaxe de wildcard do `/bases`: exato, `pref*` ou `*` (default).

```bash
curl -s "$H/drivers"               # todos (default)
curl -s "$H/drivers?match=ASP*"    # ASPClassic, ASPRazor, ASPWebForms
curl -s "$H/drivers?match=Python"  # exato
```

**Resposta** (ver `drivers_response.example.json`):
```json
{
  "drivers_dir": "/Users/.../ragnarock/drivers",
  "match": "Python", "count": 1,
  "drivers": [
    {
      "name": "tokens_Python_PTBR.drv",
      "language": "Python",
      "description": "codigo fonte Python (2.7/3.x, type hints, async/await, match)",
      "extensions": [".py", ".pyw", ".pyi", ".pyx"],
      "syllables": 3156, "keywords": 49, "vocab_size": 3205,
      "header": "RAGnaRock driver: Python"
    }
  ]
}
```

| campo | tipo | o que é |
|---|---|---|
| `name` | string | nome do arquivo `.drv` |
| `language` | string | identificador da linguagem (extraído de `tokens_<Lang>_PTBR.drv`) |
| `description` | string | descrição curta (vem da linha `# descricao:` do `.drv`) |
| `extensions` | string[] | extensões cobertas pelo driver (linha `# extensoes:`) |
| `syllables` | int | sílabas base (vindas do SourceCode) |
| `keywords` | int | keywords reservadas marcadas com `=` (Jeito B) |
| `vocab_size` | int | sílabas + keywords (= dimensões totais do embedding) |
| `header` | string | primeira linha `# ...` do `.drv` (cabeçalho descritivo) |

## 6. Interpretador (router por extensão) — `GET /interpret`

Dado um caminho/arquivo, decide **qual driver usar** com base na extensão. Lê os campos
`# extensoes:` de cada `.drv` e monta o mapa `ext → driver`. Sem match, devolve fallback
**PTBR** (silabário base do português) — nunca falha.

```bash
curl -s "$H/interpret?file=foo.py"               # match: Python
curl -s "$H/interpret?file=relatorio.docx"       # match: PTBR
curl -s "$H/interpret?file=mystery.unknownext"   # fallback: PTBR
curl -s "$H/interpret?ext=.rs"                   # sem file, só ext: Rust
```

**Resposta com match** (ver `interpret_match.example.json`):
```json
{
  "file": "foo.py", "extension": ".py",
  "drivers_dir": "drivers", "drivers_scanned": 33,
  "matched": true,
  "driver": "tokens_Python_PTBR.drv", "language": "Python"
}
```

**Resposta com fallback** (ver `interpret_fallback.example.json`):
```json
{
  "file": "mystery.unknownext", "extension": ".unknownext",
  "drivers_dir": "drivers", "drivers_scanned": 33,
  "matched": false, "fallback": "PTBR",
  "driver": "tokens_PTBR.drv", "language": "PTBR"
}
```

| campo | tipo | o que é |
|---|---|---|
| `file` | string | arquivo passado (só se vier `?file=`) |
| `extension` | string \| null | extensão extraída (lowercased); `null` se não houver |
| `matched` | bool | `true` se a extensão casa um driver; `false` no fallback |
| `driver` | string | nome do `.drv` escolhido |
| `language` | string | identificador da linguagem do driver escolhido |
| `fallback` | string | só presente em `matched=false`; nome do driver fallback (PTBR) |

> **Conflito de extensões.** Se duas linguagens declaram a mesma `extensao`, vence a
> última na ordem alfabética do scan (Delphi vence Pascal em `.pas` por convenção:
> Pascal puro ficou só com `.pp`).

## 7. Outras rotas de apoio

```bash
curl -s $H/health                       # {"status":"ok","bases":N,"drivers":M}
curl -s -X DELETE $H/bases/sdb          # descarrega a base 'sdb' da memória
```

---

## Arquivos neste diretório

| arquivo | usa em |
|---|---|
| `ingest_by_path.json` | `POST /ingest` (caminho de JSON tokenizado) |
| `ingest_by_data.json` | `POST /ingest` (dados embutidos — mini-base) |
| `ingest_raw.json` | `POST /ingest` (arquivo bruto com `raw:true`) |
| `ingest_file_auto.json` | `POST /ingest_file` (mínimo, driver auto pela ext) |
| `ingest_file_full.json` | `POST /ingest_file` (todos os opcionais) |
| `ingest_upload_multipart_response.example.json` | exemplo de **resposta** do /ingest_upload (multipart) |
| `ingest_upload_raw_response.example.json` | exemplo de **resposta** do /ingest_upload (raw body) |
| `search.json` | `POST /search` (todos os campos) |
| `search_minimal.json` | `POST /search` (só base + query) |
| `search_wildcard.json` | `POST /search` (base `sd*`) |
| `search_phonetic.json` | `POST /search` (busca por som, SOUNDEX) |
| `search_no_rerank.json` | `POST /search` (só recall) |
| `chunk.json` | `POST /chunk` (janela id ± before/after) |
| `chunk_ids.json` | `POST /chunk` (lista de ids) |
| `search_response.example.json` | exemplo de **resposta** do /search |
| `chunk_response.example.json` | exemplo de **resposta** do /chunk |
| `drivers_response.example.json` | exemplo de **resposta** do /drivers (filtro `Python`) |
| `interpret_match.example.json` | exemplo de **resposta** do /interpret (ext casa) |
| `interpret_fallback.example.json` | exemplo de **resposta** do /interpret (fallback PTBR) |
