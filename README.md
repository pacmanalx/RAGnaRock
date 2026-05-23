# 🪨 RAGnaRock

**RAG sobre a rocha.** Um RAG (Retrieval-Augmented Generation) construído **do zero, sem rede
neural** — só contagem e álgebra linear — usando a **sílaba** como token. Sem GPU, sem embeddings
que envelhecem, sem caixa-preta. Roda em qualquer hardware e dá pra inspecionar a olho nu.

> **O nome tem camadas.** `RAG` + `Ragnarök` + `Rock` (rock'n'roll) + **Rock = pedra**: um RAG sobre
> **base sólida**. Construído na rocha, não na areia (Mt 7:24-27) — enquanto os RAGs "SOTA" precisam
> de GPU e embeddings que desatualizam, este fica de pé em hardware modesto — um Raspberry Pi, por exemplo.

---

## Por que existe

A maioria dos RAGs depende de GPU, modelos de embedding de gigabytes e bancos vetoriais pesados —
o que **exclui quem não tem hardware bom**. O RAGnaRock vai pelo caminho oposto:

- **Roda em qualquer lugar** — é um binário pequeno (Rust, ~2 MB). CPU + RAM, sem GPU. Funciona
  em hardware modesto (**por exemplo**, um Raspberry Pi 3 ou um Optiplex de 2012) e também em
  Mac, Windows e Linux.
- **Inspecionável** — o "embedding" é um histograma de sílabas em JSON legível; dá pra ver o vetor,
  o `idf`, a árvore da busca. Nada de mágica.
- **Ensinável** — vem com uma trilha didática (`logic_path/`) que reconstrói cada princípio de RAG,
  passo a passo, partindo do zero.
- **Não envelhece** — sem embeddings pra reprocessar quando o modelo muda; ingestão é incremental.

> **Missão:** popularizar o conceito de RAG — acessível, transparente, rodável por qualquer um.
> O objetivo **não** é ser SOTA; é nivelar por cima e dar pra ensinar.

---

## Como funciona (em 30 segundos)

1. **Token = sílaba.** O texto é silabado (`ca`, `sa`, `tra`, `gan`...).
2. **Embedding = histograma esparso** (bag of syllables) — a contagem das sílabas de cada chunk.
3. **Busca, estágio 1 (recall):** cosseno **tf-idf** entre a query e os chunks.
4. **Busca, estágio 2 (rerank):** *matched filter* com soundex fonético — promove os chunks onde as
   palavras aparecem **em sequência e próximas**, não só presentes.
5. **Opcional — query expansion:** expande a consulta por sinônimos (dicionário → cache → IA) antes
   de buscar, pra ganhar recall sem pesar a latência.

Tudo é texto legível: você consegue abrir o JSON de uma base e entender exatamente o que indexou.

---

## Arquitetura

A mesma lib (`sylkit`) existe em três encarnações que produzem **resultados idênticos** campo a campo:

| Componente | O que é |
|---|---|
| `python_concept/` | PoC de referência em Python (só stdlib) — a "verdade" do algoritmo. |
| `rust_concept/`   | Porte Rust **congelado**, validado idêntico ao Python (serve de teste). |
| `ragd/`           | **Daemon de produção** (Rust): segura N bases em memória, busca/ingestão via **API HTTP JSON**. É onde o desenvolvimento acontece. |
| **ValHalla**      | Console web do `ragd` (visão, busca, ingestão, performance, drivers, logs). |
| `nidhoggd/`       | **Níðhöggr** — camada de **inteligência** (experimental): o *worm* que digere o conhecimento. Ver seção abaixo. |
| **MCP**           | Casca que pluga o RAGnaRock como ferramenta em agentes de IA (opencode, Claude, etc.). |
| `drivers/`        | Drivers de linguagem — tokenizam **código-fonte** (sílabas + keywords por linguagem). |
| `thesaurus/`      | Dicionários multilíngue + cross-lingual (para a query expansion). |
| `logic_path/`     | Trilha didática **00 → 10** (memorial congelado) que ensina cada princípio de RAG. |

> 📐 **Especificação completa** (os três daemons em detalhe, contratos JSON, estratégias de memória/disco,
> concorrência, modos de falha e roadmap): **[`ARCHITECTURE.md`](ARCHITECTURE.md)**.

---

## 🐉 Nidhogg — a camada de inteligência (experimental)

No mito nórdico, **Níðhöggr** é a serpente que rói as raízes de Yggdrasil. No RAGnaRock, o `nidhoggd`
é um *worm* (do bem) que **digere o conhecimento** das coleções e o destila num saber que **sobrevive
à deleção da coleção** — fechando a mitologia: *Ragnarök* (o RAG) tem seu próprio Níðhöggr.

**Como funciona:**

- **Processo separado** (porta **11497**, um "daemon de módulos"). Lê o corpus **sempre pela API do
  `ragd`** (nunca do disco) — então independe de onde os dados moram.
- **Nasce desligado** e liga **por coleção** (não fica re-mastigando a mesma). Tem dois "dials":
  **nível** (profundidade) e **cadência** (de quanto em quanto tempo mastiga).
- **Quatro níveis cumulativos:**
  | nível | precisa de IA? | o que destila |
  |---|---|---|
  | **0 · burro** | não | índice de raízes, dicionário do corpus, digestão do cache — **núcleo seguro, custo zero** |
  | **1 · consciente** | sim | insights e resumo por coleção (o saber que sobrevive à deleção) |
  | **2 · estrutural** | sim | hierarquia e encaixe de dimensões entre projetos/ingestões |
  | **3 · propositivo** | sim | acha furos, sugere, comenta, resume inteligente |
- O conhecimento destilado é persistido por coleção (`<coleção>.knowledge.json`) com **proveniência** e
  **saturação** — rastreável e auditável, com `source_hash` (referência por hash, não por nome) e coleta
  de órfãos, pra não contaminar resultados com saber obsoleto.

> **Estado:** esqueleto pronto (API, keepalive, estrutura de conhecimento, os 4 níveis e os dials); a
> inteligência por nível está em desenvolvimento. Nível 0 (sem IA) é o caminho seguro; níveis 1–3
> (com IA) são **opt-in e experimentais**. Desenho completo + riscos honestos em [`ARCHITECTURE.md`](ARCHITECTURE.md#5-nidhoggd--níðhöggr--camada-de-inteligência-11497-parcial).

---

## Build & run

```bash
# Daemon de produção (porta default 11499). Roda da raiz pra auto-carregar as bases de ragfiles/.
cd ragd && cargo build --release
./target/release/ragd

# Ingerir um arquivo bruto e buscar (PoC Rust)
cd rust_concept && cargo build --release
./rust_concept/target/release/embed_gen meu_corpus.txt --chunk 2048
./rust_concept/target/release/search_rag meu_corpus-tokenized.json "minha consulta" -k 5

# PoC Python (só stdlib)
python3 python_concept/embed_gen.py meu_corpus.txt --chunk 2048
python3 python_concept/search_rag.py meu_corpus-tokenized.json "minha consulta" -k 5
```

> Convenção do projeto: **todo script/binário chamado sem argumentos mostra o help** — nunca roda
> com defaults silenciosos.

---

## API do daemon (HTTP JSON)

`GET /health · /bases · /collections · /drivers · /interpret` ·
`POST /ingest · /ingest_file · /ingest_upload · /search · /search_expand · /chunk` ·
`DELETE /bases/{nome}`.

- Bases são organizadas por `collection/name`; busca é **scatter-gather** com wildcard
  (`"sd*"`, `"*"`) e merge por relevância.
- Cada hit traz `collection, base, corpus` (nome do arquivo), `path`, `chunk`, `matchpoint`,
  `snippet`, etc. — então a IA vai **direto no arquivo**.

Contrato completo + exemplos: **[`ragd/README.md`](ragd/README.md)** e `ragd/json_samples/`.

---

## Status

Projeto **novo** (nasceu em maio/2026), em desenvolvimento ativo — encare como **alpha**. O núcleo
funciona ponta a ponta: busca silábica (recall + rerank), daemon com API, console ValHalla, drivers
de código, query expansion, ingestão incremental e integração via MCP.

**No radar:** ingestão de repositório com update por arquivo, mais importadores (PDF/DOCX/XLSX),
concorrência (N buscas em paralelo) e build Windows de duplo-clique. Ainda **não** há suíte de
testes formal nem CI — a verificação de fidelidade é manual (gerar por um lado, ler/buscar pelo
outro; os campos batem exato).

---

## Licença

Código sob **[MIT](LICENSE.md)** — use, copie, modifique e distribua à vontade.

> ⚠️ **Dados de terceiros:** dicionários/thesaurus eventualmente incluídos podem derivar de fontes
> com licença própria (ex.: thesaurus em PT). Verifique a licença dos dados antes de redistribuí-los —
> a licença MIT cobre o **código**, não necessariamente os dados de seed.

---

## Autor

**Alexandre Pereira** — projeto pessoal, OSS.

*Construído sobre a rocha. 🤘*
