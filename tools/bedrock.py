#!/usr/bin/env python3
"""Cliente mínimo do Bedrock — SigV4 na unha, só stdlib. Zero dependência.

Lê as credenciais de ~/.aws/credentials (perfil default), como a CLI faria.
Uso:
    python3 bedrock.py models  [região]
    python3 bedrock.py converse <modelId> <arquivo.json>   # {"system": "...", "user": "..."}
"""
import configparser, datetime, hashlib, hmac, json, os, pathlib, sys, urllib.error, urllib.parse, urllib.request


def creds():
    cfg = configparser.ConfigParser()
    cfg.read(os.path.expanduser("~/.aws/credentials"))
    p = cfg["default"]
    return p["aws_access_key_id"].strip(), p["aws_secret_access_key"].strip()


def _sign(key, msg):
    return hmac.new(key, msg.encode(), hashlib.sha256).digest()


def call(service, host, method, path, region, payload="", query="", canon_path=None):
    """`canon_path` = o path como a AWS o normaliza para ASSINAR (com ':' → '%3A'); a URL
    enviada continua com o caractere literal. O Bedrock exige essa assimetria por causa do
    ':' no modelId — assinar e enviar iguais dá 403 nos dois sentidos."""
    ak, sk = creds()
    t = datetime.datetime.now(datetime.timezone.utc)
    amzdate, datestamp = t.strftime("%Y%m%dT%H%M%SZ"), t.strftime("%Y%m%d")
    ph = hashlib.sha256(payload.encode()).hexdigest()
    cp = canon_path or path
    creq = f"{method}\n{cp}\n{query}\nhost:{host}\nx-amz-date:{amzdate}\n\nhost;x-amz-date\n{ph}"
    scope = f"{datestamp}/{region}/{service}/aws4_request"
    sts = f"AWS4-HMAC-SHA256\n{amzdate}\n{scope}\n{hashlib.sha256(creq.encode()).hexdigest()}"
    k = _sign(_sign(_sign(_sign(("AWS4" + sk).encode(), datestamp), region), service), "aws4_request")
    sig = hmac.new(k, sts.encode(), hashlib.sha256).hexdigest()
    url = f"https://{host}{path}" + (f"?{query}" if query else "")
    r = urllib.request.Request(url, data=payload.encode() if payload else None, method=method)
    r.add_header("x-amz-date", amzdate)
    r.add_header("Authorization",
                 f"AWS4-HMAC-SHA256 Credential={ak}/{scope}, "
                 f"SignedHeaders=host;x-amz-date, Signature={sig}")
    if payload:
        r.add_header("Content-Type", "application/json")
    try:
        with urllib.request.urlopen(r, timeout=180) as resp:
            return resp.read().decode()
    except urllib.error.HTTPError as e:
        return json.dumps({"__erro__": e.code, "corpo": e.read().decode()[:500]})


def converse(model, region, system, user, max_tokens=2000):
    """Converse API — o mesmo contrato que o nidhoggd usa via OpenAI, traduzido."""
    # a geração 5 da Anthropic depreciou `temperature` — mandar o campo dá 400
    cfg = {"maxTokens": max_tokens}
    if "-5" not in model.rsplit(".", 1)[-1]:
        cfg["temperature"] = 0
    body = json.dumps({
        "messages": [{"role": "user", "content": [{"text": user}]}],
        "system": [{"text": system}],
        "inferenceConfig": cfg,
    })
    out = call("bedrock", f"bedrock-runtime.{region}.amazonaws.com", "POST",
               f"/model/{model}/converse", region, body,
               canon_path=f"/model/{urllib.parse.quote(model, safe='')}/converse")
    d = json.loads(out)
    if "__erro__" in d:
        return None, d
    txt = "".join(c.get("text", "") for c in d["output"]["message"]["content"])
    return txt, {"tokens_in": d["usage"]["inputTokens"], "tokens_out": d["usage"]["outputTokens"],
                 "ms": d.get("metrics", {}).get("latencyMs"), "stop": d.get("stopReason")}


if __name__ == "__main__":
    cmd = sys.argv[1]
    if cmd == "models":
        reg = sys.argv[2] if len(sys.argv) > 2 else "us-east-1"
        out = call("bedrock", f"bedrock.{reg}.amazonaws.com", "GET", "/foundation-models", reg)
        d = json.loads(out)
        for m in d.get("modelSummaries", []):
            print(m["modelId"])
    elif cmd == "converse":
        model, arq = sys.argv[2], sys.argv[3]
        reg = sys.argv[4] if len(sys.argv) > 4 else "us-east-1"
        j = json.loads(pathlib.Path(arq).read_text())
        txt, meta = converse(model, reg, j["system"], j["user"])
        print(json.dumps({"modelo": model, "meta": meta, "resposta": txt}, ensure_ascii=False))
