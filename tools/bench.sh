#!/usr/bin/env bash
# bench.sh — sobe o ragd local, roda lotes de query e mede wall-clock médio.
# Uso: bench.sh [PORT] [BIN]   (rodar de dentro da pasta com ragd/ragfiles/drivers)
set -u
PORT="${1:-11599}"
BIN="${2:-./ragd}"
HERE="$(cd "$(dirname "$0")" >/dev/null 2>&1 && pwd)"
# se chamado de tools/, sobe um nível pra achar ragfiles/drivers
[ -d "$HERE/ragfiles" ] && ROOT="$HERE" || ROOT="$(dirname "$HERE")"
cd "$ROOT"

args=()
for f in ragfiles/*/*-tokenized.json; do
  coll=$(basename "$(dirname "$f")"); name=$(basename "$f" -tokenized.json)
  args+=(--preload "${coll}/${name}=${f}")
done

"$BIN" --port "$PORT" "${args[@]}" > /tmp/ragd-bench.log 2>&1 &
PID=$!
trap 'kill $PID 2>/dev/null' EXIT
H="http://127.0.0.1:$PORT"
for i in $(seq 1 80); do curl -sf "$H/health" >/dev/null 2>&1 && break; sleep 0.1; done
HB=$(curl -s "$H/health"); echo "host=$(hostname)  arch=$(uname -m)  $HB"

# warmup
curl -s -o /dev/null -X POST "$H/search" -d '{"base":"*","query":"anel","k":5}'

N=25
run() {
  local label="$1"; shift
  local total=0 cnt=0 q t
  for q in "$@"; do
    for i in $(seq 1 $N); do
      t=$(curl -s -o /dev/null -w "%{time_total}" -X POST "$H/search" -d "$q")
      total=$(awk "BEGIN{print $total+$t}"); cnt=$((cnt+1))
    done
  done
  awk "BEGIN{printf \"  %-22s %4d reqs  média %7.2f ms/query\n\", \"$label\", $cnt, $total/$cnt*1000}"
}

run "GLOBAL (25 bases)" \
  '{"base":"*","query":"índice invertido posicional","k":5}' \
  '{"base":"*","query":"matched filter cosseno","k":5}' \
  '{"base":"*","query":"Frodo Bolseiro montanha","k":5}'

run "livros/sda (1 base)" \
  '{"collection":"livros","base":"sda","query":"Frodo Bolseiro montanha","k":5}' \
  '{"collection":"livros","base":"sda","query":"o anel de poder","k":5}'

run "ragnarock (22 bases)" \
  '{"collection":"ragnarock","base":"*","query":"índice invertido posicional","k":5}' \
  '{"collection":"ragnarock","base":"*","query":"histograma esparso embedding","k":5}'
