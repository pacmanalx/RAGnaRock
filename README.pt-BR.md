> 🌐 **Idioma.** Esta é a versão em **português (pt-BR)**. O README principal do projeto é em inglês:
> **[README.md](README.md)**.

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

| Componente | O que é | Estado |
|---|---|---|
| `python_concept/` | PoC de referência em Python (só stdlib) — a "verdade" do algoritmo. | ✅ feito |
| `rust_concept/`   | Porte Rust **congelado**, validado idêntico ao Python (serve de teste). | ✅ feito |
| `ragd/`           | **Daemon de produção** (Rust): segura N bases em memória, busca/ingestão via **API HTTP JSON**. É onde o desenvolvimento acontece. | ✅ feito |
| **ValHalla**      | Console web do `ragd` (visão, busca, ingestão, performance, drivers, logs). | ✅ feito |
| `nidhoggd/`       | **Níðhöggr** — camada de **inteligência** (experimental): o *worm* que digere o conhecimento. Ver seção abaixo. | 🚧 parcial |
| **MCP**           | Casca que pluga o RAGnaRock como ferramenta em agentes de IA (opencode, Claude, etc.). | 🚧 parcial |
| `drivers/`        | Drivers de linguagem — tokenizam **código-fonte** (sílabas + keywords por linguagem). | ✅ feito |
| `thesaurus/`      | Dicionários multilíngue + cross-lingual (para a query expansion). | ✅ feito |
| `logic_path/`     | Trilha didática **00 → 10** (memorial congelado) que ensina cada princípio de RAG. | ✅ feito |

> 📐 **Especificação completa** (os três daemons em detalhe, contratos JSON, estratégias de memória/disco,
> concorrência, modos de falha e roadmap): **[`ARCHITECTURE.pt-BR.md`](ARCHITECTURE.pt-BR.md)**.

---

## 🐉 Nidhogg — a camada de inteligência (experimental)

No mito nórdico, **Níðhöggr** é a serpente que rói as raízes de Yggdrasil. No RAGnaRock, o `nidhoggd`
é um *worm* (do bem) que **digere o conhecimento** das coleções e o destila num saber que **sobrevive
à deleção da coleção**. É a **camada analítica** do projeto: **autônoma** (o `ragd` nunca a consome; o
leitor é o humano), com cinco níveis (`0`–`4`): **`minerador`** (0, sem IA — índice léxico e digest do
cache), **`consciente`** (1, com IA — classifica em `{natureza, tipo}`, gera moldes e extrai registros
normalizados), **`estrutural`** (2, sem IA — censo determinístico de menções, dimensões e grafo
navegável), **`estrutural-llm`** (3, com IA — destila relações nas cenas mais densas e grava no mesmo
grafo) e **`propositivo`** (4, com IA — responde questões diretas sobre o conhecimento acumulado e
registra cada mudança de perspectiva na timeline).

> **Estado: 🚧 parcial.** Daemon separado na porta **11497**, API, dials de nível (`0`–`4`) e cadência,
> com armazenamento analítico no ClickHouse (níveis 1, 3 e 4 **opt-in** e com IA; níveis 0 e 2 sem IA).
> O desenho completo está em
> [`ARCHITECTURE.pt-BR.md`](ARCHITECTURE.pt-BR.md#5-nidhoggd--níðhöggr--camada-de-inteligência-11497-parcial).

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

**Configuração:** o daemon lê um `ragnarock.cfg` opcional da pasta onde roda. Copie
[`ragnarock.cfg.example`](ragnarock.cfg.example) para `ragnarock.cfg` e edite — toda chave é opcional
e documentada na própria linha. O `ragnarock.cfg` de verdade está no gitignore (guarda a senha de admin
e as chaves de API); **nunca faça commit dele**. Troque as credenciais padrão `admin/admin` antes de
qualquer uso real — o daemon se recusa a abrir sessão com elas, a menos que suba com `--dev`.

---

## API do daemon (HTTP JSON)

Rotas principais: `GET /health · /bases · /collections · /profile · /stats` ·
`POST /search · /chunk · /ingest · /ingest_file · /ingest_upload · /ingest_any · /transcribe` ·
`DELETE /bases/{nome} · /collections/{nome}` — além do login JWT (`/login`) e das rotas administrativas.

- **Uma rota de busca (#39):** `POST /search` aceita estágios opt-in — `expand` (dicionário → cache → IA),
  `phonetic`, `literal_fallback` (ligado por padrão) — e toda resposta diz quais rodaram em `via: [...]`.
  *Migração:* `POST /search_expand` continua funcionando igual; agora é um atalho para `/search` com `expand: true`.

- Bases são organizadas por `collection/name`; busca é **scatter-gather** com wildcard
  (`"sd*"`, `"*"`) e merge por relevância.
- Cada hit traz `collection, base, corpus` (nome do arquivo), `path`, `chunk`, `matchpoint`,
  `snippet`, etc. — então a IA vai **direto no arquivo**.

Contrato formal das 3 APIs (ragd, ValHalla, nidhoggd): **[`JSONCONTRACT.pt-BR.md`](JSONCONTRACT.pt-BR.md)**.
Exemplos executáveis `curl`: **[`ragd/json_samples/`](ragd/json_samples/)**.

**Plugar num agente (MCP):** um servidor MCP de referência, que expõe o `ragd` como ferramentas nativas
(busca, chunk, listagem, ingestão) para opencode, Claude Code e Kimi for Code, está em
**[`examples/mcp/`](examples/mcp/)**.

O silabificador — o algoritmo do token, com ~90 casos golden e um teste de conformidade — está
especificado em **[`specs/SYLLABLE.md`](specs/SYLLABLE.md)**.

---

## Status

Projeto **novo** (nasceu em maio/2026), em desenvolvimento ativo — encare como **alpha**. O núcleo
funciona ponta a ponta: busca silábica (recall + rerank), daemon com API, console ValHalla, drivers
de código, query expansion, ingestão incremental, buscas concorrentes que a ingestão não trava,
importadores em [`ingestors/`](ingestors/) (PDF, DOCX, XLSX, PPTX, CSV, áudio, MySQL e Postgres) e
integração via MCP.

**No radar:** ingestão de repositório com update por arquivo e um cliente com `sync` (#34), ingestão
disparada pela IA do usuário (#10), índice invertido no SSD para chegar a milhões de chunks (#43),
interfaces SQL, GraphQL e compatível com o protocolo do MySQL (#35, #36, #40) e build Windows de
duplo-clique (#11).

**Testes:** `cargo test` em `ragd/` e `nidhoggd/`; `python3 -m unittest discover -s ingestors/tests -v`
para os drivers de ingestão (os testes cuja dependência opcional falta são pulados);
`tools/e2e_ingest.sh [--bancos]` ponta a ponta contra um `ragd` descartável (e containers MySQL/Postgres
descartáveis). Ainda **não** há CI, e a fidelidade entre as três encarnações segue verificada à mão
(gerar por um lado, ler/buscar pelo outro; os campos batem exato).

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
