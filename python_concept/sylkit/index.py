#!/usr/bin/env python3
"""sylkit.index — indice invertido posicional (postings).

O coracao de todo search engine: pra cada token, a lista ORDENADA das posicoes
onde ele aparece no corpus. Com isso, buscar uma frase vira INTERSECAO de listas
(as silabas tem que aparecer em posicoes consecutivas) — sem varrer o texto.
"""
from collections import defaultdict


def postings(seq):
    """Sequencia de tokens -> {token: [posicoes ordenadas]} (postings list)."""
    pos = defaultdict(list)
    for i, tok in enumerate(seq):
        pos[tok].append(i)
    return dict(pos)
