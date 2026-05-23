#!/usr/bin/env python3
"""search_rag.py — busca na base RAG (a fase ONLINE).

Consome a base gerada pelo embed_gen.py (ex: sda-tokenized.json) e responde queries.
Como a base e' AUTO-SUFICIENTE (vocab + idf + vetores inline), a busca NAO reprocessa
o corpus — so carrega o JSON e pontua.

Pipeline (o mesmo do RAG de producao):
  estagio 1 — RECALL : cosseno da query com cada chunk (usa o vec esparso + norm da base)
  estagio 2 — RERANK : matched filter (contiguidade) + span (proximidade) sobre o texto
  -> top-k com snippet keyword-in-context (palavras da query em DESTAQUE).

Uso (a base JSON e' obrigatoria — sempre explicita, nunca inferida):
  python3 search_rag.py sda-tokenized.json "Frodo Bolseiro"      # top-5
  python3 search_rag.py sda-tokenized.json "anel" "montanha" -k 3
  python3 search_rag.py sda-tokenized.json "Gandalf" --no-rerank # so recall cosseno
  python3 search_rag.py sda-tokenized.json -i                    # modo interativo (REPL)
"""
import os, sys, json, time, unicodedata, argparse

# sylkit mora na raiz do projeto (ao lado deste arquivo) — import direto
from sylkit import syllabify, normalize, syllable_seq, WORD, tfidf, cosine

PROX_SCALE = 8.0   # rr = mf / (1 + span/PROX_SCALE): penaliza palavras distantes


# ---------------------------------------------------------------------------
# estagio 2 — rerank: contiguidade (matched filter) + proximidade (span)
# ---------------------------------------------------------------------------
def _best_positions(qs, seq):
    """Melhor fracao de casamento contiguo de qs em seq + posicoes onde ocorre."""
    k = len(qs)
    if k == 0 or len(seq) < k:
        return 0.0, [0]
    best, pos = -1, []
    for p in range(len(seq) - k + 1):
        match = sum(1 for j in range(k) if seq[p + j] == qs[j])
        if match > best:
            best, pos = match, [p]
        elif match == best:
            pos.append(p)
    return best / k, pos


def _min_span(lists):
    """Menor janela que cobre uma posicao de CADA lista (smallest range)."""
    import heapq
    heap = [(lst[0], i, 0) for i, lst in enumerate(lists)]
    heapq.heapify(heap)
    cur_max = max(lst[0] for lst in lists)
    best = cur_max - heap[0][0]
    while True:
        mn, i, j = heapq.heappop(heap)
        if cur_max - mn < best:
            best = cur_max - mn
        if j + 1 == len(lists[i]):
            return best
        nxt = lists[i][j + 1]
        cur_max = max(cur_max, nxt)
        heapq.heappush(heap, (nxt, i, j + 1))


def rerank_score(query, seq):
    """Combina contiguidade (mf) e proximidade (span) das palavras do query."""
    words = [[normalize(s) for s in syllabify(w) if normalize(s)]
             for w in WORD.findall(query.lower())]
    words = [qs for qs in words if qs]
    if not words:
        return 0.0, 0
    fracs, lists = [], []
    for qs in words:
        frac, pos = _best_positions(qs, seq)
        fracs.append(frac); lists.append(pos)
    mf = sum(fracs) / len(fracs)
    span = _min_span(lists) if len(lists) > 1 else 0
    return mf, span


def _strip(s):
    s = unicodedata.normalize("NFD", s.lower())
    return "".join(c for c in s if unicodedata.category(c) != "Mn")


def snippet(text, query, width=140):
    """Trecho centrado na 1a palavra do query, com as palavras em «destaque»."""
    flat = " ".join(text.split())
    flat_na = _strip(flat)
    qwords = WORD.findall(query.lower())
    pos = -1
    for w in qwords:
        pos = flat_na.find(_strip(w))
        if pos >= 0:
            break
    if pos < 0:
        out = (flat[:width] + "…") if len(flat) > width else flat
    else:
        a, b = max(0, pos - 35), min(len(flat), pos - 35 + width)
        out = ("…" if a else "") + flat[a:b].strip() + ("…" if b < len(flat) else "")
    # destaca as palavras da query (case-insensitive, sem acento)
    out_na = _strip(out)
    marks = []
    for w in qwords:
        wn, start = _strip(w), 0
        while True:
            i = out_na.find(wn, start)
            if i < 0:
                break
            marks.append((i, i + len(wn))); start = i + len(wn)
    for a, b in sorted(set(marks), reverse=True):
        out = out[:a] + "«" + out[a:b] + "»" + out[b:]
    return out


# ---------------------------------------------------------------------------
# a base RAG carregada em memoria
# ---------------------------------------------------------------------------
class RagBase:
    def __init__(self, path):
        with open(path, encoding="utf-8") as f:
            b = json.load(f)
        self.meta = b["meta"]
        self.vocab = self.meta["vocab"]
        self.index = {t: i for i, t in enumerate(self.vocab)}
        self.idf = {int(k): v for k, v in b["idf"].items()}
        # chunks: vec esparso (chaves str -> int), indexado por id
        self.chunks = {}
        for c in b["chunks"]:
            c["vec"] = {int(k): v for k, v in c["vec"].items()}
            self.chunks[c["id"]] = c
        self.has_text = self.meta.get("with_text", False)

    def query_vec(self, query):
        """Query -> (qvec tf-idf, qnorm, silabas, n_oov) usando o idf da base."""
        tf, syls, oov = {}, [], 0
        for w in WORD.findall(query.lower()):
            for s in syllabify(w):
                ns = normalize(s)
                if not ns:
                    continue
                syls.append(ns)
                d = self.index.get(ns)
                if d is None:
                    oov += 1
                else:
                    tf[d] = tf.get(d, 0) + 1
        qvec, qnorm = tfidf(tf, self.idf)
        return qvec, qnorm, syls, oov

    def search(self, query, k=5, rerank=True, recall_n=20):
        qvec, qnorm, syls, oov = self.query_vec(query)
        info = {"syls": syls, "oov": oov, "dims": len(qvec),
                "n_chunks": len(self.chunks), "n_converge": 0, "recall_n": 0,
                "rerank": rerank and self.has_text, "ms_recall": 0.0, "ms_rerank": 0.0}
        if not qvec:
            return [], info
        # ESTAGIO 1 — RECALL: cosseno da query com cada chunk (vec/norm ja' na base).
        # so "convergem" os chunks que compartilham ao menos uma dimensao (cos > 0).
        t0 = time.time()
        scored = []
        for cid, c in self.chunks.items():
            s = cosine(qvec, qnorm, c["vec"], c["norm"])
            if s > 0:
                scored.append((s, cid))
        info["n_converge"] = len(scored)
        scored.sort(reverse=True)
        rn = max(k, recall_n) if info["rerank"] else k
        cand = scored[:rn]
        info["recall_n"] = min(rn, len(cand))
        info["ms_recall"] = (time.time() - t0) * 1000
        # ESTAGIO 2 — RERANK: matched filter (contiguidade) + span (proximidade)
        if info["rerank"]:
            t1 = time.time()
            res = []
            for cos, cid in cand:
                mf, span = rerank_score(query, syllable_seq(self.chunks[cid]["text"]))
                rr = mf / (1.0 + span / PROX_SCALE)
                res.append((rr, mf, span, cos, cid))
            res.sort(key=lambda t: (t[0], t[3]), reverse=True)
            hits = res[:k]
            info["ms_rerank"] = (time.time() - t1) * 1000
        else:
            hits = [(None, None, None, cos, cid) for cos, cid in cand[:k]]
        return hits, info


def show(base, query, hits, info, k):
    bar = "━" * 66
    print(f"\n{bar}\n  BUSCA: {query!r}\n{bar}")
    # [2] HISTOGRAM da query
    print(f"  [2] HISTOGRAM   query → {'-'.join(info['syls']) or '(vazio)'}")
    print(f"                  vetor da busca: {info['dims']} dims ativas, {info['oov']} OOV")
    if not hits:
        print(f"  [3] RECALL      nenhuma sílaba da query no vocabulário → 0 chunks")
        print(f"  ⇒ sem resultados.")
        return
    # [3] RECALL — quantos chunks convergem no cosseno
    print(f"  [3] RECALL      {info['n_chunks']} chunks varridos → "
          f"{info['n_converge']} convergem (cosseno > 0)   [{info['ms_recall']:.1f} ms]")
    print(f"                  ↳ mantém os top-{info['recall_n']} candidatos pro rerank")
    # [4] RERANK — filtros progressivos
    if info["rerank"]:
        print(f"  [4] RERANK      filtros: P1 contiguidade (matched filter) · "
              f"P2 proximidade (span)")
        print(f"                  ↳ {info['recall_n']} candidatos reordenados por matchpoint   "
              f"[{info['ms_rerank']:.1f} ms]")
    else:
        print(f"  [4] RERANK      (pulado — só recall por cosseno)")
    # [5] RESULTADOS — melhor (topo) → pior (base)
    print(f"  [5] RESULTADOS  top-{k}  (melhor em cima, pior embaixo):")
    for rank, (rr, mf, span, cos, cid) in enumerate(hits, 1):
        c = base.chunks[cid]
        if rr is not None:
            mp = f"matchpoint {rr:.3f}   (mf {mf:.2f} · span {span} · cos {cos:.4f})"
        else:
            mp = f"matchpoint {cos:.4f}   (cosseno puro)"
        print(f"\n   #{rank}  {mp}")
        print(f"        chunk {cid} @byte {c['start']:,}")
        if c.get("text"):
            print(f"        “{snippet(c['text'], query)}”")


def main():
    ap = argparse.ArgumentParser(
        description="Busca na base RAG (fase online): recall cosseno + rerank matched filter.",
        formatter_class=argparse.RawDescriptionHelpFormatter,
        epilog="exemplos:\n"
               "  python3 search_rag.py sda-tokenized.json \"Frodo Bolseiro\"\n"
               "  python3 search_rag.py sda-tokenized.json \"anel\" \"montanha\" -k 3\n"
               "  python3 search_rag.py sda-tokenized.json \"Gandalf\" --no-rerank\n"
               "  python3 search_rag.py sda-tokenized.json -i      # modo interativo\n")
    ap.add_argument("base", help="base JSON gerada pelo embed_gen.py (obrigatorio)")
    ap.add_argument("queries", nargs="*", help="termos de busca")
    ap.add_argument("-k", "--top", dest="k", type=int, default=5,
                    help="quantos resultados mostrar (default 5; ex: -k 10)")
    ap.add_argument("--recall-n", type=int, default=20, help="candidatos do recall p/ o rerank")
    ap.add_argument("--no-rerank", action="store_true", help="so o estagio 1 (recall cosseno)")
    ap.add_argument("-i", "--interactive", action="store_true", help="modo interativo (REPL)")
    if len(sys.argv) == 1:        # sem argumentos -> mostra o help
        ap.print_help()
        sys.exit(0)
    args = ap.parse_args()
    if not os.path.exists(args.base):
        sys.exit(f"erro: base não encontrada: {args.base!r}\n"
                 f"      gere uma com:  python3 embed_gen.py <corpus.txt>")

    t0 = time.time()
    base = RagBase(args.base)
    m = base.meta
    print(f"  [1] LOAD        base '{args.base}' em RAM   [{(time.time()-t0)*1000:.0f} ms]")
    print(f"                  {m['n_chunks']} chunks · vocab {m['vocab_size']} dims · "
          f"idf {len(base.idf)} · corpus '{m.get('corpus','?')}' (por {m.get('generator','?')})")
    if not base.has_text:
        print("                  aviso: base com --sem-texto → rerank e snippet desligados")

    rerank = not args.no_rerank
    for q in args.queries:
        hits, info = base.search(q, args.k, rerank, args.recall_n)
        show(base, q, hits, info, args.k)

    if args.interactive:
        print("\n[modo interativo — digite a busca; ENTER vazio ou 'sair' encerra]")
        while True:
            try:
                q = input("\nbusca> ").strip()
            except (EOFError, KeyboardInterrupt):
                print(); break
            if not q or q.lower() in ("sair", "quit", "exit"):
                break
            hits, info = base.search(q, args.k, rerank, args.recall_n)
            show(base, q, hits, info, args.k)


if __name__ == "__main__":
    main()
