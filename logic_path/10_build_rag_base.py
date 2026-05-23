#!/usr/bin/env python3
"""PASSO 10 — CONSTRUIR A BASE RAG (a indexacao offline, persistida).

Principio de RAG ensinado (onde tudo se junta):
  RAG = uma fase OFFLINE que indexa o corpus + uma fase ONLINE que busca. Aqui
  materializamos a fase offline: corpus -> chunks (passo sylkit.chunk) -> embedding
  de cada chunk (passo 03) -> tf-idf -> JSON persistido. Esse JSON e' a BASE: o
  vector store que a busca (passo 07) consome. Auto-suficiente: carrega o
  vocabulario inline e o idf global, entao reconstroi o tf-idf sem reprocessar.

Saida: {corpus}-tokenized.json
Uso:   python3 10_build_rag_base.py sda.txt --chunk 2048
"""
import os
import json
import math
import time
import argparse

import sys
sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))  # lib sylkit (um nível acima)
from sylkit import load_vocab, histogram, tfidf, compute_idf, chunk_text


def main():
    ap = argparse.ArgumentParser(description="Constroi a base RAG (JSON) de um corpus.")
    ap.add_argument("corpus", nargs="?", default="sda.txt")
    ap.add_argument("--chunk", type=int, default=2048, help="bytes por chunk")
    ap.add_argument("--out", default=None, help="default {corpus}-tokenized.json")
    ap.add_argument("--vocab", default="syllabary.txt")
    ap.add_argument("--max-chunks", type=int, default=0, help="0 = corpus inteiro")
    ap.add_argument("--no-text", action="store_true", help="nao grava o texto do chunk")
    ap.add_argument("--compact", action="store_true", help="JSON minificado (1 linha)")
    ap.add_argument("--quiet", "-q", action="store_true", help="suprime output de status")
    args = ap.parse_args()

    out = args.out or f"{os.path.splitext(os.path.basename(args.corpus))[0]}-tokenized.json"
    vocab, index = load_vocab(args.vocab)
    with open(args.corpus, encoding="utf-8") as f:
        text = f.read()

    t0 = time.time()
    pieces = chunk_text(text, args.chunk)
    if args.max_chunks:
        pieces = pieces[:args.max_chunks]

    records, tfs = [], []
    tot_tokens = tot_oov = 0
    cursor = 0
    for cid, piece in enumerate(pieces):
        start = text.find(piece, cursor)
        if start < 0:
            start = cursor
        cursor = start + len(piece)
        tf, stats = histogram(piece, index, with_stats=True)
        tfs.append(tf)
        tot_tokens += stats["total"]; tot_oov += stats["oov"]
        records.append({"id": cid, "start": start, "len": len(piece),
                        "tokens": stats["total"], "oov": stats["oov"], "tf": tf,
                        "text": None if args.no_text else piece})

    idf = compute_idf(tfs, len(pieces))
    for r in records:
        _, norm = tfidf(r["tf"], idf)
        r["norm"] = round(norm, 6)
        r["vec"] = {str(d): c for d, c in sorted(r.pop("tf").items())}

    base = {
        "meta": {
            "corpus": os.path.basename(args.corpus),
            "bytes": len(text.encode("utf-8")),
            "chunk_size": args.chunk,
            "n_chunks": len(pieces),
            "vocab_size": len(vocab),
            "vocab_used": len(idf),
            "tokens_total": tot_tokens,
            "oov_total": tot_oov,
            "coverage": round(1 - tot_oov / tot_tokens, 4) if tot_tokens else 0.0,
            "with_text": not args.no_text,
            "built_at": time.strftime("%Y-%m-%dT%H:%M:%S"),
            "vocab": vocab,                       # a matriz inline (ordem imutavel)
        },
        "idf": {str(d): round(v, 6) for d, v in sorted(idf.items())},
        "chunks": records,
    }

    with open(out, "w", encoding="utf-8") as f:
        if args.compact:
            json.dump(base, f, ensure_ascii=False)
        else:
            json.dump(base, f, ensure_ascii=False, indent="\t")
    dt = time.time() - t0
    size = os.path.getsize(out)

    if not args.quiet:
        print(f"== BASE RAG CONSTRUIDA ==")
        print(f"corpus:   {args.corpus}  ({base['meta']['bytes']:,} bytes)")
        print(f"chunks:   {base['meta']['n_chunks']}  (~{args.chunk} B cada)  em {dt:.2f}s")
        print(f"vocab:    {base['meta']['vocab_size']} dims  ({base['meta']['vocab_used']} usadas)")
        print(f"tokens:   {tot_tokens:,}  (OOV {tot_oov:,} -> cobertura {base['meta']['coverage']*100:.2f}%)")
        print(f"saida:    {out}  ({size:,} bytes, {'com' if not args.no_text else 'sem'} texto)")


if __name__ == "__main__":
    main()
