# RAGnaRock — Arquitetura & Especificação

> **Norte de implementação.** Este documento descreve a solução **inteira** — inclusive o que
> ainda não foi construído. É a referência para evoluir o projeto sem perder coerência.
>
> Marcação de status em cada item: **[FEITO]** · **[PARCIAL]** (esqueleto/stub) · **[FUTURO]** (planejado).
>
> **Filtro de toda decisão** (vale pra qualquer linha abaixo): *isto mantém o RAGnaRock simples,
> transparente, rodando em qualquer hardware e ensinável?* Se exige caixa-preta, GPU ou complexidade
> que afasta o iniciante → é **opcional/opt-in ou fica de fora**, por melhor que seja tecnicamente.

---

## 1. Visão geral & invariantes

RAGnaRock é um RAG **sem rede neural**: token = **sílaba** (PT), embedding = **histograma esparso**
(bag of syllables), busca = **cosseno tf-idf** (recall) + **matched filter fonético** (rerank).
Tudo inspecionável (JSON legível), roda em **CPU + RAM, sem GPU**.

**Três daemons**, processos independentes que conversam por **HTTP JSON**:

| Daemon | Porta | Papel | Status |
|---|---|---|---|
| `ragd` | **11499** (API) | Motor: segura N bases em RAM, busca/ingestão | [FEITO] |
| **ValHalla** (no `ragd`) | **11498** (console) | Console web supervisório | [FEITO] |
| `nidhoggd` (Níðhöggr) | **11497** (módulos) | Camada de **inteligência**: destila conhecimento | [PARCIAL] |

**Invariantes (não quebrar):**
1. **JSON é o contrato e a persistência.** Cada base é um JSON legível em disco; a RAM é só um cache
   reconstruível. Mata o ragd → sobe de novo → recarrega de `ragfiles/`. (Responde "perde no crash?":
   não — o que está no disco é a verdade; ingestão grava o JSON **antes** de carregar em RAM.)
2. **Mesma ordem de chaves no JSON** (serde `preserve_order`) — garante equivalência byte-a-byte entre
   as três encarnações da lib (`python_concept`, `rust_concept`, `ragd`).
3. **`nidhoggd` lê o corpus SEMPRE pela API do `ragd`, nunca do disco** — independe de onde os dados moram.
4. **Inteligência (IA) é sempre opt-in e nasce desligada.** O núcleo do RAG não depende de IA nenhuma.

---

## 2. Modelo de dados

Uma **base** = `{ meta, idf, chunks }`, persistida em `ragfiles/<collection>/<name>-tokenized.json`.

```jsonc
{
  "meta": {
    "corpus": "MeuController.cs",                   // nome do arquivo (com extensão)
    "source_file": "<upload:...>",                  // origem (path ou rótulo de upload)
    "bytes": 12345, "chunk_size": 2048, "n_chunks": 117,
    "vocab_size": 1956, "vocab_used": 312,
    "tokens_total": 9001, "oov_total": 42, "coverage": 0.9953,
    "with_text": true,                              // chunks guardam o texto?
    "generator": "ragd-ingest", "tokens_file": "tokens_CSharp_PTBR.drv",
    "language": "CSharp", "matched_by_ext": true,
    "built_at": "2026-05-23T...", "vocab": ["ca","sa",...]   // vocabulário do driver (ordem fixa)
  },
  "idf": { "<dim>": 0.693147, ... },                // idf por dimensão (sílaba)
  "chunks": [
    {
      "id": 0, "start": 0, "len": 2034,             // offset/len em chars no corpus
      "tokens": 410, "oov": 3,
      "vec": { "<dim>": <count>, ... },             // tf esparso (histograma de sílabas)
      "norm": 16.374664,                            // norma L2 do vetor tf-idf (p/ cosseno)
      "text": "...",                                // o texto do chunk (se with_text)
      "words": [["fro","do"],["bol","sei","ro"]]    // [FEITO, em RAM] sílabas por palavra (cache do rerank)
    }
  ]
}
```

- **`idf` suavizado:** `idf(dim) = ln((N + 1) / df)` onde `N` = nº de chunks, `df` = nº de chunks que
  contêm a dim. O `+1` evita o colapso pra 0 numa base de **1 chunk** (com `ln(N/df)`, df=N=1 → idf=0
  → vetor nulo → base invisível). [FEITO]
- **`vec` é tf cru** (independente por chunk). Só `idf` (global da base) e `norm` (por chunk) dependem
  do corpus inteiro → por isso append recomputa só esses dois. [FEITO]
- **`words`** (sílabas por palavra) não é serializado: é derivável de `text` e cacheado em RAM no modo
  `memory`; no modo `hybrid` é recomputado sob demanda no rerank. [FEITO]

---

## 3. `ragd` — o daemon de produção

### 3.1 Processo, portas, estado, config

- Um processo Rust serve **duas portas** via `Arc<Mutex<State>>`: **11499** (API JSON) e **11498**
  (ValHalla, thread separada). [FEITO]
- `State` = bases em memória (`HashMap<collection, HashMap<name, RagBase>>`), drivers_dir, ragfiles_dir,
  config, sessões do console. [FEITO]
- **Auto-load no boot:** varre `ragfiles_dir` (cada subdir = coleção, cada `*-tokenized.json` = base). [FEITO]
- **Config `ragnarock.cfg`** (chaves):

  | chave | default | função |
  |---|---|---|
  | `api_port` | 11499 | porta da API JSON |
  | `dash_port` | 11498 | porta do console ValHalla |
  | `drivers_dir` | `drivers` | drivers de linguagem (`.drv`) |
  | `ragfiles_dir` | `ragfiles` | bases tokenizadas (auto-load) |
  | `max_upload` | 1 GB | teto do `POST /ingest_upload` |
  | `autoload` | true | carregar bases no boot |
  | `storage` | `memory` | `memory` (cacheia tokens) \| `hybrid` (recomputa) |
  | `admin_user`/`admin_pass` | admin/admin | login do console — **[FUTURO] trocar fora do dev** |
  | `active_provider` | none | `none`\|`anthropic`\|`openai` (1 ativo; p/ query-expansion) |
  | `cache_dir` | `cache` | `thesaurus.json` / `expansions.json` |
  | `log_file` | `/tmp/ragd-all.log` | arquivo lido pela aba Logs (= redirect do launcher) |
  | `log_utc_offset` | -3 | fuso dos timestamps |

  > ⚠️ `ragnarock.cfg` guarda as **chaves de API** dos providers → está no `.gitignore`. Versionar um
  > `ragnarock.cfg.example` sanitizado. [FUTURO]

### 3.2 Pipeline de busca [FEITO]

`base.search(query, k, rerank, recall_n, phonetic)` → `(hits, info)`, em dois estágios:

1. **Recall (cosseno tf-idf esparso):** tokeniza a query em sílabas → vetor tf ponderado por `idf` →
   cosseno contra cada chunk (itera o vetor menor; só dims em comum contam). Pega os `recall_n` candidatos.
2. **Rerank (matched filter fonético por proximidade):** sobre os candidatos, mede a **menor janela**
   que cobre um casamento de cada palavra da query (proximidade), **ignorando monossílabos** (stopwords),
   com soundex opcional (`phonetic`). Score combina cobertura + proximidade. Devolve top-`k`.

- **Scatter-gather:** `/search` resolve o escopo (`collection` + wildcard em `base`: `"sda"`, `"sd*"`,
  `"*"`), busca em cada base que casa (paraleliza com rayon quando há >1 base) e faz **merge por matchpoint**.
- **Hit:** `{ collection, base, corpus, path, chunk, matchpoint, mf, span, cos, start, snippet }` — o
  `path` é reconstruído (`base` decodificado `__`→`/` + `corpus`) pra **IA ir direto no arquivo**. [FEITO]
- **Query expansion (`search_expand`):** cascata **dicionário → cache → IA** (provider ativo) que expande
  a query por sinônimos antes de buscar, com **filtro por vocab** (só sinônimo que ancora no corpus do
  escopo) e peso maior no termo original. Exposto na API (11499) **e** no console. [FEITO]
  - ⚠️ Sinônimos de **idf baixo** (palavras comuns) podem dominar e poluir lookup preciso → o consumidor
    deve preferir busca **pura** (`expand=false`) para lookup de identificador/arquivo. **[FUTURO]:** podar
    sinônimos de idf baixo na expansão.

### 3.3 Ingestão [FEITO]

- `POST /ingest` (JSON tokenizado, base inline, ou bruto), `POST /ingest_file` (path), `POST /ingest_upload`
  (multipart **ou** corpo bruto + querystring — ingere **texto cru sem arquivo**).
- **Default = overwrite por nome** (`bases.insert(name, base)` — substitui a base inteira).
- **Append incremental com chunk-packing** (`append=true`): em vez de criar chunk novo, **enche o último
  chunk até `chunk_size` e transborda** o excedente; só o "rabo" (último chunk + texto novo) é
  re-tokenizado, o resto reusa o `vec`; recomputa `idf` + `norm` globais. Chunks crescem ordenados e
  cheios → com `N>1` o idf passa a discriminar.
- **Persistência:** grava `ragfiles/<collection>/<name>-tokenized.json` **antes** de carregar em RAM.

### 3.4 Estratégia de memória e disco

| modo | em RAM | trade-off | status |
|---|---|---|---|
| `memory` (default) | `meta`+`idf`+`chunks` **com `words` cacheado** | busca mais rápida, +RAM | [FEITO] |
| `hybrid` | idem **sem `words`** (recomputa só dos candidatos no rerank) | −66% RAM medido, busca ampla um pouco mais lenta | [FEITO] |

- **Durabilidade:** a verdade está no disco (`ragfiles/`); RAM é cache → crash recupera no boot.
- **`[FUTURO]` mmap/on-disk estilo Qdrant:** **não agora.** Kimi e Codex convergiram: o sistema é
  **CPU-bound na silabificação**, não I/O-bound; mmap adiciona superfície de bug (corrupção, lock,
  flush) e **trai o "roda em qualquer lugar"** (dependências nativas/FS). Só considerar se **corpus >
  ~80% da RAM**, e mesmo assim **opt-in por build/config** (modular), nunca default.
- **Pressão de memória:** o console mede RSS (`/proc/self/statm`) + estimativa text/vec/words; medido:
  ~580 bases ≈ 516 MB (`memory`) → 174 MB (`hybrid`). [FEITO]

### 3.5 Concorrência

- **Hoje:** `Arc<Mutex<State>>` global — toda operação (read ou write) compete pelo mesmo lock.
  Throughput medido: ~500 req/s num Mac M-series, ~65 num x86 de 2 cores, ~43 num Raspberry Pi 3 (busca global). [FEITO]
- **Por que basta hoje:** o uso principal é **UMA IA, sequencial** — não há contenção real. Mutex
  funciona bem até dezenas de req/s concorrentes.
- **`[FUTURO]` quando virar multi-agente:**
  - `Mutex<State>` → **`RwLock<State>`**: N **buscas read-only** em paralelo; `write()` só em
    ingest/delete. (Ressalva do Codex: o rerank em `hybrid` recomputa `words` — mas isso é leitura pura,
    cabe no read-lock; não vira write.)
  - **Granularidade por coleção** (lock por coleção, não global) → buscar na coleção A enquanto ingere na B.
  - Cuidado: starvation de writers se readers forem contínuos (usar `RwLock` justo/fair).
  - Codex sugere desacoplar ingest×busca por **canal/mensagem** (lock-light) — guardar para se o RwLock
    não bastar; YAGNI antes disso.

### 3.6 Drivers de linguagem [FEITO]

- Tokenização de **código-fonte** usa `.drv`: sílabas da base `SourceCode` (PT + sílabas de código) +
  **keywords reservadas** da linguagem. `tokens_PTBR.drv` e `tokens_SourceCode_PTBR.drv` são a **matriz
  fixa**; os demais derivam via `tools/gen_drivers.py`. `GET /interpret?file=foo.py` roteia extensão →
  driver/linguagem.

### 3.7 Contrato HTTP — rotas

**Implementadas [FEITO]:**

| método | rota | função |
|---|---|---|
| GET | `/health` | `{status, bases, collections, drivers}` |
| GET | `/bases` `?collection=&match=` | lista bases (com `corpus`, `n_chunks`...) |
| GET | `/collections` | resumo por coleção |
| GET | `/drivers` `?match=` | lista drivers |
| GET | `/interpret` `?file=\|?ext=` | extensão → driver |
| POST | `/search` | busca pura (recall+rerank) |
| POST | `/search_expand` | busca com query expansion |
| POST | `/ingest` · `/ingest_file` · `/ingest_upload` | ingestão (inclui `append=true`) |
| POST | `/chunk` | recupera chunk(s) inteiro(s) por id (`before`/`after`) |
| DELETE | `/bases/{nome}` `?collection=` | remove base |

**A definir/faltam [FUTURO]:**
- `DELETE /collections/{nome}` (remover coleção inteira).
- `GET /stats` (agregado público; hoje só interno no console).
- `GET /bases/{coll}/{name}` (metadados de 1 base sem buscar).
- Ingestão **por arquivo dentro de um repo** (base = N arquivos; update incremental por `sha` de arquivo
  — ver §6). Hoje base = 1 arquivo.

---

## 4. ValHalla — console web (11498) [FEITO]

Console supervisório embutido no `ragd` (HTML no binário), **sessão por cookie** (login `admin/admin`,
TTL; **[FUTURO]** senha real). Abas:

- **Visão** — coleções/bases/chunks/drivers, barras de distribuição, pressão de memória.
- **Buscar** — form + resultados; toggle **expandir 🧠** (chama `/api/search_expand`) e **fonético**;
  modal de chunk (arquivo + caminho + chunk N/total).
- **Ingestão** — upload de arquivos/pasta (`webkitdirectory`), escolhe coleção, status por arquivo.
- **Performance** — histograma query×chunk, matched filter com ponto de convergência, mapa de calor.
- **Drivers** — lista de linguagens/keywords.
- **Logs** — tail do `log_file`, auto-refresh, linhas coloridas (a árvore hierárquica do `search_expand`
  aparece aqui).
- **Config** — storage `memory|hybrid`, chaves de API (cofre mascarado, 1 provider ativo), restart.
- **Dicionários** — liga/desliga dicts do thesaurus (toggle por flag, não move arquivo).

> ValHalla **lê e opera** o ragd; não tem lógica de busca própria (delega à API).

---

## 5. `nidhoggd` / Níðhöggr — camada de inteligência (11497) [PARCIAL]

> No mito, Níðhöggr é a serpente que rói as raízes de Yggdrasil. Aqui, o worm rói/**digere o
> conhecimento** da árvore do RAG e o destila num saber que **sobrevive à deleção da coleção**.

### 5.1 Conceito & invariantes [FEITO: esqueleto]

- Processo **separado**, **"daemon de módulos"** (porta 11497 vai hospedar N módulos além do Nidhogg).
- Lê o corpus **sempre via API do `ragd`** (nunca disco) → independe de localização.
- **Nasce DESLIGADO** (níveis ≥1 consomem IA). Liga/desliga **global** e **por coleção** (não re-mastiga
  a mesma N vezes). Keepalive pinga o `ragd` a cada 15s e cacheia (status nunca faz curl ao vivo).
- **Dois dials ortogonais:** **nível** (profundidade) + **cadência** (segundos entre ciclos = orçamento
  de tempo).

### 5.2 Os 4 níveis (cumulativos) — o que cada um produz e persiste

| nível | nome | IA? | produz (`knowledge[]`) | status |
|---|---|---|---|---|
| 0 | **burro** | não | **3 pilares:** índice de raízes (sílabas/stems), dicionário do corpus, digestão do cache | [PARCIAL] |
| 1 | **consciente** | sim | insights + resumo **por coleção** (o saber que sobrevive à deleção) | [FUTURO] |
| 2 | **estrutural** | sim | hierarquia e **encaixe de dimensões** entre projetos/ingestões | [FUTURO] |
| 3 | **propositivo** | sim | acha **furos**, sugere, comenta, resume inteligente | [FUTURO] |

- **Nível 0 é o núcleo seguro** ("fóssil" auto-suficiente, sem IA): sempre útil, custo zero, alinhado
  com "roda em qualquer lugar". É o que liga por default quando o worm liga.
- **Níveis 1–3 são experimentais e custam IA** — só com provider ativo, por coleção habilitada.

### 5.3 Schema do conhecimento persistente — `<dir>/<coll>.knowledge.json`

Um arquivo **por coleção** (hoje: `{collection, enabled, source_hash, saturation, updated, provenance,
knowledge[]}`). Forma-alvo (sintetizada com o Kimi):

```jsonc
{
  "collection": "minha_colecao",
  "enabled": true,
  "source_hash": "sha256 do estado da coleção na última digestão",
  "saturation": 0.0,                 // 0..1 — fração do conhecimento ainda verificável (ver 5.4)
  "updated": "ISO8601",
  "provenance": [                    // rastreabilidade: de onde veio CADA digestão
    { "digestion_id":"uuid", "ts":"ISO8601", "source_hash":"sha256", "level":1,
      "inputs":["collection:minha_colecao"], "model":"kimi-for-coding|null", "tokens_in":0, "tokens_out":0 }
  ],
  "knowledge": [                     // os itens destilados
    { "type":"RootIndex|CorpusDict|Summary|DimensionMap|Gap|Suggestion",
      "level":1, "created":"ISO8601", "content":{}, "confidence":0.0,
      "derived_from":["digestion_id"], "orphaned":false }
  ]
}
```

### 5.4 Saturation, provenance, sobrevivência à deleção

- **`source_hash` (hash, não nome):** cada item de conhecimento aponta pra um hash do estado da fonte.
  Renomear/deletar a coleção **não invalida** o que já foi destilado; só marca que a fonte mudou.
- **`saturation` = (itens ainda verificáveis contra uma fonte viva) / (total de itens).** `→1.0` tudo
  ancorado; `<0.5` alerta de muito conhecimento **órfão**. Decai naturalmente se coleções somem.
- **GC lazy de órfãos:** ao carregar um `knowledge.json`, conferir se as fontes (`source_hash`) ainda
  existem; o que não existe vira `orphaned:true`; abaixo de um threshold, pode ser arquivado.
- **Invariante:** nenhum item de nível ≥1 é gerado sem `provenance` (digestion_id + source_hash + modelo).
- **Cadência ≠ saturação:** worm não re-mastiga coleção saturada (`source_hash` igual ao último) — economiza IA.

### 5.5 API do módulo [FEITO]

`GET /health` · `GET /api/nidhogg` (status: nível, cadência, keepalive do ragd, conhecimento) ·
`GET /api/nidhogg/collections` (coleções + estado de digestão) · `POST /api/nidhogg`
(`{on, level, cadence}`) · `POST /api/nidhogg/collection` (`{collection, enabled}`) ·
`POST /api/nidhogg/run` (dispara ciclo — **stub**).

### 5.6 ⚠️ Riscos & questões em aberto (a crítica honesta — Codex)

O Nidhogg é a parte **mais arriscada** do projeto. Registrado de propósito, não escondido:

- **"Solução procurando problema?"** O conhecimento destilado só vale se **alguém o consome com
  rastreabilidade**. **Decisão:** o consumidor é explícito (o módulo expõe o saber via API, auditável
  por `provenance`); e o que é **destilado** nunca se mistura com a **fonte** numa resposta sem rótulo.
- **Órfão/stale contaminando resultados** — conhecimento de fonte morta vira resposta fantasiosa.
  **Guard-rail:** `source_hash` + `saturation` + `orphaned` + GC lazy; nível 0 (sem IA) é à prova disso.
- **Custo/latência de IA vs "roda em qualquer lugar"** — **Guard-rail:** OFF por default; níveis 1–3
  opt-in por coleção; nível 0 cobre o caso sem-IA; cadência limita o orçamento.
- **Conhecimento cíclico** (nível 2 consome nível 1 que consome…) — `derived_from` + `digestion_id`
  evitam loop; nunca derivar de item órfão.
- **Framing dos níveis** pode confundir operador — o console deve descrever o que **cada nível** faz.
- **O que fica de fora até existir consumidor real:** níveis 2–3 e qualquer destilação cara só avançam
  quando houver quem **consuma e audite** o resultado. Começar pelo nível 0 (sem IA), provar valor, subir.

---

## 6. Estratégias transversais & roadmap maior

- **Repo como base (não 1-arquivo-por-base) [FUTURO]:** schema de chunk ganha `file` + linhas; `meta`
  ganha mapa de arquivos com `sha`; `POST /ingest_file {base, file}` recomputa só aquele arquivo
  (remove os chunks antigos do `file`, insere os novos, atualiza `sha`); `POST /sync {base, path}` varre
  e atualiza só o que mudou. É o coração do "RAG de código com update por arquivo".
- **Ingestores acionados pela IA do usuário [FUTURO]:** o agente dispara ingestão (repo, diff de git,
  arquivos específicos) via MCP / CLIs.
- **Importadores [FUTURO]:** PDF/DOCX/XLSX (extração no cliente vs sidecar no servidor).
- **Build Windows [FUTURO]:** Rust puro deve compilar; cuidar `/dev/urandom` (entropia Windows) e
  `log_file` default.
- **Deploy [FEITO]:** cross-compile (`cargo zigbuild --target {x86_64,aarch64}-unknown-linux-gnu.2.31`)
  + rsync do binário + launcher que sobe `ragd` + `nidhoggd` detached e redireciona stdout pro `log_file`.
- **Segurança [PARCIAL]:** trocar `admin/admin`; chaves só no `cfg` (gitignored); CORS aberto na 11497
  (rever ao expor); sessão do console com TTL (rotacionar cookie — [FUTURO]).

---

## 7. Apêndice — modos de falha

**Óbvios:** JSON de base corrompido (gravar `.bak` antes do overwrite, validar no load) · OOM (usar
`hybrid`, [FUTURO] tetos de bases/chunks) · IA fora (níveis ≥1 degradam pro nível 0) · ragd fora (o
keepalive do Nidhogg degrada gracioso, status do cache).

**Não-óbvios (Kimi/Codex):** silabificação divergente entre ingestão e busca (mesmo driver/vocab é
obrigatório — append herda o driver da base) · starvation de writers no RwLock (usar lock justo) ·
`source_hash` falso-positivo em rename (re-link manual [FUTURO]) · rerank lento em `hybrid` (aceitável;
medir) · `knowledge.json` crescendo sem fim (compactar provenance [FUTURO]) · nível 3 alucinando furos
inexistentes (confidence + auditoria humana).

---

## 8. Decisões pendentes (gatilho → ação)

| decisão | gatilho | opções |
|---|---|---|
| mmap/on-disk | corpus > ~80% RAM | binário estruturado vs LMDB; **opt-in por build** |
| `RwLock` + paralelismo inter-query | latência sob carga concorrente real | RwLock; depois lock por coleção |
| base = repo (N arquivos) | usar como RAG de código a sério | `file`+`sha` no schema; `/sync` |
| podar sinônimos idf-baixo no expand | expansão poluindo lookup | filtro por idf na cascata |
| Nidhogg níveis 1–3 | existir consumidor que audita | começar nível 0; subir provando valor |

---

> *Documento vivo — incrementar a cada ciclo. Síntese curada a partir de contraposição Kimi (gerador)
> × Codex (crítico), com o leme mantido na missão: simples, inspecionável, roda em qualquer lugar.*
