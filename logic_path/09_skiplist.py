#!/usr/bin/env python3
"""PASSO 09 — SKIP LIST: comprimir as postings sem perder o acesso aleatorio.

Principio de RAG ensinado:
  Indices reais sao GRANDES e precisam ser comprimidos. Postings em DELTAS (gaps:
  d[i]=p[i]-p[i-1]) economizam espaco — os numeros ficam pequenos. Mas isso MATA o
  acesso aleatorio: pra achar a posicao absoluta do i-esimo termo voce soma os
  deltas desde o comeco (O(n)). A SKIP LIST poe checkpoints com a soma acumulada em
  varios niveis; o "seek >= alvo" desce nivel a nivel e so entao soma os poucos
  deltas que faltam. E a forma estrutural do galloping (passo 08) — saltos
  hierarquicos PRE-construidos, o que o Lucene usa nas postings.

Metrica: somas de delta por seek aleatorio — deltas puros x skip list.
Le sda.txt. Usa sylkit.syllable_seq e sylkit.postings.
Uso: python3 09_skiplist.py ro za que --corpus sda.txt --seeks 300 --interval 16
"""
import random
import bisect
import argparse

import os, sys
sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))  # lib sylkit (um nível acima)
from sylkit import syllable_seq, postings, normalize, syllabify


def encode_deltas(P):
    """Posicoes absolutas ordenadas -> deltas (gaps). d[0]=P[0]."""
    return [P[0]] + [P[i] - P[i - 1] for i in range(1, len(P))] if P else []


def seek_deltas(D, target):
    """Menor idx com posicao absoluta >= target, decodificando do COMECO.

    Acesso aleatorio em deltas puros: cada seek recomeca do zero e soma ate o alvo.
    Conta as adicoes — o custo que a skip list ataca.
    """
    acc, adds = 0, 0
    for i, d in enumerate(D):
        acc += d; adds += 1
        if acc >= target:
            return i, adds
    return len(D), adds


class SkipList:
    """Checkpoints (posicao_absoluta, indice) a cada `interval`, e recursivamente
    uma camada mais grossa por cima. As absolutas sao PRE-calculadas, entao a
    navegacao custa COMPARACOES, nao somas; as somas de delta ficam limitadas a
    `interval` no trecho final.
    """

    def __init__(self, P, interval=16):
        self.D = encode_deltas(P)
        self.interval = interval
        self.levels = []
        cur = [(P[i], i) for i in range(0, len(P), interval)]
        while len(cur) > 1:
            self.levels.append(cur)
            cur = cur[::interval]
        self.overhead = sum(len(lv) for lv in self.levels)

    def seek(self, target):
        """Menor idx com absoluta >= target. Devolve (idx, somas, comparacoes)."""
        comps = 0
        abs_anc, idx_anc = 0, 0
        for level in reversed(self.levels):           # do grosso pro fino
            j = bisect.bisect_left([a for a, _ in level], abs_anc)
            while j < len(level) and level[j][0] <= target:
                abs_anc, idx_anc = level[j]
                j += 1; comps += 1
            comps += 1
        acc, idx, adds = abs_anc, idx_anc, 0
        if acc >= target:
            return idx, adds, comps
        while idx + 1 < len(self.D):
            idx += 1; acc += self.D[idx]; adds += 1
            if acc >= target:
                return idx, adds, comps
        return len(self.D), adds, comps


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("tokens", nargs="*", default=["ro", "za", "que"])
    ap.add_argument("--corpus", default="sda.txt")
    ap.add_argument("--seeks", type=int, default=300)
    ap.add_argument("--interval", type=int, default=16)
    ap.add_argument("--seed", type=int, default=42)
    args = ap.parse_args()

    random.seed(args.seed)
    with open(args.corpus, encoding="utf-8") as f:
        seq = syllable_seq(f.read())
    pos = postings(seq)
    print(f"corpus: {args.corpus}  ({len(seq):,} silabas)")
    print(f"skip list: interval={args.interval}\n")

    print(f"  {'token':>8}  {'ocorr':>7}  {'niveis':>6}  {'overhd':>7}  "
          f"{'somas/seek':>11}  {'somas SL':>9}  {'comps SL':>9}  {'ganho':>7}  ok")
    for t in args.tokens:
        ns = normalize(t)
        P = pos.get(ns, [])
        if len(P) < 2:
            print(f"  {ns:>8}  {len(P):>7}  (poucas ocorrencias)")
            continue
        sl = SkipList(P, args.interval)
        lo, hi = P[0], P[-1]
        sum_d = sum_s = comp_s = 0
        ok = True
        for _ in range(args.seeks):
            target = random.randint(lo, hi)
            i_ref = bisect.bisect_left(P, target)
            i_d, sd = seek_deltas(sl.D, target)
            i_s, ss, cs = sl.seek(target)
            sum_d += sd; sum_s += ss; comp_s += cs
            if not (i_d == i_s == i_ref):
                ok = False
        md, ms, mc = sum_d / args.seeks, sum_s / args.seeks, comp_s / args.seeks
        gain = (md / ms) if ms else float("inf")
        print(f"  {ns:>8}  {len(P):>7}  {len(sl.levels):>6}  {sl.overhead:>7}  "
              f"{md:>11.1f}  {ms:>9.1f}  {mc:>9.1f}  {gain:>6.1f}x  {'OK' if ok else 'ERRO'}")

    print("\n  somas/seek (deltas puros, do zero) cresce com n; somas SL fica ~constante")
    print("  -> a skip list devolve o acesso aleatorio O(1) que os deltas perderam.")


if __name__ == "__main__":
    main()
