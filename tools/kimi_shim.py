#!/usr/bin/env python3
"""Shim OpenAI → Kimi Code (conta de assinatura, api.kimi.com/coding/v1).

Irmão do `bedrock_shim.py`: o nidhoggd fala OpenAI-compatible e não muda uma linha de
Rust — troca-se apenas `llm_url` no cfg:

    llm_url = http://127.0.0.1:8082/v1/chat/completions

Uso:
    kimi_shim.py [--port 8082] [--model kimi-for-coding] [--thinking]
Sem argumentos mostra este help (convenção do repo).

⚠️ RODA SOB O PYTHON DO kimi-cli, não sob o do sistema:

    ~/.local/share/uv/tools/kimi-cli/bin/python tools/kimi_shim.py --port 8082

Isso quebra de propósito a invariante "stdlib puro" do bedrock_shim, e a razão é dura:
o `refresh_token` da conta **rotaciona** a cada renovação (`OAuthToken.from_response` o
exige de volta) e o access token dura ~20 min. Um refresh feito por fora invalidaria a
credencial que o `kimi` interativo está segurando — logout aleatório na outra ponta. Ao
reusar `kimi_cli.auth.oauth` herdamos o mesmo arquivo, o mesmo lock entre processos e a
mesma política de renovação. O preço é o acoplamento a uma API privada: se um
`uv tool upgrade kimi-cli` mudar esses nomes, o shim para e o import falha alto, no boot.

## As três traduções que este shim faz

1. **`temperature` é REMOVIDA.** A API recusa qualquer valor ≠ 1
   (`400 invalid temperature: only 1 is allowed for this model`) e o nidhoggd manda 0.
   Consequência aceita pelo Pacman em 15/ago: **não há determinismo nesta rota**. O
   comparador do L4 ("mudou a perspectiva?") passa a decidir sobre saída amostrada.
2. **`thinking` desligado por padrão.** Medido: um prompt de `relacoes` gasta 241
   reasoning_tokens e 7,3s com thinking ligado, contra 0 e 2,1s com
   `thinking={"type":"disabled"}`. Como os reasoning_tokens saem do mesmo `max_tokens`
   (2000 no relacoes, 1500 no analista), deixá-lo ligado encurta a resposta útil e
   empurra janelas para o caminho de "janela ruim". `--thinking` religa.
3. **Cerca ```json desembrulhada** na volta — o modelo embrulha mesmo mandado.

`response_format: json_schema` passa INTACTO: aqui ele funciona nativo, diferente do
Bedrock Converse (que não tem equivalente e obrigou instrução dura no system).
"""
import asyncio, json, os, sys, time
import urllib.error, urllib.request
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

BASE_URL = os.getenv("KIMI_CODE_BASE_URL", "https://api.kimi.com/coding/v1")
MODELO_PADRAO = "kimi-for-coding"
PORTA_PADRAO = 8082
OAUTH_KEY = "oauth/kimi-code"
MARGEM_RENOVACAO = 90        # renova com esta folga (s) antes do vencimento

CFG = {"model": MODELO_PADRAO, "thinking": False}


# ───────────────────────────── credencial (via kimi-cli) ─────────────────────────────
def _oauth():
    """Importa a máquina de OAuth do kimi-cli. Falha alto e cedo: sem isto o shim não serve."""
    try:
        from kimi_cli.auth.oauth import load_tokens, refresh_token, save_tokens
        from kimi_cli.config import OAuthRef
    except ImportError as e:
        sys.exit(f"kimi_shim: rode sob o Python do kimi-cli "
                 f"(~/.local/share/uv/tools/kimi-cli/bin/python). Import falhou: {e}")
    return load_tokens, refresh_token, save_tokens, OAuthRef


def access_token():
    """Token válido, renovando sob o lock do próprio kimi-cli quando falta pouco.

    A releitura depois de pegar o lock não é paranoia: o `kimi` interativo pode ter
    renovado no intervalo, e como o refresh_token rotaciona, gastar o antigo derruba
    a sessão dele."""
    load_tokens, refresh, save_tokens, OAuthRef = _oauth()
    ref = OAuthRef(storage="file", key=OAUTH_KEY)
    tok = load_tokens(ref)
    if tok is None:
        raise RuntimeError("sem credencial do Kimi — rode `kimi` e faça /login nesta máquina")
    if tok.expires_at - time.time() > MARGEM_RENOVACAO:
        return tok.access_token
    novo = asyncio.run(refresh(tok.refresh_token))
    save_tokens(ref, novo)
    return novo.access_token


# ───────────────────────────── tradução do payload ─────────────────────────────
def openai_para_kimi(req):
    """OpenAI → OpenAI, com as 2 correções que a conta exige (ver docstring do módulo)."""
    p = dict(req)
    p.pop("temperature", None)       # a API só aceita 1; mandar 0 é 400
    p.pop("top_p", None)             # mesma família de restrição
    p["model"] = req.get("model") or CFG["model"]
    if not CFG["thinking"]:
        p["thinking"] = {"type": "disabled"}
    p.pop("stream", None)            # o nidhoggd não usa streaming

    # O nidhoggd monta `json_schema: {"schema": …}` sem `name` — o llama.cpp aceita, esta
    # API não (`missing required parameter: 'response_format.json_schema.name'`). Batizar
    # aqui é mais barato que mexer no Rust e mantém as duas rotas com o mesmo binário.
    rf = p.get("response_format")
    if isinstance(rf, dict) and rf.get("type") == "json_schema":
        js = dict(rf.get("json_schema") or {})
        js.setdefault("name", "resposta")
        p["response_format"] = {"type": "json_schema", "json_schema": js}
    return p


def desembrulha(txt):
    """Tira a cerca ```json que o modelo põe mesmo quando mandado responder só JSON."""
    s = (txt or "").strip()
    if not s.startswith("```") or "\n" not in s:
        return txt          # sem cerca, ou cerca sem corpo: devolve intacto, nunca vazio
    s = s.split("\n", 1)[1]
    if s.rstrip().endswith("```"):
        s = s.rstrip()[:-3]
    return s.strip()


def chama_kimi(payload):
    body = json.dumps(payload).encode()
    req = urllib.request.Request(
        f"{BASE_URL}/chat/completions", data=body,
        headers={"Authorization": "Bearer " + access_token(),
                 "Content-Type": "application/json"})
    with urllib.request.urlopen(req, timeout=300) as r:
        return json.load(r)


# ───────────────────────────── servidor ─────────────────────────────
class Handler(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def log_message(self, *a):
        pass   # silencia o log padrão; quem loga é o _log

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
        rota = self.path.rstrip("/")
        # /v1/models é o que o check_llm() do nidhoggd sonda: sem isto o painel de
        # saúde pinta o modelo de vermelho em TODAS as telas (commit c9292b3).
        if rota in ("/v1/models", "/models"):
            self._json(200, {"object": "list",
                             "data": [{"id": CFG["model"], "object": "model"}]})
        elif rota in ("/health", "/v1/health"):
            try:
                tok = access_token()
                self._json(200, {"status": "ok", "model": CFG["model"],
                                 "auth": "ok" if tok else "sem token",
                                 "thinking": CFG["thinking"]})
            except Exception as e:
                self._json(503, {"status": "erro", "error": str(e)})
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

        t0 = time.time()
        try:
            resp = chama_kimi(openai_para_kimi(req))
        except urllib.error.HTTPError as e:
            corpo = e.read().decode(errors="replace")[:400]
            # Repassa o status ORIGINAL. Importa para 429/401: o nidhoggd trata resposta
            # não-JSON como janela falha (pulada, sem checkpoint) — que é o certo para
            # limite de uso, pois a base volta pra fila em vez de ficar marcada como
            # mastigada. Transformar isso em 200 com corpo vazio estragaria o checkpoint.
            self._log(f"upstream HTTP {e.code}: {corpo}")
            self._json(e.code, {"error": {"message": corpo, "code": e.code}})
            return
        except Exception as e:
            self._log(f"falha: {type(e).__name__}: {e}")
            self._json(502, {"error": {"message": str(e)}})
            return

        try:
            msg = resp["choices"][0]["message"]
            msg["content"] = desembrulha(msg.get("content", ""))
        except (KeyError, IndexError):
            pass

        uso = resp.get("usage", {}) or {}
        self._log(f"{CFG['model']} {time.time()-t0:.1f}s "
                  f"in={uso.get('prompt_tokens')} out={uso.get('completion_tokens')}")
        self._json(200, resp)


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
        elif a == "--thinking":
            CFG["thinking"] = True
        elif a in ("-h", "--help"):
            print(__doc__)
            return
    access_token()   # falha cedo e alto se não houver login nesta máquina
    print(f"shim OpenAI→Kimi em http://127.0.0.1:{porta}/v1/chat/completions", flush=True)
    print(f"  modelo={CFG['model']} thinking={CFG['thinking']} base={BASE_URL}", flush=True)
    ThreadingHTTPServer(("127.0.0.1", porta), Handler).serve_forever()


if __name__ == "__main__":
    main()
