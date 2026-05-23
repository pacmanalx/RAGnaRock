#!/usr/bin/env python3
"""sylkit.vocab — o vocabulario: a matriz de tokens em ORDEM FIXA.

Cada token ocupa uma dimensao com indice imutavel. Esse indice e o contrato do
embedding: o vetor do documento e do query so podem ser comparados se as
dimensoes significarem sempre a mesma coisa. (No RAG real, esse e o papel do
vocabulario/tokenizer treinado: congelar o espaco de dimensoes.)
"""


def load_vocab(path="tokens_PTBR.drv"):
    """Le o driver de tokens (.drv) -> (lista ordenada, indice token->dim).

    Formato .drv (texto, 1 token por linha):
      - linha "# ..."      -> comentario/cabecalho (ignorada)
      - linha "=palavra"   -> KEYWORD atomica (Jeito B): o token e' a palavra sem o '='
      - qualquer outra      -> silaba do vocabulario
    Tanto silabas quanto keywords ocupam dimensoes (na ordem do arquivo). A marca '='
    sinaliza ao tokenizer que essa palavra NAO deve ser silabada (ver load_driver).
    Aceita tambem .txt legado (sem '#'/'=')."""
    vocab, _ = load_driver(path)
    return vocab, {tok: i for i, tok in enumerate(vocab)}


def load_driver(path):
    """Como load_vocab, mas tambem devolve o set de KEYWORDS atomicas.

    -> (vocab[list], keywords[set]).  keywords ⊆ vocab; sao as linhas '=palavra'."""
    vocab, keywords = [], set()
    with open(path, encoding="utf-8") as f:
        for line in f:
            s = line.strip()
            if not s or s.startswith("#"):
                continue
            if s.startswith("="):          # keyword atomica (Jeito B)
                s = s[1:]
                if not s:
                    continue
                keywords.add(s)
            vocab.append(s)
    return vocab, keywords
