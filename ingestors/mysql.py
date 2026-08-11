#!/usr/bin/env python3
"""
ingestor: mysql — recebe uma RECEITA DE INGESTÃO no stdin, conecta num MySQL/MariaDB de
REDE, roda o SELECT e devolve o resultado como CSV no stdout.

Pinagem (contrato do ingestor): stdin = a receita (texto) · stdout = CSV (header + linhas) ·
exit 0 = ok, !=0 + motivo no stderr = rejeitado. python3 RAW, sem venv.
Dependência (não-stdlib): pymysql — no Debian/Ubuntu/Mint: `apt install python3-pymysql`.

A receita é EFÊMERA (montada por quem chama, nunca gravada no corpus — ver README):
diretivas em comentário `-- chave: valor` + o SQL. Banco é sempre REDE (host:porta),
nunca arquivo local — é o INVARIANTE do projeto.

    -- host: db-prod.interno:3306
    -- db:   vendas
    -- user: rag_reader
    -- pass: segredo
    SELECT id, cliente, valor, data FROM pedidos WHERE semana = 32;

`porta` na diretiva host é opcional (default 3306). Só a PRIMEIRA query é executada.
"""
import sys
import os
# uniforme com os demais drivers: remove o próprio diretório do sys.path (anti-sombreamento
# da stdlib — ver README).
_here = os.path.dirname(os.path.abspath(__file__))
sys.path[:] = [p for p in sys.path if p and os.path.abspath(p) != _here]
import csv
import io

RECIPE_KEYS = ("host", "db", "user", "pass")


def parse_recipe(text):
    """Separa as diretivas `-- chave: valor` do SQL. Devolve (dict, sql)."""
    directives = {}
    sql_lines = []
    for line in text.splitlines():
        stripped = line.strip()
        if stripped.startswith("--"):
            body = stripped[2:].strip()
            if ":" in body:
                k, v = body.split(":", 1)
                k = k.strip().lower()
                if k in RECIPE_KEYS:
                    directives[k] = v.strip()
                    continue
            continue   # comentário SQL comum — descarta
        sql_lines.append(line)
    return directives, "\n".join(sql_lines).strip()


def main():
    if sys.stdin.isatty():
        sys.stderr.write(
            "uso: python3 mysql.py < receita.mysql   (a receita vem pelo stdin)\n"
            "receita = diretivas `-- host|db|user|pass: valor` + o SELECT (ver cabeçalho)\n")
        return 2
    try:
        import pymysql
    except ImportError:
        sys.stderr.write("mysql: dependência ausente: pymysql (apt install python3-pymysql)\n")
        return 3
    text = sys.stdin.buffer.read().decode("utf-8-sig", errors="replace")
    if not text.strip():
        sys.stderr.write("mysql: entrada vazia\n")
        return 1
    directives, sql = parse_recipe(text)
    missing = [k for k in RECIPE_KEYS if k not in directives]
    if missing:
        sys.stderr.write(f"mysql: receita sem diretiva(s): {', '.join('-- ' + k for k in missing)}\n")
        return 1
    if not sql:
        sys.stderr.write("mysql: receita sem SQL\n")
        return 1
    host, _, port = directives["host"].partition(":")
    try:
        port = int(port) if port else 3306
    except ValueError:
        sys.stderr.write(f"mysql: porta inválida em host: {directives['host']!r}\n")
        return 1
    try:
        conn = pymysql.connect(
            host=host, port=port, database=directives["db"],
            user=directives["user"], password=directives["pass"],
            connect_timeout=15, read_timeout=120, charset="utf8mb4")
    except Exception as e:
        sys.stderr.write(f"mysql: falha ao conectar em {host}:{port}: {e}\n")
        return 1
    try:
        with conn.cursor() as cur:
            cur.execute(sql)
            out = io.StringIO()
            writer = csv.writer(out, delimiter=",", quoting=csv.QUOTE_MINIMAL, lineterminator="\n")
            writer.writerow([d[0] for d in cur.description])   # header = nomes das colunas
            n = 0
            for row in cur:
                writer.writerow(["" if v is None else v for v in row])
                n += 1
    except Exception as e:
        sys.stderr.write(f"mysql: erro na query: {e}\n")
        return 1
    finally:
        conn.close()
    sys.stdout.write(out.getvalue())
    sys.stderr.write(f"mysql: {n} linha(s) de {host}:{port}/{directives['db']}\n")
    return 0


if __name__ == "__main__":
    sys.exit(main())
