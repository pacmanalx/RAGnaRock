# SYLLABLE.md — RAGnaRock's syllabification spec

The whole engine rests on one deterministic function: **`syllabify`** in
[`ragd/src/tokenizer.rs`](../ragd/src/tokenizer.rs). The token is the **syllable**, so if the
syllabifier drifts, every embedding and every match drifts with it. This file is the spec plus a
conformance test (`specs/syllable_golden.tsv`, run by `cargo test`) that pins the behavior down.

> This documents the **actual code**, verified against it — not an idealized grammar. Where the
> algorithm diverges from textbook PT-BR syllabification, that's stated openly in
> [Known limitations](#known-limitations), not hidden.

## Why determinism matters more than linguistic perfection

A RAG built on syllables only needs one guarantee: **the same word is cut the same way every time** —
on ingestion and on search alike. As long as that holds, recall (tf-idf cosine) and rerank (phonetic
matched filter) line up, and retrieval works. A syllabification that is "wrong" by a grammar book but
**consistent** costs nothing in practice. That's why the limitations below are low-impact: they are
systematic, so ingest and query agree.

## The algorithm

`syllabify(word)` lowercases the word, keeps only alphabetic/vowel characters, then runs three passes:

### 1. Segment into onset / nucleus units

A left-to-right scan classifies each position as a **vowel** or a **consonant unit**, recognizing:

- **Vowels** (drive nuclei): `a e i o u` and their accented forms `à á â ã é ê í ó ô õ ú ü`.
- **Digraphs** `ch`, `lh`, `nh` → a **single** consonant sound.
- **`qu`** → one consonant unit (the `u` is mute): `quente` → `quen-te`.
- **`gu` before a front high vowel** (`e é ê i í`) → one unit, mute `u`: `guerra` → `guer-ra`,
  `guitarra` → `gui-tar-ra`. Before `a`/`o` the `u` is a real vowel (`água` → `á-gua`).
- Anything else → a single consonant.

### 2. Group vowels into nuclei (diphthong vs. hiatus)

Consecutive vowels are grouped. With `weak = {i, u}` and `accented-weak = {í, ú}`, two vowels **join**
into one nucleus (a diphthong) when:

- the current vowel is weak (`i`/`u`), **or**
- the previous vowel is weak and the current is strong;

and they **split** (a hiatus) when:

- either vowel is an **accented weak** `í`/`ú` (the accent forces the break) — `saída` → `sa-í-da`,
  `país` → `pa-ís`, `egoísta` → `e-go-ís-ta`; **or**
- both vowels are strong — `coala` → `co-a-la`, `poeta` → `po-e-ta`.

### 3. Distribute consonants between nuclei (coda × onset)

Leading consonants attach to the first nucleus. For a run of consonants between two nuclei:

- **1 consonant** → onset of the **next** syllable: `casa` → `ca-sa`.
- **≥2 consonants** → the **last two** both go to the next onset **iff** they form a valid
  mute+liquid cluster; otherwise only the **last** does, and the rest become the coda of the
  previous syllable: `abstrato` → `abs-tra-to`, `instante` → `ins-tan-te`.
- Trailing consonants at the end of the word → coda of the last syllable: `mar` → `mar`.

Valid onset clusters (mute + liquid): `bl br cl cr dl dr fl fr gl gr pl pr tl tr vl vr`.

### Normalization (the vocabulary key)

`normalize` is applied **after** syllabification to build the canonical vocab key: lowercase, then strip
diacritics via a fixed table (`á→a`, `ç→c`, `ñ→n`, `ü→u`, …). So `Narnia` and `nárnia` collapse to the
same dimension — an accent never creates a distinct token. (Note: this is a char-by-char table, not
Unicode NFD.)

## Golden cases & the conformance test

[`specs/syllable_golden.tsv`](syllable_golden.tsv) holds ~90 `word⟶syllables` pairs that the
syllabifier **must** reproduce exactly. They cover every rule above: digraphs, `qu`/`gu`, diphthongs,
hiatus, the `í`/`ú` break, onset clusters, complex codas, accents, and `ç`.

```bash
cargo test -p ragd syllable_golden   # reads specs/syllable_golden.tsv, asserts each line
```

To add a case: append a `word<TAB>space-separated-syllables` line. If the test fails, either the
syllabifier regressed or the expected value is wrong — both are caught.

## Known limitations

These are real divergences from consensus PT-BR syllabification. They are **systematic and consistent**
(same cut on ingest and search), so retrieval is unaffected; they are kept out of the golden set and
listed here instead.

| Class | Example | Algorithm | Consensus | Why |
|---|---|---|---|---|
| **Nasal diphthongs** | `coração` | `co-ra-çã-o` | `co-ra-ção` | `ã`/`õ` count as strong vowels, so `ão`/`ãe`/`õe` split as hiatus instead of joining. Also `mãe`→`mã-e`, `limões`→`li-mõ-es`, `põe`→`põ-e`, `mãos`→`mã-os`. |
| **Unwritten hiatus** | `psicologia` | `psi-co-lo-gia` | `psi-co-lo-gi-a` | `i+a` joins as a diphthong; the true hiatus is only marked by stress that PT doesn't always write, so the algorithm can't see it. |
| **`tl`/`dl`/`vl` onsets** | `atleta` | `a-tle-ta` | `at-le-ta`¹ | These clusters are kept in the onset by design (determinism). ¹Some grammars split them. |

If any of these starts to matter for a corpus, the fix lives in `group_nuclei` (nasal join) or the
cluster table — and a new golden line should pin the corrected behavior.
