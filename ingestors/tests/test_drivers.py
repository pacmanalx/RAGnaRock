#!/usr/bin/env python3
"""
Testes dos drivers de ingestão (#50): de ARQUIVO (csv, xlsx, docx, pptx, pdf, audio) e de BANCO
(mysql, postgres — receita, recusas e sigilo da senha; a conexão viva fica no
tools/e2e_ingest.sh --bancos, com containers descartáveis).

Cada driver roda exatamente como o ragd o chama (`run_ingestor`): `python3 <driver>`, payload no
stdin, `PYTHONSAFEPATH=1`, saída no stdout, motivo da recusa na última linha do stderr.

    python3 -m unittest discover -s ingestors/tests -v

Sem openpyxl / python-docx / pypdf / ffmpeg / whisper, os testes que dependem deles são PULADOS
(com o motivo) — no servidor de produção todos devem rodar.
"""
import os
import sys

AQUI = os.path.dirname(os.path.abspath(__file__))
# anti-sombreamento: rodando de dentro de ingestors/, csv.py e docx.py tomariam o lugar das
# bibliotecas (ver fixtures.py)
sys.path[:] = [p for p in sys.path if os.path.abspath(p or os.getcwd()) != os.path.dirname(AQUI)]
import importlib.util  # noqa: E402
import shutil  # noqa: E402
import subprocess  # noqa: E402
import tempfile  # noqa: E402
import unittest  # noqa: E402

INGESTORS = os.environ.get("RAG_INGESTORS_DIR") or os.path.dirname(AQUI)
sys.path.insert(0, AQUI)
import fixtures as F  # noqa: E402


def tem(modulo):
    return importlib.util.find_spec(modulo) is not None


def roda(driver, dados, env_extra=None, timeout=120):
    """(exit, stdout, última linha do stderr) — o mesmo contrato que o ragd lê."""
    env = dict(os.environ, PYTHONSAFEPATH="1")
    env.update(env_extra or {})
    p = subprocess.run([sys.executable, os.path.join(INGESTORS, f"{driver}.py")],
                       input=dados, capture_output=True, timeout=timeout, env=env)
    linhas = [l for l in p.stderr.decode("utf-8", "replace").splitlines() if l.strip()]
    return p.returncode, p.stdout.decode("utf-8", "replace"), (linhas[-1] if linhas else "")


class Comum:
    """O que todo driver de arquivo garante: entrada vazia e lixo são RECUSADOS com motivo."""
    driver = None
    lixo = b"isto nao e um arquivo valido \x00\x01\x02"

    def test_entrada_vazia_recusada(self):
        code, out, err = roda(self.driver, b"")
        self.assertNotEqual(code, 0)
        self.assertEqual(out, "")
        self.assertIn("vazia", err)

    def test_lixo_recusado_com_motivo(self):
        code, out, err = roda(self.driver, self.lixo)
        self.assertNotEqual(code, 0, out)
        self.assertTrue(err.startswith(f"{self.driver}:"), err)


class TestCsv(Comum, unittest.TestCase):
    driver = "csv"
    lixo = b"   \n  \n"          # csv aceita qualquer texto; só branco é recusado

    def test_entrada_vazia_recusada(self):
        code, _, err = roda("csv", b"")
        self.assertEqual(code, 1)
        self.assertIn("vazia", err)

    def test_lixo_recusado_com_motivo(self):
        code, _, err = roda("csv", self.lixo)
        self.assertEqual(code, 1)
        self.assertIn("vazia", err)

    def test_ponto_e_virgula_com_bom_vira_virgula(self):
        code, out, err = roda("csv", F.csv_bytes())
        self.assertEqual(code, 0, err)
        self.assertEqual(out.splitlines(),
                         ["cidade,estado,habitantes", f"{F.ANCORAS['csv']},CE,80000", "Crateús,CE,75000"])
        self.assertIn("3 linha(s)", err)

    def test_campo_com_virgula_sai_entre_aspas(self):
        code, out, _ = roda("csv", "nome;obs\nAna;chegou, enfim\n".encode())
        self.assertEqual(code, 0)
        self.assertEqual(out.splitlines()[1], 'Ana,"chegou, enfim"')

    def test_nao_e_sombreado_pelo_proprio_nome(self):
        # csv.py importa o módulo csv da stdlib: sem a proteção do sys.path importaria a si mesmo
        code, _, err = roda("csv", b"a,b\n1,2\n", env_extra={"PYTHONSAFEPATH": ""})
        self.assertEqual(code, 0, err)


@unittest.skipUnless(tem("openpyxl"), "openpyxl ausente")
class TestXlsx(Comum, unittest.TestCase):
    driver = "xlsx"

    def test_abas_viram_csv_e_vazias_somem(self):
        code, out, err = roda("xlsx", F.xlsx_bytes())
        self.assertEqual(code, 0, err)
        blocos = out.split("\n\n")
        self.assertEqual(len(blocos), 2, out)                  # "Vazia" pulada
        self.assertEqual(blocos[0].splitlines(), ["cidade,estado,habitantes", f"{F.ANCORAS['xlsx']},SP,50000"])
        self.assertEqual(blocos[1].splitlines(), ["observação", "segunda aba com conteúdo"])
        self.assertIn("2 aba(s), 4 linha(s)", err)


@unittest.skipUnless(tem("docx"), "python-docx ausente")
class TestDocx(Comum, unittest.TestCase):
    driver = "docx"

    def test_paragrafos_e_tabela(self):
        code, out, err = roda("docx", F.docx_bytes())
        self.assertEqual(code, 0, err)
        self.assertEqual(out.splitlines(), [
            "Relatório de viagem",
            f"A comitiva passou por {F.ANCORAS['docx']} na terça-feira.",
            "item\tvalor",
            "diária\t350",
        ])

    def test_doc_antigo_explica_o_motivo(self):
        code, _, err = roda("docx", b"\xd0\xcf\x11\xe0\xa1\xb1\x1a\xe1" + b"\x00" * 512)   # cabeçalho OLE (.doc)
        self.assertEqual(code, 1)
        self.assertIn(".docx", err)


class TestPptx(Comum, unittest.TestCase):
    driver = "pptx"

    def test_ordem_runs_tabela_e_notas(self):
        code, out, err = roda("pptx", F.pptx_bytes())
        self.assertEqual(code, 0, err)
        slides = out.strip().split("\n\n")
        self.assertEqual(slides[0].splitlines(), [f"Visita a {F.ANCORAS['pptx']}", "Roteiro da serra"])
        self.assertEqual(slides[1].splitlines(), ["Custos", "item\tvalor", "trem\t42", "Lembrar do horário"])
        self.assertIn("2/2 slide(s) com texto, 1 com nota, 1 tabela(s)", err)

    def test_sem_presentation_usa_ordem_dos_nomes(self):
        code, out, err = roda("pptx", F.pptx_bytes(com_ordem=False))
        self.assertEqual(code, 0, err)
        self.assertTrue(out.startswith("Custos"), out)          # slide1.xml antes de slide2.xml

    def test_ppt_antigo_explica_o_motivo(self):
        code, _, err = roda("pptx", b"\xd0\xcf\x11\xe0\xa1\xb1\x1a\xe1" + b"\x00" * 512)
        self.assertEqual(code, 1)
        self.assertIn(".pptx", err)


@unittest.skipUnless(tem("pypdf"), "pypdf ausente")
class TestPdf(Comum, unittest.TestCase):
    driver = "pdf"

    def test_texto_de_cada_pagina_com_acentos(self):
        code, out, err = roda("pdf", F.pdf_bytes())
        self.assertEqual(code, 0, err)
        self.assertIn(f"Relatório da unidade de {F.ANCORAS['pdf']}", out)
        self.assertIn("Produção de café em alta", out)
        self.assertIn("Segunda página", out)
        self.assertLess(out.index(F.ANCORAS["pdf"]), out.index("Segunda página"))
        self.assertIn("2/2 página(s)", err)

    def test_pagina_sem_texto_e_pulada(self):
        code, out, err = roda("pdf", F.pdf_bytes([[], ["só a segunda tem texto"]]))
        self.assertEqual(code, 0, err)
        self.assertEqual(out.strip(), "só a segunda tem texto")
        self.assertIn("1/2 página(s)", err)

    def test_pdf_so_de_imagem_e_recusado(self):
        code, _, err = roda("pdf", F.pdf_bytes([[], []]))
        self.assertEqual(code, 1)
        self.assertIn("nenhum texto", err)


class TestAudio(unittest.TestCase):
    """Os caminhos de recusa rodam em qualquer máquina; a transcrição só onde há ffmpeg + whisper
    e um áudio de fala em RAG_TEST_AUDIO (o tools/e2e_ingest.sh cobre isso no servidor)."""

    def test_sem_whisper_recusa_com_motivo(self):
        code, out, err = roda("audio", b"x", env_extra={"RAG_WHISPER_BIN": "/nao/existe/whisper-cli"})
        self.assertEqual(code, 3)
        self.assertEqual(out, "")
        self.assertIn("não encontrado", err)

    @unittest.skipUnless(shutil.which("ffmpeg"), "ffmpeg ausente")
    def test_formato_ilegivel_recusado(self):
        with tempfile.NamedTemporaryFile() as falso:      # whisper "existe" — o ffmpeg recusa antes
            env = {"RAG_WHISPER_BIN": falso.name, "RAG_WHISPER_MODEL": falso.name}
            code, _, err = roda("audio", b"isto nao e audio", env_extra=env)
        self.assertEqual(code, 1)
        self.assertIn("ffmpeg não decodificou", err)

    @unittest.skipUnless(os.environ.get("RAG_TEST_AUDIO"), "RAG_TEST_AUDIO não definido")
    def test_transcreve_fala(self):
        caminho = os.environ["RAG_TEST_AUDIO"]
        esperado = os.environ.get("RAG_TEST_AUDIO_PALAVRA", "")
        with open(caminho, "rb") as f:
            code, out, err = roda("audio", f.read(), timeout=600)
        self.assertEqual(code, 0, err)
        self.assertTrue(out.strip(), err)
        if esperado:
            self.assertIn(esperado.lower(), out.lower())


def carrega_driver(nome):
    """Importa o driver como módulo (para testar funções puras como parse_recipe). O import das
    dependências pesadas fica dentro do main(), então isto não exige pymysql/psycopg2."""
    spec = importlib.util.spec_from_file_location(f"driver_{nome}", os.path.join(INGESTORS, f"{nome}.py"))
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


SENHA_TESTE = "s3nh4-que-nao-pode-vazar"


def receita(host="127.0.0.1:1", db="vendas", user="leitor", senha=SENHA_TESTE, sql="SELECT 1;", sem=()):
    d = {"host": host, "db": db, "user": user, "pass": senha}
    linhas = [f"-- {k}: {v}" for k, v in d.items() if k not in sem]
    return ("\n".join(linhas) + "\n" + sql + "\n").encode()


class ReceitaDeBanco:
    """mysql e postgres: mesma receita (diretivas `-- chave: valor` + SQL), mesmas recusas.
    Nenhum destes testes precisa de banco: a conexão viva é do tools/e2e_ingest.sh --bancos."""
    driver = None
    modulo_dep = None
    porta_padrao = None

    def setUp(self):
        self.mod = carrega_driver(self.driver)

    def test_parse_recipe_separa_diretivas_do_sql(self):
        d, sql = self.mod.parse_recipe(
            "-- host: db.interno:3307\n-- DB: vendas\n--user:leitor\n-- pass: a:b:c\n"
            "-- comentário comum do SQL some\nSELECT id,\n  nome FROM t;\n")
        self.assertEqual(d, {"host": "db.interno:3307", "db": "vendas", "user": "leitor", "pass": "a:b:c"})
        self.assertEqual(sql, "SELECT id,\n  nome FROM t;")

    def test_parse_recipe_ignora_diretiva_desconhecida(self):
        d, sql = self.mod.parse_recipe("-- host: h\n-- path: /etc/passwd\nSELECT 1")
        self.assertEqual(d, {"host": "h"})
        self.assertEqual(sql, "SELECT 1")

    def test_entrada_vazia_recusada(self):
        code, out, err = roda(self.driver, b"   \n")
        self.assertEqual(code, 1 if tem(self.modulo_dep) else 3)
        self.assertEqual(out, "")

    def test_lixo_recusado_com_motivo(self):
        if not tem(self.modulo_dep):
            self.skipTest(f"{self.modulo_dep} ausente")
        code, _, err = roda(self.driver, b"isto nao e uma receita")
        self.assertEqual(code, 1)
        self.assertIn("sem diretiva", err)

    def test_recusas_da_receita(self):
        if not tem(self.modulo_dep):
            self.skipTest(f"{self.modulo_dep} ausente")
        casos = [
            (receita(sem=("pass",)), "-- pass"),
            (receita(sem=("host", "db")), "-- host, -- db"),
            (receita(sql=""), "sem SQL"),
            (receita(host="db:porta"), "porta inválida"),
        ]
        for dados, motivo in casos:
            with self.subTest(motivo=motivo):
                code, out, err = roda(self.driver, dados)
                self.assertEqual(code, 1)
                self.assertEqual(out, "")
                self.assertIn(motivo, err)

    def test_falha_de_conexao_nao_vaza_a_senha(self):
        if not tem(self.modulo_dep):
            self.skipTest(f"{self.modulo_dep} ausente")
        code, out, err = roda(self.driver, receita(host="127.0.0.1:1"), timeout=60)
        self.assertEqual(code, 1)
        self.assertIn("falha ao conectar em 127.0.0.1:1", err)
        p = subprocess.run([sys.executable, os.path.join(INGESTORS, f"{self.driver}.py")],
                           input=receita(host="127.0.0.1:1"), capture_output=True, timeout=60,
                           env=dict(os.environ, PYTHONSAFEPATH="1"))
        self.assertNotIn(SENHA_TESTE, (p.stdout + p.stderr).decode("utf-8", "replace"))

    def test_porta_padrao(self):
        # sem `:porta` no host, vale a porta padrão do banco (aparece na mensagem de falha)
        if not tem(self.modulo_dep):
            self.skipTest(f"{self.modulo_dep} ausente")
        code, _, err = roda(self.driver, receita(host="127.0.0.254"), timeout=60)
        self.assertEqual(code, 1)
        self.assertIn(f"127.0.0.254:{self.porta_padrao}", err)


class TestMysql(ReceitaDeBanco, unittest.TestCase):
    driver, modulo_dep, porta_padrao = "mysql", "pymysql", 3306


class TestPostgres(ReceitaDeBanco, unittest.TestCase):
    driver, modulo_dep, porta_padrao = "postgres", "psycopg2", 5432


if __name__ == "__main__":
    unittest.main(verbosity=2)
