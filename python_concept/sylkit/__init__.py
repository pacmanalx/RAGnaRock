"""sylkit — toolkit silabico pra aula de RAG.

A lib reune as primitivas reaproveitadas pelos passos da aula. Cada passo
numerado importa daqui o que precisa e foca em UM principio de RAG.

    from sylkit import syllabify, normalize, syllable_seq, WORD
    from sylkit import load_vocab
    from sylkit import histogram, tfidf, cosine, compute_idf
    from sylkit import chunk_text
"""
from .tokenizer import syllabify, normalize, syllable_seq, WORD
from .vocab import load_vocab, load_driver
from .vector import histogram, tfidf, cosine, compute_idf
from .chunk import chunk_text
from .index import postings

__all__ = [
    "syllabify", "normalize", "syllable_seq", "WORD",
    "load_vocab", "load_driver",
    "histogram", "tfidf", "cosine", "compute_idf",
    "chunk_text",
    "postings",
]
