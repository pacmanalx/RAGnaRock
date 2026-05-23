#!/usr/bin/env python3
"""EXTRA — o cosseno do recall rodando como BYTECODE no cluster VEGA (opcode DOT4).

NAO faz parte da trilha principal de RAG: e' uma ponte com o projeto VEGA (cluster
de ESP32). Mostra que a CONTA do estagio 1 (cosseno) e' a mesma que o cluster faria
— produto interno via DOT4 + normalizacao, rodando na VM VEGA.

DEPENDENCIA EXTERNA: o repo VEGA2 (vega_asm, vega_vm_sim). Sem ele, este script nao
roda — por isso fica em extras/ e nao na aula. Ajuste VEGA2_HOST abaixo.

Uso:  python3 extras/cosine_bytecode_vega.py "Frodo Bolseiro" --candidates 12
"""
import os
import sys
import argparse

# sylkit mora na raiz do projeto, dois niveis acima de extras/
sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__)))))

VEGA2_HOST = "/Volumes/512Gb(SSD)/dev/VEGA2/host"
sys.path.insert(0, VEGA2_HOST)
try:
    import vega_asm
    from vega_vm_sim import run as vm_run
except ImportError:
    sys.exit(f"[extra] requer o repo VEGA2 em {VEGA2_HOST} (vega_asm, vega_vm_sim). "
             "Este passo e' opcional — pule-o na aula.")

from sylkit import load_vocab, histogram, tfidf, cosine, compute_idf, chunk_text


def build_cos_kernel(M):
    """Bytecode VEGA do cosseno p/ M dims alinhadas (M multiplo de 4)."""
    a = vega_asm.Assembler()
    ACC = 0
    for b in range(M // 4):
        base = 4 * b
        for j in range(4):
            a.emit("LOADI", rd=1 + j, imm_u=base + j)        # q[base+j] -> R1..R4
        for j in range(4):
            a.emit("LOADI", rd=5 + j, imm_u=M + base + j)    # c[base+j] -> R5..R8
        a.emit("DOT4", rd=9, ra=1, rb=5)
        a.emit("MOV", rd=ACC, ra=9) if b == 0 else a.emit("ADD", rd=ACC, ra=ACC, rb=9)
    a.emit("LOADI", rd=1, imm_u=2 * M)        # qnorm
    a.emit("LOADI", rd=2, imm_u=2 * M + 1)    # cnorm
    a.emit("MUL", rd=1, ra=1, rb=2)           # |q|*|c|
    a.emit("DIV", rd=ACC, ra=ACC, rb=1)       # cosseno = dot / (|q||c|)
    a.emit("STOREO", ra=ACC, imm_u=0)
    a.emit("HALT")
    return a.assemble(), [], 2 * M + 2, 1


def align(qvec, cvec):
    """Dims do query -> vetores densos q,c (padded a multiplo de 4)."""
    dims = sorted(qvec)
    M = ((len(dims) + 3) // 4) * 4 or 4
    q = [float(qvec.get(d, 0.0)) for d in dims] + [0.0] * (M - len(dims))
    c = [float(cvec.get(d, 0.0)) for d in dims] + [0.0] * (M - len(dims))
    return q, c, M


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("query", nargs="?", default="Frodo Bolseiro")
    ap.add_argument("--corpus", default="sda.txt")
    ap.add_argument("--max-chunks", type=int, default=400)
    ap.add_argument("--candidates", type=int, default=12)
    args = ap.parse_args()

    _, index = load_vocab("syllabary.txt")
    text = open(args.corpus, encoding="utf-8").read()

    pieces = chunk_text(text, 1024)[:args.max_chunks]
    tfs = [histogram(p, index) for p in pieces]
    idf = compute_idf(tfs, len(pieces))
    chunks = {}
    for cid, tf in enumerate(tfs):
        vec, norm = tfidf(tf, idf)
        chunks[cid] = (vec, norm)

    qvec, qnorm = tfidf(histogram(args.query, index), idf)
    if not qvec:
        print("query sem dimensoes no vocabulario."); return

    ranked = sorted(((cosine(qvec, qnorm, v, n), cid) for cid, (v, n) in chunks.items()),
                    reverse=True)[:args.candidates]

    dims = sorted(qvec)
    M = ((len(dims) + 3) // 4) * 4 or 4
    bc, consts, n_in, n_out = build_cos_kernel(M)
    inputs = []
    for _, cid in ranked:
        v, n = chunks[cid]
        q, c, _ = align(qvec, v)
        inputs += q + c + [qnorm, n]

    outs = vm_run(bc, consts, n_in, n_out, inputs)

    print(f"query: {args.query!r}  ->  {len(qvec)} dims (M={M}, {M//4} bloco(s) DOT4)")
    print(f"kernel VEGA: {len(bc)//8} instrucoes, {len(bc)} bytes\n")
    print(f"  {'chunk':>6}  {'cos_python':>12}  {'cos_VM(DOT4)':>14}  {'|diff|':>10}")
    maxdiff = 0.0
    for t, (cos_py, cid) in enumerate(ranked):
        cos_vm = outs[t]
        d = abs(cos_py - cos_vm); maxdiff = max(maxdiff, d)
        print(f"  {cid:>6}  {cos_py:>12.8f}  {cos_vm:>14.8f}  {d:>10.2e}")
    print(f"\nmax |diff| = {maxdiff:.2e}  ->  "
          f"{'BIT-A-BIT' if maxdiff < 1e-9 else 'OK' if maxdiff < 1e-6 else 'DIVERGE!'}")


if __name__ == "__main__":
    main()
