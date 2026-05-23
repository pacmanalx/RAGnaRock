#!/usr/bin/env python3
"""PASSO 06 — MATCHED FILTER (visualizar o casamento de sequencia).

Principio de RAG ensinado:
  Por que a busca por sequencia (passo 05) e robusta? Deslizando o query sobre o
  texto e medindo a fracao de silabas que casam em cada posicao, surge um PICO
  onde a cadeia existe (correlacao -> 1.0) e curva chata onde nao existe. E o
  matched filter do processamento de sinais — a base intuitiva do "quao bem o
  query casa aqui" que todo reranker calcula.

Le sample.txt e matrix.csv. Requer matplotlib (pip install matplotlib).
Saida: matched_filter.png
Uso:   python3 06_matched_filter.py
"""
import re
import matplotlib
matplotlib.use("Agg")
import matplotlib.pyplot as plt

import os, sys
sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))  # lib sylkit (um nível acima)
from sylkit import syllabify, normalize, WORD

# documento como SEQUENCIA de silabas (matched filter precisa da ordem)
seq = []
with open("sample.txt", encoding="utf-8") as f:
    for w in WORD.findall(f.read().lower()):
        for s in syllabify(w):
            ns = normalize(s)
            if ns:
                seq.append(ns)
N = len(seq)

# histograma (matrix.csv do passo 03) na ordem fixa
vocab, count = [], []
with open("matrix.csv", encoding="utf-8") as f:
    next(f)
    for line in f:
        _, tok, c = line.rstrip("\n").split(",")
        vocab.append(tok); count.append(int(c))
index = {t: i for i, t in enumerate(vocab)}


def correlation(word):
    """Matched filter: fracao de silabas que casam em cada deslocamento p."""
    qs = [normalize(s) for s in syllabify(word)]
    k = len(qs)
    xs, ys = [], []
    for p in range(N - k + 1):
        match = sum(1 for j in range(k) if seq[p + j] == qs[j])
        xs.append(p); ys.append(match / k)
    return qs, xs, ys


riq_qs, rx, ry = correlation("riqueza")
gal_qs, gx, gy = correlation("galera")

fig, (ax1, ax2) = plt.subplots(2, 1, figsize=(13, 8))

ax1.plot(gx, gy, color="#c0392b", lw=1.0, label=f"galera ({'-'.join(gal_qs)}) — sem pico")
ax1.plot(rx, ry, color="#2471a3", lw=1.2, label=f"riqueza ({'-'.join(riq_qs)}) — PICO=1.0")
peak = max(range(len(ry)), key=lambda i: ry[i])
ax1.annotate(f"pico em p={rx[peak]}", xy=(rx[peak], 1.0), xytext=(rx[peak] + 20, 0.72),
             arrowprops=dict(arrowstyle="->", color="#2471a3"), color="#2471a3")
ax1.set_title("Matched filter — query deslizando sobre o texto")
ax1.set_xlabel("posicao no documento (sequencia de silabas)")
ax1.set_ylabel("fracao de silabas\nque casam")
ax1.set_ylim(0, 1.08); ax1.legend(loc="upper right"); ax1.grid(alpha=0.25)

dims = range(len(vocab))
ax2.bar(dims, count, width=1.0, color="#bdc3c7", label="documento (embedding)")
for s in riq_qs:
    if s in index:
        ax2.bar(index[s], count[index[s]], width=3.0, color="#2471a3")
for s in gal_qs:
    if s in index:
        ax2.bar(index[s], count[index[s]], width=3.0, color="#c0392b")
ax2.bar([], [], color="#2471a3", label="query: riqueza")
ax2.bar([], [], color="#c0392b", label="query: galera")
ax2.set_title("Embedding do documento (cinza) x dimensoes do query")
ax2.set_xlabel(f"dimensao (indice fixo do vocabulario, 0..{len(vocab)-1})")
ax2.set_ylabel("contagem")
ax2.legend(loc="upper right"); ax2.grid(alpha=0.2)

plt.tight_layout()
plt.savefig("matched_filter.png", dpi=110)
print("salvo: matched_filter.png")
print(f"riqueza: pico = {max(ry):.2f} (em p={rx[max(range(len(ry)), key=lambda i: ry[i])]})")
print(f"galera : pico = {max(gy):.2f}  (nunca fecha a sequencia)")
