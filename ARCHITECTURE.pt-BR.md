> 🌐 **Idioma.** Versão em **português (pt-BR)**. Versão principal em inglês: **[ARCHITECTURE.md](ARCHITECTURE.md)**.

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

### 2.1 Silabificação — o algoritmo do token [FEITO]

O token é a **sílaba**, produzida por um silabador determinístico de PT-BR em `ragd/src/tokenizer.rs`
(`syllabify`, `syllable_seq`, `normalize`). Regras efetivas:

- **Vogais/semivogais:** vogais `a e i o u` (+ acentuadas); fracas `i u` formam **ditongo**; `í ú`
  acentuados **quebram** o ditongo (forçam hiato).
- **Núcleos:** vogal forte + forte → **hiato** (separa: "co-a-lha"); fraca + forte ou inverso → **ditongo**
  (junta: "pou-so", "coi-sa").
- **Onsets:** dígrafos `ch lh nh` = **um** som; `qu`/`gu` + vogal alta → `u` mudo; encontros muta+líquida
  (`bl br cl cr dl dr fl fr gl gr pl pr tl tr vl vr`) ficam juntos no onset.
- **Coda × onset:** consoante isolada entre núcleos vira coda da anterior + onset da próxima; em grupo, as
  **2 últimas** vão pro onset seguinte **se** formam encontro válido, senão só a última.
- **Normalização = chave canônica:** minúsculas → Unicode NFD → remove diacríticos. "Narnia"→"narnia",
  "Élrond"→"elrond" — acento **não** cria dimensão distinta.

> As PoCs `python_concept/` e `rust_concept/` são **referência congelada** (validação histórica); a spec
> viva é a do `ragd`. Regras completas, ~90 casos-de-ouro e o teste de conformance (`cargo test
> syllable_golden`) vivem em **[`specs/SYLLABLE.md`](specs/SYLLABLE.md)** — incluindo as divergências
> documentadas (ditongos nasais etc.).

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
  | `storage` | `memory` | `memory` (cacheia tokens) \| `hybrid` (recomputa) \| `disk` (texto via mmap, #41) |
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
   O `vec` do chunk guarda **contagem crua** e sua `norm` é a do vetor **tf-idf** — escalas diferentes.
   Para o cosseno fechar em `[0,1]`, o `ragd` dobra o `idf` no lado da **query**, usando a identidade
   `Σ (tf_q·idf)(tf_c·idf) = Σ (tf_q·idf²)·tf_c`: o peso extra sai de graça (calculado uma vez por busca)
   e o caminho quente segue batendo no `tf` cru. **As PoCs `python_concept`/`rust_concept` ainda usam o
   esquema antigo** (tf-idf da query × tf cru, sobre norma tf-idf), que não é cosseno e passa de 1 —
   os JSONs seguem intercambiáveis campo a campo, mas os **rankings divergem** a partir de 05/ago/2026.
2. **Rerank (matched filter fonético por proximidade):** sobre os candidatos, mede a **menor janela**
   que cobre um casamento de cada palavra da query (proximidade), **ignorando monossílabos** (stopwords),
   com soundex opcional (`phonetic`). Score combina cobertura + proximidade. Devolve top-`k`.

- **Fonética do rerank (SOUNDEX — `ragd/src/rag.rs`):** com `phonetic:true`, dois termos casam quando têm
  o **mesmo código SOUNDEX** (consoantes mapeadas 1–6; vogais/`h`/`w` = 0; retenção clássica em `h`/`w`;
  truncado a 4). Aplicado **só a termos de ≤3 sílabas** (nomes/grafias variantes: `"Aslan"`→`"Aslam"`);
  termos longos discriminam pela própria sequência silábica (evita falso casamento `ressurreição`~`rigorosa`).
  Calculado **1× por query** (não por candidato). É feature **do `ragd`** — não existe nas PoCs congeladas.
- **Recall unificado por coleção (`unified`, DEFAULT quando o escopo tem coleção com >1 base — [#8]):** em vez do idf local de cada base, o recall
  roda num **espaço unificado por coleção** — um `CollectionProfile` (vocab dos drivers das bases mesclado +
  idf recomputado sobre a coleção inteira = "idf de repo"), em memória e cacheado, auto-invalidado por um
  fingerprint `(nº bases, total chunks)`. O `vec` de cada chunk é remapeado local→global on-the-fly. Permite a
  query casar entre **arquivos de linguagens diferentes** (ex.: Python + Rust na mesma coleção) com idf de repo
  discriminante. O rerank não muda.
  **Passou a ser o default em 05/ago/2026.** Sem ele cada base pontua na sua própria escala de raridade e o
  merge cross-base compara números incomparáveis: medido em 335 livros, `"whale"` abandonava o Moby Dick —
  onde a sílaba é comum (idf 0,038) — e subia em obras onde é rara (Mabinogion, 0,471). Identificação da obra
  certa no top-1: **45% local → 65% unificado**, ao custo de +17 ms no p50 e +0,5 GB de perfil cacheado.
  Liga só quando há coleção com **>1 base no escopo** (base única não tem o que unificar); opt-out explícito
  com `"unified": false`.

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
| `disk` | idem `hybrid` **sem o texto dos chunks** — o texto fica em `<base>-tokenized.textblob` (ao lado do JSON, regravado a cada carga) e é lido por mmap | o texto sai do heap; mesmos resultados do `memory`; busca na velocidade do `hybrid` | [FEITO] #41 |

- **Durabilidade:** a verdade está no disco (`ragfiles/`); RAM é cache → crash recupera no boot.
- **mmap — só do texto dos chunks, opt-in (`storage = disk`, #41):** o recall nunca toca texto (só
  vetores/idf/índice), então o texto é a única coisa que sai da RAM sem mexer no caminho quente.
  Segue **opt-in, nunca default**: o sistema é CPU-bound na silabificação, e o JSON continua a única
  fonte da verdade — o `.textblob` é derivado, regerado a cada carga (sem defasagem nem migração) e
  apagado junto com a base no `DELETE ?purge=1`. Os vetores ficam na RAM; índice inteiro em disco
  (estilo Qdrant) segue fora de escopo.
  Medido em 04/out/2026 com 335 livros (668 MB de JSON, Xeon E5-2680 v4): memória do processo após a
  carga `memory` 9,2 GB → `hybrid` 794 MB → `disk` 559 MB (+246 MB de páginas do arquivo após buscas,
  liberáveis pelo SO); 30 buscas: `memory` 7,1 s, `hybrid` 16,1 s, `disk` 16,3 s; resultados idênticos
  (cos até 1e-10; a ordem só muda em empate exato no corte do k, como `memory`×`hybrid` já fazia).
  Os ~1,5 GB que ficavam depois da 1ª busca (em todos os modos) eram o pico transitório da montagem do
  perfil da coleção — resolvido no #57: após as buscas `hybrid` 853 MB, `disk` 618 MB.
- **Pressão de memória:** o console mede RSS (`/proc/self/statm`) + estimativa text/vec/words; medido:
  ~580 bases ≈ 516 MB (`memory`) → 174 MB (`hybrid`). [FEITO]

### 3.5 Concorrência [FEITO]

- **Trava geral:** `parking_lot::RwLock<State>` (justa, #6) — N requisições de leitura em paralelo; `write()` só
  para config/auth/drivers e deleções.
- **Bases e perfis têm travas próprias e curtas (#37):** o mapa de bases é `BasesLock(RwLock<Arc<Bases>>)` com
  `Arc<RagBase>` dentro, e o cache de perfis por coleção guarda `Arc<CollectionProfile>`. A busca pega uma
  **fotografia** (copia ponteiros) e solta a trava na hora; roda sobre a fotografia o tempo que precisar. Quem
  escreve copia o mapa de forma rasa (só ponteiros) e troca — nunca espera uma busca em andamento, então a trava
  justa não enfileira mais as buscas novas atrás da escrita.
- **Ingestão em duas fases (#37):** `/ingest`, `/ingest_file` e uploads **preparam** sem trava (driver, tokenização,
  gravação do JSON, carga — a parte de segundos; as ingestões só fazem fila entre si para duas não gravarem o mesmo
  arquivo) e **efetivam** em microssegundos sob a trava geral de LEITURA (insere + invalida só o perfil daquela
  coleção; antes, toda ingestão invalidava todos). A remontagem de perfil também é fora da trava do cache, uma por vez.
- **Medido (#37, Aron, 335 livros, 8 clientes simultâneos, livros de 5 MB entrando em outra coleção):** busca p50
  durante a ingestão 5,7 s → 1,1 s (= p50 sem ingestão); buscas atendidas na janela 64 → 119; respostas
  idênticas byte a byte.
- **Em aberto:** latência de cauda com muitas buscas globais simultâneas (laços `rayon` aninhados sobre bases e
  trechos dividem um pool; aparece com ou sem ingestão) — tratado à parte.

### 3.6 Drivers de linguagem [FEITO]

- Tokenização de **código-fonte** usa `.drv`: sílabas da base `SourceCode` (PT + sílabas de código) +
  **keywords reservadas** da linguagem. `tokens_PTBR.drv` e `tokens_SourceCode_PTBR.drv` são a **matriz
  fixa**; os demais derivam via `tools/gen_drivers.py`. `GET /interpret?file=foo.py` roteia extensão →
  driver/linguagem.

### 3.7 Contrato HTTP — rotas

> 📐 **Contrato detalhado** (request/response de cada rota dos 3 daemons): **[`JSONCONTRACT.pt-BR.md`](JSONCONTRACT.pt-BR.md)**.
> Exemplos executáveis `curl -d @`: `ragd/json_samples/`. Abaixo, o resumo das rotas.

**Implementadas [FEITO]:**

| método | rota | função |
|---|---|---|
| GET | `/health` | `{status, bases, collections, drivers}` |
| GET | `/bases` `?collection=&match=` | lista bases (com `corpus`, `n_chunks`...) |
| GET | `/bases/{coll}/{name}` | metadados de 1 base sem buscar (#4) |
| GET | `/collections` | resumo por coleção |
| GET | `/profile` `?collection=&base=` | **perfil léxico** (`vocab_used`, `dims`, `top_idf[]`) que alimenta o nível 0 do Nidhogg (#1, §5.3) |
| GET | `/stats` | agregado público (#3) |
| GET | `/drivers` `?match=` | lista drivers |
| GET | `/interpret` `?file=\|?ext=` | extensão → driver |
| POST | `/search` | busca (recall+rerank) com estágios opt-in `expand`, `phonetic`, `literal_fallback` (#39) |
| POST | `/search_expand` | atalho para `/search` com `expand: true` |
| POST | `/ingest` · `/ingest_file` · `/ingest_upload` | ingestão (inclui `append=true`) |
| POST | `/ingest_any` · `/transcribe` | ingestão por driver de `ingestors/` (MIME → extensão) · transcrição de áudio |
| POST | `/chunk` | recupera chunk(s) inteiro(s) por id (`before`/`after`) |
| DELETE | `/bases/{nome}` `?collection=` | remove base |
| DELETE | `/collections/{nome}` | remove a coleção inteira |

**A definir/faltam [FUTURO]:**
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

### 5.1.1 Diário de mastigação (`llm-ledger.jsonl`) — retenção (#15)
Toda chamada de IA (classificador, modelador, relações, analista, comparador) grava uma linha em
`<dir>/llm-ledger.jsonl`, com prompt e resposta **completos**. Para não crescer sem limite:
- ao passar de `llm_ledger_max_mb` (padrão 20) ele **gira** para `llm-ledger-AAAAMMDD-HHMMSS.jsonl`,
  e só os `llm_ledger_keep` (padrão 6) girados mais novos ficam no disco;
- `llm_ledger_meta = colA,colB` faz essas coleções gravarem **só metadados**: papel, coleção, latência,
  tamanhos e uma impressão digital (FNV-1a) do prompt e da resposta, sem texto nem nome de base. O L4
  com escopo amplo (`*`) cai em metadados se qualquer coleção estiver marcada;
- `llm_ledger = full | meta | off` vale para tudo (padrão `full`, o comportamento anterior).
O ValHalla (`/api/nidhogg/llm_ledger`) lê o arquivo corrente; as entradas só de metadados aparecem sem texto.

### 5.1.2 Bases renomeadas e removidas (#14, #21)
Classe, entidades e nós são guardados pelo **nome** da base. A cada ciclo (nível ≥ 1), antes de classificar,
o Nidhogg compara as bases do `ragd` com as do ciclo anterior (`bases_vistas` no `knowledge.json`):
- **Renomeada:** some uma base e aparece outra com o **mesmo conteúdo** (`n_chunks`, `vocab_size`, `corpus`),
  com casamento único → relink automático: a classe (inclusive a re-tipagem humana), as entidades e os nós
  passam para o nome novo, sem chamar IA. Fora disso (conteúdo também mudou, ou mais de um candidato):
  `POST /api/nidhogg/relink {"collection","de","para"}`.
- **Removida:** nome que o store conhece e o `ragd` não tem mais entra em `ausentes` com o instante em que sumiu.
  Passada a carência `prune_grace_h` (padrão 24 h; `0` = no mesmo ciclo; negativo = nunca), classe, entidades
  e nós dela são apagados em lote. Se a base voltar antes, nada se perde.
- O diff do último ciclo com mudança (novas, mudadas, removidas, renomeadas, podadas) fica em `l0_diff`.

O L0 (RootIndex/CorpusDict) continua sendo da coleção inteira: vocabulário unificado e idf mudam com qualquer
base e custam duas chamadas ao `ragd`. O trabalho caro (classificar, extrair, recensear) já é por base, pelo `state_hash`.

### 5.1.3 Porteira Laya do L1 — treino recorrente (#53)
Um modelo de decisão local (Laya, Apache 2.0, base mmBERT/MIT) aprende a responder "qual tipo?" e "segue o tipo?"
a partir das classes já gravadas, para decidir sem LLM as bases que ele reconhece com segurança. O treino é
**recorrente**, porque o banco é dinâmico: `tools/laya_porteira/agendado.sh` roda agendado na Aron (cron do usuário),
todo local e offline (Laya base, pacotes e `SHA256SUMS` em `/dados/modelos/laya`).
- Rótulos: `doc_class FINAL` (a re-tipagem humana prevalece); texto = os 1000 primeiros caracteres do chunk 0,
  o mesmo que o classificador LLM vê.
- Gatilho: só treina com ≥ `--min-novos` rótulos novos desde o modelo atual. Tipos com ≥ `--min-por-tipo` bases são
  elegíveis; os demais entram como órfãos (negativos no treino, "não aceitar" na avaliação).
- Avaliação num conjunto separado por base. **Promove** a versão nova só com zero aceito-errado (conhecidos e órfãos)
  e cobertura ≥ à do modelo atual no mesmo conjunto. Cada tipo só é liberado com cobertura mínima e zero erro.
- Saída: `/dados/ragnarock/laya/versoes/<id>/` com `manifest.json` (tipos liberados, descrições, limiares, métricas,
  hash), `atual` aponta para a campeã, `historico.jsonl` registra toda execução.
- Na promoção, a versão é exportada (`exporta.py`): `onnx/laya.onnx`, `rust.json` (tokens especiais, temperaturas, limites)
  e `paridade.json` (decisões de referência do Laya oficial). Classes com origem `laya` não entram no treino.
- **Uso no `nidhoggd`** (opcional; sem `laya_dir` nada muda): `laya_dir = /dados/ragnarock/laya/atual`,
  `laya_ort_lib = <libonnxruntime.so>` (a do ambiente de treino; nada é baixado), `laya_threads`. Inferência em Rust
  puro (crate `ort` com a biblioteca carregada dinamicamente + `tokenizers`); a sequência reproduz a do Laya token a token.
  No `mine_classes`, antes do LLM: planilha não passa; escolhe entre os `tipos`, aceita só tipo `liberado` com confiança
  e "segue" acima dos limiares → classe com origem `laya`; o resto vai ao LLM como antes. Recarrega sozinho quando `atual` muda.
  `nidhoggd --laya-check <versão>` prova a paridade com o `paridade.json`.

### 5.2 Natureza & consumo — o Nidhogg é AUTÔNOMO; o leitor é HUMANO

> **Decisão (Pacman):** o Nidhogg é um **projeto autônomo**, um **analisador crítico**. O `ragd`
> **NUNCA** o consome — daemons desacoplados. O valor está no **artefato em si**; **não depende** de
> ser consumido por outra máquina. *"Não interessa se alguém vai consumir ou não"* — o **entendimento
> acumulado É o produto** (como um caderno de erudito que engorda sozinho). Isso responde a crítica do
> Codex pela raiz: o consumidor é o **humano que lê**, não um sistema.

- **Consumidor = o humano**, via ValHalla (e export): abre e **lê** os artefatos destilados.
- **Artefatos de primeira classe** (entregáveis, não índice auxiliar de busca):
  - **Questões diretas & documento vivo** (nível **propositivo** — L4): questões (`tabular`, `oneshot`,
    `vivo`) respondidas a cada ciclo sobre o conhecimento acumulado, onde cada mudança de perspectiva vira
    etapa na timeline (`nidhogg.resposta`). Caso de uso do Pacman: *abrir depois de 15 dias e ler um
    resumo profundo de uma obra (ex.: O Senhor dos Anéis), com nuances de detalhe, em estilos (moderno,
    arcaico…)* — um "companion" que aprofunda no tempo.
  - **Árvore de conhecimento / mapa mental** (níveis **estrutural** — L2, determinístico — e
    **estrutural-llm** — L3, relações por IA): grafo navegável (`no_valor`, censo de menções, dimensões e
    lacunas), partindo da obra — vale pra **código-fonte, texto, livro, artigo**, qualquer ingestão da base.
- **`GET /api/nidhogg/knowledge?collection=&type=&level=`** e as rotas analíticas (`/classes`, `/entities`,
  `/tree`, `/relacoes`, `/dimensoes`, `/perguntas`, `/respostas`) servem esses artefatos (pra ValHalla e export).
- O `ragd` **não lê nem injeta** isso na busca. Se um dia um agente quiser usar os artefatos como
  contexto, ele lê pela API do Nidhogg — **uso secundário e opcional**, não a razão de existir.

### 5.3 `source_hash`, diff e incrementalidade [FEITO]

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
> `{vocab_size, vocab_used, dims, top_idf:[{dim,syllable,idf,df}]}`. Alimenta o nível 0 barato. **[FEITO — #1, ver §3.7]**

### 5.4 Os 5 níveis — algoritmos e schemas

| nível | nome | IA? | produz | status |
|---|---|---|---|---|
| 0 | **minerador** | não | 3 pilares: `RootIndex` · `CorpusDict` · `CacheDigest` | [FEITO] |
| 1 | **consciente** | sim | Classificação `{natureza, tipo}` (`doc_class`, porteira Laya + LLM), moldes (`template`) e extração normalizada (`entidade`, medida por `NQI`) | [FEITO] |
| 2 | **estrutural** | não | Grafo determinístico (`no_valor`, `KnowledgeTree`), censo de menções (`mencao`), co-ocorrência de cena, **dimensões** e lacunas (`gaps`) | [FEITO] |
| 3 | **estrutural-llm** | sim | Relações por IA nas cenas densas do censo (`{a, rel, b, tema}`, com âncora determinística) gravadas no mesmo grafo (`no_valor`) | [FEITO] |
| 4 | **propositivo** | sim | Questões diretas (`pergunta`: `tabular` · `oneshot` · `vivo`) sobre contexto determinístico (dump + pistas L3 + prova `/search`+`/chunk`), com analista + comparador e timeline (`resposta`) | [FEITO] |

> 🏗️ **ESCALA (Hinode, milhões de docs) — decisões 09/ago e 13/ago/2026.** Para a escala real, a régua foi
> cravada em **5 níveis cumulativos (`0..4`)** apoiados no ClickHouse:
> - **L0 (`minerador`, sem IA):** minera a estrutura léxica do corpus (`RootIndex`, `CorpusDict`, `CacheDigest`).
> - **L1 (`consciente`, com IA):** classifica cada base em `{natureza, tipo}` com assinatura estrutural `forma`
>   (via porteira Laya local sem LLM quando reconhecida com segurança, §5.1.3, ou LLM leve com vocabulário
>   editável de `doctypes`), cria/mantém moldes regex (`template`) sob gatilho (cobertura ≥ 70% ou
>   `POST /api/nidhogg/molde`) e extrai registros deterministicamente (`csv=1` tabular ou `csv=0` via molde,
>   zero LLM linha-a-linha), pontuando cada registro pelo **NQI** (Normalization Quality Index).
> - **L2 (`estrutural`, sem IA):** grafa as relações **deterministicamente** sobre o dado normalizado — censo de
>   menções em prosa (`mencao`, no miolo 5%–95% da obra), nós de valor (`no_valor`), co-ocorrência de cena e
>   eixos de **dimensões** (`/dimensoes`, `/dimensoes/gaps`).
> - **L3 (`estrutural-llm`, com IA):** o mesmo grafo do L2, mas **100% LLM-bound** para o que o determinístico
>   não alcança — toma as cenas mais densas do censo do L2, destila relações `{a, rel, b, tema}` via LLM e aplica
>   **âncora determinística** (o censo veta pontas fora das entidades presentes na cena e verbos de ligação puros)
>   antes de gravar no mesmo grafo (`no_valor`, com selo de origem).
> - **L4 (`propositivo`, com IA):** responde às **questões diretas** cadastradas pelo operador (`tabular`,
>   `oneshot`, `vivo`). É determinístico no ponto de inferência (`l4_contexto` monta agregados/amostra do dump +
>   pistas do L3 + trechos inteiros de **prova** via `/search` e `/chunk`), o **analista** responde com `fontes`
>   obrigatórias e sugere `proximas` perguntas-filhas, e nas questões `vivo` o **comparador** só grava nova etapa
>   na timeline (`nidhogg.resposta`) quando a perspectiva muda (com gate de saturação por fingerprint `l4_visto`).
> Design de escala em [`docs/2026-08-09_nidhogg-arquitetura-escala.md`](docs/2026-08-09_nidhogg-arquitetura-escala.md).

**Nível 0 (`minerador`, sem IA) — os 3 pilares.** ⚠️ **Honestidade (Codex):** nível 0 é **navegação / índice /
health-check** ("minha coleção está íntegra e navegável?"), **não "conhecimento"** — não vender como tal.
Mesmo assim entrega valor sozinho (base pros níveis superiores + observabilidade) e custa zero IA.

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

**Níveis 1–4 — entrada, amostragem e saída:**

| nível | nome | entrada | amostragem / motor | saída (ClickHouse / API) |
|---|---|---|---|---|
| 1 | `consciente` (IA) | chunk 0 (1000 chars) + assinatura estrutural `forma` das bases novas/alteradas | Porteira Laya local (§5.1.3) ou LLM (`classificador`); moldes regex por `tipo@forma` (`modelador`, cobertura ≥ 70%); extração determinística | `doc_class {natureza, tipo, forma, csv, extraivel}` · `template` · `entidade` (`nqi`, `campos`) |
| 2 | `estrutural` (sem IA) | registros de `entidade` (L1) + chunks de bases narrativas (miolo 5%–95%) | Determinístico (zero IA): censo de sintagmas nominais/técnicos + indexação reversa de valores e co-ocorrência por `(base, idx)` | `mencao` · `no_valor` (`KnowledgeTree`: `/tree`, `/suggest`, `/node`) · `dimensoes` e `/dimensoes/gaps` |
| 3 | `estrutural-llm` (IA) | cenas mais densas do censo do L2 (chunks com mais entidades co-ocorrendo) | Até 4 janelas da base narrativa mais densa pendente por ciclo (`relacoes`) + filtro de âncora determinística | `entidade` (`tipo="relacao": {a, rel, b, tema}`) colada no mesmo grafo `no_valor` (`/relacoes`) |
| 4 | `propositivo` (IA) | questão ativa (`pergunta`) + dump estruturado (L1) + pistas de relações (L3) + chunks de prova do `ragd` | Contexto montado por regra (`l4_contexto`) + gate de saturação (`l4_fingerprint`); LLM `analista` + LLM `comparador` (em questões `vivo`) | `resposta` (etapas versionadas na timeline com `fontes[]` obrigatórias e `proximas[]`) |

- **Orçamento é DECISÃO DE QUEM RODA:** cadência = orçamento de **tempo** por ciclo; somar teto de
  **tokens/ciclo** para os níveis IA. Nasce **OFF**, opt-in por coleção (§5.1), níveis 0 e 2 são zero-IA, e
  **sem teto de gasto por default** — escolha consciente, com **disclaimer obrigatório ao ligar** (§4). O
  grafo de IAs multiplica o consumo (N IAs × N níveis) — é a camada $$$ por excelência.
- **Rate-limit = a própria configuração das dimensões (decisão Pacman):** não há rate-limiter à parte. O
  dono define **quando cada dimensão dispara dentro do ciclo** — *isso já é* o controle de vazão/custo. Soma-se a
  isso o teto opcional de tokens/ciclo. Assim, "consumir muita IA" vira uma escolha explícita na config, não um acidente.
- **Incremental:** cada camada rastreia seu checkpoint por `(state_hash, cfg_hash)` (incluindo `llm_tag` e hash do
  prompt/vocabulário); o L4 só chama IA quando o fingerprint `(dump, corpus, pergunta, prompt, llm_tag)` muda.
- **Ordem HIERÁRQUICA (`0 → 1 → 2 → 3 → 4`):** os níveis acontecem **em sequência cumulativa** — o L1 classifica
  e normaliza sobre o L0; o L2 grafa deterministicamente sobre o L1; o L3 enriquece o grafo do L2 com relações
  por IA; o L4 responde questões cruzando o dump (L1), as pistas (L3) e a prova bruta (`/search` + `/chunk`). O
  **dial seleciona o nível-topo (`0..4`)**; o worker roda `0..N` em ordem dentro do ciclo.
- **Aditivo, versionado + gate de aceite (decisão Pacman, ciclo 4):** o artefato de cada dimensão é
  **versionado** — toda re-derivação cria uma `version` nova e arquiva a anterior como `frozen_version`
  (§5.7); o conjunto de versões **só acumula** (aditivo), mesmo quando o corpo ativo é substituído. Entre
  uma dimensão e a seguinte há um **gate de aceite opcional, ligável POR DIMENSÃO** (`accept_gate` =
  conjunto de níveis com gate, no ValHalla) — **não** global e **não** por item gerado. Ligar o gate na
  dimensão N significa: *o artefato de N só libera a dimensão N+1 depois de aprovado*:
  - **0→1 nunca tem gate** — nível 0 é auto-organização autônoma do RAG sobre si mesmo (a semente do
    Nidhogg, ver acima); não há o que aprovar.
  - **4 não tem gate** — é o nível-topo, não há dimensão seguinte pra liberar.
  - **1→2 raramente fica ligado na prática** — a dim. 1 emite *muito* mais artefatos (por base);
    aprovar tudo seria inviável. O gate antes da camada propositiva é o mais palatável (bem menos artefatos).
  - **dimensão sem gate (default):** cascata automática — o artefato alimenta a dimensão seguinte no mesmo ciclo.
  - **dimensão com gate ON:** o artefato fica `pending` e **só libera a próxima dimensão após o aceite
    humano** (botão no ValHalla). É o checkpoint de qualidade — o humano valida a árvore/grafo **antes** de a
    camada propositiva (L4) derivar dela. O aceite é também o **sinal de utilidade**
    (fecha o feedback-loop apontado pelo Kimi) sem o `ragd` jamais consumir o artefato.
  - **Trade-off consciente (frisar):** ligar o gate **quebra o ciclo autônomo** (§5.2) e injeta
    **dependência humana** — o worm para e *espera* o aceite, deixa de andar sozinho. Isso pode ser
    aceitável (quero revisar antes de aprofundar) ou inaceitável (quero o worm 100% autônomo). Não há
    resposta certa: é por isso que é **feature opt-in**, uma troca *autonomia × controle* decidida caso a
    caso por quem opera — nunca imposta.
- **Prompts por papel = o TOM (decisão Pacman):** cada papel de IA (`classificador`, `extrator`, `modelador`,
  `relacoes`, `analista`, `comparador`) tem um **template de prompt configurável** em `prompts.json` (editável no
  ValHalla via `/api/nidhogg/prompts/template`) — e o seu hash entra no checkpoint da camada correspondente.
- **Cascata-delta — 3 modos de re-derivação (decisão Pacman, ciclo 4):** quando um nível inferior muda, o
  superior re-deriva pelo vínculo `derived_from`/`digestion_id`, mas **não assume crescimento monotônico**
  (a crítica do Kimi: rastrear proveniência ≠ rastrear impacto semântico). O artefato cresce, **encolhe ou
  é refeito**, em três modos:
  - **aditivo** — anexa o delta (a obra ganhou conteúdo; a timeline ganha etapa, a árvore ganha galho).
  - **substituição estrutural** — troca uma **seção / galho inteiro**, não só a ponta. É o caso comum em
    **código**: mudar uma linha estrutural reescreve a *linha de raciocínio* toda — *"a corda não só
    cresce; às vezes o trecho é trocado por completo"*.
  - **reescrita total** — a mudança invalida o framing (themes/entities centrais mudaram no nível 1);
    o artefato é **refeito do zero**.
- **O que é trocado nunca some:** a versão anterior do galho/documento vira `frozen_version` (preserva o
  histórico — §5.7); o **corpo ativo** é sempre o atual. O gatilho do modo: mudança aditiva → local;
  *reframing* detectado no nível 1 → substituição/reescrita. (Mecanismo fino de impacto — assinatura
  semântica por galho pra decidir local vs. global — fica **[FUTURO — implementação]**.)
- **Auto-melhoria embutida na camada propositiva (dispensa "Synthesis"):** no L4 (`vivo`), o **comparador** lê a
  etapa anterior da resposta e confronta com a nova — só grava nova etapa quando a perspectiva realmente muda.
  Por isso **não há mecanismo de consolidação à parte**: o risco que o Kimi levantou (documento vivo
  incha/repete/contradiz) se resolve por construção, dentro do próprio nível 4, a cada ciclo.
- **Grafo de IAs em confronto — exclusivo da camada propositiva (decisão Pacman, ciclo 4):** no L4 o
  artefato final **não precisa sair de uma única IA** (hoje já opera com a dupla `analista` × `comparador`). O
  operador monta no ValHalla um **grafo de inferência**: nós = IAs disponíveis (providers plugáveis — Bedrock,
  Kimi, Codex, local…), arestas = **quem confronta / alimenta quem**, em **N níveis** de confronto até *"o
  artefato que gera o artefato final"*. É o padrão Side AI (gerador × crítico × árbitro) **institucionalizado
  dentro do Nidhogg**.
  - **Só vale para o L4.** L1 e L3 usam **IA direta** (1 chamada por extração) — confronto multi-IA
    é custo que só a camada propositiva justifica.
  - O **provider** aqui deixa de ser "escolher 1 modelo" e vira "orquestrar vários num DAG de confronto"
    (plugáveis: Bedrock, escolha de modelo, round-robin). Config do grafo no `nidhogg.cfg`/ValHalla. **[FUTURO]**

### 5.5 Ciclo do worker, arquivos e resumabilidade [PARCIAL]

**Layout em disco e ClickHouse:** `<dir>/<coll>.knowledge.json` guarda os pilares do L0, procedência e
`l0_diff`; `<dir>/prompts.json` guarda os templates de prompt; `<dir>/llm-ledger.jsonl` registra as chamadas de
IA (§5.1.1); o ClickHouse (`nidhogg.*`) guarda `doc_class`, `doctype`, `template`, `entidade` (com a view
`entidade_atual`), `no_valor`, `dimensao`, `pergunta` e `resposta`.

**Fluxo real de um ciclo (`run_cycle`):**

```
para cada coleção HABILITADA:
  nível 0 (sempre): RootIndex + CorpusDict → <coll>.knowledge.json
  se nivel_topo >= 1:
    reconcilia_bases (relink de renomeadas + poda após prune_grace_h — §5.1.2)
    L1 (consciente): mine_classes (porteira Laya / LLM) → mine_templates (1 molde/ciclo) → mine_entities (NQI)
  se nivel_topo >= 2:
    L2 (estrutural, zero IA): mine_fichas (censo de menções em prosa) → mine_links (grafo no_valor)
  se nivel_topo >= 3:
    L3 (estrutural-llm): mine_relacoes (relações por IA nas cenas densas, com âncora) → mine_links
global (uma vez por ciclo):
  CacheDigest (/expansions do ragd) → _cachedigest.json
  se nivel_topo >= 4:
    L4 (propositivo): mine_respostas (contexto determinístico → analista → comparador → timeline)
```

**Design futuro de layout append-only por coleção** (em `dir/`), para resumabilidade fina:

```
<dir>/
  <coll>.knowledge.jsonl    # 1 item de conhecimento por linha (escrita atômica, append)
  <coll>.checkpoint.json    # { base_name: state_hash } + última base processada
  <coll>.provenance.jsonl   # 1 digestão por linha
  <coll>.config.json        # { enabled, level, cadence_s, last_run, accept_gate:[...] }
```

**Resumabilidade:** no ClickHouse, cada etapa grava seu checkpoint (`state_hash` + `cfg_hash` + `llm_tag`),
então se a IA falha no meio de um ciclo, o próximo ciclo **retoma** das bases/perguntas pendentes sem
reprocessar o que já convergiu.

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

> Contrato detalhado em **[`JSONCONTRACT.pt-BR.md` §3](JSONCONTRACT.pt-BR.md#3-nidhoggd--inteligência-11497)**.

**[FEITO]**
- **Estado e controle (§3.1):** `GET /health` · `GET /api/nidhogg` (status, nível `0`–`4`, `levels[]`, cadência, keepalive do ragd e da IA) · `POST /api/nidhogg` (`{on, level, cadence}`) · `GET /api/nidhogg/collections` · `POST /api/nidhogg/collection` (`{collection, enabled}`) · `POST /api/nidhogg/run` (dispara ciclo assíncrono).
- **Conhecimento e prompts (§3.2):** `GET /api/nidhogg/knowledge` · `GET /api/nidhogg/cachedigest` · `GET /api/nidhogg/prompts` · `POST /api/nidhogg/prompts/template` · `GET /api/nidhogg/llm_ledger`.
- **Classificação, extração e grafo — L1/L2/L3 (§3.3):** `GET|POST /api/nidhogg/doctypes` · `GET /api/nidhogg/doctypes/uso` · `GET /api/nidhogg/classes` · `POST /api/nidhogg/reclass` · `POST /api/nidhogg/relink` · `POST /api/nidhogg/molde` · `GET /api/nidhogg/templates` · `GET /api/nidhogg/rejeitados` · `GET /api/nidhogg/entities` · `GET /api/nidhogg/relacoes` · `GET /api/nidhogg/tree` · `GET /api/nidhogg/suggest` · `GET /api/nidhogg/node`.
- **Dimensões — L2 (§3.4):** `GET|POST /api/nidhogg/dimensoes` · `POST /api/nidhogg/dimensoes/upsert` · `POST /api/nidhogg/dimensoes/remover` · `GET /api/nidhogg/dimensao/valores` · `GET /api/nidhogg/dimensoes/gaps`.
- **Perguntas e respostas — L4 (§3.5):** `GET|POST /api/nidhogg/perguntas` · `POST /api/nidhogg/perguntas/upsert` · `POST /api/nidhogg/perguntas/remover` · `POST /api/nidhogg/perguntar` · `GET /api/nidhogg/respostas` · `POST /api/nidhogg/respostas/limpar`.

**[FUTURO]** `POST /api/nidhogg/accept` (`{collection, type, level, version}` → marca `status:accepted`, libera o
nível seguinte quando `accept_gate=on` — §5.4).

### 5.9 Questões em aberto (pauta das próximas rodadas Side AI)

> Seção **viva**: registra só o que **ainda não foi decidido**. Quando uma questão fecha, ela **sai daqui
> e vira decisão no corpo** — não fica como "risco resolvido" (seria redundante). Os riscos dos ciclos
> 1–4 (solução-procurando-problema, órfão/stale, custo, framing) foram resolvidos e moram hoje em §5.1
> (off/opt-in), §5.2 (autonomia/consumidor humano), §5.4 (hierarquia, gate, grafo, orçamento, prompt por
> nível) e §5.7 (sobrevivência por congelamento).

- **Impacto semântico da cascata-delta** — quando uma fonte muda, decidir entre re-derivar *só o galho* ou
  *reescrever* hoje é heurística por *reframing* (§5.4). O mecanismo fino (assinatura semântica por galho
  pra medir propagação) é **[FUTURO — implementação]**.
- *(próximas rodadas Side AI registram aqui o que surgir.)*

---

## 6. Estratégias transversais & roadmap maior

- **Repo como base (não 1-arquivo-por-base) [FUTURO]:** schema de chunk ganha `file` + linhas; `meta`
  ganha mapa de arquivos com `sha`; `POST /ingest_file {base, file}` recomputa só aquele arquivo
  (remove os chunks antigos do `file`, insere os novos, atualiza `sha`); `POST /sync {base, path}` varre
  e atualiza só o que mudou. É o coração do "RAG de código com update por arquivo". O cliente com
  `.ragnarock/` + `sync` está na #34.
- **Ingestores acionados pela IA do usuário [FUTURO] (#10):** o agente dispara ingestão (repo, diff de git,
  arquivos específicos) via MCP / CLIs.
- **Importadores [FEITO] (#9, #50):** PDF, DOCX, XLSX, PPTX, CSV, áudio, MySQL e Postgres, como sidecar no
  servidor: o `ragd` chama o driver de `ingestors/` por shell (`POST /ingest_any`, roteamento MIME → extensão;
  ver [`ingestors/README.md`](ingestors/README.md)). Testes em `ingestors/tests/` e `tools/e2e_ingest.sh`.
- **Índice invertido no SSD [FUTURO] (#43):** postings por sílaba, para o recall escalar a milhões de chunks.
- **Interfaces SQL, GraphQL e compatível com o protocolo do MySQL [FUTURO] (#35, #36, #40):** consultar o RAG
  com `SELECT`, com GraphQL ou conectando um cliente MySQL (HeidiSQL, DBeaver) na porta 11500.
- **Build Windows [FUTURO] (#11):** Rust puro deve compilar; cuidar `/dev/urandom` (entropia Windows) e
  `log_file` default.
- **Testes [PARCIAL]:** `cargo test` em `ragd/` e `nidhoggd/`, `unittest` em `ingestors/tests/` e
  `tools/e2e_ingest.sh` (ponta a ponta, com MySQL/Postgres descartáveis em `--bancos`). Falta CI.
- **Deploy [FEITO]:** cross-compile (`cargo zigbuild --target {x86_64,aarch64}-unknown-linux-gnu.2.31`)
  + rsync do binário + launcher que sobe `ragd` + `nidhoggd` detached e redireciona stdout pro `log_file`.
- **Segurança [PARCIAL]:** trocar `admin/admin`; chaves só no `cfg` (gitignored); CORS aberto na 11497
  (rever ao expor); sessão do console com TTL (rotacionar cookie — [FUTURO]).

---

## 7. Apêndice — modos de falha

**Óbvios:** JSON de base corrompido (`.bak` gravado antes do overwrite; JSON inválido é pulado+logado no
load, o boot nunca cai — #12) · OOM (usar `hybrid`; os tetos configuráveis `max_bases` /
`max_chunks_per_base` recusam ingestões grandes demais — #13) · IA fora (níveis ≥1 degradam pro nível 0) ·
ragd fora (o keepalive do Nidhogg degrada gracioso, status do cache).

**Não-óbvios (Kimi/Codex):** silabificação divergente entre ingestão e busca (mesmo driver/vocab é
obrigatório — append herda o driver da base) · starvation de writers no RwLock (usar lock justo) ·
`source_hash` falso-positivo em rename (re-link manual [FUTURO]) · rerank lento em `hybrid` (aceitável;
medir) · `knowledge.json` crescendo sem fim (compactar provenance [FUTURO]) · nível 3 alucinando furos
inexistentes (confidence + auditoria humana).

---

## 8. Decisões pendentes (gatilho → ação)

| decisão | gatilho | opções |
|---|---|---|
| mmap/on-disk | corpus > ~80% RAM | **texto: feito (#41, `storage = disk`)**; vetores/índice em disco seguem futuros |
| `RwLock` + paralelismo inter-query | latência sob carga concorrente real | **feito: RwLock justo (#6) + fotografia das bases com travas curtas (#37)**; latência de cauda de buscas globais simultâneas segue em aberto (§3.5) |
| base = repo (N arquivos) | usar como RAG de código a sério | `file`+`sha` no schema; `/sync` |
| podar sinônimos idf-baixo no expand | expansão poluindo lookup | filtro por idf na cascata |
| Nidhogg níveis 1–4 (IA: 1, 3, 4; sem IA: 0, 2) | decisão do dono (orçamento + cadência) | começa no nível 0; sobe por dial (`0`–`4`), gated por budget+disclaimer (§5.2/§5.4) — **não** por "esperar consumidor" |

---

> *Documento vivo — incrementar a cada ciclo. Síntese curada a partir de contraposição Kimi (gerador)
> × Codex (crítico), com o leme mantido na missão: simples, inspecionável, roda em qualquer lugar.*
