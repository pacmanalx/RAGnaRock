#!/usr/bin/env python3
"""
ingestor: pptx — extrai o TEXTO de um .pptx recebido no stdin e devolve texto puro no stdout.

Pinagem (contrato do ingestor): stdin = bytes do arquivo · stdout = texto extraído ·
exit 0 = ok, !=0 + motivo no stderr = rejeitado. python3 RAW, sem venv.
Dependência: NENHUMA fora da stdlib. Um .pptx é um ZIP de XML, e `zipfile` + `xml.etree`
dão conta — diferente do docx.py (python-docx) e do pdf.py (pypdf), aqui não há apt install.
Só .pptx (OOXML); o .ppt binário antigo não é suportado por esta engine.

Saída: texto de cada slide, slides separados por linha em branco — o mesmo formato do pdf.py,
onde o slide faz o papel da página. Dentro do slide, um parágrafo por linha; tabelas viram
linhas com células separadas por TAB, como no docx.py.

As NOTAS DO APRESENTADOR entram junto, ao final do slide a que pertencem. Num deck o slide
carrega o título e a nota carrega o argumento — descartá-la jogaria fora justamente a parte
narrativa. O stderr informa quantos slides tinham nota, para quem precise saber a proporção.

ORDEM: vem do `sldIdLst` de `ppt/presentation.xml` resolvido pelos rels — a ordem real da
apresentação, que não é a ordem dos nomes de arquivo: um deck reordenado mantém `slide7.xml`
na quarta posição, e ordenar por nome contaria a história na sequência errada.
"""
import sys
import os
# uniforme com os demais drivers: remove o próprio diretório do sys.path (anti-sombreamento
# da stdlib — ver README).
_here = os.path.dirname(os.path.abspath(__file__))
sys.path[:] = [p for p in sys.path if p and os.path.abspath(p) != _here]
import io
import re
import zipfile
import xml.etree.ElementTree as ET

A = "{http://schemas.openxmlformats.org/drawingml/2006/main}"
P = "{http://schemas.openxmlformats.org/presentationml/2006/main}"
R = "{http://schemas.openxmlformats.org/officeDocument/2006/relationships}"
RELS = "{http://schemas.openxmlformats.org/package/2006/relationships}"


def _para_text(p):
    """Um `a:p` vira UMA linha: todos os `a:t` dele, na ordem, colados.

    O texto de um parágrafo vem picado em vários `a:t` porque cada trecho com formatação
    própria (uma palavra em negrito no meio da frase) abre um run novo. Juntar sem separador
    é o certo: o corte é de estilo, não de palavra.
    """
    return "".join(t.text or "" for t in p.iter(A + "t")).strip()


def _table(tbl, out):
    """`a:tbl` → uma linha por `a:tr`, células separadas por TAB (mesma escolha do docx.py)."""
    n = 0
    for tr in tbl.iter(A + "tr"):
        cells = []
        for tc in tr.iter(A + "tc"):
            txt = " ".join(x for x in (_para_text(p) for p in tc.iter(A + "p")) if x)
            cells.append(txt.strip())
        if any(cells):
            out.append("\t".join(cells))
            n += 1
    return n


def _walk(el, out, stats):
    """Percorre a árvore na ordem do documento, colhendo parágrafos e tabelas.

    Ao encontrar um `a:tbl` trata a tabela inteira e NÃO desce nela de novo — senão cada
    célula viraria uma linha solta e a estrutura da tabela se perderia. Ao encontrar um
    `a:p` também não desce: `_para_text` já leva o parágrafo inteiro.
    """
    for child in el:
        if child.tag == A + "tbl":
            stats["tabelas"] += 1
            stats["linhas_tab"] += _table(child, out)
        elif child.tag == A + "p":
            t = _para_text(child)
            if t:
                out.append(t)
        else:
            _walk(child, out, stats)


def _texto_da_parte(z, nome, stats):
    """Texto de uma parte XML do pacote (um slide ou um notesSlide)."""
    try:
        raiz = ET.fromstring(z.read(nome))
    except (KeyError, ET.ParseError) as e:
        sys.stderr.write(f"pptx: {nome} ilegível ({e}); pulando\n")
        return []
    linhas = []
    _walk(raiz, linhas, stats)
    return linhas


def _rels_de(z, parte):
    """Mapa rId → alvo ABSOLUTO no pacote, lendo o `_rels` da parte.

    O Target do rels é relativo ao diretório da parte (`../notesSlides/notesSlide1.xml` visto
    de `ppt/slides/`), então normalizamos contra ele — é o que faz a resolução funcionar tanto
    para `ppt/presentation.xml` quanto para um slide lá dentro.
    """
    d = os.path.dirname(parte)
    rels = f"{d}/_rels/{os.path.basename(parte)}.rels"
    try:
        raiz = ET.fromstring(z.read(rels))
    except (KeyError, ET.ParseError):
        return {}
    fora = {}
    for rel in raiz.iter(RELS + "Relationship"):
        alvo = rel.get("Target") or ""
        if not alvo or "://" in alvo:      # link externo não é parte do pacote
            continue
        fora[rel.get("Id")] = os.path.normpath(os.path.join(d, alvo)).replace(os.sep, "/")
    return fora


def _ordem_dos_slides(z):
    """A ordem REAL da apresentação, pelo `sldIdLst`. Cai na ordem numérica dos nomes se
    o `presentation.xml` faltar ou vier sem lista — deck estranho continua ingerindo."""
    try:
        pres = ET.fromstring(z.read("ppt/presentation.xml"))
        rels = _rels_de(z, "ppt/presentation.xml")
        ordem = []
        for sld in pres.iter(P + "sldId"):
            alvo = rels.get(sld.get(R + "id"))
            if alvo and alvo in z.namelist():
                ordem.append(alvo)
        if ordem:
            return ordem
    except (KeyError, ET.ParseError) as e:
        sys.stderr.write(f"pptx: sem ordem declarada ({e}); usando a ordem dos nomes\n")
    nomes = [n for n in z.namelist() if re.fullmatch(r"ppt/slides/slide\d+\.xml", n)]
    return sorted(nomes, key=lambda n: int(re.search(r"(\d+)", os.path.basename(n)).group(1)))


def _nota_do_slide(z, slide, stats):
    """Notas do apresentador daquele slide, resolvidas pelo rels dele."""
    for alvo in _rels_de(z, slide).values():
        if alvo.startswith("ppt/notesSlides/"):
            return _texto_da_parte(z, alvo, stats)
    return []


def main():
    if sys.stdin.isatty():
        sys.stderr.write("uso: python3 pptx.py < arquivo.pptx   (a fonte vem pelo stdin)\n")
        return 2
    data = sys.stdin.buffer.read()
    if not data:
        sys.stderr.write("pptx: entrada vazia\n")
        return 1
    try:
        z = zipfile.ZipFile(io.BytesIO(data))
    except zipfile.BadZipFile as e:
        sys.stderr.write(f"pptx: arquivo inválido (.ppt antigo não é suportado; use .pptx): {e}\n")
        return 1
    with z:
        slides = _ordem_dos_slides(z)
        if not slides:
            sys.stderr.write("pptx: nenhum slide no pacote\n")
            return 1
        stats = {"tabelas": 0, "linhas_tab": 0}
        com_nota = 0
        blocos = []
        for slide in slides:
            linhas = _texto_da_parte(z, slide, stats)
            nota = _nota_do_slide(z, slide, stats)
            if nota:
                com_nota += 1
                linhas += nota
            if linhas:
                blocos.append("\n".join(linhas))
    if not blocos:
        sys.stderr.write(
            f"pptx: nenhum texto em {len(slides)} slide(s) (deck só de imagens?)\n")
        return 1
    sys.stdout.write("\n\n".join(blocos))
    sys.stdout.write("\n")
    sys.stderr.write(
        f"pptx: {len(blocos)}/{len(slides)} slide(s) com texto, "
        f"{com_nota} com nota, {stats['tabelas']} tabela(s)/{stats['linhas_tab']} linha(s)\n")
    return 0


if __name__ == "__main__":
    sys.exit(main())
