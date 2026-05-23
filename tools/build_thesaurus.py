#!/usr/bin/env python3
"""build_thesaurus.py — baixa dicionarios abertos e gera os NOSSOS thesaurus.

Filosofia (dentro do conceito RAGnaRock):
  - Os dicionarios que geramos sao ARTEFATOS CONGELADOS (`expandable:false`).
    Quem cresce sozinho e' so o cache da IA em thesaurus/_cache/. Aqui e' curadoria.
  - Formato JSONL, inspecionavel a olho nu / grep-avel:
        linha 0  -> {"meta": {...}}            (fonte, licenca, contagem, kind)
        linha N  -> {"w":"casa","s":["lar","moradia","residencia"]}
    Cada linha e' JSON valido; acentos preservados (ensure_ascii=False).
  - Chave `w` em minuscula (lookup normaliza dos dois lados); sinonimos legiveis.

Codigo de idioma = [lingua-query][lingua-alvo] (4 letras):
  metades iguais  = MONO  (PTPT, ENEN, DEDE, ESES, PTBR)
  metades difer.  = CROSS (PTEN, ENPT)  -> gerado por COMPOSICAO (--compose)

Uso:
    python3 tools/build_thesaurus.py                 # help (sem args nao roda)
    python3 tools/build_thesaurus.py --list          # lista fontes conhecidas
    python3 tools/build_thesaurus.py --only ENEN     # baixa+gera um codigo
    python3 tools/build_thesaurus.py --all           # gera todos os MONO
    python3 tools/build_thesaurus.py --compose PTEN  # gera cross por composicao
    python3 tools/build_thesaurus.py --only ENEN --refetch   # re-baixa a fonte
"""
import os, sys, json, argparse, unicodedata, urllib.request, gzip, io, re, time

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
THES = os.path.join(ROOT, "thesaurus")
SRC = os.path.join(THES, ".sources")

# ----------------------------------------------------------------------------
# Registro de fontes MONO. parser = funcao(path_da_fonte) -> {palavra: [syns]}
# ----------------------------------------------------------------------------
SOURCES = {
    "ENEN": {
        "kind": "mono", "lang_query": "en", "lang_target": "en",
        "source": "Moby Thesaurus II — Grady Ward",
        "source_url": "https://www.gutenberg.org/ebooks/3202",
        "license": "Public Domain",
        "fetch": [("https://raw.githubusercontent.com/words/moby/master/words.txt", "moby_en.txt")],
        "parser": "parse_moby",
    },
    "DEDE": {
        "kind": "mono", "lang_query": "de", "lang_target": "de",
        "source": "OpenThesaurus (Deutsch)",
        "source_url": "https://www.openthesaurus.de/about/download",
        "license": "LGPL-3.0",
        "fetch": [("https://www.openthesaurus.de/export/OpenThesaurus-Textversion.zip", "openthesaurus_de.zip")],
        "parser": "parse_openthesaurus_zip",
    },
    "ESES": {
        "kind": "mono", "lang_query": "es", "lang_target": "es",
        "source": "LibreOffice dictionaries — th_es (MyThes)",
        "source_url": "https://github.com/LibreOffice/dictionaries/tree/master/es",
        "license": "LibreOffice dict (ver README_th_es; tipic. LGPL/GPL) — VERIFICAR",
        "fetch": [("https://raw.githubusercontent.com/LibreOffice/dictionaries/master/es/th_es_v2.dat", "th_es.dat")],
        "parser": "parse_mythes",
    },
    "PTBR": {
        "kind": "mono", "lang_query": "pt", "lang_target": "br",
        "source": "LibreOffice dictionaries — th_pt_BR (MyThes)",
        "source_url": "https://github.com/LibreOffice/dictionaries/tree/master/pt_BR",
        "license": "LibreOffice dict (ver README; tipic. LGPL/GPL) — VERIFICAR",
        "fetch": [("https://raw.githubusercontent.com/LibreOffice/dictionaries/master/pt_BR/th_pt_BR.dat", "th_pt_BR.dat")],
        "parser": "parse_mythes",
    },
    "PTPT": {
        "kind": "mono", "lang_query": "pt", "lang_target": "pt",
        "source": "LibreOffice dictionaries — th_pt_PT (MyThes)",
        "source_url": "https://github.com/LibreOffice/dictionaries/tree/master/pt_PT",
        "license": "LibreOffice dict (ver README; tipic. LGPL/GPL) — VERIFICAR",
        "fetch": [("https://raw.githubusercontent.com/LibreOffice/dictionaries/master/pt_PT/th_pt_PT.dat", "th_pt_PT.dat")],
        "parser": "parse_mythes",
    },
    # alternativa PTBR: TeP 2.0 (stavarengo, layout-one synsets) — ⚠️ licenca academica nao declarada
}

# ----------------------------------------------------------------------------
# Composicao cross-lingual:  CROSS = ALVO( traduz( query(palavra) ) )
# precisa do bilingue de traducao + os dois mono. (a montar quando os mono existirem)
# ----------------------------------------------------------------------------
COMPOSE = {
    "PTEN": {  # query PT -> alvo EN. ponte = traducao por->eng; expande com sinonimos PT antes
        "kind": "cross", "lang_query": "pt", "lang_target": "en",
        "source": "composicao: PTBR (sinonimos) + FreeDict por-eng (traducao)",
        "source_url": "https://download.freedict.org/dictionaries/por-eng/",
        "license": "deriva de PTBR + FreeDict por-eng (copyleft) — VERIFICAR",
        "syn_mono": "PTBR",
        "bridge": ("https://download.freedict.org/dictionaries/por-eng/0.2/freedict-por-eng-0.2.src.tar.xz",
                   "fd_por_eng.tar.xz", "por-eng/por-eng.tei"),
    },
    "ENPT": {  # query EN -> alvo PT
        "kind": "cross", "lang_query": "en", "lang_target": "pt",
        "source": "composicao: ENEN (sinonimos) + FreeDict eng-por (traducao)",
        "source_url": "https://download.freedict.org/dictionaries/eng-por/",
        "license": "deriva de ENEN (dominio publico) + FreeDict eng-por (copyleft) — VERIFICAR",
        "syn_mono": "ENEN",
        "bridge": ("https://download.freedict.org/dictionaries/eng-por/0.3/freedict-eng-por-0.3.src.tar.xz",
                   "fd_eng_por.tar.xz", "eng-por/eng-por.tei"),
    },
}


def norm_key(s):
    """Forma canonica de busca: minuscula, sem acento (igual ao tokenizer)."""
    s = unicodedata.normalize("NFD", s.strip().lower())
    return "".join(c for c in s if unicodedata.category(c) != "Mn")


def clean_term(s):
    """Termo legivel (preserva acento): minuscula, espacos colapsados."""
    return re.sub(r"\s+", " ", s.strip().lower())


# ---- parsers de fonte -------------------------------------------------------
def parse_moby(path):
    """Moby: cada linha `raiz,rel,rel,...` (raiz -> demais sao relacionados)."""
    d = {}
    with open(path, encoding="latin-1") as f:
        for line in f:
            parts = [clean_term(p) for p in line.rstrip("\n").split(",") if p.strip()]
            if len(parts) < 2:
                continue
            root, rest = parts[0], parts[1:]
            d.setdefault(root, [])
            d[root].extend(rest)
    return d


def parse_openthesaurus_zip(path):
    """OpenThesaurus texto: cada linha = SYNSET `a;b;c` (grupo de sinonimos).
    Invertemos: cada termo do grupo -> os demais do grupo."""
    import zipfile
    d = {}
    with zipfile.ZipFile(path) as z:
        name = next(n for n in z.namelist() if n.endswith(".txt"))
        raw = z.read(name).decode("utf-8", "replace")
    for line in raw.splitlines():
        if not line or line.startswith("#"):
            continue
        group = [clean_term(t) for t in line.split(";") if t.strip()]
        # OpenThesaurus marca categoria com (...) — limpa parenteticos
        group = [re.sub(r"\s*\(.*?\)\s*", "", t).strip() for t in group]
        group = [t for t in group if t]
        if len(group) < 2:
            continue
        for i, term in enumerate(group):
            others = group[:i] + group[i + 1:]
            d.setdefault(term, []).extend(others)
    return d


def parse_mythes(path):
    """MyThes (.dat LibreOffice): linha 0 = encoding; entrada `palavra|N` seguida de
    N linhas `(pos)|syn1|syn2|...`. Une os sinonimos de todos os sentidos da palavra."""
    with open(path, "rb") as f:
        raw = f.read()
    enc = raw.split(b"\n", 1)[0].decode("ascii", "replace").strip() or "ISO8859-1"
    try:
        text = raw.decode(enc, "replace")
    except LookupError:
        text = raw.decode("latin-1", "replace")
    lines = text.split("\n")
    d, i, n = {}, 1, len(lines)
    while i < n:
        head = lines[i].rstrip("\r"); i += 1
        if "|" not in head:
            continue
        word, _, cnt = head.partition("|")
        if not cnt.strip().isdigit():
            continue
        word = clean_term(word)
        for _ in range(int(cnt)):
            if i >= n:
                break
            fields = lines[i].rstrip("\r").split("|"); i += 1
            for s in fields[1:]:  # campo 0 = classe gramatical
                s = clean_term(re.sub(r"\s*\(.*?\)\s*", "", s))
                if s and word:
                    d.setdefault(word, []).append(s)
    return d


PARSERS = {"parse_moby": parse_moby, "parse_openthesaurus_zip": parse_openthesaurus_zip,
           "parse_mythes": parse_mythes}


# ---- pipeline ---------------------------------------------------------------
def fetch(url, fname, refetch=False):
    os.makedirs(SRC, exist_ok=True)
    dst = os.path.join(SRC, fname)
    if os.path.exists(dst) and not refetch:
        return dst
    print(f"  baixando {url}")
    req = urllib.request.Request(url, headers={"User-Agent": "RAGnaRock-thesaurus-builder"})
    with urllib.request.urlopen(req, timeout=120) as r, open(dst, "wb") as out:
        out.write(r.read())
    return dst


def finalize(raw):
    """Dedup, tira auto-referencia e termos vazios; ordena chaves."""
    out = {}
    for w, syns in raw.items():
        wk = clean_term(w)
        if not wk:
            continue
        seen, lst = set(), []
        for s in syns:
            s = clean_term(s)
            if not s or s == wk or s in seen:
                continue
            seen.add(s)
            lst.append(s)
        if lst:
            out.setdefault(wk, [])
            for s in lst:
                if s not in out[wk]:
                    out[wk].append(s)
    return out


def write_jsonl(code, info, data):
    outdir = os.path.join(THES, code)
    os.makedirs(outdir, exist_ok=True)
    path = os.path.join(outdir, "synonyms.jsonl")
    meta = {
        "code": code, "kind": info["kind"],
        "lang_query": info["lang_query"], "lang_target": info["lang_target"],
        "source": info["source"], "source_url": info["source_url"],
        "license": info["license"], "expandable": False,
        "built": time.strftime("%Y-%m-%d"), "entries": len(data),
    }
    with open(path, "w", encoding="utf-8") as f:
        f.write(json.dumps({"meta": meta}, ensure_ascii=False) + "\n")
        for w in sorted(data):
            f.write(json.dumps({"w": w, "s": data[w]}, ensure_ascii=False) + "\n")
    # NOTICE de atribuicao (respeito de licenca p/ OSS)
    with open(os.path.join(outdir, "NOTICE.txt"), "w", encoding="utf-8") as f:
        f.write(f"{code} synonyms — derived from:\n  {info['source']}\n  {info['source_url']}\n  License: {info['license']}\n")
    return path, meta


def build_one(code, refetch=False):
    info = SOURCES[code]
    print(f"[{code}] {info['source']} ({info['license']})")
    paths = [fetch(u, n, refetch) for (u, n) in info["fetch"]]
    raw = PARSERS[info["parser"]](paths[0])
    data = finalize(raw)
    path, meta = write_jsonl(code, info, data)
    avg = sum(len(v) for v in data.values()) / max(1, len(data))
    print(f"  -> {path}  ({meta['entries']} palavras, media {avg:.1f} syns)")


def load_bridge_tei(url, fname, member, refetch=False):
    """Baixa o tar.xz do FreeDict, extrai o .tei e retorna {palavra_origem: [traducoes]}."""
    import tarfile
    os.makedirs(SRC, exist_ok=True)
    arc = os.path.join(SRC, fname)
    if not os.path.exists(arc) or refetch:
        print(f"  baixando ponte {url}")
        req = urllib.request.Request(url, headers={"User-Agent": "RAGnaRock-thesaurus-builder"})
        with urllib.request.urlopen(req, timeout=180) as r, open(arc, "wb") as out:
            out.write(r.read())
    tei = os.path.join(SRC, os.path.basename(member))
    if not os.path.exists(tei) or refetch:
        with tarfile.open(arc) as t:
            for m in t.getmembers():
                if m.name.endswith(member.split("/")[-1]):
                    with t.extractfile(m) as fsrc, open(tei, "wb") as fdst:
                        fdst.write(fsrc.read())
                    break
    text = open(tei, encoding="utf-8", errors="replace").read()
    d = {}
    for m in re.finditer(r"<entry>(.*?)</entry>", text, re.S):
        body = m.group(1)
        orth = re.search(r"<orth>(.*?)</orth>", body, re.S)
        if not orth:
            continue
        word = clean_term(re.sub(r"<.*?>", "", orth.group(1)))
        trans = [clean_term(re.sub(r"<.*?>", "", q)) for q in
                 re.findall(r'<cit type="trans">.*?<quote>(.*?)</quote>', body, re.S)]
        trans = [t for t in trans if t]
        if word and trans:
            d.setdefault(word, []).extend(trans)
    return d


def load_our_jsonl(code):
    path = os.path.join(THES, code, "synonyms.jsonl")
    d = {}
    with open(path, encoding="utf-8") as f:
        next(f)  # meta
        for line in f:
            e = json.loads(line)
            d[e["w"]] = e["s"]
    return d


def compose_cross(code, refetch=False):
    info = COMPOSE[code]
    print(f"[{code}] {info['source']}")
    bridge = load_bridge_tei(*info["bridge"], refetch=refetch)
    syn = load_our_jsonl(info["syn_mono"])
    print(f"  ponte {len(bridge)} palavras · mono {info['syn_mono']} {len(syn)} palavras")
    out = {}
    for word, direct in bridge.items():
        # CROSS(palavra) = traduz(palavra) U traduz(sinonimos da palavra)
        bag = list(direct)
        for s in syn.get(word, []):
            bag.extend(bridge.get(s, []))
        out[word] = bag
    data = finalize(out)
    path, meta = write_jsonl(code, info, data)
    avg = sum(len(v) for v in data.values()) / max(1, len(data))
    print(f"  -> {path}  ({meta['entries']} palavras, media {avg:.1f} traducoes)")


def main():
    ap = argparse.ArgumentParser(add_help=False)
    ap.add_argument("--list", action="store_true")
    ap.add_argument("--only")
    ap.add_argument("--all", action="store_true")
    ap.add_argument("--compose")
    ap.add_argument("--refetch", action="store_true")
    args, _ = ap.parse_known_args()

    if not any([args.list, args.only, args.all, args.compose]):
        print(__doc__)
        sys.exit(0)
    if args.list:
        for c, i in SOURCES.items():
            print(f"  {c}  {i['kind']:5}  {i['license']:14}  {i['source']}")
        for c in COMPOSE:
            print(f"  {c}  cross  (composicao)")
        return
    if args.only:
        build_one(args.only.upper(), args.refetch)
    elif args.all:
        for c in SOURCES:
            build_one(c, args.refetch)
    elif args.compose:
        compose_cross(args.compose.upper(), args.refetch)


if __name__ == "__main__":
    main()
