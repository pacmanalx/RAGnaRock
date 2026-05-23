#!/usr/bin/env python3
"""PASSO 02 — PREPARAR O CORPUS (limpeza antes de indexar).

Principio de RAG ensinado:
  Texto cru nunca esta pronto pra indexar. Ligaduras tipograficas (fi/fl), capitular
  (drop-cap "B ilbo" -> "Bilbo") e hifenizacao de quebra de linha ("fami-\\nlias")
  viram tokens lixo se nao forem limpos ANTES. Garbage in, garbage out — a qualidade
  do RAG comeca aqui.

Recorta uma pagina do cap. I de "O Senhor dos Aneis" e limpa -> sample.txt.
Uso: python3 02_extract_page.py [--corpus sda.txt]
"""
import re
import unicodedata
import argparse

START, END = 869, 916              # linhas no sda.txt (cap. I: Uma Festa Muito Esperada)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--corpus", default="sda.txt")
    ap.add_argument("--out", default="sample.txt")
    args = ap.parse_args()

    with open(args.corpus, encoding="utf-8") as f:
        lines = f.readlines()
    text = "".join(lines[START - 1:END])

    # 1) normaliza ligaduras (fi->fi, fl->fl) e unicode
    text = unicodedata.normalize("NFKC", text)
    # 2) junta drop-cap: consoante-maiuscula isolada + espaco + minuscula (B ilbo -> Bilbo)
    text = re.sub(r"(?<![A-Za-zÀ-ÿ])([BCDFGHJKLMNPQRSTVXZ]) (?=[a-zà-ÿ])", r"\1", text)
    # 3) dehifeniza quebras de linha no meio da palavra (fami-\nlias -> familias)
    text = re.sub(r"-\n(?=[a-zà-ÿ])", "", text)

    with open(args.out, "w", encoding="utf-8") as f:
        f.write(text)

    words = len(re.findall(r"\S+", text))
    print(f"{args.out}: {len(text)} chars, {len(text.encode('utf-8'))} bytes, {words} palavras")
    print("--- preview ---")
    print("\n".join(text.splitlines()[:8]))


if __name__ == "__main__":
    main()
