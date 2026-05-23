# Aula prática: princípios de RAG, do zero

Um pipeline de RAG construído **do zero, sem rede neural** — só contagem e álgebra
linear — usando a **sílaba** como token. Cada passo numerado ensina um princípio e
produz uma saída observável. Rode na ordem; cada passo consome o artefato do anterior.

> Por que sílabas? Unidade pequena, recombinável, com vocabulário finito. Deixa o
> embedding/índice **inspecionável a olho nu** — perfeito pra ensinar o que um RAG
> de produção faz por baixo dos panos.

## A lib `sylkit/`

As primitivas reusadas por todos os passos moram na lib **`sylkit/`**, que fica na
**raiz do projeto** (um nível acima desta pasta) — assim a mesma lib serve tanto a
aula quanto o gerador de produção (`../embed_gen.py`). Cada passo importa o que precisa
e foca em UM conceito:

| módulo | o que oferece |
|---|---|
| `../sylkit/tokenizer.py` | `syllabify`, `normalize`, `syllable_seq`, `WORD` |
| `../sylkit/vocab.py` | `load_vocab` (a matriz de tokens em ordem fixa) |
| `../sylkit/vector.py` | `histogram`, `tfidf`, `cosine`, `compute_idf` |
| `../sylkit/chunk.py` | `chunk_text` |
| `../sylkit/index.py` | `postings` (índice invertido posicional) |

> Cada passo tem no topo uma linha que adiciona a raiz ao `sys.path` (`sys.path.insert`),
> pra achar a `sylkit` um nível acima rodando direto com `python3 NN_*.py`.

## A trilha (rode em ordem)

| # | arquivo | princípio de RAG | produz |
|---|---|---|---|
| 00 | `00_build_syllabary.py` | **Vocabulário** fixo (espaço de dimensões) | `syllabary.txt` |
| 01 | `01_tokenizer_zipf.py` | **Tokenização** + **lei de Zipf** (cauda longa) | `zipf.json/csv` |
| 02 | `02_extract_page.py` | **Limpar o corpus** antes de indexar | `sample.txt` |
| 03 | `03_histogram.py` | **Embedding** = histograma esparso (bag-of-tokens) | `matrix.csv`, `histogram.txt` |
| 04 | `04_vector_search.py` | **Busca vetorial** (cosseno, cobertura, recall) | — |
| 05 | `05_sequence_search.py` | **A ordem importa** (bag → bigrama → frase) | — |
| 06 | `06_matched_filter.py` | **Matched filter** (visualizar o casamento) | `matched_filter.png` |
| 07 | `07_distributed_engine.py` | **Ranking de 2 estágios** + **sharding/scatter-gather** | — |
| 08 | `08_phrase_galloping.py` | **Índice invertido** + skip eficiente (galloping) | — |
| 09 | `09_skiplist.py` | **Compressão de postings** (deltas) + **skip lists** | — |
| 10 | `10_build_rag_base.py` | **Construir e persistir a base** RAG | `sda-tokenized.json` |

## Como rodar tudo

```bash
cd logic_path
python3 00_build_syllabary.py      # vocabulário
python3 01_tokenizer_zipf.py       # Zipf no corpus inteiro (sda.txt)
python3 02_extract_page.py         # página de teste -> sample.txt
python3 03_histogram.py            # embedding do sample
python3 04_vector_search.py riqueza galera
python3 05_sequence_search.py riqueza galera
python3 06_matched_filter.py       # gera matched_filter.png  (requer matplotlib)
python3 07_distributed_engine.py "Frodo Bolseiro" --shards 3 --k 5
python3 08_phrase_galloping.py riqueza Bolseiro montanha
python3 09_skiplist.py ro za que
python3 10_build_rag_base.py sda.txt --chunk 2048
```

## Dependências

- **Python 3** (só stdlib na maior parte)
- **matplotlib** — apenas o passo 06 (`pip install matplotlib`)
- **corpus** `sda.txt` (incluído nesta pasta — auto-contido)

## Extra (fora da trilha)

`extras/cosine_bytecode_vega.py` — mostra o cosseno do recall rodando como **bytecode
no cluster VEGA** (ESP32). Depende do repo externo `VEGA2`; é opcional e pode ser
pulado na aula.
