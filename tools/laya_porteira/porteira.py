#!/usr/bin/env python3
"""Porteira Laya do L1 do Nidhogg — rotina RECORRENTE de treino, avaliação e promoção (#53).

O banco é dinâmico: a cada execução (agendada), a rotina
  1. coleta os rótulos atuais — `nidhogg.doc_class FINAL` no ClickHouse (a re-tipagem humana
     prevalece: o LLM nunca a sobrescreve) — e o MESMO texto que o classificador vê (os 1000
     primeiros caracteres do chunk 0, via ragd `/chunk`);
  2. gatilho: só treina se entraram >= --min-novos rótulos desde o modelo atual (ou --forcar);
  3. tipos elegíveis: os que têm >= --min-por-tipo bases. Os demais viram ÓRFÃOS — exemplos
     negativos no treino e teste de "não aceitar o que não conhece" na avaliação;
  4. separa a avaliação POR BASE (estratificada) e aumenta o treino com variações de layout;
  5. fine-tune (laya_ft.train, o roteiro oficial) e avaliação no conjunto separado;
  6. PROMOVE só se: zero aceito-errado (conhecidos E órfãos) e cobertura >= a do modelo atual
     medido no mesmo conjunto. Senão a campeã continua.
Cada versão vai para <dir>/versoes/<id>/ com manifest.json (tipos liberados, descrições,
limiares, métricas, hashes); <dir>/atual aponta para a campeã; <dir>/historico.jsonl guarda
uma linha por execução (inclusive as que não treinaram).

Treino em Python FORA dos daemons; o uso em produção é do nidhoggd (ONNX), que lê o manifest.
"""
import argparse, hashlib, json, os, random, shutil, sys, time, urllib.parse, urllib.request
from pathlib import Path

os.environ.setdefault("USE_TF", "0")
AQUI = Path(__file__).resolve().parent
sys.path.insert(0, str(AQUI))

MAX_CHARS = 1000            # = CLASSIFY_MAX_CHARS do nidhoggd: a porteira vê o que o LLM vê
LIM_CONF, LIM_SEGUE = 0.9, 0.5   # aceita só com confiança E "segue o tipo?" acima disso (experimentos 03/out)
FORA = {"sem-texto", "?", "outro"}   # não são tipos que a porteira decide

# Descrição de cada tipo (o Laya decide pelo TEXTO das opções). Sobrescreva com --descricoes.
DESCRICOES = {
    "cadastro": "planilha ou ficha de cadastro de pessoas, clientes ou fornecedores",
    "contrato": "contrato ou instrumento particular com partes, cláusulas, vigência e assinaturas",
    "comprovante": "comprovante de pagamento ou transferência bancária",
    "nota_fiscal": "nota fiscal (NF-e/DANFE) com emitente, destinatário, itens e impostos",
    "recibo": "recibo simples de quitação de um valor",
    "boleto": "boleto bancário de cobrança com linha digitável e vencimento",
    "balanco": "balanço patrimonial com ativo, passivo e patrimônio líquido",
    "extrato": "extrato bancário ou de conta com lançamentos e saldos",
    "dre": "demonstração do resultado do exercício (receitas, custos, lucro)",
    "folha_pagamento": "folha de pagamento ou holerite com salário, descontos e líquido",
    "ordem_compra": "ordem ou pedido de compra a um fornecedor",
    "cotacao": "cotação ou orçamento de preços",
    "relatorio": "relatório técnico ou gerencial com análise e resultados",
    "livro": "livro ou obra literária longa em capítulos",
    "artigo": "artigo de opinião, acadêmico ou jornalístico",
    "ata": "ata de reunião com participantes, pauta e deliberações",
    "carta": "carta ou correspondência pessoal ou comercial",
    "oficio": "ofício ou comunicado formal entre órgãos ou empresas",
    "memorial": "memorial descritivo ou justificativo",
    "curriculo": "currículo profissional com formação e experiência",
    "discurso": "discurso, palestra ou pronunciamento",
    "codigo_fonte": "código-fonte de programa",
    "config": "arquivo de configuração de sistema",
    "log": "registro (log) de eventos de sistema",
}


def log(msg):
    print(f"[{time.strftime('%Y-%m-%d %H:%M:%S')}] [porteira] {msg}", flush=True)


# ─────────────────────────────── fontes de rótulos ───────────────────────────────
def fonte_producao(ch_url, ragd):
    """Rótulos de doc_class FINAL + texto do chunk 0 do ragd. Devolve [{id, tipo, origem, txt}]."""
    sql = ("SELECT collection, name, tipo, origem FROM nidhogg.doc_class FINAL "
           "ORDER BY collection, name FORMAT JSONEachRow")
    corpo = urllib.request.urlopen(f"{ch_url}/?query={urllib.parse.quote(sql)}", timeout=60).read().decode()
    docs = []
    for l in corpo.splitlines():
        r = json.loads(l)
        # FORA não é tipo; `laya` é a própria porteira — treinar nas próprias respostas a faria
        # reforçar os próprios erros. Só LLM e humano ensinam.
        if r["tipo"] in FORA or r["origem"] == "laya":
            continue
        req = json.dumps({"collection": r["collection"], "base": r["name"], "id": 0}).encode()
        try:
            v = json.load(urllib.request.urlopen(urllib.request.Request(f"{ragd}/chunk", req,
                         {"Content-Type": "application/json"}), timeout=30))
            txt = (v.get("chunks") or [{}])[0].get("text") or ""
        except Exception:
            txt = ""
        if txt.strip():
            docs.append({"id": f'{r["collection"]}/{r["name"]}', "tipo": r["tipo"], "origem": r["origem"],
                         "txt": txt[:MAX_CHARS]})
    return docs


def fonte_sintetica(raiz):
    """Corpus de teste: <raiz>/<tipo>/*.txt (o nome da pasta é o tipo). Para validar a rotina."""
    docs = []
    for p in sorted(Path(raiz).iterdir()):
        if not p.is_dir() or p.name.startswith(("_", ".")):
            continue
        for f in sorted(p.iterdir()):
            if f.is_file():
                docs.append({"id": f"{p.name}/{f.name}", "tipo": p.name, "origem": "sintetico",
                             "txt": f.read_text(encoding="utf-8", errors="replace")[:MAX_CHARS]})
    return docs


# ─────────────────────────────── preparo ───────────────────────────────
def eh_tabela(txt):
    """Mesma regra do `tabular_spec` do nidhoggd: >= 3 linhas e >= 80% delas com o mesmo nº de
    delimitadores da 1ª. Planilha NÃO passa pela porteira (forma fora do que ela aprende; o
    nidhoggd já a detecta antes de classificar) — validado em 05/out: um CSV órfão era aceito."""
    ls = [l.rstrip("\r") for l in txt.splitlines() if l.strip()]
    if len(ls) < 3:
        return False
    for d in (",", ";", "\t"):
        h = ls[0].count(d)
        if h and sum(1 for l in ls if l.count(d) == h) * 100 >= len(ls) * 80:
            return True
    return False



def impressao(d):
    return hashlib.sha1(f'{d["id"]}|{d["tipo"]}'.encode()).hexdigest()[:16]


def separa(docs, min_por_tipo, frac_aval, rnd, max_treino=10**9, max_aval=10**9):
    """Por tipo: elegível se >= min_por_tipo bases. Separa avaliação POR BASE em todos os tipos
    (os não elegíveis viram órfãos de treino e de avaliação). Teto por tipo no treino e na
    avaliação (sorteio): classes equilibradas e tempo de treino previsível com o banco crescendo."""
    por = {}
    for d in docs:
        por.setdefault(d["tipo"], []).append(d)
    elegiveis = sorted(t for t, l in por.items() if len(l) >= min_por_tipo)
    treino, aval = [], []
    for t, l in por.items():
        l = l[:]
        rnd.shuffle(l)
        n_av = max(3, round(len(l) * frac_aval)) if t in elegiveis else max(1, len(l) // 2)
        aval += l[:n_av][:max_aval]
        treino += l[n_av:][:max_treino]
    return elegiveis, treino, aval


def variacoes(txt, rnd):
    """Aumento de layout (o modelo dos experimentos aprendeu a FORMA; isto força o conteúdo)."""
    ls = [l for l in txt.splitlines() if l.strip()]
    if len(ls) < 3:
        return [txt]
    corpo = ls[1:]
    rnd.shuffle(corpo)
    return [txt, "\n".join([ls[0]] + corpo), "\n".join(l.lower() for l in corpo)]


PERGUNTA_CHOICE = "Que tipo de documento é este?"
PERGUNTA_SEGUE = "Este documento é um(a) {desc}?"     # o nidhoggd lê as duas do manifest


def q_choice(tipos, desc):
    return {"type": "choice", "instructions": PERGUNTA_CHOICE, "criteria": {t: desc[t] for t in tipos}}


def q_segue(t, desc):
    return {"type": "noul", "instructions": PERGUNTA_SEGUE.replace("{desc}", desc[t])}


def monta_itens(treino, elegiveis, desc, base, rnd):
    import laya_ft
    from transformers import AutoTokenizer
    laya_ft._fix_tokenizer_config(str(base))
    cfg = json.load(open(Path(base) / "rl_agent_config.json"))
    cfg.update({"max_len": 1024, "head_max_len": 256})
    tok = AutoTokenizer.from_pretrained(Path(base) / "tokenizer")
    itens = []
    add = lambda st, q, probs: itens.append(it) if (it := laya_ft.build_training_item(tok, cfg, st, q, {"probabilities": probs})) else None
    for d in treino:
        for v in variacoes(d["txt"], rnd):
            st = {"body": v}
            if d["tipo"] in elegiveis:
                t = d["tipo"]
                outros = [k for k in elegiveis if k != t]
                if outros:   # listas de opções VARIADAS (contra a instabilidade ao conjunto de opções)
                    ops = rnd.sample(outros, rnd.randint(1, len(outros))) + [t]
                    rnd.shuffle(ops)
                    add(st, q_choice(ops, desc), {k: float(k == t) for k in ops})
                add(st, q_segue(t, desc), {"true": 1.0, "false": 0.0})
                if outros:
                    add(st, q_segue(rnd.choice(outros), desc), {"true": 0.0, "false": 1.0})
            else:            # órfão: "segue o tipo X?" = NÃO, para TODOS os elegíveis
                for t in elegiveis:
                    add(st, q_segue(t, desc), {"true": 0.0, "false": 1.0})
    rnd.shuffle(itens)
    return itens


# ─────────────────────────────── avaliação ───────────────────────────────
def decide(agent, docs, tipos, desc):
    """Mesma regra do nidhoggd: escolhe entre os tipos liberados e aceita só com confiança e
    'segue' acima dos limiares. Devolve [(doc, escolhido, conf, segue, aceita)]."""
    out = []
    for i in range(0, len(docs), 16):
        bloco = docs[i:i + 16]
        sts = [{"body": d["txt"]} for d in bloco]
        if len(tipos) > 1:
            rs = agent.predict_batch(sts, {"m": q_choice(tipos, desc)})
            esc = [(r["answers"]["m"]["choice"], r["answers"]["m"].get("answer_confidence", r["answers"]["m"].get("confidence", 0.0))) for r in rs]
        else:
            esc = [(tipos[0], 1.0)] * len(bloco)
        for d, st, (t, c) in zip(bloco, sts, esc):
            s = agent.predict(st, {"s": q_segue(t, desc)})["answers"]["s"]["noul"]
            out.append((d, t, float(c), float(s), c >= LIM_CONF and s >= LIM_SEGUE))
    return out


def metricas(res, tipos, aceitos=None):
    """`aceitos` = tipos que o nidhoggd aceitaria (liberados); None = todos os `tipos`. Um tipo
    escolhido fora de `aceitos` vai ao LLM — não conta como aceito, nem certo nem errado."""
    aceitos = set(tipos if aceitos is None else aceitos)
    res = [(d, t, c, s, a and t in aceitos) for d, t, c, s, a in res]
    conh = [r for r in res if r[0]["tipo"] in tipos]
    orf = [r for r in res if r[0]["tipo"] not in tipos]
    certo = sum(1 for d, t, c, s, a in conh if a and t == d["tipo"])
    errado = sum(1 for d, t, c, s, a in conh if a and t != d["tipo"])
    orf_aceitos = sum(1 for *_, a in orf if a)
    por_tipo = {}
    for t in tipos:
        rt = [r for r in conh if r[0]["tipo"] == t]
        por_tipo[t] = {"n": len(rt), "aceito_certo": sum(1 for d, e, c, s, a in rt if a and e == t),
                       "aceito_errado": sum(1 for d, e, c, s, a in rt if a and e != t),
                       # o que PROÍBE liberar t: outro documento (conhecido ou órfão) aceito COMO t
                       "invadido": sum(1 for d, e, c, s, a in res if a and e == t and d["tipo"] != t)}
        por_tipo[t]["cobertura"] = round(por_tipo[t]["aceito_certo"] / max(1, len(rt)), 3)
    return {"n_conhecidos": len(conh), "n_orfaos": len(orf),
            "cobertura": round(certo / max(1, len(conh)), 4), "aceito_errado": errado,
            "orfaos_aceitos": orf_aceitos, "por_tipo": por_tipo}


# ─────────────────────────────── ciclo ───────────────────────────────
def hash_arquivo(p):
    h = hashlib.sha256()
    with open(p, "rb") as f:
        for b in iter(lambda: f.read(1 << 20), b""):
            h.update(b)
    return h.hexdigest()


def le_manifest(p):
    try:
        return json.load(open(Path(p) / "manifest.json"))
    except Exception:
        return None


def historico(dirp, reg):
    with open(Path(dirp) / "historico.jsonl", "a") as f:
        f.write(json.dumps(reg, ensure_ascii=False) + "\n")


def ciclo(a):
    import torch
    torch.set_num_threads(a.threads)
    rnd = random.Random(a.semente)
    dirp = Path(a.dir)
    (dirp / "versoes").mkdir(parents=True, exist_ok=True)
    desc = dict(DESCRICOES)
    if a.descricoes:
        desc.update(json.load(open(a.descricoes)))
    docs = fonte_sintetica(a.fonte[len("sintetico:"):]) if a.fonte.startswith("sintetico:") else fonte_producao(a.ch_url, a.ragd)
    docs = [d for d in docs if d["tipo"] in desc and not eh_tabela(d["txt"])]
    impr = sorted(impressao(d) for d in docs)
    atual = le_manifest(dirp / "atual")
    novos = len(set(impr) - set(atual["impressoes"])) if atual else len(impr)
    reg = {"at": time.strftime("%Y-%m-%d %H:%M:%S"), "rotulos": len(docs), "novos": novos,
           "humanos": sum(d["origem"] == "humano" for d in docs)}
    log(f"{len(docs)} rótulos ({reg['humanos']} humanos), {novos} novos desde o modelo atual")
    if novos < a.min_novos and not a.forcar and not a.reavaliar:
        reg["decisao"] = f"sem treino: {novos} novo(s) < {a.min_novos}"
        log(reg["decisao"]); historico(dirp, reg); return 0
    elegiveis, treino, aval = separa(docs, a.min_por_tipo, a.frac_aval, rnd, a.max_treino_tipo, a.max_aval_tipo)
    reg["elegiveis"] = elegiveis
    if not elegiveis:
        cont = {}
        for d in docs:
            cont[d["tipo"]] = cont.get(d["tipo"], 0) + 1
        reg["decisao"] = f"sem treino: nenhum tipo com >= {a.min_por_tipo} bases"
        reg["por_tipo"] = dict(sorted(cont.items(), key=lambda x: -x[1]))
        log(f'{reg["decisao"]} — {reg["por_tipo"]}'); historico(dirp, reg); return 0

    import laya, laya_ft
    if a.reavaliar:   # reaplica avaliação/liberação/promoção a uma versão já treinada (sem treinar)
        out = Path(a.reavaliar).resolve()
        velho = le_manifest(out) or {}
        vid = out.name
        if velho.get("tipos") and velho["tipos"] != elegiveis:
            log(f"aviso: tipos do treino {velho['tipos']} ≠ elegíveis hoje {elegiveis} — avaliando com os do treino")
            elegiveis = velho["tipos"]
        itens = [None] * velho.get("n_itens", 0)
        reg["reavaliacao"] = True
        log(f"{vid}: reavaliação · tipos {elegiveis} · avaliação {len(aval)} bases")
    else:
        vid = time.strftime("v%Y%m%d-%H%M%S")
        out = dirp / "versoes" / vid
        itens = monta_itens(treino, elegiveis, desc, a.base, rnd)
        itens_p = dirp / f"itens-{vid}.pt"
        torch.save(itens, itens_p)
        log(f"{vid}: tipos {elegiveis} · treino {len(treino)} bases → {len(itens)} exemplos · avaliação {len(aval)} bases")
        t0 = time.time()
        targs = argparse.Namespace(epochs=a.epocas, micro_batch=2, grad_accum=16, calib_max=400,
                                   output_dir=str(out), no_checkpointing=not a.checkpointing)
        laya_ft.train(targs, str(a.base), str(itens_p), torch.device("cpu"))
        shutil.rmtree(out / "checkpoint_latest", ignore_errors=True)
        itens_p.unlink(missing_ok=True)
        reg["treino_min"] = round((time.time() - t0) / 60, 1)

    res = decide(laya.load(str(out)), aval, elegiveis, desc)
    bruto = metricas(res, elegiveis)
    log(f"{vid} avaliado (todos os tipos): {json.dumps({k: v for k, v in bruto.items() if k != 'por_tipo'})}")
    # libera só o tipo que ninguém invadiu: zero erro, zero órfão aceito COMO ele, cobertura mínima
    liberados = [t for t, m in bruto["por_tipo"].items()
                 if m["n"] >= 3 and m["aceito_errado"] == 0 and m["invadido"] == 0 and m["cobertura"] >= a.min_cobertura_tipo]
    desafiante = metricas(res, elegiveis, liberados)
    log(f"{vid} restrito aos liberados {liberados}: {json.dumps({k: v for k, v in desafiante.items() if k != 'por_tipo'})}")
    campea = None
    if atual:
        campea = metricas(decide(laya.load(str((dirp / "atual").resolve())), aval, atual["tipos"],
                                 {**desc, **atual["descricoes"]}), atual["tipos"], atual["liberados"])
        log(f"campeã atual ({atual['versao']}) no mesmo conjunto: cobertura {campea['cobertura']} · errado {campea['aceito_errado']} · órfãos aceitos {campea['orfaos_aceitos']}")
    seguro = desafiante["aceito_errado"] == 0 and desafiante["orfaos_aceitos"] == 0
    melhor = (campea is None or campea["aceito_errado"] > 0 or campea["orfaos_aceitos"] > 0
              or desafiante["cobertura"] >= campea["cobertura"])
    man = {"versao": vid, "criado": reg["at"], "base": str(a.base), "fonte": a.fonte.split(":")[0],
           "tipos": elegiveis, "liberados": liberados, "descricoes": {t: desc[t] for t in elegiveis},
           "limiares": {"confianca": LIM_CONF, "segue": LIM_SEGUE}, "max_chars": MAX_CHARS,
           "perguntas": {"choice": PERGUNTA_CHOICE, "segue": PERGUNTA_SEGUE},
           "regra": "choice entre `tipos`; aceita só se o escolhido está em `liberados`, confiança >= limiar e "
                    "'segue o tipo?' >= limiar; planilha (regra do tabular_spec) não passa pela porteira",
           "avaliacao": desafiante, "avaliacao_todos_os_tipos": bruto, "campea_no_mesmo_conjunto": campea, "n_treino": len(treino),
           "n_itens": len(itens), "epocas": a.epocas, "impressoes": impr,
           "hash_modelo": hash_arquivo(out / "model.safetensors")}
    json.dump(man, open(out / "manifest.json", "w"), ensure_ascii=False, indent=1)
    if seguro and melhor and liberados:
        # exporta ANTES de apontar `atual`: o nidhoggd só usa versão exportada (ONNX + rust.json +
        # paridade.json), e não pode ver o symlink trocar para uma versão sem grafo
        try:
            import exporta
            if not (out / "onnx" / "laya.onnx").exists():
                exporta.exporta_onnx(out)
            tok, cfg = exporta.rust_json(out)
            casos = exporta.paridade(out, tok, cfg, random.Random(53).sample(aval, min(40, len(aval))))
            log(f"{vid} exportada para o nidhoggd (onnx + rust.json + paridade de {len(casos)} casos)")
            tmp = dirp / "atual.tmp"
            tmp.unlink(missing_ok=True)
            tmp.symlink_to(Path("versoes") / vid)
            os.replace(tmp, dirp / "atual")
            reg["decisao"] = f"PROMOVIDA {vid}: liberados {liberados}"
        except Exception as e:
            reg["decisao"] = f"reprovada {vid}: exportação falhou ({e}) — campeã mantida"
    else:
        motivo = "aceitou errado" if not seguro else ("cobertura abaixo da campeã" if not melhor else "nenhum tipo liberado")
        reg["decisao"] = f"reprovada {vid}: {motivo} — campeã mantida"
    reg.update({"versao": vid, "cobertura": desafiante["cobertura"], "aceito_errado": desafiante["aceito_errado"],
                "orfaos_aceitos": desafiante["orfaos_aceitos"], "liberados": liberados})
    log(reg["decisao"]); historico(dirp, reg)
    # retenção: mantém a campeã e as --manter versões mais novas
    alvo = (dirp / "atual").resolve() if (dirp / "atual").exists() else None
    vs = sorted(p for p in (dirp / "versoes").iterdir() if p.is_dir())
    for p in vs[:-a.manter]:
        if p.resolve() != alvo:
            shutil.rmtree(p, ignore_errors=True)
    return 0


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--dir", default="/dados/ragnarock/laya", help="raiz das versões/manifest/histórico")
    ap.add_argument("--base", default="/dados/modelos/laya/base/laya-multilingual", help="Laya base (cópia local)")
    ap.add_argument("--fonte", default="producao", help="producao | sintetico:<dir com uma pasta por tipo>")
    ap.add_argument("--ch-url", default="http://127.0.0.1:8123")
    ap.add_argument("--ragd", default="http://127.0.0.1:11499")
    ap.add_argument("--descricoes", help="JSON {tipo: descrição} que sobrescreve/estende as padrão")
    ap.add_argument("--min-por-tipo", type=int, default=20)
    ap.add_argument("--min-novos", type=int, default=10)
    ap.add_argument("--min-cobertura-tipo", type=float, default=0.5)
    ap.add_argument("--frac-aval", type=float, default=0.25)
    ap.add_argument("--max-treino-tipo", type=int, default=40, help="teto de bases por tipo no treino")
    ap.add_argument("--max-aval-tipo", type=int, default=15, help="teto de bases por tipo na avaliação")
    ap.add_argument("--epocas", type=int, default=2)
    # medido na Aron (Xeon E5-2680 v4, 14 núcleos/28 threads, 05/out): 14 threads sem checkpointing
    # = 0,99 s/exemplo; 8 threads com checkpointing = 1,31; 28 threads (hyper-threading) = 1,58 a 2,22
    ap.add_argument("--threads", type=int, default=14, help="= núcleos FÍSICOS (hyper-threading piora)")
    ap.add_argument("--checkpointing", action="store_true", help="economiza RAM recalculando ativações (~25%% mais lento)")
    ap.add_argument("--manter", type=int, default=3)
    ap.add_argument("--semente", type=int, default=2026)
    ap.add_argument("--forcar", action="store_true", help="treina mesmo sem rótulos novos suficientes")
    ap.add_argument("--reavaliar", metavar="DIR_VERSAO", help="não treina: reavalia/promove uma versão já treinada")
    sys.exit(ciclo(ap.parse_args()))


if __name__ == "__main__":
    main()
