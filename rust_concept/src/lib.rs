//! sylkit — toolkit silabico pra RAG (porte fiel da lib Python).
//!
//! Mesma API conceitual: tokenizer (syllabify/normalize/syllable_seq), vocab,
//! vector (histogram/tfidf/cosine/compute_idf), chunk e index (postings).
pub mod tokenizer;
pub mod vocab;
pub mod vector;
pub mod chunk;
pub mod index;

pub use tokenizer::{normalize, syllabify, syllable_seq, words};
pub use vocab::load_vocab;
pub use vector::{compute_idf, cosine, histogram, tfidf_norm};
pub use chunk::{chunk_text, find_chars};
pub use index::postings;
