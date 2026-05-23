#!/usr/bin/env python3
"""sylkit.vector — embedding esparso e similaridade.

O texto vira um HISTOGRAMA de tokens (bag-of-syllables): um vetor esparso onde
cada dimensao e um token do vocabulario e o valor e quantas vezes ele aparece.
Esse histograma E o embedding usado na busca vetorial — sem rede neural, so
contagem + algebra linear, que e o suficiente pra ensinar o principio do RAG.
"""
import math
from collections import defaultdict

from .tokenizer import syllabify, normalize, WORD


def histogram(text, index, with_stats=False):
    """Texto -> {dim: count} (term-frequency esparso) contra o vocabulario.

    with_stats=True devolve tambem cobertura: quantas silabas casaram no vocab e
    quantas ficaram de fora (OOV = out-of-vocabulary).
    """
    tf = defaultdict(int)
    total = 0
    oov = defaultdict(int)
    for w in WORD.findall(text.lower()):
        for s in syllabify(w):
            ns = normalize(s)
            if not ns:
                continue
            total += 1
            d = index.get(ns)
            if d is None:
                oov[ns] += 1
            else:
                tf[d] += 1
    if with_stats:
        n_oov = sum(oov.values())
        stats = {"total": total, "matched": total - n_oov,
                 "oov": n_oov, "oov_tokens": dict(oov)}
        return dict(tf), stats
    return dict(tf)


def compute_idf(tfs, n_docs):
    """idf global = log(N / df). Silaba em todo doc -> idf~0 (vira stopword)."""
    df = defaultdict(int)
    for tf in tfs:
        for d in tf:
            df[d] += 1
    n_docs = n_docs or 1
    return {d: math.log(n_docs / dfd) for d, dfd in df.items()}


def tfidf(tf, idf):
    """Pondera o histograma pela raridade (idf) -> (vetor, norma L2).

    Sem idf, sílabas comuns (que, ra, do) dominam. Com idf, o peso vai pras
    silabas RARAS — as que de fato discriminam o documento.
    """
    vec = {d: c * idf.get(d, 0.0) for d, c in tf.items()}
    vec = {d: w for d, w in vec.items() if w}      # idf=0 (stopword) some
    norm = math.sqrt(sum(w * w for w in vec.values())) or 1.0
    return vec, norm


def cosine(qvec, qnorm, cvec, cnorm):
    """Cosseno de dois vetores esparsos. Itera o MENOR (so dims em comum contam)."""
    if len(qvec) > len(cvec):
        qvec, cvec = cvec, qvec
    dot = sum(v * cvec.get(d, 0) for d, v in qvec.items())
    return dot / (qnorm * cnorm)
