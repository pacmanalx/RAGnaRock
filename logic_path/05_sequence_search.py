#!/usr/bin/env python3
"""PASSO 05 — A ORDEM IMPORTA (o limite do bag-of-words).

Principio de RAG ensinado:
  O embedding bag-of-tokens (passo 03) IGNORA a ordem. "riqueza" e "queriza" tem o
  mesmo vetor — falso positivo. A correcao e' subir de nivel:
    [1] BAG       soma das silabas soltas (ignora ordem) -> da falso positivo
    [2] BIGRAMAS  pares consecutivos (captura ordem local)
    [3] SEQUENCIA a cadeia inteira, contigua (phrase query do search engine)
  E o mesmo salto que o RAG moderno faz: do bag-of-words pro contexto/ordem.

Le sample.txt (passo 02).
Uso: python3 05_sequence_search.py [palavra ...]   (default: galera riqueza)
"""
import sys
from collections import Counter, defaultdict

import os, sys
sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))  # lib sylkit (um nível acima)
from sylkit import syllabify, normalize, WORD


def build(text):
    """Documento como SEQUENCIA de silabas + a qual palavra cada uma pertence."""
    seq, wid = [], []
    for w_i, w in enumerate(WORD.findall(text.lower())):
        for s in syllabify(w):
            ns = normalize(s)
            if ns:
                seq.append(ns); wid.append(w_i)
    bag = Counter(seq)
    bigrams = Counter((seq[i], seq[i + 1])
                      for i in range(len(seq) - 1) if wid[i] == wid[i + 1])
    pos = defaultdict(list)
    for i, s in enumerate(seq):
        pos[s].append(i)
    return seq, wid, bag, bigrams, pos


def phrase_hits(qs, seq, wid, pos):
    """Conta a cadeia qs como silabas CONTIGUAS dentro de uma palavra."""
    if not qs:
        return 0
    k, hits = len(qs), 0
    for p in pos.get(qs[0], []):
        if p + k - 1 >= len(seq):
            continue
        if all(seq[p + j] == qs[j] and wid[p + j] == wid[p] for j in range(k)):
            hits += 1
    return hits


def main():
    words = sys.argv[1:] or ["galera", "riqueza"]
    with open("sample.txt", encoding="utf-8") as f:
        seq, wid, bag, bigrams, pos = build(f.read())

    for w in words:
        qs = [normalize(s) for s in syllabify(w)]
        bag_score = sum(bag.get(s, 0) for s in qs)
        qbi = [(qs[i], qs[i + 1]) for i in range(len(qs) - 1)]
        bi_present = [(b, bigrams.get(b, 0)) for b in qbi]
        bi_ok = sum(1 for _, c in bi_present if c > 0)
        hits = phrase_hits(qs, seq, wid, pos)

        print(f"\n========= '{w}'  ->  {'-'.join(qs)} =========")
        print(f"  [1] BAG       score={bag_score:<4} (silabas soltas — ignora ordem)")
        if qbi:
            parts = "  ".join(f"{a}>{b}:{c}" for (a, b), c in bi_present)
            print(f"  [2] BIGRAMAS  {bi_ok}/{len(qbi)} pares em sequencia -> {parts}")
        else:
            print(f"  [2] BIGRAMAS  (palavra de 1 silaba)")
        print(f"  [3] SEQUENCIA hits={hits}  (cadeia contigua numa palavra)")
        verdict = ("EXISTE no texto" if hits > 0 else
                   "PARECIDA (ordem parcial)" if bi_ok > 0 else "NAO existe")
        print(f"  => {verdict}")


if __name__ == "__main__":
    main()
