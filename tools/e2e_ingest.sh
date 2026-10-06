#!/usr/bin/env bash
# e2e_ingest.sh — teste ponta a ponta dos drivers de ingestão de ARQUIVO (#50).
#
# Sobe um ragd DESCARTÁVEL (porta própria, pasta temporária, nada da produção é gravado), manda
# cada formato pelo POST /ingest_any e confere, para cada um:
#   1. o driver que o daemon escolheu (campo `driver` da resposta) — prova o roteamento MIME/extensão;
#   2. que a busca acha a palavra-âncora daquele arquivo na base criada — prova o cano inteiro
#      arquivo → driver → texto → base tokenizada → busca.
# No fim derruba o ragd e apaga a pasta. Os drivers de banco (mysql/postgres) ficam de fora.
#
# Uso (na máquina do servidor, que tem as dependências dos drivers):
#   tools/e2e_ingest.sh [--ragd BIN] [--ingestors DIR] [--drivers DIR] [--port N]
#                       [--audio ARQUIVO --audio-palavra PALAVRA]
# Padrões: o binário, os drivers de ingestão e os de linguagem da instalação em /opt/ragnarock.
# Áudio é opcional: passe um recado de fala e uma palavra que ele diz.
set -euo pipefail

RAIZ=$(cd "$(dirname "$0")/.." && pwd)
RAGD=/opt/ragnarock/bin/ragd
INGESTORS=/opt/ragnarock/ingestors
DRIVERS=/opt/ragnarock/drivers
PORTA=11595
AUDIO=""
AUDIO_PALAVRA=""
while [ $# -gt 0 ]; do
  case "$1" in
    --ragd) RAGD=$2; shift 2 ;;
    --ingestors) INGESTORS=$2; shift 2 ;;
    --drivers) DRIVERS=$2; shift 2 ;;
    --port) PORTA=$2; shift 2 ;;
    --audio) AUDIO=$2; shift 2 ;;
    --audio-palavra) AUDIO_PALAVRA=$2; shift 2 ;;
    *) echo "opção desconhecida: $1" >&2; exit 2 ;;
  esac
done
DASH=$((PORTA + 1))

for p in "$PORTA" "$DASH"; do
  if ss -ltn 2>/dev/null | grep -q ":$p "; then
    echo "porta $p ocupada — escolha outra com --port" >&2; exit 2
  fi
done
[ -x "$RAGD" ] || { echo "ragd não encontrado: $RAGD" >&2; exit 2; }
[ -d "$INGESTORS" ] || { echo "drivers de ingestão não encontrados: $INGESTORS" >&2; exit 2; }

TMP=$(mktemp -d -t e2e-ingest-XXXXXX)
PID=""
limpa() {
  [ -n "$PID" ] && kill "$PID" 2>/dev/null && wait "$PID" 2>/dev/null || true
  rm -rf "$TMP"
}
trap limpa EXIT

python3 "$RAIZ/ingestors/tests/fixtures.py" --out "$TMP/exemplos" > /dev/null
cat > "$TMP/ragd.cfg" <<EOF
api_port      = $PORTA
dash_port     = $DASH
drivers_dir   = $DRIVERS
ingestors_dir = $INGESTORS
ragfiles_dir  = $TMP/ragfiles
cache_dir     = $TMP/cache
thesaurus_dir = $TMP/thesaurus
log_file      = $TMP/ragd.log
autoload      = true
workers       = 4
EOF
(cd "$TMP" && exec "$RAGD" --config "$TMP/ragd.cfg" --dev > "$TMP/stdout.log" 2>&1) &
PID=$!
for _ in $(seq 1 60); do
  curl -s -m2 "localhost:$PORTA/health" > /dev/null && break
  kill -0 "$PID" 2>/dev/null || { echo "o ragd de teste não subiu:" >&2; tail -5 "$TMP/stdout.log" >&2; exit 1; }
  sleep 0.5
done

E2E_PORTA=$PORTA E2E_TMP=$TMP E2E_AUDIO=$AUDIO E2E_AUDIO_PALAVRA=$AUDIO_PALAVRA \
  python3 - "$RAIZ/ingestors/tests" <<'PY'
import json, os, sys, time, urllib.parse, urllib.request
sys.path.insert(0, sys.argv[1])
from fixtures import ANCORAS
porta, tmp = os.environ["E2E_PORTA"], os.environ["E2E_TMP"]
X = "application/vnd.openxmlformats-officedocument."
# (formato, arquivo, Content-Type, driver esperado, palavra que a busca tem que achar)
casos = [
    ("csv  (pelo MIME)",          "exemplo.csv",  "text/csv",                          "csv.py",  ANCORAS["csv"]),
    ("csv  (text/plain + .csv)",  "exemplo.csv",  "text/plain",                        "csv.py",  ANCORAS["csv"]),
    ("xlsx",                      "exemplo.xlsx", X + "spreadsheetml.sheet",            "xlsx.py", ANCORAS["xlsx"]),
    ("docx",                      "exemplo.docx", X + "wordprocessingml.document",      "docx.py", ANCORAS["docx"]),
    ("pptx (octet-stream + ext)", "exemplo.pptx", "application/octet-stream",          "pptx.py", ANCORAS["pptx"]),
    ("pdf",                       "exemplo.pdf",  "application/pdf",                   "pdf.py",  ANCORAS["pdf"]),
]
if os.environ.get("E2E_AUDIO"):
    caminho = os.environ["E2E_AUDIO"]
    casos.append(("audio (octet-stream + ext)", caminho, "application/octet-stream", "audio.py",
                  os.environ.get("E2E_AUDIO_PALAVRA", "")))

def post(rota, dados, ctype):
    req = urllib.request.Request(f"http://127.0.0.1:{porta}{rota}", dados, {"Content-Type": ctype})
    try:
        with urllib.request.urlopen(req, timeout=900) as r:
            return r.status, json.load(r)
    except urllib.error.HTTPError as e:
        return e.code, json.loads(e.read() or b"{}")

falhas = 0
for i, (rotulo, arq, ctype, esperado, palavra) in enumerate(casos):
    caminho = arq if os.path.isabs(arq) else os.path.join(tmp, "exemplos", arq)
    if not os.path.exists(caminho):
        print(f"PULADO  {rotulo:28} exemplo não gerado (dependência ausente)"); continue
    nome = f"caso{i}"
    q = urllib.parse.urlencode({"filename": os.path.basename(caminho), "collection": "e2e", "name": nome})
    t0 = time.time()
    code, r = post(f"/ingest_any?{q}", open(caminho, "rb").read(), ctype)
    ms = (time.time() - t0) * 1000
    problemas = []
    if code != 200:
        problemas.append(f"HTTP {code}: {r.get('error')}")
    elif r.get("driver") != esperado:
        problemas.append(f"driver {r.get('driver')!r}, esperado {esperado!r}")
    if not problemas and palavra:
        _, s = post("/search", json.dumps({"collection": "e2e", "base": nome, "query": palavra, "k": 1}).encode(),
                    "application/json")
        hits = s.get("hits") or []
        if not hits or palavra.lower() not in (hits[0].get("snippet") or hits[0].get("text") or "").lower():
            problemas.append(f"a busca por {palavra!r} não achou o texto na base")
    if problemas:
        falhas += 1
        print(f"FALHOU  {rotulo:28} {'; '.join(problemas)}")
    else:
        print(f"ok      {rotulo:28} driver={esperado:8} {ms:7.0f} ms" + (f"  busca '{palavra}' ✓" if palavra else ""))
print(f"\n{len(casos) - falhas}/{len(casos)} casos ok")
sys.exit(1 if falhas else 0)
PY
