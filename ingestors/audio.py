#!/usr/bin/env python3
"""
ingestor: audio — TRANSCREVE um áudio recebido no stdin e devolve o texto puro no stdout.

Pinagem (contrato do ingestor): stdin = bytes do arquivo · stdout = texto transcrito ·
exit 0 = ok, !=0 + motivo no stderr = rejeitado. python3 RAW, sem venv, só stdlib.

Dependências (não-Python, e é de propósito): `ffmpeg` (apt) e o `whisper-cli` do whisper.cpp,
compilado nativo. Nenhum pip: o driver só orquestra dois processos, e a inteligência mora nos
binários — trocar de engine é trocar o que este arquivo chama, sem tocar no daemon.

    stdin (opus/ogg/m4a/mp3/…) → ffmpeg → wav 16k mono → whisper-cli → texto

O áudio ORIGINAL NÃO É GUARDADO. O que sobra é o texto; o arquivo temporário morre no fim,
inclusive quando dá erro. É a mesma regra do resto: o RAGnaRock guarda texto.

Por que ffmpeg é obrigatório: o whisper-cli não decodifica `.opus` nem `.m4a` (testado — ele
responde "failed to read audio data"). E o áudio do WhatsApp é opus no Android e m4a no iOS,
que são justamente os dois casos que motivaram este driver.

Configuração por ambiente (o driver herda o ambiente do daemon):

    RAG_WHISPER_BIN      caminho do whisper-cli   [/dados/whisper/whisper.cpp/build/bin/whisper-cli]
    RAG_WHISPER_MODEL    caminho do modelo .bin   [/dados/whisper/whisper.cpp/models/ggml-large-v3-turbo-q5_0.bin]
    RAG_WHISPER_LANG     idioma                   [pt]
    RAG_WHISPER_THREADS  threads                  [metade dos processadores, no mínimo 4]

O idioma é FIXADO, não detectado: recado de WhatsApp começa curto ("oi, tudo bem?") e a
detecção automática erra com confiança justamente aí — e transcrição errada com aparência de
certa é o pior defeito num sistema que guarda histórico.
"""
import sys
import os
# uniforme com os demais drivers: remove o próprio diretório do sys.path (anti-sombreamento
# da stdlib — ver README).
_here = os.path.dirname(os.path.abspath(__file__))
sys.path[:] = [p for p in sys.path if p and os.path.abspath(p) != _here]
import re
import shutil
import subprocess
import tempfile
import time

BIN = os.environ.get("RAG_WHISPER_BIN", "/dados/whisper/whisper.cpp/build/bin/whisper-cli")
MODELO = os.environ.get("RAG_WHISPER_MODEL",
                        "/dados/whisper/whisper.cpp/models/ggml-large-v3-turbo-q5_0.bin")
IDIOMA = os.environ.get("RAG_WHISPER_LANG", "pt")
THREADS = os.environ.get("RAG_WHISPER_THREADS") or str(max(4, (os.cpu_count() or 8) // 2))


def _duracao(wav):
    """Segundos de áudio, lidos do cabeçalho do wav — para o diagnóstico dizer o que processou."""
    try:
        import wave
        with wave.open(wav) as w:
            return w.getnframes() / float(w.getframerate() or 1)
    except Exception:
        return 0.0


def _limpar(texto):
    """
    O whisper devolve um bloco só, com espaço no começo de cada segmento. Isto junta as frases
    num parágrafo legível — nada de reescrever palavra, só espaçamento.
    """
    t = re.sub(r"[ \t]+", " ", texto.replace("\r", " ").replace("\n", " ")).strip()
    # uma linha por frase deixa o texto navegável na caixa de conteúdo do Odin, e é assim que
    # o operador revisa antes de registrar.
    return re.sub(r"(?<=[.!?…]) (?=[A-ZÀ-Þ])", "\n", t)


def main():
    if sys.stdin.isatty():
        sys.stderr.write("uso: python3 audio.py < recado.opus   (o áudio vem pelo stdin)\n")
        return 2

    for nome, caminho in (("ffmpeg", shutil.which("ffmpeg")), ("whisper-cli", BIN)):
        if not caminho or not (os.path.isfile(caminho) if nome == "whisper-cli" else True):
            sys.stderr.write(f"audio: {nome} não encontrado ({caminho or 'fora do PATH'})\n")
            return 3
    if not os.path.isfile(MODELO):
        sys.stderr.write(f"audio: modelo não encontrado em {MODELO}\n")
        return 3

    dados = sys.stdin.buffer.read()
    if not dados:
        sys.stderr.write("audio: entrada vazia\n")
        return 1

    # O temporário existe porque os dois binários querem arquivo — e some no fim, dê certo ou
    # não. Nada de áudio fica no disco do RAGnaRock.
    caixa = tempfile.mkdtemp(prefix="rag-audio-")
    bruto, wav = os.path.join(caixa, "entrada"), os.path.join(caixa, "audio.wav")
    try:
        with open(bruto, "wb") as f:
            f.write(dados)

        # 16 kHz mono PCM é o que o whisper quer; qualquer outra taxa ele reamostra pior.
        conv = subprocess.run(
            ["ffmpeg", "-v", "error", "-y", "-i", bruto,
             "-ar", "16000", "-ac", "1", "-c:a", "pcm_s16le", wav],
            capture_output=True)
        if conv.returncode != 0 or not os.path.exists(wav):
            motivo = (conv.stderr.decode("utf-8", "replace").strip().splitlines() or ["formato ilegível"])[-1]
            sys.stderr.write(f"audio: ffmpeg não decodificou ({motivo})\n")
            return 1

        segundos = _duracao(wav)
        relogio = time.monotonic()
        # -np: sem barulho de progresso · -nt: sem marca de tempo, é texto corrido que vai para
        # a caixa de conteúdo · -l: idioma fixado.
        fala = subprocess.run(
            [BIN, "-m", MODELO, "-f", wav, "-l", IDIOMA, "-t", THREADS, "-np", "-nt"],
            capture_output=True)
        gasto = time.monotonic() - relogio
        if fala.returncode != 0:
            motivo = (fala.stderr.decode("utf-8", "replace").strip().splitlines() or ["sem motivo"])[-1]
            sys.stderr.write(f"audio: whisper falhou ({motivo})\n")
            return 1

        texto = _limpar(fala.stdout.decode("utf-8", "replace"))
        if not texto:
            sys.stderr.write(f"audio: nenhuma fala reconhecida em {segundos:.0f}s de áudio\n")
            return 1

        sys.stdout.write(texto + "\n")
        sys.stderr.write(
            f"audio: {segundos:.0f}s de áudio → {len(texto)} caracteres em {gasto:.0f}s "
            f"({os.path.basename(MODELO)}, {THREADS} threads, {IDIOMA})\n")
        return 0
    finally:
        shutil.rmtree(caixa, ignore_errors=True)


if __name__ == "__main__":
    sys.exit(main())
