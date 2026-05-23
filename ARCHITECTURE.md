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
- `GET /profile?collection=&base=` — **perfil léxico** (`vocab_used`, `dims`, `top_idf[]`) para alimentar
  o **nível 0 do Nidhogg** sem sondar via `/search` (caro). Achado no ciclo de revisão (§5.3).
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
- **Nidhogg** **[FUTURO]** — a "tela gigante" da camada de inteligência: liga/desliga global e por
  coleção, dial de nível + cadência/janela, prompt por nível, toggle do **gate de aceite** + botão de
  **aceitar** cada versão de artefato, e a leitura dos artefatos versionados (documento vivo, árvore de
  conhecimento). **Ao LIGAR: disclaimer obrigatório** de consumo de IA.

> ValHalla **lê e opera** o ragd; não tem lógica de busca própria (delega à API).

---

## 5. `nidhoggd` / Níðhöggr — camada de inteligência (11497) [PARCIAL]

> No mito, Níðhöggr é a serpente que rói as raízes de Yggdrasil. Aqui, o worm rói/**digere o
> conhecimento** da árvore do RAG e o destila num saber que **sobrevive à deleção da coleção**.

> 💎 **Por que o Nidhogg importa (posicionamento — decisão Pacman).** É a **camada analítica** — o
> **ponto de virada onde o projeto vira produto de valor ($$$)**. O núcleo (`ragd`) é OSS e roda em
> qualquer lugar; o Nidhogg é onde o **open source subsidia seus usuários**: gera **análises concretas,
> assistidas por IA**, sobre qualquer assunto (código, livros, artigos), permitindo a um
> **consultor / estudante / empresa chegar embasado**. Quem liga a IA colhe entendimento que vale dinheiro.

### 5.1 Conceito & invariantes [FEITO: esqueleto]

- Processo **separado**, **"daemon de módulos"** (porta 11497 vai hospedar N módulos além do Nidhogg).
- Lê o corpus **sempre via API do `ragd`** (nunca disco) → independe de localização.
- **Nasce DESLIGADO** (níveis ≥1 consomem IA). Liga/desliga **global** e **por coleção** (não re-mastiga
  a mesma N vezes). Keepalive pinga o `ragd` a cada 15s e cacheia (status nunca faz curl ao vivo).
- **Dois dials ortogonais:** **nível** (profundidade) + **cadência** (segundos entre ciclos = orçamento
  de tempo).

### 5.2 Natureza & consumo — o Nidhogg é AUTÔNOMO; o leitor é HUMANO

> **Decisão (Pacman):** o Nidhogg é um **projeto autônomo**, um **analisador crítico**. O `ragd`
> **NUNCA** o consome — daemons desacoplados. O valor está no **artefato em si**; **não depende** de
> ser consumido por outra máquina. *"Não interessa se alguém vai consumir ou não"* — o **entendimento
> acumulado É o produto** (como um caderno de erudito que engorda sozinho). Isso responde a crítica do
> Codex pela raiz: o consumidor é o **humano que lê**, não um sistema.

- **Consumidor = o humano**, via ValHalla (e export): abre e **lê** os artefatos destilados.
- **Artefatos de primeira classe** (entregáveis, não índice auxiliar de busca):
  - **Documento vivo** (nível **propositivo**): cresce **indefinidamente** a cada ciclo. Caso de uso do
    Pacman: *abrir depois de 15 dias e ler um resumo profundo de uma obra (ex.: O Senhor dos Anéis), com
    nuances de detalhe, em estilos (moderno, arcaico…)* — um "companion" que aprofunda no tempo.
  - **Árvore de conhecimento / mapa mental** (nível **estrutural**): navegável, partindo da obra — vale
    pra **código-fonte, texto, livro, artigo**, qualquer ingestão da base.
- **`GET /api/nidhogg/knowledge?collection=&type=&level=`** serve esses artefatos (pra ValHalla e export).
- O `ragd` **não lê nem injeta** isso na busca. Se um dia um agente quiser usar os artefatos como
  contexto, ele lê pela API do Nidhogg — **uso secundário e opcional**, não a razão de existir.

### 5.3 `source_hash`, diff e incrementalidade [FUTURO]

Kimi e Codex convergiram: detectar mudança real **barato**, sem falso-positivo, e digerir **só o que mudou**.

- **`state_hash` por base** = `hash(base_name, n_chunks, vocab_size, corpus)` — barato, vem direto do
  `GET /bases` (não lê o conteúdo). **Nunca usa path** (rename de path não muda; `base_name` é id estável).
- **`source_hash` da coleção** = hash da lista **ordenada** dos `state_hash` das suas bases.
- **Diff por ciclo:** compara o checkpoint anterior (`{base → state_hash}`) com o atual → bases
  **novas / alteradas / removidas**. Processa só as mudadas; marca órfãs (removidas); mantém as intactas.
- **Não re-mastiga** coleção/base com `state_hash` igual ao último → economiza IA (cadência ≠ re-trabalho).

> ⚠️ **Furo de contrato achado (Kimi):** o `ragd` **não expõe hoje** `idf`/`dims`/vocabulário efetivo por
> base num endpoint — o nível 0 teria que **sondar** via `/search` com sílabas-probe (caro). **Decisão:**
> adicionar um endpoint de **perfil** no `ragd` → `GET /profile?collection=&base=` retornando
> `{vocab_size, vocab_used, dims, top_idf:[{dim,syllable,idf,df}]}`. Alimenta o nível 0 barato. **[FUTURO — contrato novo no ragd]**

### 5.4 Os 4 níveis — algoritmos e schemas

| nível | nome | IA? | produz | status |
|---|---|---|---|---|
| 0 | **burro** | não | 3 pilares: RootIndex · CorpusDict · CacheDigest | [PARCIAL] |
| 1 | **consciente** | sim | `Summary` por coleção (saber que sobrevive à deleção) | [FUTURO] |
| 2 | **estrutural** | sim | **Árvore de conhecimento / mapa mental** da obra (`KnowledgeTree`) | [FUTURO] |
| 3 | **propositivo** | sim | **Documento vivo incremental** (`LivingDocument`, cresce no tempo) + `Gap`/`Suggestion` | [FUTURO] |

**Nível 0 (sem IA) — os 3 pilares.** ⚠️ **Honestidade (Codex):** nível 0 é **navegação / índice /
health-check** ("minha coleção está íntegra e navegável?"), **não "conhecimento"** — não vender como tal.
Mesmo assim entrega valor sozinho (base pros níveis IA + observabilidade) e custa zero IA.

> 🌱 **A semente do Nidhogg (origem da ideia).** O nível 0 é o pedaço que devolve ao RAGnaRock **coleções
> organizadas sobre as próprias coleções** — um **agente de auto-organização autônomo** do RAG sobre si
> mesmo. Foi daqui que o Nidhogg nasceu. Por isso o salto **0→1 nunca tem gate de aceite** (§5.4): não há
> o que um humano aprovar quando o RAG só está se arrumando pra si próprio.

- **RootIndex** — sílabas/dims mais salientes por coleção (rank por `idf × freq`), agrupadas por raiz.
  `content:{ bases_count, total_chunks, roots:[{stem, dims, df_chunks, idf_score, bases}], coverage_ratio }`.
- **CorpusDict** — vocabulário efetivo (dims usadas, top por `idf`, cobertura/`oov` por base, dims
  compartilhadas vs únicas). `content:{ vocab_size, active_dims, top_idf:[{dim,syllable,idf,df}], shared_dims, unique_dims }`.
- **CacheDigest** — consolida o cache de query-expansion: sinônimos vistos ≥ N vezes que mapeiam os
  **mesmos** chunks viram clusters de equivalência. `content:{ entries:[{canonical, variants, shared_chunk_ids, hit_count}], hit_rate }`.

**Níveis 1–3 (IA) — entrada, amostragem e saída:**

| nível | entrada pro LLM | amostragem | saída (`type`) |
|---|---|---|---|
| 1 | chunks **novos/alterados** desde o `source_hash` + meta da base | até `MAX_CHUNKS_PER_LEVEL` (~100) espaçados + top-N por `idf`; se poucos, todos | `Summary {themes, entities, key_chunks, abstract, chunk_range}` |
| 2 | `Summary` de nível 1 da obra/coleção (metadado — não amostra) | — | `KnowledgeTree {root, nodes[], edges[]}` — mapa mental navegável (hierarquia/encaixe de dimensões é a base) |
| 3 | a obra + `KnowledgeTree` + `Summary` + a versão anterior do documento vivo | incremental: só o que entrou desde o último ciclo | `LivingDocument {sections[], style, version, grows:true}` (resumo profundo que cresce, variantes de estilo) + `Gap`/`Suggestion` |

- **Orçamento:** cadência = orçamento de **tempo** por ciclo; somar teto de **tokens/ciclo** para os níveis IA.
- **Incremental:** nível 1 processa só chunks novos; nível 0 reprocessa a base alterada inteira (é barato).
- **Ordem HIERÁRQUICA (1→2→3):** os níveis IA acontecem **em sequência** — não há nível 2 sem o 1, nem 3
  sem o 2 (a dimensão do conhecimento é hierárquica por natureza). O **dial seleciona o nível-topo**; o
  worker roda `1..N` em ordem dentro do ciclo. Nível 0 (sem IA) é sempre a base.
- **Aditivo, versionado + gate de aceite (decisão Pacman, ciclo 4):** o artefato de cada dimensão é
  **versionado** — toda re-derivação cria uma `version` nova e arquiva a anterior como `frozen_version`
  (§5.7); o conjunto de versões **só acumula** (aditivo), mesmo quando o corpo ativo é substituído. Entre
  uma dimensão e a seguinte há um **gate de aceite opcional, ligável POR DIMENSÃO** (`accept_gate` =
  conjunto de níveis com gate, no ValHalla) — **não** global e **não** por item gerado. Ligar o gate na
  dimensão N significa: *o artefato de N só libera a dimensão N+1 depois de aprovado*. **Só há dois pontos
  lógicos de gate: `accept_gate ⊆ {1, 2}`** (controlam 1→2 e 2→3):
  - **0→1 nunca tem gate** — nível 0 é auto-organização autônoma do RAG sobre si mesmo (a semente do
    Nidhogg, ver acima); não há o que aprovar.
  - **3 não tem gate** — é o nível-topo, não há dimensão seguinte pra liberar.
  - **1→2 raramente fica ligado na prática** — a dim. 1 emite *muito* mais artefatos (um `Summary` por
    coleção/base); aprovar tudo seria inviável. O gate em **2→3** é o palatável (bem menos artefatos).
  - **dimensão sem gate (default):** cascata automática — o artefato alimenta a dimensão seguinte no mesmo ciclo.
  - **dimensão com gate ON:** o artefato fica `pending` e **só libera a próxima dimensão após o aceite
    humano** (botão no ValHalla). É o checkpoint de qualidade — ex.: com gate na dim. 2, o humano valida a
    árvore **antes** de o documento vivo (dim. 3) nascer dela. O aceite é também o **sinal de utilidade**
    (fecha o feedback-loop apontado pelo Kimi) sem o `ragd` jamais consumir o artefato.
  - **Trade-off consciente (frisar):** ligar o gate **quebra o ciclo autônomo** (§5.2) e injeta
    **dependência humana** — o worm para e *espera* o aceite, deixa de andar sozinho. Isso pode ser
    aceitável (quero revisar antes de aprofundar) ou inaceitável (quero o worm 100% autônomo). Não há
    resposta certa: é por isso que é **feature opt-in**, uma troca *autonomia × controle* decidida caso a
    caso por quem opera — nunca imposta.
- **Prompt por nível = o TOM (decisão Pacman):** cada nível IA (1, 2, 3) tem um **prompt configurável**
  (editável no ValHalla / `nidhogg.cfg`: `prompt_consciente`, `prompt_estrutural`, `prompt_propositivo`)
  — é como você dita o **tom/estilo** de cada extração (ex.: moderno vs arcaico no `LivingDocument`).
- **Cascata-delta — 3 modos de re-derivação (decisão Pacman, ciclo 4):** quando um nível inferior muda, o
  superior re-deriva pelo vínculo `derived_from`/`digestion_id`, mas **não assume crescimento monotônico**
  (a crítica do Kimi: rastrear proveniência ≠ rastrear impacto semântico). O artefato cresce, **encolhe ou
  é refeito**, em três modos:
  - **aditivo** — anexa o delta (a obra ganhou conteúdo; o `LivingDocument` estende, a árvore ganha galho).
  - **substituição estrutural** — troca uma **seção / galho inteiro**, não só a ponta. É o caso comum em
    **código**: mudar uma linha estrutural reescreve a *linha de raciocínio* toda — *"a corda não só
    cresce; às vezes o trecho é trocado por completo"*.
  - **reescrita total** — a mudança invalida o framing (themes/entities centrais mudaram no nível 1);
    o artefato é **refeito do zero**.
- **O que é trocado nunca some:** a versão anterior do galho/documento vira `frozen_version` (preserva o
  histórico — §5.7); o **corpo ativo** é sempre o atual. O gatilho do modo: mudança aditiva → local;
  *reframing* detectado no nível 1 → substituição/reescrita. (Mecanismo fino de impacto — assinatura
  semântica por galho pra decidir local vs. global — fica **[FUTURO — implementação]**.)
- **Auto-melhoria embutida na camada propositiva (dispensa "Synthesis"):** a dim. 3 **lê a versão
  anterior do documento e a aprimora** — refinar É a análise propositiva. Por isso **não há mecanismo de
  consolidação à parte**: o risco que o Kimi levantou (documento vivo incha/repete/contradiz) se resolve
  por construção, dentro do próprio nível 3, a cada ciclo.
- **Grafo de IAs em confronto — exclusivo da camada propositiva (decisão Pacman, ciclo 4):** na dim. 3 o
  artefato final **não precisa sair de uma única IA**. O operador monta no ValHalla um **grafo de
  inferência**: nós = IAs disponíveis (providers plugáveis — Bedrock, Kimi, Codex, local…), arestas =
  **quem confronta / alimenta quem**, em **N níveis** de confronto até *"o artefato que gera o artefato
  final"*. É o padrão Side AI (gerador × crítico × árbitro) **institucionalizado dentro do Nidhogg**. Os
  artefatos intermediários do grafo são **insumo**; só o nó-raiz emite o `LivingDocument` versionado.
  - **Só vale para a dim. 3.** Dims 1 e 2 usam **IA direta** (1 chamada por extração) — confronto multi-IA
    é custo que só a camada propositiva justifica.
  - Conecta com o **provider plugável** do §5.9: aqui ele deixa de ser "escolher 1 modelo" e vira
    "orquestrar vários num DAG de confronto". Config do grafo no `nidhogg.cfg`/ValHalla. **[FUTURO]**

### 5.5 Ciclo do worker, arquivos e resumabilidade [FUTURO]

**Layout por coleção** (em `dir/`), append-only para ser resumível:

```
<dir>/
  <coll>.knowledge.jsonl    # 1 item de conhecimento por linha (escrita atômica, append)
  <coll>.checkpoint.json    # { base_name: state_hash } + última base processada
  <coll>.provenance.jsonl   # 1 digestão por linha
  <coll>.config.json        # { enabled, level, cadence_s, last_run, accept_gate:[⊆{1,2}] }
```

**Pseudo-fluxo de um ciclo** (sintetizado com Kimi):

```
para cada coleção HABILITADA:
  bases_atuais = GET /bases?collection=coll
  diff = compara(checkpoint_anterior, state_hash(bases_atuais))   # novas/alteradas/removidas
  digestion_id = uuid()
  nivel_topo = config[coll].level                       # dial = nível-topo (0..3)
  nível 0 (sempre): RootIndex + CorpusDict + CacheDigest da coleção → append no .jsonl
  para cada base ALTERADA: atualiza checkpoint[base] = state_hash
  # níveis IA em sequência ESTRITA, sempre por coleção/obra (nunca cross-coleção).
  # gate é POR DIMENSÃO: o nível N só roda se o N-1 não tem gate, ou sua versão está accepted:
  liberado(N) = ((N-1) not in accept_gate) or (versao_atual(N-1).status == "accepted")
  se nivel_topo >= 1 e liberado(1): Summary dos chunks NOVOS → nova version → append (pending|accepted)
  se nivel_topo >= 2 e liberado(2): KnowledgeTree da obra ← Summary (re-deriva o galho) → version → append
  se nivel_topo >= 3 e liberado(3): LivingDocument (anexa delta) + Gap/Suggestion
                                    ← KnowledgeTree + Summary + versão anterior → version → append
  grava provenance(digestion_id, inputs=bases_alteradas)
  recomputa saturation ; marca órfãos (bases removidas)
```

**Resumabilidade:** se a IA falha no meio, o `.jsonl` já gravado é válido (append atômico) e o
`checkpoint` aponta a última base concluída → o próximo ciclo **retoma** dali. Nunca append cego:
dedup por `digestion_id`/`derived_from`. O `<coll>.knowledge.json` consolidado (§5.6) é uma **view**
do `.jsonl`+checkpoint (ou o `.jsonl` vira o canônico e o `.json` é gerado). **[decisão de implementação]**

### 5.6 Schema do conhecimento consolidado — `<dir>/<coll>.knowledge.json`

Um arquivo **por coleção** (hoje: `{collection, enabled, source_hash, saturation, updated, provenance,
knowledge[]}`). Forma-alvo:

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
    { "type":"RootIndex|CorpusDict|CacheDigest|Summary|KnowledgeTree|LivingDocument|Gap|Suggestion",
      "level":1, "version":1, "created":"ISO8601", "content":{}, "confidence":0.0,
      "derived_from":["digestion_id"], "frozen":false,   // frozen=true quando a fonte morre/muda
      "status":"pending|accepted" }   // gate de aceite (§5.4): pending bloqueia o nível seguinte se accept_gate=on
  ]
}
```

### 5.7 Saturation, provenance, sobrevivência à deleção

- **`source_hash` (hash, não nome):** cada item de conhecimento aponta pra um hash do estado da fonte.
  Renomear/deletar a coleção **não invalida** o que já foi destilado; só marca que a fonte mudou.
- **`saturation` = (itens ainda verificáveis contra uma fonte viva) / (total de itens).** `→1.0` tudo
  ancorado; `<0.5` alerta de muito conhecimento **órfão**. Decai naturalmente se coleções somem.
- **Fonte morta → artefato CONGELADO (decisão Pacman):** quando a fonte some/muda, o destilado **nunca
  é apagado** — sobreviver à deleção é a *feature*. Vira `frozen:true` com **selo de frescor** (fonte
  **viva** / **alterada** desde X / **congelada** em X) pro leitor humano saber o estado. `saturation`
  é só esse **indicador de frescor**, **nunca** gatilho de poda.
- **Invariante:** nenhum item de nível ≥1 é gerado sem `provenance` (digestion_id + source_hash + modelo).
- **Cadência ≠ saturação:** worm não re-mastiga coleção saturada (`source_hash` igual ao último) — economiza IA.

### 5.8 API do módulo

**[FEITO]** `GET /health` · `GET /api/nidhogg` (status: nível, cadência, keepalive do ragd, conhecimento) ·
`GET /api/nidhogg/collections` (coleções + estado de digestão) · `POST /api/nidhogg`
(`{on, level, cadence}`) · `POST /api/nidhogg/collection` (`{collection, enabled}`) ·
`POST /api/nidhogg/run` (dispara ciclo — **stub**).

**[FUTURO]** `GET /api/nidhogg/knowledge?collection=&type=&level=` (consumo do saber destilado — §5.2) ·
`POST /api/nidhogg/accept` (`{collection, type, level, version}` → marca `status:accepted`, libera o
nível seguinte quando `accept_gate=on` — §5.4).

### 5.9 ⚠️ Riscos & questões em aberto (a crítica honesta — Codex)

O Nidhogg é a parte **mais arriscada** do projeto. Registrado de propósito, não escondido:

- **"Solução procurando problema?" — RESOLVIDO (ver §5.2).** O Nidhogg é **autônomo** e o consumidor é
  o **humano** que lê os artefatos (documento vivo, árvore de conhecimento). O valor é o **entendimento
  acumulado em si** — *não depende* de consumo por máquina; o `ragd` nunca o lê. Rastreável por
  `provenance`; o destilado nunca se mistura com a fonte sem rótulo.
- **Órfão/stale — ATENUADO (§5.2/§5.7).** Como o Nidhogg é autônomo (ninguém consome na busca), não há
  resultado a contaminar. Fonte morta → artefato **CONGELADO e rotulado**, nunca deletado (sobreviver é
  feature). `saturation` = rótulo de frescor pro leitor humano; nível 0 (sem IA) é à prova disso.
- **Custo/latência de IA — orçamento é DECISÃO DE QUEM RODA.** OFF por default; opt-in por coleção;
  nível 0 cobre o sem-IA; cadência + janela = *quando* roda. **DISCLAIMER obrigatório** ao ligar no
  ValHalla: *"ativar consome IA — qualquer provider"*. Provider **plugável** ([FUTURO]: Amazon Bedrock,
  escolha de modelo, round-robin). **Sem teto de gasto por default** — escolha consciente do usuário.
  Na **dim. 3** o **grafo de IAs em confronto** (§5.4) multiplica o consumo (N IAs × N níveis de confronto)
  — o disclaimer e a cadência pesam em dobro ali; é a camada $$$ por excelência.
- **Framing dos níveis pode confundir o operador — MITIGADO.** Como cada nível tem **prompt próprio**
  (§5.4) e a **aba Nidhogg** (§4) descreve o que cada um produz (consciente=`Summary`, estrutural=árvore,
  propositivo=documento vivo), o operador vê e edita o tom de cada camada — o framing fica explícito na
  tela, não escondido no código.
- **"O que avança e quando" — gated por roadmap + budget, não por consumidor.** O bullet antigo ("esperar
  um consumidor real") **caiu**: o consumidor é o **humano** e o artefato **é** o produto (§5.2). Os
  níveis 1→2→3 avançam conforme o **roadmap** do Pacman, **gated por orçamento + disclaimer** (§5.9 acima),
  não por "esperar quem consuma". Começa pelo nível 0 (sem IA, à prova de risco) e sobe na cadência que o
  dono escolher.

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
