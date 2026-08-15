#!/usr/bin/env python3
"""Bateria A/B: o MESMO caso do L3 em N modelos. Mede o que importa — quantas relações
sobrevivem à âncora do censo, não quantas o modelo cospe."""
import json, pathlib, sys, time
import bedrock

S = pathlib.Path(__file__).parent
ENTS = {"hinode", "lake", "skyone", "trade policy", "regra de comissão"}
DEFEITOS = {"é", "são", "e", "tem", "possui", "está", "foi", "ser"}


def avalia(txt):
    if not txt:
        return None
    i, j = txt.find("{"), txt.rfind("}")
    try:
        rels = json.loads(txt[i:j + 1]).get("relacoes", [])
    except Exception:
        return {"erro": "json inválido", "amostra": txt[:150]}
    passa, vet, defeito, auto = [], 0, 0, 0
    for x in rels:
        a, b = str(x.get("a", "")).strip(), str(x.get("b", "")).strip()
        rel = str(x.get("rel", "")).strip().lower()
        if a.lower() == b.lower():
            auto += 1
        if rel in DEFEITOS:
            defeito += 1
        if a.lower() in ENTS and b.lower() in ENTS:
            passa.append(f"{a} —[{x.get('rel')}]→ {b}")
        else:
            vet += 1
    return {"propostas": len(rels), "passam": len(passa), "vetadas": vet,
            "verbo_ligacao": defeito, "auto_relacao": auto, "boas": passa}


if __name__ == "__main__":
    caso = json.loads((S / sys.argv[1]).read_text())
    modelos = sys.argv[2:]
    for m in modelos:
        t0 = time.time()
        txt, meta = bedrock.converse(m, "us-east-1", caso["system"], caso["user"])
        wall = int((time.time() - t0) * 1000)
        if txt is None:
            print(f"\n### {m}\n  ERRO {meta.get('__erro__')}: {str(meta.get('corpo'))[:180]}")
            continue
        r = avalia(txt)
        print(f"\n### {m}")
        print(f"  {wall} ms · in={meta['tokens_in']} out={meta['tokens_out']} · stop={meta['stop']}")
        if r is None or "erro" in r:
            print("  ", r)
            continue
        print(f"  propostas={r['propostas']} · PASSAM={r['passam']} · vetadas={r['vetadas']} "
              f"· verbo-ligação={r['verbo_ligacao']} · auto-relação={r['auto_relacao']}")
        for b in r["boas"]:
            print("    ✅", b)
