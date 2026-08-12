#!/usr/bin/env python3
"""
ingestor: pdf — extrai o TEXTO de um .pdf recebido no stdin e devolve texto puro no stdout.

Pinagem (contrato do ingestor): stdin = bytes do arquivo · stdout = texto extraído ·
exit 0 = ok, !=0 + motivo no stderr = rejeitado. python3 RAW, sem venv.
Dependência (não-stdlib): pypdf — no Debian/Ubuntu/Mint: `apt install python3-pypdf`.
(pdfplumber extrai layout/tabelas melhor, mas só existe via pip; pypdf vem do apt e
resolve o caso narrativo. Se um dia precisar de tabela de PDF, troca-se a engine AQUI,
sem mexer no daemon — é a graça da pinagem.)

Saída: texto de cada página, páginas separadas por linha em branco. PDF cifrado tenta
senha vazia; PDF de imagem (scan sem OCR) é rejeitado por "nenhum texto extraível".
"""
import sys
import os
# uniforme com os demais drivers: remove o próprio diretório do sys.path (anti-sombreamento
# da stdlib — ver README).
_here = os.path.dirname(os.path.abspath(__file__))
sys.path[:] = [p for p in sys.path if p and os.path.abspath(p) != _here]
import io
import re


def _avg_words_per_line(txt):
    nb = [l for l in txt.split("\n") if l.strip()]
    return sum(len(l.split()) for l in nb) / len(nb) if nb else 0.0


def _clean_layout(txt):
    # o modo layout indenta com espaços (posição visual) — tira a indentação, colapsa
    # espaços internos e quebras triplas, devolvendo linhas legíveis.
    lines = [re.sub(r"[ \t]{2,}", " ", l).strip() for l in txt.split("\n")]
    return re.sub(r"\n{3,}", "\n\n", "\n".join(lines)).strip()


def _extract_page(page):
    # PDFs com posicionamento por token saem "picados" (~1 palavra/linha) no modo default.
    # Nesses, o modo layout do pypdf reagrupa as linhas visuais — mas ele volta VAZIO em
    # outros PDFs, então só o adotamos quando o default está picado E o layout melhora.
    default = (page.extract_text() or "").strip()
    if _avg_words_per_line(default) < 2.5:
        try:
            lay = _clean_layout(page.extract_text(extraction_mode="layout"))
            if _avg_words_per_line(lay) > _avg_words_per_line(default):
                return lay
        except Exception:
            pass
    return default


def main():
    if sys.stdin.isatty():
        sys.stderr.write("uso: python3 pdf.py < arquivo.pdf   (a fonte vem pelo stdin)\n")
        return 2
    try:
        from pypdf import PdfReader
    except ImportError:
        sys.stderr.write("pdf: dependência ausente: pypdf (apt install python3-pypdf)\n")
        return 3
    data = sys.stdin.buffer.read()
    if not data:
        sys.stderr.write("pdf: entrada vazia\n")
        return 1
    try:
        reader = PdfReader(io.BytesIO(data))
        if reader.is_encrypted:
            reader.decrypt("")   # tenta senha vazia; falhou -> exceção -> rejeitado
    except Exception as e:
        sys.stderr.write(f"pdf: arquivo inválido/cifrado: {e}\n")
        return 1
    pages = []
    for i, page in enumerate(reader.pages):
        try:
            t = _extract_page(page)
        except Exception as e:
            sys.stderr.write(f"pdf: página {i + 1} ilegível ({e}); pulando\n")
            continue
        if t:
            pages.append(t)
    if not pages:
        sys.stderr.write(f"pdf: nenhum texto extraível em {len(reader.pages)} página(s) (scan sem OCR?)\n")
        return 1
    sys.stdout.write("\n\n".join(pages))
    sys.stdout.write("\n")
    sys.stderr.write(f"pdf: {len(pages)}/{len(reader.pages)} página(s) com texto\n")
    return 0


if __name__ == "__main__":
    sys.exit(main())
