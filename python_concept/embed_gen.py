#!/usr/bin/env python3
"""embed_gen.py — corpus .txt -> base de vetores-histograma em JSON.

Materializa a INDEXACAO offline: fatia o texto em chunks, tokeniza cada um contra o
vocabulario de tokens (ordem FIXA) e grava o histograma esparso (bag-de-tokens = o
"embed" do chunk). E a base de busca, persistida e reproduzivel.

Depende da lib **sylkit** (na raiz, ao lado) — o tokenizer/vetorizacao ja organizados.
O JSON e' AUTO-SUFICIENTE: carrega o vocabulario (vocab) inline e o idf global, entao
da' pra reconstruir o tf-idf e o cosseno sem reprocessar o corpus. Campos em ingles.

Layout:
  meta      -> corpus, chunk_size, contagens, coverage, generator, tokens_file, vocab[]
  idf       -> {dim: log(N/df)} idf global por dimensao vista
  chunks[]  -> {id, start (byte), len, tokens, oov, norm, text?, vec {dim: count}}

Uso:
    python3 embed_gen.py sda.txt                       # chunk 2048, sda-tokenized.json
    python3 embed_gen.py sda.txt --chunk 1024 --saida sda-1k.json
    python3 embed_gen.py sda.txt --sem-texto --max-chunks 50
"""
import os, sys, json, time, argparse, glob

# sylkit ao lado deste arquivo (em python_concept/) — import direto
from sylkit import load_vocab, load_driver, histogram, tfidf, compute_idf, chunk_text

# raiz do projeto = um nível acima; pastas por papel (independe do cwd)
ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
DRIVERS = os.path.join(ROOT, "drivers")          # vocabularios de tokens
IMPORTABLES = os.path.join(ROOT, "importables")  # corpora crus pra ingerir
RAGFILES = os.path.join(ROOT, "ragfiles")        # bases RAG geradas


def list_drivers(drivers_dir):
    """Lista os drivers .drv instalados (nome, idioma, nº silabas, nº keywords)."""
    out = []
    for path in sorted(glob.glob(os.path.join(drivers_dir, "*.drv"))):
        vocab, keywords = load_driver(path)
        fname = os.path.basename(path)
        # tokens_<Lang>_PTBR.drv -> idioma = <Lang>
        lang = fname[len("tokens_"):-len(".drv")] if fname.startswith("tokens_") else fname[:-4]
        lang = lang[:-len("_PTBR")] if lang.endswith("_PTBR") else lang
        header = ""
        with open(path, encoding="utf-8") as f:
            first = f.readline().strip()
            header = first[1:].strip() if first.startswith("#") else ""
        out.append({
            "name": fname,
            "language": lang,
            "syllables": len(vocab) - len(keywords),
            "keywords": len(keywords),
            "vocab_size": len(vocab),
            "header": header,
        })
    return {"drivers_dir": drivers_dir, "count": len(out), "drivers": out}


def main():
    ap = argparse.ArgumentParser(
        description="Gera base de vetores-histograma (JSON) de um corpus.",
        formatter_class=argparse.RawDescriptionHelpFormatter,
        epilog="exemplos (defaults: corpus em importables/, tokens em drivers/, saida em ragfiles/):\n"
               "  python3 embed_gen.py importables/sda.txt           # -> ragfiles/sda-tokenized.json\n"
               "  python3 embed_gen.py importables/outro.txt --chunk 1024\n"
               "  python3 embed_gen.py --sem-texto                   # usa o corpus default (importables/sda.txt)\n")
    ap.add_argument("corpus", nargs="?", default=os.path.join(IMPORTABLES, "sda.txt"),
                    help="corpus .txt de entrada (default importables/sda.txt)")
    ap.add_argument("--chunk", type=int, default=2048, help="bytes por chunk (default 2048)")
    ap.add_argument("--saida", default=None, help="JSON de saida (default ragfiles/{corpus}-tokenized.json)")
    ap.add_argument("--tokens", default=os.path.join(DRIVERS, "tokens_PTBR.drv"),
                    help="driver de tokens .drv (default drivers/tokens_PTBR.drv)")
    ap.add_argument("--list-drivers", action="store_true",
                    help="lista os drivers .drv instalados em drivers/ (saida JSON) e sai")
    ap.add_argument("--max-chunks", type=int, default=0, help="limita chunks (0 = corpus inteiro)")
    ap.add_argument("--sem-texto", action="store_true", help="nao grava o texto de cada chunk")
    ap.add_argument("--compacto", action="store_true",
                    help="grava minificado (1 linha); default e' indentado com TAB p/ ler no editor")
    ap.add_argument("--quiet", "-q", action="store_true",
                    help="suprime output de status (so gera o JSON)")
    if len(sys.argv) == 1:        # sem argumentos -> mostra o help (nao dispara a geracao)
        ap.print_help()
        sys.exit(0)
    args = ap.parse_args()

    if args.list_drivers:
        print(json.dumps(list_drivers(DRIVERS), ensure_ascii=False, indent=2))
        sys.exit(0)

    stem = os.path.splitext(os.path.basename(args.corpus))[0]
    saida = args.saida or os.path.join(RAGFILES, f"{stem}-tokenized.json")
    vocab, idx = load_vocab(args.tokens)
    with open(args.corpus, encoding="utf-8") as f:
        text = f.read()

    t0 = time.time()
    # fatia preservando o byte de inicio de cada chunk (pra rastrear no texto original)
    pieces = chunk_text(text, args.chunk)
    if args.max_chunks:
        pieces = pieces[:args.max_chunks]

    # passada 1: histograma (tf) + cobertura de cada chunk
    chunks, tfs = [], []
    tot_tokens = tot_oov = 0
    cursor = 0
    for cid, piece in enumerate(pieces):
        start = text.find(piece, cursor)          # offset real no texto original
        if start < 0:
            start = cursor
        cursor = start + len(piece)
        tf, stats = histogram(piece, idx, with_stats=True)
        tfs.append(tf)
        tot_tokens += stats["total"]; tot_oov += stats["oov"]
        chunks.append({"id": cid, "start": start, "len": len(piece),
                       "tokens": stats["total"], "oov": stats["oov"], "vec": tf,
                       "text": None if args.sem_texto else piece})

    # idf global = log(N/df): token em todo chunk -> idf~0 (vira stopword)
    idf = compute_idf(tfs, len(pieces))

    # passada 2: norma tf-idf de cada chunk (pra cosseno rapido depois)
    for c in chunks:
        _, norm = tfidf(c["vec"], idf)
        c["norm"] = round(norm, 6)
        # JSON exige chaves string; ordena por dim p/ legibilidade/diff estavel
        c["vec"] = {str(d): n for d, n in sorted(c["vec"].items())}

    base = {
        "meta": {
            "corpus": os.path.basename(args.corpus),
            "bytes": len(text.encode("utf-8")),
            "chunk_size": args.chunk,
            "n_chunks": len(pieces),
            "vocab_size": len(vocab),
            "vocab_used": len(idf),
            "tokens_total": tot_tokens,
            "oov_total": tot_oov,
            "coverage": round(1 - tot_oov / tot_tokens, 4) if tot_tokens else 0.0,
            "with_text": not args.sem_texto,
            "generator": os.path.basename(__file__),       # quem gerou esta base
            "tokens_file": os.path.basename(args.tokens),  # qual vocabulario foi usado
            "built_at": time.strftime("%Y-%m-%dT%H:%M:%S"),
            "vocab": vocab,                                 # a matriz inline (ordem imutavel)
        },
        "idf": {str(d): round(v, 6) for d, v in sorted(idf.items())},
        "chunks": chunks,
    }

    with open(saida, "w", encoding="utf-8") as f:
        if args.compacto:
            json.dump(base, f, ensure_ascii=False)
        else:
            json.dump(base, f, ensure_ascii=False, indent="\t")  # TAB: visivel no vim/vscode
    dt = time.time() - t0
    size = os.path.getsize(saida)

    if not args.quiet:
        print(f"== EMBEDDINGS GERADOS ==")
        print(f"corpus:    {args.corpus}  ({base['meta']['bytes']:,} bytes)")
        print(f"chunks:    {base['meta']['n_chunks']}  (~{args.chunk} B cada)  em {dt:.2f}s")
        print(f"tokens:    {base['meta']['tokens_file']}  ->  {base['meta']['vocab_size']} dims "
              f"({base['meta']['vocab_used']} usadas)")
        print(f"silabas:   {tot_tokens:,}  (OOV {tot_oov:,}  ->  cobertura {base['meta']['coverage']*100:.2f}%)")
        print(f"saida:     {saida}  ({size:,} bytes, {'com' if not args.sem_texto else 'sem'} texto)")


if __name__ == "__main__":
    main()
