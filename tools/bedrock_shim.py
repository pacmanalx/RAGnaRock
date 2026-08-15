#!/usr/bin/env python3
"""Shim OpenAI → AWS Bedrock Converse. Só stdlib, zero dependência.

O nidhoggd (e o ragd) falam OpenAI-compatible: POST /v1/chat/completions com
{messages, temperature, max_tokens}. O Bedrock fala Converse e exige SigV4. Este processo
fica no meio: recebe OpenAI, chama Bedrock, devolve OpenAI.

Assim o motor em Rust não muda — troca-se apenas `llm_url` no cfg:
    llm_url = http://127.0.0.1:8081/v1/chat/completions

Uso:
    bedrock_shim.py [--port 8081] [--model <id>] [--region us-east-1]
Sem argumentos mostra este help (convenção do repo).

Credenciais: ~/.aws/credentials, perfil default (o mesmo que a CLI usaria).
"""
import configparser, datetime, hashlib, hmac, json, os, sys, time
import urllib.error, urllib.parse, urllib.request
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

MODELO_PADRAO = "moonshotai.kimi-k2.5"   # campeão do bench de 15/ago (docs/2026-08-15_*)
REGIAO_PADRAO = "us-east-1"
PORTA_PADRAO = 8081

CFG = {"model": MODELO_PADRAO, "region": REGIAO_PADRAO}


def creds():
    """Perfil de AWS_PROFILE (ou 'default'). O RAGnaRock roda sob `innovaped` — credencial
    de P&D, só-Bedrock, separada da pessoal: é o que faz o CloudTrail distinguir quem gastou."""
    c = configparser.ConfigParser()
    c.read(os.path.expanduser("~/.aws/credentials"))
    perfil = os.environ.get("AWS_PROFILE", "default")
    if perfil not in c:
        raise RuntimeError(f"perfil '{perfil}' não existe em ~/.aws/credentials")
    p = c[perfil]
    return p["aws_access_key_id"].strip(), p["aws_secret_access_key"].strip()


def _sign(key, msg):
    return hmac.new(key, msg.encode(), hashlib.sha256).digest()


def bedrock_converse(model, region, system, messages, max_tokens, temperature):
    """Chama a Converse API. O ':' do modelId vai percent-encoded na ASSINATURA e literal
    na URL — assinar e enviar iguais dá 403 nos dois sentidos (gotcha medido em 15/ago)."""
    ak, sk = creds()
    host = f"bedrock-runtime.{region}.amazonaws.com"
    path = f"/model/{model}/converse"
    canon = f"/model/{urllib.parse.quote(model, safe='')}/converse"

    cfg = {"maxTokens": max_tokens}
    # a geração 5 da Anthropic depreciou `temperature` — mandar o campo dá 400
    if temperature is not None and "-5" not in model.rsplit(".", 1)[-1]:
        cfg["temperature"] = temperature
    body = {"messages": messages, "inferenceConfig": cfg}
    if system:
        body["system"] = [{"text": system}]
    payload = json.dumps(body)

    t = datetime.datetime.now(datetime.timezone.utc)
    amzdate, datestamp = t.strftime("%Y%m%dT%H%M%SZ"), t.strftime("%Y%m%d")
    ph = hashlib.sha256(payload.encode()).hexdigest()
    creq = f"POST\n{canon}\n\nhost:{host}\nx-amz-date:{amzdate}\n\nhost;x-amz-date\n{ph}"
    scope = f"{datestamp}/{region}/bedrock/aws4_request"
    sts = f"AWS4-HMAC-SHA256\n{amzdate}\n{scope}\n{hashlib.sha256(creq.encode()).hexdigest()}"
    k = _sign(_sign(_sign(_sign(("AWS4" + sk).encode(), datestamp), region), "bedrock"), "aws4_request")
    sig = hmac.new(k, sts.encode(), hashlib.sha256).hexdigest()

    req = urllib.request.Request(f"https://{host}{path}", data=payload.encode(), method="POST")
    req.add_header("x-amz-date", amzdate)
    req.add_header("Content-Type", "application/json")
    req.add_header("Authorization",
                   f"AWS4-HMAC-SHA256 Credential={ak}/{scope}, "
                   f"SignedHeaders=host;x-amz-date, Signature={sig}")
    with urllib.request.urlopen(req, timeout=180) as r:
        return json.loads(r.read().decode())


def openai_para_bedrock(req):
    """{messages:[{role,content}]} → (system, [{role, content:[{text}]}]).
    Bedrock separa o system do array e exige alternância user/assistant."""
    system, msgs = None, []
    for m in req.get("messages", []):
        papel, texto = m.get("role"), m.get("content", "")
        if isinstance(texto, list):   # content multimodal → concatena as partes de texto
            texto = "".join(p.get("text", "") for p in texto if isinstance(p, dict))
        if papel == "system":
            system = f"{system}\n\n{texto}" if system else texto
        elif papel in ("user", "assistant"):
            # Bedrock recusa dois turnos seguidos do mesmo papel — funde
            if msgs and msgs[-1]["role"] == papel:
                msgs[-1]["content"][0]["text"] += "\n\n" + texto
            else:
                msgs.append({"role": papel, "content": [{"text": texto}]})
    if not msgs:
        msgs = [{"role": "user", "content": [{"text": ""}]}]
    return system, msgs


class Handler(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def log_message(self, *a):
        pass   # silencia o log padrão; quem loga é o _log abaixo

    def _log(self, msg):
        print(f"[{time.strftime('%Y-%m-%d %H:%M:%S')}] {msg}", flush=True)

    def _json(self, code, obj):
        b = json.dumps(obj).encode()
        self.send_response(code)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(b)))
        self.end_headers()
        self.wfile.write(b)

    def do_GET(self):
        if self.path.rstrip("/") in ("/v1/models", "/models"):
            self._json(200, {"object": "list",
                             "data": [{"id": CFG["model"], "object": "model"}]})
        elif self.path.rstrip("/") in ("/health", "/v1/health"):
            self._json(200, {"status": "ok", "model": CFG["model"], "region": CFG["region"]})
        else:
            self._json(404, {"error": {"message": f"rota não encontrada: {self.path}"}})

    def do_POST(self):
        if self.path.rstrip("/") not in ("/v1/chat/completions", "/chat/completions"):
            self._json(404, {"error": {"message": f"rota não encontrada: {self.path}"}})
            return
        n = int(self.headers.get("Content-Length", 0))
        try:
            req = json.loads(self.rfile.read(n).decode())
        except Exception as e:
            self._json(400, {"error": {"message": f"JSON inválido: {e}"}})
            return

        modelo = req.get("model") or CFG["model"]
        if modelo in ("local", "default", ""):      # o cfg do nidhoggd manda rótulo, não id
            modelo = CFG["model"]
        system, msgs = openai_para_bedrock(req)
        # COERÇÃO DE JSON — o llama.cpp honra `response_format`; o Bedrock Converse não tem
        # equivalente direto. Sem traduzir isso o modelo responde em prosa e quem chamou
        # quebra no parse (foi o 502 do L4 em 15/ago: resposta boa, formato errado).
        rf = req.get("response_format") or {}
        if isinstance(rf, dict) and rf.get("type") in ("json_object", "json_schema"):
            regra = ("\n\nFORMATO DA SAÍDA: responda EXCLUSIVAMENTE com um objeto JSON válido. "
                     "Sem texto antes ou depois, sem explicação, e SEM cerca de markdown "
                     "(nada de ```json). A primeira letra da resposta deve ser '{'.")
            esquema = (rf.get("json_schema") or {}).get("schema")
            if esquema:
                regra += f"\nO JSON deve obedecer a este schema:\n{json.dumps(esquema, ensure_ascii=False)}"
            system = (system + regra) if system else regra.strip()
        max_tokens = int(req.get("max_tokens") or 2000)
        temp = req.get("temperature", 0)
        t0 = time.time()
        try:
            d = bedrock_converse(modelo, CFG["region"], system, msgs, max_tokens, temp)
        except urllib.error.HTTPError as e:
            corpo = e.read().decode()[:300]
            self._log(f"ERRO {e.code} · {modelo} · {corpo}")
            self._json(e.code, {"error": {"message": corpo, "type": "bedrock_error"}})
            return
        except Exception as e:
            self._log(f"ERRO {type(e).__name__} · {modelo} · {e}")
            self._json(502, {"error": {"message": str(e), "type": "shim_error"}})
            return

        txt = "".join(c.get("text", "") for c in d["output"]["message"]["content"])
        # cerca markdown: mesmo mandado, o modelo às vezes embrulha em ```json … ```
        # (medido no Kimi K2.5). Quem consome espera JSON cru — desembrulha aqui.
        if rf:
            t = txt.strip()
            if t.startswith("```"):
                t = t.split("\n", 1)[1] if "\n" in t else t[3:]
                if t.rstrip().endswith("```"):
                    t = t.rstrip()[:-3]
                txt = t.strip()
        uso = d.get("usage", {})
        ms = int((time.time() - t0) * 1000)
        self._log(f"{modelo} · {ms}ms · in={uso.get('inputTokens')} out={uso.get('outputTokens')} "
                  f"· stop={d.get('stopReason')}")
        # stopReason do Bedrock → finish_reason do OpenAI (o nidhoggd usa 'length' pra
        # detectar resposta cortada e pular a janela)
        fim = {"end_turn": "stop", "max_tokens": "length",
               "stop_sequence": "stop", "content_filtered": "content_filter"}
        self._json(200, {
            "id": f"chatcmpl-shim-{int(time.time()*1000)}",
            "object": "chat.completion",
            "created": int(time.time()),
            "model": modelo,
            "choices": [{"index": 0, "finish_reason": fim.get(d.get("stopReason"), "stop"),
                         "message": {"role": "assistant", "content": txt}}],
            "usage": {"prompt_tokens": uso.get("inputTokens", 0),
                      "completion_tokens": uso.get("outputTokens", 0),
                      "total_tokens": uso.get("totalTokens", 0)},
        })


def main():
    if len(sys.argv) == 1 and sys.stdin.isatty():
        print(__doc__)
        return
    porta = PORTA_PADRAO
    args = sys.argv[1:]
    for i, a in enumerate(args):
        if a == "--port" and i + 1 < len(args):
            porta = int(args[i + 1])
        elif a == "--model" and i + 1 < len(args):
            CFG["model"] = args[i + 1]
        elif a == "--region" and i + 1 < len(args):
            CFG["region"] = args[i + 1]
        elif a in ("-h", "--help"):
            print(__doc__)
            return
    creds()   # falha cedo e alto se não houver credencial
    print(f"shim OpenAI→Bedrock em http://127.0.0.1:{porta}/v1/chat/completions", flush=True)
    print(f"  modelo={CFG['model']} região={CFG['region']}", flush=True)
    ThreadingHTTPServer(("127.0.0.1", porta), Handler).serve_forever()


if __name__ == "__main__":
    main()
