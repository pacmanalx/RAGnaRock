#!/usr/bin/env python3
"""PASSO 04 — BUSCA VETORIAL (cosseno, cobertura, recall).

Principio de RAG ensinado:
  Buscar = comparar o vetor do QUERY com o vetor do DOCUMENTO. Tres sinais:
    score     = soma das contagens das silabas do query no documento
    cobertura = fracao das silabas do query que ocorrem no documento
    cosseno   = similaridade angular dos dois vetores (independe do tamanho)
  E o "recall" do RAG: dado um query, quao parecido ele e com o documento.

Le o embedding do documento gerado no passo 03 (matrix.csv).
Uso: python3 04_vector_search.py [palavra ...]   (default: galera riqueza)
"""
import sys
import math

import os, sys
sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))  # lib sylkit (um nível acima)
from sylkit import syllabify, normalize


def load_matrix(path="matrix.csv"):
    vocab, count = [], []
    with open(path, encoding="utf-8") as f:
        next(f)
        for line in f:
            _, tok, c = line.rstrip("\n").split(",")
            vocab.append(tok); count.append(int(c))
    index = {t: i for i, t in enumerate(vocab)}
    doc_norm = math.sqrt(sum(c * c for c in count)) or 1.0
    return vocab, count, index, doc_norm


def search(word, index, count, doc_norm):
    syls = [normalize(s) for s in syllabify(word)]
    rows = [(s, count[index[s]], "vocab") if s in index else (s, 0, "OOV")
            for s in syls]
    score = sum(c for _, c, _ in rows)
    present = sum(1 for _, c, _ in rows if c > 0)
    coverage = present / len(rows) if rows else 0
    qvec = {}
    for s in syls:
        if s in index:
            qvec[index[s]] = qvec.get(index[s], 0) + 1
    dot = sum(v * count[i] for i, v in qvec.items())
    qn = math.sqrt(sum(v * v for v in qvec.values())) or 1.0
    cos = dot / (qn * doc_norm)
    return syls, rows, score, present, coverage, cos


def main():
    words = sys.argv[1:] or ["galera", "riqueza"]
    vocab, count, index, doc_norm = load_matrix("matrix.csv")
    for w in words:
        syls, rows, score, present, cov, cos = search(w, index, count, doc_norm)
        print(f"\n=== '{w}'  ->  {'-'.join(syls)} ===")
        for s, c, tag in rows:
            mark = "OK " if c > 0 else ("·  " if tag == "vocab" else "x  ")
            print(f"   {mark} {s:<6} count_no_doc={c:<3} [{tag}]")
        print(f"   SCORE = {score}   cobertura = {present}/{len(rows)} ({cov:.0%})"
              f"   cosseno = {cos:.4f}")
        verdict = ("PRESENTE (todas as silabas ocorrem)" if cov == 1 and score > 0
                   else "PARCIAL" if score > 0 else "AUSENTE")
        print(f"   => {verdict}")


if __name__ == "__main__":
    main()
