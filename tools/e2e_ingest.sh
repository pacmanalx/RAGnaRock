#!/usr/bin/env bash
# e2e_ingest.sh — teste ponta a ponta dos drivers de ingestão de ARQUIVO (#50).
#
# Sobe um ragd DESCARTÁVEL (porta própria, pasta temporária, nada da produção é gravado), manda
# cada formato pelo POST /ingest_any e confere, para cada um:
#   1. o driver que o daemon escolheu (campo `driver` da resposta) — prova o roteamento MIME/extensão;
#   2. que a busca acha a palavra-âncora daquele arquivo na base criada — prova o cano inteiro
#      arquivo → driver → texto → base tokenizada → busca.
# No fim derruba o ragd e apaga a pasta.
#
# --bancos: também os drivers de BANCO. Sobe MySQL (mysql:8.4) e Postgres (postgres:16-alpine) em
# containers DESCARTÁVEIS (--rm, só em 127.0.0.1, porta aleatória, senha gerada na hora e nunca
# impressa), semeia uma tabela e manda as receitas pelo /ingest_any. Confere ainda que uma senha
# errada é recusada sem aparecer no erro e que a senha não ficou gravada em disco nem em log.
# Nunca aponta para um banco real.
#
# Uso (na máquina do servidor, que tem as dependências dos drivers):
#   tools/e2e_ingest.sh [--ragd BIN] [--ingestors DIR] [--drivers DIR] [--port N] [--bancos]
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
BANCOS=""
while [ $# -gt 0 ]; do
  case "$1" in
    --ragd) RAGD=$2; shift 2 ;;
    --ingestors) INGESTORS=$2; shift 2 ;;
    --drivers) DRIVERS=$2; shift 2 ;;
    --port) PORTA=$2; shift 2 ;;
    --audio) AUDIO=$2; shift 2 ;;
    --audio-palavra) AUDIO_PALAVRA=$2; shift 2 ;;
    --bancos) BANCOS=1; shift ;;
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
CONTAINERS=()
limpa() {
  local rc=$?
  if [ "$rc" -ne 0 ] && [ -f "$TMP/stdout.log" ]; then
    echo "--- fim da saída do ragd de teste:" >&2; tail -8 "$TMP/stdout.log" >&2
  fi
  [ -n "$PID" ] && kill "$PID" 2>/dev/null && wait "$PID" 2>/dev/null || true
  for c in "${CONTAINERS[@]}"; do docker rm -f "$c" > /dev/null 2>&1 || true; done
  rm -rf "$TMP"
}
trap limpa EXIT

# bancos efêmeros: sobem primeiro (o MySQL leva ~20 s inicializando) e ficam prontos durante o resto
MYSQL_PORTA=""; PG_PORTA=""; SENHA=""
if [ -n "$BANCOS" ]; then
  command -v docker > /dev/null || { echo "--bancos precisa de docker" >&2; exit 2; }
  SENHA=$(python3 -c 'import secrets; print(secrets.token_urlsafe(18))')
  CM="e2e-ingest-mysql-$$"; CP="e2e-ingest-pg-$$"
  CONTAINERS=("$CM" "$CP")
  docker run -d --rm --name "$CM" -p 127.0.0.1::3306 -e MYSQL_ROOT_PASSWORD="$SENHA" \
    -e MYSQL_DATABASE=e2e -e MYSQL_USER=leitor -e MYSQL_PASSWORD="$SENHA" mysql:8.4 > /dev/null
  docker run -d --rm --name "$CP" -p 127.0.0.1::5432 -e POSTGRES_PASSWORD="$SENHA" \
    -e POSTGRES_DB=e2e -e POSTGRES_USER=leitor postgres:16-alpine > /dev/null
  MYSQL_PORTA=$(docker port "$CM" 3306/tcp | head -1 | sed 's/.*://')
  PG_PORTA=$(docker port "$CP" 5432/tcp | head -1 | sed 's/.*://')
fi

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
E2E_SENHA=$SENHA E2E_MYSQL=$MYSQL_PORTA E2E_PG=$PG_PORTA \
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
# ── bancos efêmeros: espera ficarem prontos, semeia e monta as receitas ──
senha = os.environ.get("E2E_SENHA", "")
bancos = {}
if senha:
    def conecta(qual, porta):
        if qual == "mysql":
            import pymysql
            return pymysql.connect(host="127.0.0.1", port=int(porta), user="leitor", password=senha,
                                   database="e2e", connect_timeout=3, charset="utf8mb4", autocommit=True)
        import psycopg2
        c = psycopg2.connect(host="127.0.0.1", port=int(porta), user="leitor", password=senha,
                             dbname="e2e", connect_timeout=3)
        c.autocommit = True
        return c
    # p_banco, não `porta`: `porta` é a do ragd de teste (sobrescrevê-la mandava o POST ao banco)
    for qual, p_banco, ancora in (("mysql", os.environ["E2E_MYSQL"], "Itaquaquecetuba"),
                                  ("postgres", os.environ["E2E_PG"], "Pirassununga")):
        prazo, t_pronto = time.time() + 180, time.time()
        while True:
            try:
                conn = conecta(qual, p_banco)
                print(f"        {qual} efêmero pronto em {time.time() - t_pronto:.0f} s (porta {p_banco})", flush=True)
                break
            except Exception as e:
                if time.time() > prazo:
                    print(f"FALHOU  {qual} efêmero não ficou pronto: {' '.join(str(e).split())}"); sys.exit(1)
                time.sleep(2)
        with conn.cursor() as cur:
            cur.execute("CREATE TABLE cidades (id INT PRIMARY KEY, nome VARCHAR(80), obs VARCHAR(200) NULL)")
            cur.execute("INSERT INTO cidades VALUES (1, %s, 'fábrica de pão e café'), (2, 'Jaboticabal', NULL)",
                        (ancora,))
        conn.close()
        bancos[qual] = (p_banco, ancora)

def receita(qual, p_banco, pw, sql="SELECT id, nome, obs FROM cidades ORDER BY id;"):
    return (f"-- host: 127.0.0.1:{p_banco}\n-- db: e2e\n-- user: leitor\n-- pass: {pw}\n"
            f"-- receita efêmera do teste ponta a ponta\n{sql}\n").encode()

for qual, (p_banco, ancora) in bancos.items():
    casos.append((f"{qual} (receita .{qual})", (f"receita.{qual}", receita(qual, p_banco, senha)),
                  "text/plain", f"{qual}.py", ancora))

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
    if isinstance(arq, tuple):                  # (nome do arquivo, bytes) — receitas, montadas aqui
        arquivo, dados = arq
    else:
        caminho = arq if os.path.isabs(arq) else os.path.join(tmp, "exemplos", arq)
        if not os.path.exists(caminho):
            print(f"PULADO  {rotulo:28} exemplo não gerado (dependência ausente)"); continue
        arquivo, dados = os.path.basename(caminho), open(caminho, "rb").read()
    nome = f"caso{i}"
    q = urllib.parse.urlencode({"filename": arquivo, "collection": "e2e", "name": nome})
    t0 = time.time()
    code, r = post(f"/ingest_any?{q}", dados, ctype)
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
# ── sigilo da senha (só com --bancos) ──
extras = 0
if bancos:
    qual, (p_banco, _) = next(iter(bancos.items()))
    errada = "senha-errada-" + senha[:6]
    q = urllib.parse.urlencode({"filename": f"errada.{qual}", "collection": "e2e", "name": "errada"})
    code, r = post(f"/ingest_any?{q}", receita(qual, p_banco, errada), "text/plain")
    erro = r.get("error") or ""
    extras += 1
    if code == 200 or "falha ao conectar" not in erro or errada in erro:
        falhas += 1; print(f"FALHOU  {'senha errada':28} HTTP {code}: {erro[:120]}")
    else:
        print(f"ok      {'senha errada':28} recusada (HTTP {code}) sem mostrar a senha")
    extras += 1
    vazou = []
    for raiz, _, arqs in os.walk(tmp):
        if os.path.join(tmp, "exemplos") in raiz:
            continue
        for a in arqs:
            try:
                conteudo = open(os.path.join(raiz, a), "rb").read()
            except OSError:
                continue
            if senha.encode() in conteudo or errada.encode() in conteudo:
                vazou.append(os.path.relpath(os.path.join(raiz, a), tmp))
    if vazou:
        falhas += 1; print(f"FALHOU  {'senha fora do disco/log':28} encontrada em: {', '.join(vazou)}")
    else:
        print(f"ok      {'senha fora do disco/log':28} nenhuma senha nas bases, no log nem na saída do ragd")
total = len(casos) + extras
print(f"\n{total - falhas}/{total} casos ok")
sys.exit(1 if falhas else 0)
PY
