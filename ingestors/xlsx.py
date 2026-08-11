#!/usr/bin/env python3
"""
ingestor: xlsx — extrai a(s) planilha(s) de um .xlsx recebido no stdin e devolve CSV no stdout.

Pinagem (contrato do ingestor): stdin = bytes do arquivo · stdout = CSV · exit 0 = ok,
!=0 + motivo no stderr = rejeitado. python3 RAW, sem venv.
Dependência (não-stdlib): openpyxl — no Debian/Ubuntu/Mint: `apt install python3-openpyxl`.

Saída: cada aba NÃO-vazia vira um bloco CSV; abas múltiplas são separadas por linha em
branco (aba única = CSV puro, que casa no `tabular_spec` → Fase 4 zero-LLM). Células
vazias viram string vazia; fórmulas saem pelo VALOR calculado (data_only).
"""
import sys
import os
# uniforme com os demais drivers: remove o próprio diretório do sys.path (anti-sombreamento
# da stdlib — ver README; aqui csv.py do lado é quem sombrearia o `import csv`).
_here = os.path.dirname(os.path.abspath(__file__))
sys.path[:] = [p for p in sys.path if p and os.path.abspath(p) != _here]
import csv
import io


def cell_str(v):
    if v is None:
        return ""
    return str(v)


def main():
    if sys.stdin.isatty():
        sys.stderr.write("uso: python3 xlsx.py < arquivo.xlsx   (a fonte vem pelo stdin)\n")
        return 2
    try:
        import openpyxl
    except ImportError:
        sys.stderr.write("xlsx: dependência ausente: openpyxl (apt install python3-openpyxl)\n")
        return 3
    data = sys.stdin.buffer.read()
    if not data:
        sys.stderr.write("xlsx: entrada vazia\n")
        return 1
    try:
        wb = openpyxl.load_workbook(io.BytesIO(data), read_only=True, data_only=True)
    except Exception as e:
        sys.stderr.write(f"xlsx: arquivo inválido: {e}\n")
        return 1
    out = io.StringIO()
    writer = csv.writer(out, delimiter=",", quoting=csv.QUOTE_MINIMAL, lineterminator="\n")
    sheets_out = 0
    total_rows = 0
    for ws in wb.worksheets:
        rows = [[cell_str(c) for c in row] for row in ws.iter_rows(values_only=True)]
        rows = [r for r in rows if any(s.strip() for s in r)]
        if not rows:
            continue
        if sheets_out > 0:
            out.write("\n")   # separador entre abas (aba única = CSV puro)
        for r in rows:
            writer.writerow(r)
        sheets_out += 1
        total_rows += len(rows)
        sys.stderr.write(f"xlsx: aba '{ws.title}': {len(rows)} linha(s)\n")
    wb.close()
    if sheets_out == 0:
        sys.stderr.write("xlsx: nenhuma aba com conteúdo\n")
        return 1
    sys.stdout.write(out.getvalue())
    sys.stderr.write(f"xlsx: {sheets_out} aba(s), {total_rows} linha(s) no total\n")
    return 0


if __name__ == "__main__":
    sys.exit(main())
