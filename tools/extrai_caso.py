#!/usr/bin/env python3
"""Extrai do llm-ledger a chamada REAL do L3 numa base — system + user idênticos ao que o
Qwen local recebeu. É o que permite o A/B honesto: só o modelo muda."""
import json, sys, pathlib

alvo = sys.argv[1] if len(sys.argv) > 1 else "BRIEFING"
saida = pathlib.Path(sys.argv[2] if len(sys.argv) > 2 else "caso.json")
casos = []
for ln in sys.stdin:
    try:
        e = json.loads(ln)
    except Exception:
        continue
    if e.get("tag") == "relacoes" and alvo in str(e.get("ctx", "")):
        casos.append(e)
if not casos:
    print("nenhuma chamada encontrada para", alvo)
    sys.exit(1)
e = casos[-1]
msgs = e["messages"]
system = next(m["content"] for m in msgs if m["role"] == "system")
user = next(m["content"] for m in msgs if m["role"] == "user")
saida.write_text(json.dumps({"system": system, "user": user}, ensure_ascii=False))
print("caso:", e["ctx"], "|", e["ts"])
print("entidades:", user.split("\n")[0][:300])
print("resposta do QWEN LOCAL (7B-Q4):")
try:
    r = json.loads(e["resposta"][e["resposta"].index("{"):e["resposta"].rindex("}") + 1])
    for x in r.get("relacoes", []):
        print("   %s —[%s]→ %s" % (x.get("a"), x.get("rel"), x.get("b")))
except Exception:
    print("  ", str(e.get("resposta"))[:300])
