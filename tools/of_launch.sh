#!/usr/bin/env bash
# of_launch.sh — sobe os daemons do RAGnaRock detached, lendo seus configs locais.
#   ragd      — núcleo RAG (./ragnarock.cfg): portas API+ValHalla, dirs, auto-load.
#   nidhoggd  — camada de inteligência (./nidhogg.cfg), porta 11497. OPCIONAL: só sobe
#               se o binário existir. Nasce desligado; liga-se pelo ValHalla.
set -u
cd "$(dirname "$0")"

pkill -x ragd 2>/dev/null
sleep 1
setsid nohup ./ragd </dev/null >/tmp/ragd-all.log 2>&1 &
disown
sleep 2
echo "pid ragd: $(pgrep -x ragd || echo NENHUM)"

if [ -x ./nidhoggd ]; then
  pkill -x nidhoggd 2>/dev/null
  sleep 1
  setsid nohup ./nidhoggd </dev/null >/tmp/nidhoggd.log 2>&1 &
  disown
  sleep 1
  echo "pid nidhoggd: $(pgrep -x nidhoggd || echo NENHUM)"
else
  echo "nidhoggd: binário ausente (camada de inteligência não instalada — opcional)"
fi
