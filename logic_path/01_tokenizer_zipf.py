#!/usr/bin/env python3
"""PASSO 01 — O TOKENIZER e a LEI DE ZIPF (por que vocabulario finito funciona).

Principio de RAG ensinado:
  Tokenizar = quebrar texto em unidades. Contando os tokens de um corpus real,
  emerge a LEI DE ZIPF: poucos tokens sao muito frequentes e uma cauda enorme e
  rara. Isso e o que torna RAG viavel — um vocabulario finito cobre quase todo o
  texto, e o idf (passo 04) usa justamente a raridade pra discriminar.

O tokenizer (silabador) mora em sylkit.tokenizer; aqui so o USAMOS e medimos a
distribuicao. Rode contra o corpus que quiser.

Saidas: zipf.json, zipf.csv  (token -> count, rank, freq)
Uso:    python3 01_tokenizer_zipf.py [corpus.txt ...]   (default: sda.txt)
"""
import sys
import json
import math
from collections import Counter

import os, sys
sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))  # lib sylkit (um nível acima)
from sylkit import syllabify, WORD


def count_corpus(texts):
    cnt = Counter()
    nwords = 0
    for txt in texts:
        for w in WORD.findall(txt.lower()):
            syls = syllabify(w)
            if syls:
                nwords += 1
                cnt.update(syls)
    return cnt, nwords


def zipf_table(cnt):
    items = cnt.most_common()
    total = sum(c for _, c in items)
    table = {s: {"count": c, "rank": r, "freq": c / total}
             for r, (s, c) in enumerate(items, start=1)}
    return table, total, items


def estimate_s(items):
    """Ajuste log-log: log(count) = log(C) - s*log(rank). Retorna o expoente s."""
    pts = [(math.log(r), math.log(c)) for r, (_, c) in enumerate(items, 1) if c > 0]
    n = len(pts)
    if n < 2:
        return float("nan")
    sx = sum(x for x, _ in pts); sy = sum(y for _, y in pts)
    sxx = sum(x * x for x, _ in pts); sxy = sum(x * y for x, y in pts)
    denom = n * sxx - sx * sx
    return -((n * sxy - sx * sy) / denom) if denom else float("nan")


def main():
    paths = sys.argv[1:] or ["sda.txt"]
    texts = []
    for p in paths:
        with open(p, encoding="utf-8", errors="ignore") as f:
            texts.append(f.read())

    cnt, nwords = count_corpus(texts)
    table, total, items = zipf_table(cnt)
    s = estimate_s(items)

    print(f"corpus: {', '.join(paths)}")
    print(f"tokens distintos (vocabulario): {len(table)}")
    print(f"ocorrencias totais:             {total}")
    if nwords:
        print(f"palavras:                       {nwords}  ({total/nwords:.2f} silabas/palavra)")
    print(f"expoente de Zipf estimado s ~ {s:.3f}  (lei classica: s ~ 1)")

    print("\nTop 20 tokens:")
    print(f"{'rank':>4}  {'token':<6} {'count':>6} {'freq':>7}  cumul")
    cum = 0
    for r, (tok, c) in enumerate(items[:20], 1):
        cum += c
        print(f"{r:>4}  {tok:<6} {c:>6} {c/total:>7.2%}  {cum/total:>6.1%}")

    for n in (50, 100, 200):
        if n <= len(items):
            cov = sum(c for _, c in items[:n]) / total
            print(f"top {n:>4} tokens cobrem {cov:.1%} das ocorrencias  (a cauda longa de Zipf)")

    with open("zipf.json", "w", encoding="utf-8") as f:
        json.dump(table, f, ensure_ascii=False, indent=1)
    with open("zipf.csv", "w", encoding="utf-8") as f:
        f.write("rank,token,count,freq\n")
        for tok, info in sorted(table.items(), key=lambda kv: kv[1]["rank"]):
            f.write(f'{info["rank"]},{tok},{info["count"]},{info["freq"]:.8f}\n')
    print("\nsalvos: zipf.json  zipf.csv")


if __name__ == "__main__":
    main()
