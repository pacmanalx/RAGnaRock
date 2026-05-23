//! ragd — biblioteca interna que expõe os módulos da sylkit para o daemon
//! (main.rs) e para os binários auxiliares (src/bin/*). Mantém a sylkit numa
//! crate só, sem duplicar `mod` em cada bin.
#![allow(dead_code, unused_imports)]
pub mod tokenizer;
pub mod vocab;
pub mod vector;
pub mod chunk;
pub mod index;
pub mod rag;
pub mod ingestor;
pub mod multipart;
