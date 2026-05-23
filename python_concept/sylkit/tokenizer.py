#!/usr/bin/env python3
"""sylkit.tokenizer — o tokenizer silabico (onset/nucleo/coda).

E o coracao de tudo: transforma texto em TOKENS. Aqui o token e a silaba
linguistica (separa ca-sa, nao cas-a), porque a silaba e uma unidade pequena,
recombinavel e com vocabulario finito — perfeita pra ensinar embedding/RAG sem
depender de um modelo neural.

API publica:
  syllabify(word)   -> list[str]   silabas da palavra
  normalize(s)      -> str         minuscula + sem acento (a chave do vocabulario)
  syllable_seq(txt) -> list[str]   o texto inteiro como SEQUENCIA de silabas
  WORD              regex          extrai palavras (respeita acentos)
"""
import re
import unicodedata

# fonologia (ortografica) do portugues brasileiro
VOWEL      = set("aàáâãeéêiíoóôõuúü")
WEAK       = set("iu")            # semivogais (formam ditongo)
WEAK_ACC   = set("íú")            # i/u acentuado QUEBRA ditongo (vira hiato)
DIGRAPHS   = {"ch", "lh", "nh"}   # onset unico
ONSET2     = {"bl", "br", "cl", "cr", "dl", "dr", "fl", "fr", "gl", "gr",
              "pl", "pr", "tl", "tr", "vl", "vr"}   # muta + liquida


def _is_vowel(c):
    return c in VOWEL


def _segment(p):
    """Quebra a palavra em unidades V (vogal) ou C (consoante/digrafo)."""
    segs, i, n = [], 0, len(p)
    while i < n:
        c = p[i]
        if _is_vowel(c):
            segs.append(("V", c)); i += 1; continue
        pair = p[i:i + 2]
        if pair in DIGRAPHS:
            segs.append(("C", pair)); i += 2; continue
        if pair == "qu":                          # q sempre vem com u
            segs.append(("C", "qu")); i += 2; continue
        if pair == "gu" and i + 2 < n and p[i + 2] in "eéêií":   # gue/gui: u mudo
            segs.append(("C", "gu")); i += 2; continue
        segs.append(("C", c)); i += 1
    return segs


def _group_nuclei(vowels):
    """Junta vogais adjacentes em nucleos (ditongo) ou separa (hiato)."""
    nuclei, cur = [], ""
    for v in vowels:
        if not cur:
            cur = v; continue
        prev = cur[-1]
        if v in WEAK_ACC or prev in WEAK_ACC:
            join = False                          # i/u acentuado = hiato
        elif v in WEAK:
            join = True                           # V + i/u  -> ditongo decrescente
        elif prev in WEAK and v not in WEAK:
            join = True                           # i/u + V  -> ditongo crescente
        else:
            join = False                          # forte+forte -> hiato
        if join:
            cur += v
        else:
            nuclei.append(cur); cur = v
    if cur:
        nuclei.append(cur)
    return nuclei


def syllabify(word):
    """Palavra -> lista de silabas. Distribui as consoantes entre os nucleos."""
    p = "".join(ch for ch in word.lower() if ch in VOWEL or ch.isalpha())
    if not p:
        return []
    segs = _segment(p)
    if not any(t == "V" for t, _ in segs):
        return [p]                                # sem vogal (sigla) -> token unico
    # agrupa em blocos: ("C", cluster) e ("N", nucleo)
    blocks, j, n = [], 0, len(segs)
    while j < n:
        t, val = segs[j]
        if t == "C":
            blocks.append(("C", val)); j += 1
        else:
            run = [val]; j += 1
            while j < n and segs[j][0] == "V":
                run.append(segs[j][1]); j += 1
            for nuc in _group_nuclei(run):
                blocks.append(("N", nuc))
    if not any(b[0] == "N" for b in blocks):
        return [p]
    # monta as silabas a partir dos blocos
    k, syl = 0, []
    pre = ""
    while k < len(blocks) and blocks[k][0] == "C":
        pre += blocks[k][1]; k += 1
    cur = pre + blocks[k][1]; k += 1
    while k < len(blocks):
        cc = []
        while k < len(blocks) and blocks[k][0] == "C":
            cc.append(blocks[k][1]); k += 1
        if k >= len(blocks):                      # consoantes finais = coda do ultimo
            cur += "".join(cc); break
        nxt = blocks[k][1]; k += 1
        t = len(cc)
        if t == 0:
            syl.append(cur); cur = nxt            # hiato: corta entre nucleos
        elif t == 1:
            syl.append(cur); cur = cc[0] + nxt
        else:
            pair = cc[-2] + cc[-1]                # 2 ultimas formam cluster valido?
            if cc[-2] in "bcdfgptv" and cc[-1] in "lr" and pair in ONSET2:
                onset, coda = cc[-2] + cc[-1], "".join(cc[:-2])
            else:
                onset, coda = cc[-1], "".join(cc[:-1])
            syl.append(cur + coda); cur = onset + nxt
    syl.append(cur)
    return [s for s in syl if s]


def normalize(s):
    """Minuscula e sem acento — a forma canonica usada como chave do vocabulario."""
    s = unicodedata.normalize("NFD", s.lower())
    return "".join(c for c in s if unicodedata.category(c) != "Mn")


WORD = re.compile(r"[a-zàáâãäçéêèíïóôõòúûü]+", re.IGNORECASE)


def syllable_seq(text):
    """Texto -> SEQUENCIA de silabas normalizadas (preserva ordem)."""
    seq = []
    for w in WORD.findall(text.lower()):
        for s in syllabify(w):
            ns = normalize(s)
            if ns:
                seq.append(ns)
    return seq
