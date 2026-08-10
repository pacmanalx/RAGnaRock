# ingestors/ — drivers de ingestão (pinagem via shell)

Um **ingestor** é uma caixa-preta com **pinagem fixa**: o daemon chama o script via shell, e o
núcleo não quer saber o que tem dentro (Python, bash, um binário). É o mesmo caminho para
**arquivos** (pdf/docx/xlsx) e para **bancos vivos** (mysql/postgres/sqlserver/oracle/mongo/sqlite,
via *receitas de ingestão*). Sidecar **opcional**: o binário de ~2 MB sozinho continua fazendo RAG
de texto sem nenhum destes scripts.

## A pinagem (contrato)

```
  stdin   →  os bytes crus do POST (um arquivo, ou uma receita interpretativa)
  stdout  →  o conteúdo extraído: TEXTO (narrativo/docs) ou CSV (tabular)
  exit 0  →  ok        ·   exit ≠0 + motivo no stderr  →  rejeitado
```

- Diagnóstico do driver (quantas linhas, engine usada) vai pro **stderr** — nunca polui o stdout.
- O **timeout** é do chamador (Rust), não do driver — um driver lento/pendurado jamais trava o daemon.
- `argv[1]` opcional = o nome de arquivo original (dica de contexto; a maioria dos drivers ignora).

## RAW python3, sem venv

Chamamos `python3` **cru** (todo Linux moderno tem no PATH) — nada de venv. **stdlib primeiro**:
os drivers de partida usam só a biblioteca padrão (`csv`, `sqlite3`, `json`) → **zero `pip install`**.
Formatos pesados (`openpyxl` p/ xlsx, `pdfplumber` p/ pdf, `python-docx`) entram depois, ainda sem
venv, com a dependência documentada no cabeçalho de cada driver.

## Roteamento: MIME primeiro, extensão para a família texto

O binário chega por **HTTP POST**. O driver é resolvido pelo **MIME** — mas MIME é cego para a
família texto: pro wire, `.txt`/`.csv`/`.c`/`.mysql`/`.sqlite` são todos `text/plain`. Então:
**MIME resolve o binário** (application/pdf, os MIMEs de xlsx/docx); **quando cai em `text/plain`,
a extensão desempata**. Registro = `(mime | ext) → driver`.

## Determinismo pela origem

O formato é um prior forte da `natureza`: um driver que cospe **CSV** (xlsx, um banco de rede) casa
no `tabular_spec` do lado Rust → **Fase 4 extrai zero-LLM**. O driver é o *primeiro classificador,
pela origem*; o LLM (Fase 1) só entra quando o formato não decide.

## Convenções

- Um arquivo por driver: `csv.py`, `sqlite.py`, `xlsx.py`, … O nome do driver = o nome do arquivo.
- **Cuidado com o sombreamento da stdlib:** um driver chamado `csv.py`/`sqlite.py`/`json.py`
  tem o mesmo nome de um módulo da stdlib, e rodar `python3 ingestors/csv.py` põe `ingestors/`
  no `sys.path[0]` — aí `import csv` acharia o próprio driver, não a stdlib. Cada driver Python
  abre removendo o próprio diretório do `sys.path` (2 linhas, uniforme) antes de importar a
  stdlib. Quando o Rust chamar, passa também `PYTHONSAFEPATH=1`/`-P` como reforço.
- Sem input no stdin (terminal interativo) → o driver imprime o **uso** e sai (nunca roda mudo).
- Testável na unha — é a filosofia do projeto (mostrar as tripas):

```bash
  python3 ingestors/csv.py < dados.csv
```

## INVARIANTE: o dado nunca mora no disco do RAGnaRock

O RAGnaRock **não alcança o filesystem da fonte**. O dado só chega por **dois caminhos**:

1. **Push** — os bytes vêm no corpo do POST (um arquivo). O driver lê do **stdin**.
2. **Pull de rede** — o driver conecta num **endpoint** (`host:porta`) com as credenciais que
   vieram na receita efêmera, e puxa por TCP.

**Nunca por um caminho local no disco do RAGnaRock.** Por isso **não há driver `sqlite`** (nem
qualquer coisa *filesystem-dependent*): SQLite é um arquivo local, presumiria que o `.db` está
alcançável na máquina do daemon — e não está. Banco = **rede**. Arquivo = **stdin**.

## Receita de ingestão (drivers de banco — de REDE)

Uma receita é **efêmera** — montada on-the-fly por quem chama (uma app nossa que já tem seus
secrets), consumida e morta na memória, nunca gravada no corpus. Diretivas em comentário
`-- chave: valor` + o SQL. O driver conecta no endpoint e puxa:

```sql
  -- host: db-prod.interno:3306
  -- db:   vendas
  -- user: rag_reader
  -- pass: ${efêmero, veio inline}
  SELECT id, cliente, valor, data FROM pedidos WHERE semana = 32;
```

Drivers de banco de rede precisam de um conector (`pymysql`, `psycopg2`, `pymssql`…) — **não é
stdlib**, então entram numa fase posterior, com a dependência declarada no cabeçalho do driver
(ainda sem venv). A prova de pinagem stdlib é o `csv.py` (arquivo, via stdin) — não um banco.

## Fronteira (não confundir)

Isto é o **driver de ingestão** — dado entra, RAGnaRock é *cliente* da fonte. **Não** é a interface
SQL (#35/#40), onde apps plugam *no* RAGnaRock (RAGnaRock é *servidor*). São dois mundos; um dia se
encontram (um `INSERT <blob> VIA <driver>` da interface SQL despacha pra estes mesmos drivers), mas
não hoje.
