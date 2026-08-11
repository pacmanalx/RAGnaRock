#!/usr/bin/env python3
"""
ingestor: postgres — recebe uma RECEITA DE INGESTÃO no stdin, conecta num PostgreSQL de
REDE, roda o SELECT e devolve o resultado como CSV no stdout.

Pinagem (contrato do ingestor): stdin = a receita (texto) · stdout = CSV (header + linhas) ·
exit 0 = ok, !=0 + motivo no stderr = rejeitado. python3 RAW, sem venv.
Dependência (não-stdlib): psycopg2 — no Debian/Ubuntu/Mint: `apt install python3-psycopg2`.

Irmão do driver mysql: MESMO contrato de receita efêmera (montada por quem chama, nunca
gravada no corpus — ver README). Diretivas em comentário `-- chave: valor` + o SQL. Banco
é sempre REDE (host:porta), nunca arquivo local — o INVARIANTE do projeto.

    -- host: db-prod.interno:5432
    -- db:   vendas
    -- user: rag_reader
    -- pass: segredo
    SELECT id, cliente, valor, data FROM pedidos WHERE semana = 32;

`porta` na diretiva host é opcional (default 5432). Só a PRIMEIRA query é executada.
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
            "uso: python3 postgres.py < receita.postgres   (a receita vem pelo stdin)\n"
            "receita = diretivas `-- host|db|user|pass: valor` + o SELECT (ver cabeçalho)\n")
        return 2
    try:
        import psycopg2
    except ImportError:
        sys.stderr.write("postgres: dependência ausente: psycopg2 (apt install python3-psycopg2)\n")
        return 3
    text = sys.stdin.buffer.read().decode("utf-8-sig", errors="replace")
    if not text.strip():
        sys.stderr.write("postgres: entrada vazia\n")
        return 1
    directives, sql = parse_recipe(text)
    missing = [k for k in RECIPE_KEYS if k not in directives]
    if missing:
        sys.stderr.write(f"postgres: receita sem diretiva(s): {', '.join('-- ' + k for k in missing)}\n")
        return 1
    if not sql:
        sys.stderr.write("postgres: receita sem SQL\n")
        return 1
    host, _, port = directives["host"].partition(":")
    try:
        port = int(port) if port else 5432
    except ValueError:
        sys.stderr.write(f"postgres: porta inválida em host: {directives['host']!r}\n")
        return 1
    try:
        conn = psycopg2.connect(
            host=host, port=port, dbname=directives["db"],
            user=directives["user"], password=directives["pass"],
            connect_timeout=15)
    except Exception as e:
        sys.stderr.write(f"postgres: falha ao conectar em {host}:{port}: {e}\n")
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
        sys.stderr.write(f"postgres: erro na query: {e}\n")
        return 1
    finally:
        conn.close()
    sys.stdout.write(out.getvalue())
    sys.stderr.write(f"postgres: {n} linha(s) de {host}:{port}/{directives['db']}\n")
    return 0


if __name__ == "__main__":
    sys.exit(main())
