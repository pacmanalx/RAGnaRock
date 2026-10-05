#!/usr/bin/env python3
"""Exporta uma versão da porteira para o nidhoggd (#53): ONNX + rust.json + paridade.json.

  <versao>/onnx/laya.onnx (+ .data)  grafo fp32 (mesmos nomes/eixos do export oficial do Laya;
                                     int8 NÃO: o próprio Laya mede perda real de acerto)
  <versao>/rust.json                 o que o Rust precisa e não deve adivinhar: ids dos tokens
                                     especiais, token de máscara, max_len/head_max_len, temperaturas
  <versao>/paridade.json             referência do Python (tokens, marcadores, probabilidades e
                                     decisão) para `nidhoggd --laya-check` provar que o Rust bate

Uso: exporta.py <dir_versao> [--docs N] [--ch-url ...] [--ragd ...]
"""
import argparse, json, os, random, sys
from pathlib import Path

os.environ.setdefault("USE_TF", "0")
sys.path.insert(0, str(Path(__file__).resolve().parent))
import porteira as P


def exporta_onnx(versao: Path):
    import torch
    from laya.agent import Agent
    out = versao / "onnx" / "laya.onnx"
    out.parent.mkdir(exist_ok=True)
    agent = Agent(str(versao), compile=False, device="cpu")
    # dummies com batch/seq/marcadores > 1 e distintos (o export oficial explica: dimensão 1 é
    # especializada no trace e o grafo quebra com batch >= 2)
    b, s, m = 2, 17, 3
    ins = (torch.randint(0, 100, (b, s), dtype=torch.long), torch.ones((b, s), dtype=torch.long),
           torch.tensor([[1, 5, 9]] * b, dtype=torch.long), torch.ones((b, m), dtype=torch.bool),
           torch.zeros(b, dtype=torch.long))
    eixos = {"input_ids": {0: "batch_size", 1: "seq_len"}, "attention_mask": {0: "batch_size", 1: "seq_len"},
             "marker_pos": {0: "batch_size", 1: "num_markers"}, "marker_mask": {0: "batch_size", 1: "num_markers"},
             "qtype": {0: "batch_size"}, "logits": {0: "batch_size", 1: "num_markers"}, "act_logits": {0: "batch_size"}}
    torch.onnx.export(agent.model, ins, str(out), export_params=True, opset_version=18, do_constant_folding=True,
                      input_names=["input_ids", "attention_mask", "marker_pos", "marker_mask", "qtype"],
                      output_names=["logits", "act_logits"], dynamic_axes=eixos)
    return out


def rust_json(versao: Path):
    from transformers import AutoTokenizer
    from laya.agent import _fix_tokenizer_config
    _fix_tokenizer_config(str(versao))
    tok = AutoTokenizer.from_pretrained(versao / "tokenizer")
    cfg = json.load(open(versao / "rl_agent_config.json"))
    r = {"cls_id": tok.cls_token_id, "sep_id": tok.sep_token_id, "pad_id": tok.pad_token_id,
         "mask_id": tok.mask_token_id, "mask_token": tok.mask_token,
         "max_len": cfg.get("max_len", 512), "head_max_len": cfg.get("head_max_len", 192),
         "temperature": cfg.get("temperature", [1.0, 1.0, 1.0]),
         "temperature_by_options": cfg.get("temperature_by_options", {})}
    json.dump(r, open(versao / "rust.json", "w"), ensure_ascii=False, indent=1)
    return tok, cfg


def paridade(versao: Path, tok, cfg, docs):
    """Para cada doc: tokens e marcadores das DUAS perguntas (choice e 'segue'), probabilidades
    pelo ONNXAgent oficial e a decisão final pela regra do manifest."""
    from laya.common import build_sequence
    from laya.onnx_agent import ONNXAgent
    man = json.load(open(versao / "manifest.json"))
    desc, tipos, lib = man["descricoes"], man["tipos"], set(man["liberados"])
    lc, ls = man["limiares"]["confianca"], man["limiares"]["segue"]
    ag = ONNXAgent(str(versao), onnx_path=str(versao / "onnx" / "laya.onnx"))
    casos = []
    for d in docs:
        st = {"body": d["txt"]}
        qc = P.q_choice(tipos, desc)
        rc = ag.predict(st, {"m": qc})["answers"]["m"]
        esc = rc["choice"]
        qs = P.q_segue(esc, desc)
        rs = ag.predict(st, {"s": qs})["answers"]["s"]
        seq = {}
        for nome, q in (("choice", qc), ("segue", qs)):
            iq = ONNXAgent._to_internal(q)
            ids, mk = build_sequence(tok, st, iq, cfg.get("max_len", 512), cfg.get("head_max_len", 192))
            seq[nome] = {"ids": ids, "markers": mk}
        aceita = esc in lib and rc["answer_confidence"] >= lc and rs["noul"] >= ls
        casos.append({"id": d["id"], "txt": d["txt"], "seq": seq,
                      "choice": esc, "prob_choice": rc["probabilities"], "conf": rc["answer_confidence"],
                      "segue": rs["noul"], "aceita": aceita})
    json.dump({"versao": man["versao"], "casos": casos}, open(versao / "paridade.json", "w"), ensure_ascii=False)
    return casos


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("versao")
    ap.add_argument("--docs", type=int, default=40)
    ap.add_argument("--ch-url", default="http://127.0.0.1:8123")
    ap.add_argument("--ragd", default="http://127.0.0.1:11499")
    a = ap.parse_args()
    versao = Path(a.versao).resolve()
    P.log(f"exportando {versao.name} para ONNX ...")
    exporta_onnx(versao)
    tok, cfg = rust_json(versao)
    docs = [d for d in P.fonte_producao(a.ch_url, a.ragd) if not P.eh_tabela(d["txt"])]
    docs = random.Random(53).sample(docs, min(a.docs, len(docs)))
    casos = paridade(versao, tok, cfg, docs)
    P.log(f"{versao.name}: onnx + rust.json + paridade ({len(casos)} casos, {sum(c['aceita'] for c in casos)} aceitos)")


if __name__ == "__main__":
    main()
