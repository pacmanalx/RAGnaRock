#!/usr/bin/env python3
"""PASSO 03 — O EMBEDDING (texto -> histograma esparso de tokens).

Principio de RAG ensinado:
  Embedding e' transformar texto em VETOR. Aqui o embedding e o histograma
  bag-of-tokens: conta quantas vezes cada token do vocabulario aparece. O vetor
  e ESPARSO (quase todas as dimensoes sao zero) e mora no espaco FIXO do
  vocabulario (passo 00). Sem rede neural — so contagem — mas ja e um embedding.

Mostra tambem a COBERTURA: quantas silabas do texto caem no vocabulario (e quantas
ficam de fora, OOV), revelando o que o vocabulario nao cobre.

Saidas: matrix.csv (vetor completo idx,token,count) e histogram.txt (visual)
Uso:    python3 03_histogram.py [texto.txt]   (default: sample.txt)
"""
import sys

import os, sys
sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))  # lib sylkit (um nível acima)
from sylkit import load_vocab, histogram


def main():
    text_path = sys.argv[1] if len(sys.argv) > 1 else "sample.txt"
    vocab, index = load_vocab("syllabary.txt")

    with open(text_path, encoding="utf-8") as f:
        text = f.read()

    tf, stats = histogram(text, index, with_stats=True)
    count = [0] * len(vocab)
    for d, c in tf.items():
        count[d] = c
    nz = sum(1 for c in count if c)

    # matrix.csv: o vetor inteiro na ordem FIXA do vocabulario
    with open("matrix.csv", "w", encoding="utf-8") as f:
        f.write("idx,token,count\n")
        for i, tok in enumerate(vocab):
            f.write(f"{i},{tok},{count[i]}\n")

    mx = max(count) if nz else 1
    with open("histogram.txt", "w", encoding="utf-8") as f:
        for i, tok in enumerate(vocab):
            c = count[i]
            bar = "#" * round(40 * c / mx) if c else ""
            f.write(f"{i:>4} {tok:<6} {c:>3} {bar}\n")

    total = stats["total"]
    print(f"texto: {text_path}")
    print(f"vocabulario (dimensoes fixas): {len(vocab)}")
    print(f"silabas no texto:              {total}")
    print(f"  casadas no vocabulario:      {stats['matched']}  ({stats['matched']/total:.1%})")
    print(f"  fora do vocabulario (OOV):   {stats['oov']}  ({stats['oov']/total:.1%})")
    print(f"dimensoes ativas: {nz} de {len(vocab)}  (esparsidade {1-nz/len(vocab):.1%})")

    rank = sorted(((count[i], vocab[i]) for i in range(len(vocab)) if count[i]),
                  reverse=True)
    print("\nTop 20 tokens (o embedding do documento):")
    for c, tok in rank[:20]:
        print(f"  {tok:<5} {c:>3} {'#'*max(1, round(40*c/mx))}")

    print("\nTop OOV (o que o vocabulario NAO cobre):")
    for ns, c in sorted(stats["oov_tokens"].items(), key=lambda kv: -kv[1])[:15]:
        print(f"  {ns:<6} {c}")

    print("\nsalvos: matrix.csv  histogram.txt")


if __name__ == "__main__":
    main()
