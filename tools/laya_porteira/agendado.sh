#!/usr/bin/env bash
# Porteira Laya (#53) — execução AGENDADA na Aron (cron do usuário, sem root).
# Tudo local e offline: Laya base e pacotes em /dados/modelos/laya (cópias próprias, com
# SHA256SUMS), rótulos do ClickHouse local, texto do ragd local. Sem rótulos novos suficientes,
# a rotina só registra no histórico e sai.
#
#   crontab -e  →  0 3 * * 0  /dados/dev/ragnarock/tools/laya_porteira/agendado.sh
#
# Recriar o ambiente sem internet:
#   python3 -m venv /dados/modelos/laya/venv
#   /dados/modelos/laya/venv/bin/pip install --no-index --find-links /dados/modelos/laya/wheels \
#       -r /dados/modelos/laya/requirements-lock.txt
set -euo pipefail
M=${LAYA_MODELOS:-/dados/modelos/laya}
DIR=${LAYA_DIR:-/dados/ragnarock/laya}
AQUI="$(cd "$(dirname "$0")" && pwd)"
mkdir -p "$DIR/logs"
LOG="$DIR/logs/$(date +%Y%m%d-%H%M%S).log"
# trava: nunca duas execuções ao mesmo tempo (o treino leva minutos a horas na CPU)
exec 9>"$DIR/.trava"
flock -n 9 || { echo "$(date '+%F %T') já há uma execução em andamento" >> "$DIR/logs/trava.log"; exit 0; }
export HF_HUB_OFFLINE=1 TRANSFORMERS_OFFLINE=1 USE_TF=0 PYTHONUNBUFFERED=1
nice -n 19 "$M/venv/bin/python" "$AQUI/porteira.py" --dir "$DIR" --base "$M/base/laya-multilingual" "$@" > "$LOG" 2>&1
# logs: mantém os 30 mais recentes
ls -1t "$DIR"/logs/2*.log 2>/dev/null | tail -n +31 | xargs -r rm -f
