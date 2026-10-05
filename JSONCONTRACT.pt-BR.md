> 🌐 **Idioma.** Versão em **português (pt-BR)**, tradução mantida em sincronia. Versão principal em inglês: **[JSONCONTRACT.md](JSONCONTRACT.md)**.

# RAGnaRock — Contrato da API JSON

Referência **formal** das APIs HTTP/JSON dos três daemons, escrita a partir do código (`ragd/src/main.rs`,
`ragd/src/auth.rs`, `nidhoggd/src/main.rs`). Para **exemplos executáveis** (`curl -d @arquivo.json`), veja
[`ragd/json_samples/`](ragd/json_samples/) — este documento é a especificação; aquele é o tutorial.

| Daemon | Porta | Papel |
|---|---|---|
| [`ragd`](#1-ragd--api-de-dados-11499) | **11499** | Motor: busca, ingestão, descoberta, administração |
| [ValHalla](#2-valhalla--console-11498) | **11498** | Console web (React) servido pelo `ragd`, com proxy para a API e para o `nidhoggd` |
| [`nidhoggd`](#3-nidhoggd--inteligência-11497) | **11497** | Camada de inteligência (níveis 0–4, ClickHouse) |

## Convenções

- **Transporte:** HTTP/1.1, corpo `application/json` — exceto `/ingest_upload`, `/ingest_any` e `/transcribe`,
  que aceitam multipart (campo `file`) ou corpo cru com os metadados na querystring.
- **Coleções:** toda base pertence a uma `collection`; sem `collection` num POST → `"default"`.
  No disco: `ragfiles/<coleção>/<nome>-tokenized.json`.
- **Padrão de base** (em `/search`, `/bases`): `"sda"` (exato) · `"sd*"` (prefixo) · `"*"` (todas);
  comparação sem diferenciar maiúsculas, com normalização NFC.
- **Erros:** HTTP 4xx/5xx com corpo `{ "error": "<mensagem>" }`.
- **Autenticação:** JWT HS256 (§1.1). **Só as rotas administrativas exigem token**; as rotas de dados
  (busca, ingestão, remoção, `/chunk`) estão abertas, e o `nidhoggd` não tem autenticação. Exponha as
  portas só em rede confiável.

---

## 1. `ragd` — API de dados (11499)

### 1.1 Autenticação

`POST /login` devolve um token `access` (15 min) e um `refresh` (TTL `session_ttl`, padrão 12 h). Rotas
protegidas recebem `Authorization: Bearer <access>`: 401 sem token ou token inválido, 403 sem a capacidade.
Usuários e perfis ficam em `auth_file` (padrão `ragnarock-auth.json`, senha PBKDF2); a primeira execução cria
`admin/admin` e os perfis `admin`, `operador`, `leitor`, `auditor`. Capacidades: `buscar`, `ingerir`,
`apagar`, `nidhogg.ver`, `nidhogg.operar`, `admin.config`, `admin.usuarios`, `admin.servicos`, `*`
(hoje o servidor só exige as `admin.*`; as demais orientam a interface).

| Método | Rota | Exige | Requisição | Resposta |
|---|---|---|---|---|
| POST | `/login` | — | `{login, password}` | `{access, refresh, expires_in:900, usuario:{login, nome, perfil, caps, colls}}` · 401 |
| POST | `/refresh` | — | `{refresh}` | `{access, expires_in}` · 401 |
| GET | `/auth/caps` | — | — | `{caps:[…]}` |
| GET | `/auth/me` | token | — | `{usuario:{login, nome, perfil, caps, colls}, exp}` |
| POST | `/auth/password` | token | `{atual, nova}` (≥ 6 caracteres) | `{ok, login}` · 401 se `atual` errada |
| GET | `/auth/perfis` | `admin.usuarios` | — | `{perfis}` |
| POST | `/auth/perfis` | `admin.usuarios` | `{nome, desc, caps[], colls[]}` (`colls` padrão `["*"]`) | `{ok, perfil}` · 400 capacidade desconhecida |
| DELETE | `/auth/perfis/{nome}` | `admin.usuarios` | — | `{ok, removed}` · 409 em uso · 404 |
| GET | `/auth/usuarios` | `admin.usuarios` | — | `{usuarios:[{login, nome, perfil, ativo}]}` |
| POST | `/auth/usuarios` | `admin.usuarios` | `{login, nome, perfil, ativo=true, password}` (senha obrigatória ao criar) | `{ok, usuario}` · 409 último admin ativo |
| DELETE | `/auth/usuarios/{login}` | `admin.usuarios` | — | `{ok, removed}` · 409 último admin · 404 |

### 1.2 Descoberta

| Método | Rota | Requisição | Resposta |
|---|---|---|---|
| GET | `/health` | — | `{status, bases, collections, drivers}` |
| GET | `/bases` | `?collection=` (padrão todas) `&match=` (padrão `*`) | `{collection, match, count, bases:[{collection, name, n_chunks, vocab_size, corpus, generator, has_text}]}` |
| GET | `/bases/{coleção}/{nome}` | — | `{collection, name, corpus, generator, n_chunks, vocab_size, vocab_used, has_text, mtime}` · 404 |
| GET | `/collections` | — | `{count, total_bases, collections:[{collection, bases}]}` |
| GET | `/profile` | ver abaixo | perfil léxico de uma base ou da coleção |
| GET | `/stats` | — | `{version, uptime_secs, collections, bases, chunks, drivers, dicts_active, word_syn_entries, ragfiles_dir, collections_detail:[{collection, bases, chunks}], mem}` |
| GET | `/expansions` | — | `{count, expansions:{<query normalizada>:[variantes]}}` (cache de expansão) |
| GET | `/drivers` | `?match=` (prefixo na linguagem) | `{drivers_dir, match, count, drivers:[{name, language, description, extensions, syllables, keywords, vocab_size, header}]}` |
| GET | `/drivers_out` | — | mesmo formato, listando `<drivers_dir>.out` (desinstalados) |
| GET | `/ingestors` | — | `{ingestors_dir, count, ingestors:[{name, bytes, description}]}` |
| GET | `/interpret` | `?file=` ou `?ext=` | `{file?, extension, drivers_dir, drivers_scanned, matched, driver, language, fallback?}` · 400 sem nenhum |
| GET | `/thesaurus` | — | `{thesaurus_dir, count, active, dicts:[{code, active, entries, source, source_url, license, kind, lang_query, lang_target, size_bytes}]}` |

**`GET /profile`** — `?collection=` obrigatório (400 sem ele).
- Com `&base=`: `?top=20` → `{scope:"base", collection, base, corpus, n_chunks, vocab_size, vocab_used, has_text, mtime, top_idf:[{dim, syllable, idf}]}`.
- Sem `base` (coleção): `?top=20&rank=uidf|idffreq&min_freq=0&vectors=1` → `{scope:"collection", collection, bases, chunks, unified_vocab_size, shared_vocab, unique_vocab, rank, min_freq, top_uidf:[{dim, syllable, uidf, df, freq}]}`;
  com `vectors` acrescenta `base_vectors:[{name, corpus, n_chunks, dims_used, coverage, unique_dims, shared_dims, vec[]}]`.
- 404 base ou coleção desconhecida.

### 1.3 Busca — `POST /search`

Rota única de busca (#39), com estágios **opt-in**: padrão conservador = busca léxica precisa.

**Requisição:**
```jsonc
{
  "base": "*",              // obrigatório — exato | "pref*" | "*"
  "query": "Frodo Bolseiro",// obrigatório
  "collection": "livros",   // opcional; ausente ou "*" = todas
  "k": 5,                   // resultados após o merge (padrão 5)
  "rerank": true,           // estágio 2 (cobertura/proximidade); false = só recall (padrão true)
  "recall_n": 20,           // candidatos por base enviados ao rerank (padrão 20)
  "unified": true,          // vocab+idf unificados da coleção (#8); padrão true quando o escopo tem
                            // coleção com >1 base; false força o idf local de cada base
  "phonetic": false,        // casa pelo SOM (SOUNDEX): "Aslan" acha "Aslam"
  "literal_fallback": true, // needles alfanuméricos COM dígito (OE-6016, M31May-23h28): grep literal,
                            // achados exatos vêm na frente (#38); false desliga
  "expand": false           // true = cascata dicionário → cache → IA (mesmo motor de /search_expand)
}
```
**Resposta** (`expand: false`):
```jsonc
{
  "via": ["silabico", "literal_fallback"],   // estágios efetivos: silabico | phonetic | literal_fallback
  "query": "M31May-23h28",
  "query_syllables": "m-may-h",
  "scope": ["sessoes/marcadores", "livros/EN_aesop_fables", "…"],  // bases buscadas ("coleção/base")
  "searched": [ { "collection":"livros", "base":"EN_aesop_fables", "n_chunks":133, "n_converge":127,
                  "dims":4, "oov":0, "ms_recall":1.2, "ms_rerank":22.3 } ],
  "needles": ["m31may-23h28"],               // só quando a query tem token alfanumérico com dígito
  "hits": [
    { "rank":1, "collection":"sessoes", "base":"marcadores", "corpus":"marcador.txt",
      "matchpoint":1.0, "coverage":1.0, "span":1, "cos":0, "chunk":0, "start":0,
      "snippet":"…O marcador M31May-23h28 indica…", "needles_matched":["m31may-23h28"],
      "via":"literal_fallback" },                       // hit literal
    { "rank":2, "collection":"livros", "base":"…", "corpus":"…", "path":"…",
      "matchpoint":0.8, "coverage":0.8, "span":2, "cos":0.2664, "chunk":28, "start":57193,
      "snippet":"…«Frodo» «Bolseiro»…", "recency":"1.000" }   // hit silábico
  ]
}
```
- Ordem: `coverage` ↓ · `span` ↑ · `cos` ↓ · recência (só desempata); hits literais na frente, sem repetir chunk.
- `coverage`/`span` só existem com `rerank`. `recency` é texto, só para exibição. `cos` dos hits literais é 0.
- Com `expand: true`, a resposta é a do `/search_expand` (abaixo), com `via`.
- 404 quando nenhuma base casa com o escopo. A busca é **determinística**: mesma query, mesma resposta (#56).

### 1.4 Busca com expansão — `POST /search_expand`

Preset de `/search` com `expand: true` (compatível com o contrato antigo).
**Requisição:** `{query, base="*", collection?, k=8, phonetic=false, two_phase=true, literal_fallback=true}`.

Pipeline: (1) literal primeiro — se a query tem needles e o grep acha, encerra; (2) **duas fases** — roda a
busca original e, se o topo cobre a query inteira (≥ 0,999) com ≥ min(k,2) hits, devolve sem expandir
(`two_phase: false` força a cascata); (3) cascata dicionários ativos → cache (`cache/expansions.json`) → IA
(grava no cache); merge das variantes com rescore contra a query original.

| `source` | `via` | Quando |
|---|---|---|
| `literal` | `["literal"]` | needles achados por grep, antes de tudo |
| `phase1` | `["silabico"]` | busca original já forte (`recall:"strong"`) |
| `dict` · `cache` · `llm` | `["silabico", <source>]` | expansão usada |
| `literal_fallback` | `["silabico","literal_fallback"]` | sem dicionário/cache/IA, mas o literal achou |

**Resposta (normal):** `{via, query, provider, source, expansions:[…], absent:false, dropped:[…], hits}` —
hits com os campos da busca silábica + `var_cov`; `via` do hit = `"original"` ou a variante que o trouxe.
**Ausente** (nem a query nem variante ancoram no vocabulário): `{…, absent:true, dropped, reason, did_you_mean, hits:[]}`.
**Erros:** 400 sem dicionário, cache nem provider de IA (e o literal não achou) · 502 falha da IA.
Estas respostas não trazem `query_syllables`, `scope` nem `searched`.

### 1.5 Trechos e diagnóstico

| Método | Rota | Requisição | Resposta |
|---|---|---|---|
| POST | `/chunk` | `{base, collection?, id, before=0, after=0}` ou `{base, collection?, ids:[…]}` | `{collection, base, corpus, n_chunks, chunks:[{id, start, len, tokens, oov, norm, text}]}` · 404 |
| POST | `/histogram` | mesmo corpo do `/search` (usa o hit #1) | `{found:true, collection, base, chunk_id, coverage, cos, query_syllables, vocab_size, query, query_oov, chunk, seq_len, mf}` ou `{found:false, query_syllables}` |

### 1.6 Ingestão

Todas preparam a base **fora da trava** (driver, tokenização, gravação) e só inserem no fim (#37): buscas não
esperam a ingestão. Sucesso: `{ok, collection, name, n_chunks, bases, …}`. **413** quando `max_bases` é
atingido (só base nova) ou a base passa de `max_chunks_per_base` (0 = sem teto).

| Método | Rota | Requisição | Campos extras na resposta |
|---|---|---|---|
| POST | `/ingest` | (a) `{name, path}` JSON tokenizado · (b) `{name, data:{meta, idf, chunks}}` · (c) `{name, path, raw:true, chunk=2048, driver?, with_text=true, max_chunks=0}` arquivo bruto; + `collection` | `raw, saved_to?` |
| POST | `/ingest_file` | `{path, name?, collection?, chunk?, driver?, with_text?, max_chunks?}` (arquivo na máquina do daemon; `name` derivado do caminho) | `corpus, saved_to` |
| POST | `/ingest_upload` | multipart (`file` + campos) ou corpo cru + `?filename=&name=&collection=&chunk=&max_chunks=&with_text=&driver=&append=` | `filename, corpus, bytes, appended, driver, saved_to, via` (`multipart`/`raw`) |
| POST | `/ingest_any` | como `/ingest_upload`, mas antes roda o driver de ingestão `ingestors/<tipo>.py` (pdf, xlsx, xls, docx, doc, pptx, csv, áudio — pelo MIME ou extensão; 120 s) | idem; `driver` = script usado |

- `chunk` = caracteres por trecho (2048) · `driver` = `.drv` explícito (omitido = pela extensão, fallback PTBR)
  · `with_text` = guarda o texto (true) · `max_chunks` = 0 todos.
- `append=true` (só nos uploads): acrescenta à base existente (recalcula só `idf` e `norm`).
- Upload aceita só UTF-8 (binário → 400, salvo em `/ingest_any`); acima de `--max-upload` → 413.

**`POST /transcribe`** — áudio entra, texto sai (não ingere nem guarda o áudio). Mesma entrada do upload; roda
fora da trava. Resposta `{ok, text, chars, filename, bytes, ms, driver}` · 400 corpo vazio · 413 · 415 sem
driver · 422 sem texto · 500 falha ou tempo (`transcribe_timeout_s`, padrão 900 s).

### 1.7 Remoção

| Método | Rota | Requisição | Resposta |
|---|---|---|---|
| DELETE | `/bases/{nome}` | `?collection=` (padrão `default`) `&purge=1` (apaga também o JSON e o `.textblob`) | `{ok, removed, collection, purged, bases}` · 404 |
| DELETE | `/collections/{nome}` | `?purge=1` (apaga `ragfiles/<nome>/`) | `{ok, collection, bases_removed, purged, bases, collections}` · 404 |

Sem `purge`, a base volta no próximo boot (o autoload relê o disco).

### 1.8 Administração

| Método | Rota | Exige | Requisição | Resposta |
|---|---|---|---|---|
| GET | `/config` | `admin.config` | — | `{storage, config_path, drivers_dir, ingestors_dir, ragfiles_dir, max_upload_mb, max_bases, max_chunks_per_base, session_ttl, dev_mode, anthropic_key_set, anthropic_key_masked, openai_key_set, openai_key_masked, active_provider, local_url, nidhogg_url, cache_dir, expansions_entries, thesaurus_dir, dicts_active, word_syn_entries}` |
| POST | `/config` | `admin.config` | qualquer de `storage` (memory\|hybrid\|disk — recarrega as bases), `active_provider` (none\|anthropic\|openai\|local), `anthropic_key`, `openai_key`, `clear_anthropic`, `clear_openai`, `local_url`, `session_ttl` (≥ 60) | `{ok, notes[], reloaded, config:{…}}` · 400 |
| POST | `/config/test_provider` | `admin.config` | `{provider: anthropic\|openai}` | `{provider, ok, message}` |
| POST | `/driver_move` | `admin.config` | `{file:"x.drv", action: install\|uninstall}` | `{ok, file, action, installed}` · 400 · 404 |
| POST | `/thesaurus_toggle` | `admin.config` | `{code, action: enable\|disable}` | `{ok, code, action, active, word_entries}` · 404 |
| GET | `/logs` | `admin.servicos` | `?n=300` (máx. 5000) | `{file, log}` |

---

## 2. ValHalla — console (11498)

Console web (React, `web/dist`) servido pelo próprio `ragd` na `dash_port` (padrão 11498), com fallback de SPA
para `index.html`. **Não tem API de dados própria:** faz proxy HTTP repassando o cabeçalho `Authorization`:

- `/api/*` → `ragd` na porta da API, sem o prefixo `/api` (ex.: `/api/search` → `:11499/search`);
- `/nidhogg-api/*` → `nidhogg_url` sem o prefixo (ex.: `/nidhogg-api/api/nidhogg/tree` → `:11497/api/nidhogg/tree`);
- falha do proxy → 502; o proxy corta em 120 s.

O login é o JWT do §1.1 (`/api/login`), guardado pela interface. A aba de busca usa `POST /search`
(`expand: true` no modo semântico) e a de ingestão usa os uploads do §1.6.

> Limitações conhecidas do proxy: reenvia o corpo como `application/json` (upload multipart pela 11498 perde a
> fronteira — use corpo cru com `?filename=`, ou a 11499 direto) e o corte de 120 s pode interromper `/transcribe` longo.

---

## 3. `nidhoggd` — inteligência (11497)

Daemon de módulo: lê o corpus **sempre pela API do `ragd`** (§1), nunca do disco, e guarda o que aprende no
ClickHouse (`store = clickhouse`; `sqlite` é o reserva — rotas que dependem do ClickHouse devolvem vazio com
`note` ou 400). **Sem autenticação**; CORS só com `cors_origin` configurado.

Níveis: `minerador` (0, sem IA) · `consciente` (1, classifica) · `estrutural` (2, entidades e grafo) ·
`estrutural-llm` (3, relações por IA) · `propositivo` (4, perguntas e respostas).

### 3.1 Estado e controle

| Método | Rota | Requisição | Resposta |
|---|---|---|---|
| GET | `/health` | — | `{status, module, version, on, level, llm_online, llm_tag}` |
| GET | `/api/nidhogg` | — | `{module, version, uptime_secs, on, level, level_name, levels:[{n, name, ia, desc}], needs_ia, cadence_secs, dir, collections_known, cycle_running, last_cycle, ragd_api, ragd_online, llm_online, llm_tag, llm_url, llm_auth, llm_erro, llm_checked, ragd}` |
| POST | `/api/nidhogg` | `{on?, level? (nome ou 0–4), cadence? (≥ 10 s)}` (grava no cfg) | igual ao GET |
| GET | `/api/nidhogg/collections` | — | `{collections:[{collection, bases, chunks, enabled, saturation, updated, has_knowledge}]}` |
| POST | `/api/nidhogg/collection` | `{collection, enabled}` | `{ok, collection, enabled}` |
| POST | `/api/nidhogg/run` | — | 202 `{ok, started:true, note}` (ciclo forçado, assíncrono) · 200 `{ok, started:false, reason}` se já há ciclo |

### 3.2 Conhecimento e prompts

| Método | Rota | Requisição | Resposta |
|---|---|---|---|
| GET | `/api/nidhogg/knowledge` | `?collection=&type=&level=` | o `knowledge.json` da coleção (pilares do L0, procedência, `l0_diff`…), ou `{collections:[…]}` sem `collection` |
| GET | `/api/nidhogg/cachedigest` | — | `{type:"CacheDigest", level, scope, updated, content:{n_queries, n_variants_total, avg_variants, entries, note?}}` |
| GET | `/api/nidhogg/prompts` | — | biblioteca `prompts.json` (com `templates{}`) |
| POST | `/api/nidhogg/prompts/template` | `{name, system (≤ 6000), description, max_tokens (64–4000)}` | `{ok, template}` |
| GET | `/api/nidhogg/llm_ledger` | `?n=30` (máx. 200) | `{file, entries:[{ts, tag, coll, ctx, meta, ms, ok, finish, system, system_len, user, user_len, resposta, resposta_len}]}` (diário de IA; textos truncados) |

### 3.3 Classificação, extração e grafo

| Método | Rota | Requisição | Resposta |
|---|---|---|---|
| GET | `/api/nidhogg/doctypes` | — | `{naturezas, tipos}` |
| POST | `/api/nidhogg/doctypes` | `{naturezas[], tipos[]}` (não vazios) | `{ok, naturezas, tipos}` (contagens) |
| GET | `/api/nidhogg/doctypes/uso` | — | `{uso}` |
| GET | `/api/nidhogg/classes` | `?collection=` | `{collection, count, naturezas, tipos, bases}` |
| POST | `/api/nidhogg/reclass` | `{collection, base, tipo}` | `{ok, collection, base, tipo, natureza, csv, extraivel, nota, purgadas}` (origem `humano`; a IA não sobrescreve) · 400 tipo desconhecido |
| POST | `/api/nidhogg/relink` | `{collection, de, para}` | `{ok, collection, de, para, classes, entidades}` (move classe, entidades e nós de uma base renomeada) · 409 `de` ainda existe ou ciclo rodando · 404 `para` não existe · 502 |
| POST | `/api/nidhogg/molde` | `{tipo, instrucao, collection, base}` | `{ok, tipo, campos, cobertura, amostra}` (molde de extração dirigido) · 404 · 502 |
| GET | `/api/nidhogg/templates` | — | `{templates}` (moldes por tipo) |
| GET | `/api/nidhogg/rejeitados` | — | `{count, por_motivo, rejeitados}` |
| GET | `/api/nidhogg/entities` | `?collection=&base=` | `{count, nqi_global, por_base, por_tipo, amostra}` |
| GET | `/api/nidhogg/relacoes` | `?collection=&n=200` (máx. 1000) | `{count, bases, relacoes}` |
| GET | `/api/nidhogg/tree` | `?collection=` (obrigatório) `&q=` | `{collection, count, nodes}` |
| GET | `/api/nidhogg/suggest` | `?q=` (obrigatório) `&collection=*` | `{collection, count, nodes}` |
| GET | `/api/nidhogg/node` | `?norm=` (obrigatório) `&collection=` | `{found, valor, valor_norm, registros, bases, co, facetas}` |

### 3.4 Dimensões

| Método | Rota | Requisição | Resposta |
|---|---|---|---|
| GET | `/api/nidhogg/dimensoes` | — | `{dimensoes:[{nome, descricao, campos[], tipos[]}]}` |
| POST | `/api/nidhogg/dimensoes` | `{dimensoes[], substituir_tudo?}` | `{ok, dimensoes}` · 409 `{error, sumiriam, dica}` se eixos sumiriam sem `substituir_tudo` |
| POST | `/api/nidhogg/dimensoes/upsert` | `{dimensao}` | `{ok, criou, dimensoes}` |
| POST | `/api/nidhogg/dimensoes/remover` | `{nome}` | `{ok, nome, dimensoes}` · 404 |
| GET | `/api/nidhogg/dimensao/valores` | `?nome=` (obrigatório) `&collection=&q=` | `{count, valores, nome}` · 404 |
| GET | `/api/nidhogg/dimensoes/gaps` | `?collection=` | `{collection, gaps:[{nome, alvo, cobertos, gaps[], nota}]}` |

### 3.5 Perguntas e respostas (L4)

| Método | Rota | Requisição | Resposta |
|---|---|---|---|
| GET | `/api/nidhogg/perguntas` | — | `{perguntas:[{nome, texto, tipo (tabular\|oneshot\|vivo), escopo, ativa, pai}]}` |
| POST | `/api/nidhogg/perguntas` | `{perguntas[], substituir_tudo?}` | `{ok, perguntas}` · 409 `{sumiriam}` |
| POST | `/api/nidhogg/perguntas/upsert` | `{pergunta}` | `{ok, criou, pergunta, perguntas}` |
| POST | `/api/nidhogg/perguntas/remover` | `{nome, purgar?}` | `{ok, nome, etapas_apagadas, perguntas}` · 404 |
| POST | `/api/nidhogg/perguntar` | `{pergunta}` (nome) | `{ok, pergunta, nova_etapa, …}` (responde agora: analista + comparador) · 404 · 502 |
| GET | `/api/nidhogg/respostas` | `?pergunta=` (obrigatório) | `{pergunta, count, etapas}` |
| POST | `/api/nidhogg/respostas/limpar` | `{pergunta}` | `{ok, pergunta, etapas_apagadas}` |

### 3.6 Modos de linha de comando

- `nidhoggd --laya-check <versão> [--laya-ort-lib <libonnxruntime.so>]` — paridade da porteira Laya (#53) contra
  o `paridade.json` da versão (saída 0 = ok, 1 = divergência, 2 = erro).
- `nidhoggd --classify-list <arquivo.json>` — classifica `[[coleção, base], …]` e imprime JSONL
  `{coll, base, natureza, tipo, csv, nat_llm}`, sem daemon.

> Configuração (`nidhogg.cfg`): veja `ARCHITECTURE.pt-BR.md` §5 (`llm_*`, `llm_ledger*`, `prune_grace_h`, `laya_*`).

---

> Fonte da verdade: o código em `ragd/src/` e `nidhoggd/src/`. Este documento foi reconciliado com o código em
> 05/out/2026 (#39).
