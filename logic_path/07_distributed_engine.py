#!/usr/bin/env python3
"""PASSO 07 — MOTOR DISTRIBUIDO (ranking de 2 estagios + scatter-gather).

Principio de RAG ensinado (o coracao do RAG de producao):
  1) RANKING DE 2 ESTAGIOS:
       estagio 1 (recall)  — barato, sobre TODOS os chunks: cosseno do embedding.
       estagio 2 (rerank)  — caro, so sobre os top-N do recall: matched filter
                             (contiguidade) + span (proximidade das palavras).
     E exatamente o pipeline do RAG moderno: retriever rapido + reranker preciso.
  2) SHARDING / SCATTER-GATHER:
       o indice e' fatiado entre WORKERS (shards). O query e' BROADCAST a todos,
       cada um pontua os seus chunks (top-k local), e o coordenador faz o MERGE.
     E a arquitetura de Elasticsearch/Google serving — aqui simulada em RAM.

Le sda.txt. Usa as primitivas de sylkit.
Uso: python3 07_distributed_engine.py "anel" "Frodo Bolseiro" --shards 3 --k 5
"""
import math
import time
import argparse

import os, sys
sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))  # lib sylkit (um nível acima)
from sylkit import (load_vocab, histogram, tfidf, cosine, compute_idf,
                    chunk_text, syllable_seq, syllabify, normalize, WORD)

PROX_SCALE = 8.0   # escala de proximidade no rerank: rr = mf / (1 + span/PROX_SCALE)


# ---------------------------------------------------------------------------
# rerank (estagio 2): contiguidade (matched filter) + proximidade (span)
# ---------------------------------------------------------------------------
def _best_positions(qs, seq):
    """Melhor fracao de casamento de qs em seq + as posicoes onde ela ocorre."""
    k = len(qs)
    if k == 0 or len(seq) < k:
        return 0.0, [0]
    best, pos = -1, []
    for p in range(len(seq) - k + 1):
        match = sum(1 for j in range(k) if seq[p + j] == qs[j])
        if match > best:
            best, pos = match, [p]
        elif match == best:
            pos.append(p)
    return best / k, pos


def _min_span(lists):
    """Menor janela que cobre uma posicao de CADA lista (smallest range)."""
    import heapq
    heap = [(lst[0], i, 0) for i, lst in enumerate(lists)]
    heapq.heapify(heap)
    cur_max = max(lst[0] for lst in lists)
    best = cur_max - heap[0][0]
    while True:
        mn, i, j = heapq.heappop(heap)
        if cur_max - mn < best:
            best = cur_max - mn
        if j + 1 == len(lists[i]):
            return best
        nxt = lists[i][j + 1]
        cur_max = max(cur_max, nxt)
        heapq.heappush(heap, (nxt, i, j + 1))


def rerank_score(query, seq):
    """Combina contiguidade (mf) e proximidade (span) das palavras do query."""
    words = [[normalize(s) for s in syllabify(w) if normalize(s)]
             for w in WORD.findall(query.lower())]
    words = [qs for qs in words if qs]
    if not words:
        return 0.0, 0
    fracs, lists = [], []
    for qs in words:
        frac, pos = _best_positions(qs, seq)
        fracs.append(frac); lists.append(pos)
    mf = sum(fracs) / len(fracs)
    span = _min_span(lists) if len(lists) > 1 else 0
    return mf, span


# ---------------------------------------------------------------------------
# Worker = um shard (um ESP32 no projeto VEGA). Guarda (id, vetor, norma).
# ---------------------------------------------------------------------------
class Worker:
    def __init__(self, wid):
        self.wid = wid
        self.shard = []                  # lista de (chunk_id, vec, norm)

    def load_shard(self, cid, vec, norm):
        self.shard.append((cid, vec, norm))

    def query(self, qvec, qnorm, k):
        scored = [(cosine(qvec, qnorm, vec, norm), cid)
                  for cid, vec, norm in self.shard]
        scored.sort(reverse=True)
        return scored[:k]


# ---------------------------------------------------------------------------
# Cluster = coordenador + workers. Faz o scatter-gather.
# ---------------------------------------------------------------------------
class Cluster:
    def __init__(self, n_shards, index, use_idf=True):
        self.workers = [Worker(w) for w in range(n_shards)]
        self.index = index
        self.use_idf = use_idf
        self.idf = {}
        self.chunk_text = {}
        self.chunk_seq = {}

    def build(self, text, size=1024, max_chunks=0):
        chunks = chunk_text(text, size)
        if max_chunks:
            chunks = chunks[:max_chunks]
        t0 = time.time()
        tfs = []
        for cid, piece in enumerate(chunks):
            tf = histogram(piece, self.index)
            tfs.append(tf)
            self.chunk_text[cid] = piece
            self.chunk_seq[cid] = syllable_seq(piece)
        if self.use_idf:
            self.idf = compute_idf(tfs, len(chunks))
        else:
            self.idf = {d: 1.0 for tf in tfs for d in tf}
        for cid, tf in enumerate(tfs):
            vec, norm = tfidf(tf, self.idf)
            self.workers[cid % len(self.workers)].load_shard(cid, vec, norm)
        return len(chunks), time.time() - t0

    def search(self, query, k=5, rerank=True, recall_n=20):
        qvec, qnorm = tfidf(histogram(query, self.index), self.idf)
        rn = max(k, recall_n) if rerank else k
        # ESTAGIO 1 — broadcast do query + cosseno local por shard
        locals_ = [w.query(qvec, qnorm, rn) for w in self.workers]
        cand = sorted((hit for top in locals_ for hit in top), reverse=True)[:rn]
        bcast = len(qvec) * 3                 # idx 2B + count 1B por dim
        resp = sum(len(t) for t in locals_) * 6
        # ESTAGIO 2 — rerank na contiguidade + proximidade
        if rerank:
            scored = []
            for cos, cid in cand:
                mf, span = rerank_score(query, self.chunk_seq[cid])
                rr = mf / (1.0 + span / PROX_SCALE)
                scored.append((rr, mf, span, cos, cid))
            scored.sort(key=lambda t: (t[0], t[3]), reverse=True)
            hits = scored[:k]
        else:
            hits = [(None, None, None, cos, cid) for cos, cid in cand[:k]]
        return qvec, hits, bcast, resp


def snippet(text, query, width=130):
    """Trecho centrado na 1a palavra do query achada (keyword-in-context)."""
    import unicodedata
    flat = " ".join(text.split())

    def na(s):
        s = unicodedata.normalize("NFD", s.lower())
        return "".join(c for c in s if unicodedata.category(c) != "Mn")

    flat_na = na(flat)
    pos = -1
    for w in WORD.findall(query.lower()):
        pos = flat_na.find(na(w))
        if pos >= 0:
            break
    if pos < 0:
        return (flat[:width] + "…") if len(flat) > width else flat
    a, b = max(0, pos - 30), min(len(flat), pos - 30 + width)
    return ("…" if a else "") + flat[a:b].strip() + ("…" if b < len(flat) else "")


def main():
    ap = argparse.ArgumentParser(description="Motor de busca distribuido (RAG simulado).")
    ap.add_argument("queries", nargs="*", default=["anel"])
    ap.add_argument("--corpus", default="sda.txt")
    ap.add_argument("--shards", type=int, default=3)
    ap.add_argument("--k", type=int, default=5)
    ap.add_argument("--chunk", type=int, default=1024)
    ap.add_argument("--max-chunks", type=int, default=400, help="0 = corpus inteiro")
    ap.add_argument("--no-idf", action="store_true")
    ap.add_argument("--no-rerank", action="store_true")
    ap.add_argument("--recall-n", type=int, default=20)
    args = ap.parse_args()

    _, index = load_vocab("syllabary.txt")
    with open(args.corpus, encoding="utf-8") as f:
        text = f.read()

    cluster = Cluster(args.shards, index, use_idf=not args.no_idf)
    n_chunks, dt = cluster.build(text, args.chunk, args.max_chunks)

    print(f"== INDEXACAO ==")
    print(f"corpus: {args.corpus} ({len(text):,} bytes)  ->  {n_chunks} chunks em {dt:.2f}s")
    load = [len(w.shard) for w in cluster.workers]
    print(f"shards: {args.shards}  (min={min(load)} max={max(load)} chunks/shard)")
    print(f"ponderacao: {'tf-idf' if cluster.use_idf else 'tf puro (--no-idf)'}")

    rerank = not args.no_rerank
    for q in args.queries:
        qvec, hits, bcast, resp = cluster.search(q, args.k, rerank, args.recall_n)
        print(f"\n================ busca: {q!r} ================")
        sil = "-".join(normalize(s) for w in WORD.findall(q.lower()) for s in syllabify(w))
        print(f"  query -> silabas: {sil}  ({len(qvec)} dims ativas)")
        stage = ("recall cosseno -> rerank matched filter" if rerank
                 else "so recall cosseno")
        print(f"  scatter-gather: broadcast {bcast} B -> respostas {resp} B  [{stage}]")
        for rank, (rr, mf, span, cos, cid) in enumerate(hits, 1):
            wid = cid % args.shards
            mfs = f"rr={rr:.2f} (mf={mf:.2f} span={span})  " if rr is not None else ""
            print(f"  #{rank}  {mfs}cos={cos:.4f}  chunk={cid} (shard {wid})")
            print(f"        “{snippet(cluster.chunk_text[cid], q)}”")


if __name__ == "__main__":
    main()
