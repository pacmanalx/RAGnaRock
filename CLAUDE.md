# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## O que é

RAGnaRock é um RAG **construído do zero, sem rede neural** — só contagem e álgebra
linear — usando a **sílaba** como token. O embedding é um histograma esparso (bag of
syllables); a busca é recall por cosseno tf-idf (estágio 1) + rerank por matched filter
com soundex fonético (estágio 2). Tudo inspecionável a olho nu.

A mesma lib (`sylkit`) existe em três encarnações que produzem **resultados idênticos**
campo a campo (o mesmo JSON gerado por uma é lido pela outra sem conversão):

- `python_concept/` — PoC Python de referência (`sylkit/` + `embed_gen.py` + `search_rag.py`).
- `rust_concept/` — porte Rust **congelado** da PoC (validado idêntico ao Python; não evoluir).
- `ragd/` — o **daemon de produção**. Tem sua **própria cópia evoluível** da sylkit em
  `ragd/src/` (`tokenizer/vocab/vector/chunk/index.rs`); o motor de busca vive em `rag.rs`.
  É aqui que o desenvolvimento ativo acontece.

> Ao mexer no algoritmo, evolua **`ragd/src/`**. `rust_concept/` e `python_concept/`
> são PoCs de referência — alterá-los quebra a equivalência validada que serve de teste.

## Build & run

```bash
# daemon de produção (porta default 11499)
cd ragd && cargo build --release
./target/release/ragd            # AUTO-CARREGA todas as bases de ragfiles/ (rodar da raiz)
# opções: --port N (default 11499) --drivers-dir <p> --ragfiles-dir <p> --max-upload <bytes>
#         --no-autoload (sobe vazio) --preload nome=caminho.json (aditivo, repetível) --help

# PoC Rust (rodar da raiz, onde estão os corpora e os .drv)
cd rust_concept && cargo build --release
./rust_concept/target/release/embed_gen sda.txt --chunk 2048
./rust_concept/target/release/search_rag sda-tokenized.json "Frodo Bolseiro" -k 5

# PoC Python (só stdlib, exceto matplotlib no passo 06 da trilha)
python3 python_concept/embed_gen.py <corpus> --chunk 2048
python3 python_concept/search_rag.py <base>-tokenized.json "query" -k 5
```

Não há suíte de testes formal nem CI. A verificação de fidelidade é manual: gerar a base
por um lado e ler/buscar pelo outro — `vocab`, `vec`, `start`, `len`, `tokens` batem
exato; `idf`/`norm` dentro de `1e-6`. Os rankings/matchpoints do `ragd` batem com a PoC.

## API do daemon (HTTP JSON)

`GET /health · /bases · /collections · /drivers · /interpret` ·
`POST /ingest · /ingest_file · /ingest_upload · /search · /chunk` · `DELETE /bases/{nome}`.

Contrato completo + exemplos `curl -d @`: `ragd/README.md` e `ragd/json_samples/`.

Conceitos transversais da API:
- **Bases organizadas por `collection/name`.** Sem `collection`, busca/lista em todas;
  com `collection:"X"`, restringe.
- **Wildcard na base**: `"sda"` exata, `"sd*"` prefixo, `"*"` todas.
- **`/search` é scatter-gather**: busca em cada base que casa o escopo e faz merge dos
  hits por `matchpoint` global. Cada hit traz `base, rank, matchpoint, mf, span, cos,
  chunk, start, snippet`.
- **`/chunk`** recupera chunk(s) inteiros por `id` com `before`/`after` para montar contexto.
- Ingestão de arquivo bruto grava o JSON tokenizado em `ragfiles/<collection>/<name>-tokenized.json`.
- **Auto-load no boot (default):** o daemon varre `ragfiles-dir` na subida e carrega tudo
  (cada subdir = coleção, cada `*-tokenized.json` = base). O `name` é saneado (`safe_name`,
  sem ponto inicial) pra nunca virar dotfile e escapar do load. Persistência = os JSONs no disco.

## Drivers de linguagem (`drivers/*.drv`)

Tokenização de código-fonte usa **drivers**: cada `.drv` = sílabas da base `SourceCode`
(PT + sílabas de código) **+** keywords reservadas da linguagem (linhas marcadas com `=`).
`tokens_PTBR.drv` e `tokens_SourceCode_PTBR.drv` são as bases **fixas** (a "matriz" do
projeto); os demais derivam delas. O cabeçalho são 4 linhas `#` (nome, descrição, extensões, base).

`GET /interpret?file=foo.py` (ou `?ext=.py`) roteia uma extensão para o driver/linguagem.

Para adicionar uma linguagem: uma entrada em `LANG_INFO` dentro de `tools/gen_drivers.py`,
depois `python3 tools/gen_drivers.py` (ou `--only <Lang>`, `--list`). Reexecuta e o `.drv` aparece.

## Convenções

- **Script chamado sem argumentos mostra o help**, nunca roda com defaults silenciosos.
  Vale para todos os binários/scripts CLI deste repo.
- `logic_path/` é um **memorial congelado** da jornada de aprendizado (trilha numerada
  00→10 que ensina cada princípio de RAG). É didático, não código vivo — **não acople o
  produto a ele** nem o trate como dependência.
- O JSON sai sempre na **mesma ordem de chaves** (Rust usa `serde_json` com
  `preserve_order`; é o que garante a equivalência byte-a-byte com o Python).
