# sylkit (Rust) — RAG silábico, versão acelerada

Porte em Rust da lib `sylkit` + os dois binários (`embed_gen`, `search_rag`) do
projeto Python. Mesma lógica, mesmo schema de JSON, **mesmos resultados** — só que
bem mais rápido.

## Build

```bash
cd rust_concept
cargo build --release
# binários em rust_concept/target/release/{embed_gen,search_rag}
```

## Uso (rodar da raiz do projeto, onde estão sda.txt / tokens_PTBR.txt)

```bash
# offline: corpus -> base de vetores
./rust_concept/target/release/embed_gen sda.txt --chunk 2048

# online: busca na base (base obrigatória, igual ao Python)
./rust_concept/target/release/search_rag sda-tokenized.json "Frodo Bolseiro" -k 5
./rust_concept/target/release/search_rag sda-tokenized.json -i      # interativo
```

Sem argumentos, ambos mostram o help.

## Fidelidade (validado)

O `embed_gen` Rust reproduz a base do Python **idêntica**: `vocab`, `vec` (contagens),
`start`, `len`, `tokens`, `oov`, `text` batem campo a campo; `idf`/`norm` dentro de
`1e-6` (arredondamento float). O `search_rag` Rust devolve **os mesmos rankings,
chunks e matchpoints** — a base gerada por um lado é lida pelo outro sem conversão.

O silabador foi portado linha a linha (`src/tokenizer.rs`) — o `vec` idêntico em
~1 milhão de sílabas é a prova.

## Velocidade (Rust vs Python, mesmo hardware)

| etapa | Python | Rust | speedup |
|---|---:|---:|---:|
| `embed_gen` (corpus inteiro, 1489 chunks) | ~4,1 s | ~0,7 s | **~5,6×** |
| `search` rerank (20 candidatos) | ~50 ms | ~6 ms | **~8×** |
| `search` wall-clock (3 queries) | ~0,30 s | ~0,06 s | **~5×** |

(O `load` da base é dominado pelo parse do JSON — parecido nos dois; o ganho está
na tokenização e no recall/rerank.)

## Estrutura

```
rust_concept/
├── Cargo.toml
└── src/
    ├── lib.rs          # sylkit: re-exporta os módulos
    ├── tokenizer.rs    # syllabify, normalize, syllable_seq, words
    ├── vocab.rs        # load_vocab
    ├── vector.rs       # histogram, tfidf_norm, cosine, compute_idf
    ├── chunk.rs        # chunk_text, find_chars
    ├── index.rs        # postings
    └── bin/
        ├── embed_gen.rs
        └── search_rag.rs
```

Dependências: só `serde_json` (+ `serde`), com `preserve_order` pra o JSON sair na
mesma ordem de chaves do Python.
