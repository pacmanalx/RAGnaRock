#!/usr/bin/env python3
"""PASSO 00 — O VOCABULARIO (o espaco de dimensoes do embedding).

Principio de RAG ensinado:
  Antes de vetorizar qualquer coisa, voce precisa de um VOCABULARIO de tokens em
  ordem FIXA. Cada token e uma dimensao; a ordem nunca muda. E o contrato que
  permite comparar vetores depois.

Aqui o vocabulario e gerado por COMBINATORIA fonotatica do portugues (consoante +
vogal, digrafos, encontros, codas, ditongos) — sem corpus, e o "universo" de
silabas possiveis. (No RAG real, esse papel cabe ao tokenizer/vocab treinado.)

Saida: syllabary.txt — uma silaba por linha, ordem imutavel.
Uso:   python3 00_build_syllabary.py
"""

VOWELS = "aeiou"

# consoantes simples -> vogais validas
SIMPLE = {c: "aeiou" for c in "bcdfgjlmnprstvxz"}

# compostas / digrafos
COMPOUND = {"qu": "aeio", "gu": "ei", "ch": "aeiou", "lh": "aeiou", "nh": "aeiou"}

# encontros consonantais (muta + liquida)
CLUSTERS = {
    "br": "aeiou", "cr": "aeiou", "dr": "aeiou", "fr": "aeiou", "gr": "aeiou",
    "pr": "aeiou", "tr": "aeiou", "vr": "aeiou",
    "bl": "aeiou", "cl": "aeiou", "fl": "aeiou", "gl": "aeiou", "pl": "aeiou",
}

CODAS = "rlsmnzx"                  # consoantes que FECHAM a silaba (bar, mas, paz)

DIPHTHONGS = [                     # ditongos no nucleo (de-nasalizados)
    "ai", "ei", "oi", "ui", "au", "eu", "iu", "ou",
    "ia", "ie", "io", "ua", "ue", "uo",
    "ao", "ae", "oe",
]


def open_syllables(group):
    return [onset + v for onset, vowels in group.items() for v in vowels]


def close(open_syls):
    return [s + c for s in open_syls for c in CODAS]


def main():
    simple = open_syllables(SIMPLE)
    compound = open_syllables(COMPOUND)
    clusters = open_syllables(CLUSTERS)
    open_syls = simple + compound + clusters

    vc = [v + c for v in VOWELS for c in CODAS]      # sem onset: ar, es, an
    cvc = close(open_syls)                            # com onset: bar, tras
    closed = vc + cvc

    onsets = list(SIMPLE) + list(COMPOUND) + list(CLUSTERS)
    dip_cv = [o + d for o in onsets for d in DIPHTHONGS]
    dip_v = list(DIPHTHONGS)
    diphthongs = dip_cv + dip_v

    everything = open_syls + closed + diphthongs
    seen, ordered = set(), []
    for s in everything:
        if s not in seen:
            seen.add(s); ordered.append(s)

    with open("syllabary.txt", "w", encoding="utf-8") as f:
        f.write("\n".join(ordered) + "\n")

    print("ABERTAS (terminam em vogal):")
    print(f"  simples   : {len(simple):>5}  ({len(SIMPLE)} onsets)")
    print(f"  compostas : {len(compound):>5}  ({len(COMPOUND)} onsets)")
    print(f"  encontros : {len(clusters):>5}  ({len(CLUSTERS)} onsets)")
    print(f"FECHADAS (coda em '{CODAS}'):")
    print(f"  V+coda    : {len(vc):>5}")
    print(f"  CV+coda   : {len(cvc):>5}")
    print(f"DITONGOS ({len(DIPHTHONGS)} ditongos):")
    print(f"  onset+dit : {len(dip_cv):>5}")
    print(f"  pelados   : {len(dip_v):>5}")
    print("-" * 32)
    print(f"TOTAL       : {len(ordered):>5} dimensoes  -> syllabary.txt")


if __name__ == "__main__":
    main()
