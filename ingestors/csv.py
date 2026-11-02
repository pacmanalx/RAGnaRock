#!/usr/bin/env python3
"""
ingestor: csv — normaliza um CSV/TSV recebido no stdin e devolve CSV limpo no stdout.

Pinagem (contrato do ingestor): stdin = bytes do arquivo · stdout = CSV normalizado ·
exit 0 = ok, !=0 + motivo no stderr = rejeitado. python3 RAW, só stdlib (csv, io).

Por que existe se CSV já é texto? Pra garantir que o `tabular_spec` do lado Rust reconheça a
tabela: detecta o delimitador (Sniffer), tira o BOM, e reemite com vírgula + aspas mínimas +
quebra LF. Assim a base cai determinística na Fase 4 (extração zero-LLM).
"""
import sys
import os
# o driver se chama csv.py → remove o próprio diretório do sys.path pra `import csv` achar a STDLIB,
# não a si mesmo (rodar `python3 ingestors/csv.py` põe ingestors/ no sys.path[0]). Ver README.
_here = os.path.dirname(os.path.abspath(__file__))
sys.path[:] = [p for p in sys.path if p and os.path.abspath(p) != _here]
import csv
import io


def main():
    if sys.stdin.isatty():
        sys.stderr.write("uso: python3 csv.py < arquivo.csv   (a fonte vem pelo stdin)\n")
        return 2
    dados = sys.stdin.buffer.read()
    if b"\x00" in dados:
        # texto não tem byte NUL: é binário ou UTF-16/32, que sairia como lixo com exit 0
        sys.stderr.write("csv: não é texto (byte NUL: arquivo binário ou UTF-16?)\n")
        return 1
    try:
        text = dados.decode("utf-8-sig")  # utf-8-sig tira o BOM
    except UnicodeDecodeError:
        # Excel em português salva em Windows-1252: em vez de engolir os acentos
        # com U+FFFD, decodifica direito e avisa qual codificação entrou
        try:
            text = dados.decode("cp1252")
        except UnicodeDecodeError:
            # 0x81, 0x8D, 0x8F, 0x90 e 0x9D não existem no Windows-1252: binário ou outra
            # codificação (Mac Roman, DOS 850...)
            sys.stderr.write("csv: codificação não reconhecida (nem UTF-8 nem Windows-1252)\n")
            return 1
        sys.stderr.write("csv: codificação Windows-1252 detectada (não é UTF-8)\n")
    if not text.strip():
        sys.stderr.write("csv: entrada vazia\n")
        return 1
    # detecta o dialeto (delimitador) numa amostra; cai pra vírgula (excel) se não decidir
    try:
        dialect = csv.Sniffer().sniff(text[:8192], delimiters=",;\t|")
    except csv.Error:
        dialect = csv.excel
    # newline="" como pede a doc do csv: aceita fim de linha só CR (Excel de Mac antigo)
    reader = csv.reader(io.StringIO(text, newline=""), dialect)
    out = io.StringIO()
    writer = csv.writer(out, delimiter=",", quoting=csv.QUOTE_MINIMAL, lineterminator="\n")
    n = 0
    try:
        for row in reader:
            writer.writerow(row)
            n += 1
    except csv.Error as e:
        # ex.: campo acima do limite, quase sempre aspas sem fechar engolindo o resto do arquivo
        sys.stderr.write(f"csv: CSV malformado (linha {reader.line_num}): {e}\n")
        return 1
    if n == 0:
        sys.stderr.write("csv: nenhuma linha\n")
        return 1
    sys.stdout.write(out.getvalue())
    sys.stderr.write(f"csv: {n} linha(s) normalizada(s)\n")
    return 0


if __name__ == "__main__":
    sys.exit(main())
