#!/usr/bin/env python3
"""sylkit.chunk — fatiamento do corpus em pedacos (chunking).

RAG nao indexa o documento inteiro: ele quebra em CHUNKS e indexa cada pedaco.
O tamanho do chunk e um trade-off classico: pequeno = preciso mas perde contexto;
grande = contexto rico mas recall ruidoso. Aqui cortamos em fronteira de palavra
pra nao partir uma palavra no meio.
"""


def chunk_text(text, size=2048):
    """Fatia em pedacos de ~size bytes, cortando no ultimo espaco antes do limite."""
    chunks, i, n = [], 0, len(text)
    while i < n:
        end = min(i + size, n)
        if end < n:
            cut = text.rfind(" ", i, end)         # nao parte palavra no meio
            if cut > i:
                end = cut
        piece = text[i:end].strip()
        if piece:
            chunks.append(piece)
        i = end
    return chunks
