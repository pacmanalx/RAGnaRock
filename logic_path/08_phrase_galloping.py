#!/usr/bin/env python3
"""PASSO 08 — BUSCA DE FRASE no INDICE INVERTIDO (linear x galloping).

Principio de RAG ensinado:
  Buscar uma frase = INTERSECTAR as postings (passo sylkit.index): as silabas tem
  que aparecer em posicoes CONSECUTIVAS. Duas estrategias, contando comparacoes:
    (A) merge LINEAR  — avanca cada ponteiro de 1 em 1.
    (B) GALLOPING     — ancora na silaba MAIS RARA (menos tentativas) e localiza as
                        vizinhas por salto exponencial + binaria (pula os buracos).
  Ambas dao o MESMO resultado; galloping faz menos trabalho quando as listas sao
  desbalanceadas (uma rara, outra comum). E o que o Lucene faz nas postings.

Le sda.txt. Usa sylkit.syllable_seq e sylkit.postings.
Uso: python3 08_phrase_galloping.py riqueza Bolseiro montanha --corpus sda.txt
"""
import math
import bisect
import argparse

import os, sys
sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))  # lib sylkit (um nível acima)
from sylkit import syllable_seq, postings, syllabify, normalize


def phrase_linear(qs, pos):
    """Ancora em qs[0]; pra cada vizinha avanca um ponteiro LINEARMENTE."""
    lists = [pos.get(s, []) for s in qs]
    if any(not l for l in lists):
        return [], 0
    ptr = [0] * len(qs)
    hits, ops = [], 0
    for p in lists[0]:
        ok = True
        for j in range(1, len(qs)):
            target = p + j
            lj = lists[j]
            while ptr[j] < len(lj) and lj[ptr[j]] < target:
                ptr[j] += 1; ops += 1
            ops += 1
            if ptr[j] >= len(lj) or lj[ptr[j]] != target:
                ok = False; break
        if ok:
            hits.append(p)
    return hits, ops


def gallop_ge(lst, lo, target):
    """Menor idx>=lo com lst[idx]>=target, por salto exponencial + binaria."""
    n = len(lst)
    if lo >= n:
        return n, 0
    ops, bound = 0, 1
    while lo + bound < n and lst[lo + bound] < target:
        bound *= 2; ops += 1
    a = lo + bound // 2
    b = min(lo + bound + 1, n)
    if b > a:
        ops += max(1, int(math.log2(b - a)) + 1)
    return bisect.bisect_left(lst, target, a, b), ops


def phrase_gallop(qs, pos):
    """Ancora na lista MAIS CURTA; localiza as vizinhas por galloping."""
    lists = [pos.get(s, []) for s in qs]
    if any(not l for l in lists):
        return [], 0
    anchor = min(range(len(qs)), key=lambda j: len(lists[j]))
    ptr = [0] * len(qs)
    hits, ops = [], 0
    for p in lists[anchor]:
        base = p - anchor
        ok = True
        for j in range(len(qs)):
            if j == anchor:
                continue
            target = base + j
            idx, o = gallop_ge(lists[j], ptr[j], target); ops += o
            ptr[j] = idx
            if idx >= len(lists[j]) or lists[j][idx] != target:
                ok = False; break
        if ok:
            hits.append(base)
    return hits, ops


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("words", nargs="*", default=["riqueza", "Bolseiro", "montanha"])
    ap.add_argument("--corpus", default="sda.txt")
    args = ap.parse_args()

    with open(args.corpus, encoding="utf-8") as f:
        seq = syllable_seq(f.read())
    pos = postings(seq)
    print(f"corpus: {args.corpus}  ({len(seq):,} silabas, {len(pos)} tokens distintos)\n")

    for w in args.words:
        qs = [normalize(s) for s in syllabify(w) if normalize(s)]
        if len(qs) < 2:
            print(f"'{w}' -> {'-'.join(qs)}  (1 silaba, sem frase)\n")
            continue
        sizes = [(s, len(pos.get(s, []))) for s in qs]
        rare = min(sizes, key=lambda t: t[1])
        hL, oL = phrase_linear(qs, pos)
        hG, oG = phrase_gallop(qs, pos)
        gain = (oL / oG) if oG else float("inf")
        print(f"'{w}'  ->  {'-'.join(qs)}")
        print(f"   listas: {', '.join(f'{s}:{n}' for s, n in sizes)}  (ancora rara = {rare[0]}:{rare[1]})")
        print(f"   match? linear={len(hL)}  galloping={len(hG)}  (iguais: {sorted(hL)==sorted(hG)})")
        print(f"   COMPARACOES: linear={oL:>6}  galloping={oG:>6}  ganho={gain:.1f}x\n")


if __name__ == "__main__":
    main()
