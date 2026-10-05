//! [#53] Porteira Laya do L1: decide "qual tipo?" SEM LLM para as bases que reconhece com segurança.
//!
//! O modelo é treinado fora do daemon (tools/laya_porteira, Python, agendado) e exportado em ONNX.
//! Aqui só há inferência: ONNX Runtime (crate `ort`, biblioteca carregada dinamicamente — a do
//! próprio ambiente de treino, sem download) + tokenizador da Hugging Face em Rust lendo o mesmo
//! `tokenizer.json`. A montagem da sequência reproduz `laya.common.build_head/build_sequence`
//! token a token; `nidhoggd --laya-check <versão>` prova a paridade contra o `paridade.json` que o
//! exportador gera com o Laya oficial.
//!
//! Regra (a mesma que a avaliação usou para promover a versão — `manifest.json`):
//!   1. choice entre TODOS os `tipos` treinados;
//!   2. aceita só se o escolhido está em `liberados`, confiança (max p) >= limiar e
//!      "segue o tipo?" (p de sim) >= limiar;
//!   3. planilha não passa pela porteira (o chamador checa `tabular_spec` antes).
//! Qualquer outro caso devolve None e a base segue para o LLM, exatamente como antes.

use serde_json::Value;
use std::path::{Path, PathBuf};

const TEMP_MIN: f32 = 0.5;
const TEMP_MAX: f32 = 5.0;
const OPT_MAX_TOKENS: usize = 48;
// Textos das perguntas — IGUAIS aos de porteira.py (q_choice/q_segue). O manifest pode trazê-los.
const PERGUNTA_CHOICE: &str = "Que tipo de documento é este?";
const PERGUNTA_SEGUE: &str = "Este documento é um(a) {desc}?";

pub struct Porteira {
    pub versao: String,
    pub dir: PathBuf,
    tipos: Vec<String>,
    liberados: Vec<String>,
    descricoes: Vec<String>, // alinhado com `tipos`
    lim_conf: f32,
    lim_segue: f32,
    pub max_chars: usize,
    perg_choice: String,
    perg_segue: String,
    cls: i64,
    sep: i64,
    mask: i64,
    mask_token: String,
    max_len: usize,
    head_max_len: usize,
    temperatura: [f32; 3],
    temp_por_opcoes: serde_json::Map<String, Value>,
    tok: tokenizers::Tokenizer,
    sessao: ort::session::Session,
}

/// Uma decisão da porteira (aceita ou não), com o que foi medido.
#[derive(Debug, Clone)]
pub struct Decisao {
    pub tipo: String,
    pub confianca: f32,
    pub segue: f32,
    pub aceita: bool,
}

fn ler_json(p: &Path) -> Result<Value, String> {
    let s = std::fs::read_to_string(p).map_err(|e| format!("{}: {e}", p.display()))?;
    serde_json::from_str(&s).map_err(|e| format!("{}: {e}", p.display()))
}

fn clamp_temp(t: f64) -> f32 {
    if !t.is_finite() { return 1.0; }
    (t as f32).clamp(TEMP_MIN, TEMP_MAX)
}

/// `{"body": "<texto>"}` exatamente como `json.dumps(..., ensure_ascii=False)` do Python
/// (separador ", " / ": "; serde_json escapa os mesmos caracteres de controle).
fn estado_body(txt: &str) -> String {
    format!("{{\"body\": {}}}", serde_json::to_string(txt).unwrap_or_else(|_| "\"\"".into()))
}

fn bucket(qt: usize, k: usize) -> String {
    let nome = ["choice", "score", "noul"][qt];
    let tam = if k <= 2 { "2" } else if k <= 5 { "3-5" } else if k <= 10 { "6-10" } else { "11+" };
    format!("{nome}:{tam}")
}

impl Porteira {
    /// Carrega a versão apontada por `dir` (normalmente `<laya_dir>/atual`, um symlink).
    pub fn carrega(dir: &Path, threads: usize) -> Result<Porteira, String> {
        let dir = std::fs::canonicalize(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        let man = ler_json(&dir.join("manifest.json"))?;
        let rj = ler_json(&dir.join("rust.json"))?;
        let onnx = dir.join("onnx").join("laya.onnx");
        if !onnx.exists() { return Err(format!("{} sem onnx/laya.onnx — rode exporta.py", dir.display())); }
        let strs = |v: &Value| -> Vec<String> {
            v.as_array().map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect()).unwrap_or_default()
        };
        let tipos = strs(&man["tipos"]);
        if tipos.is_empty() { return Err("manifest sem tipos".into()); }
        let descricoes: Vec<String> = tipos.iter()
            .map(|t| man["descricoes"][t].as_str().unwrap_or(t).to_string()).collect();
        let mut tok = tokenizers::Tokenizer::from_file(dir.join("tokenizer").join("tokenizer.json"))
            .map_err(|e| format!("tokenizer: {e}"))?;
        tok.with_truncation(None).map_err(|e| format!("tokenizer: {e}"))?;
        tok.with_padding(None);
        let temp = rj["temperature"].as_array().cloned().unwrap_or_default();
        let t = |i: usize| clamp_temp(temp.get(i).and_then(|x| x.as_f64()).unwrap_or(1.0));
        let sessao = ort::session::Session::builder().map_err(|e| format!("onnx: {e}"))?
            .with_optimization_level(ort::session::builder::GraphOptimizationLevel::Level3).map_err(|e| format!("onnx: {e}"))?
            .with_intra_threads(threads.max(1)).map_err(|e| format!("onnx: {e}"))?
            .commit_from_file(&onnx).map_err(|e| format!("onnx: {e}"))?;
        Ok(Porteira {
            versao: man["versao"].as_str().unwrap_or("?").to_string(),
            dir: dir.clone(),
            liberados: strs(&man["liberados"]),
            tipos,
            descricoes,
            lim_conf: man["limiares"]["confianca"].as_f64().unwrap_or(0.9) as f32,
            lim_segue: man["limiares"]["segue"].as_f64().unwrap_or(0.5) as f32,
            max_chars: man["max_chars"].as_u64().unwrap_or(1000) as usize,
            perg_choice: man["perguntas"]["choice"].as_str().unwrap_or(PERGUNTA_CHOICE).to_string(),
            perg_segue: man["perguntas"]["segue"].as_str().unwrap_or(PERGUNTA_SEGUE).to_string(),
            cls: rj["cls_id"].as_i64().ok_or("rust.json sem cls_id")?,
            sep: rj["sep_id"].as_i64().ok_or("rust.json sem sep_id")?,
            mask: rj["mask_id"].as_i64().ok_or("rust.json sem mask_id")?,
            mask_token: rj["mask_token"].as_str().ok_or("rust.json sem mask_token")?.to_string(),
            max_len: rj["max_len"].as_u64().unwrap_or(512) as usize,
            head_max_len: rj["head_max_len"].as_u64().unwrap_or(192) as usize,
            temperatura: [t(0), t(1), t(2)],
            temp_por_opcoes: rj["temperature_by_options"].as_object().cloned().unwrap_or_default(),
            tok,
            sessao,
        })
    }

    pub fn liberados(&self) -> &[String] { &self.liberados }

    fn codifica(&self, s: &str) -> Result<Vec<i64>, String> {
        let e = self.tok.encode(s, false).map_err(|e| format!("tokenize: {e}"))?;
        Ok(e.get_ids().iter().map(|&x| x as i64).collect())
    }

    /// `build_head` do Laya: `[CLS] <t> question: <ins> [SEP] [MASK] opt0 [MASK] opt1 ... [SEP]`.
    fn cabeca(&self, qt: &str, ins: &str, opcoes: &[String]) -> Result<(Vec<i64>, Vec<usize>), String> {
        let ins = ins.replace(&self.mask_token, " ");
        let mut head = self.codifica(&format!("{qt} question: {ins}"))?;
        let mut opt_ids: Vec<Vec<i64>> = Vec::with_capacity(opcoes.len());
        for o in opcoes {
            let mut t = self.codifica(&format!(" {}", o.replace(&self.mask_token, " ")))?;
            t.truncate(OPT_MAX_TOKENS);
            let mut v = vec![self.mask];
            v.extend(t);
            opt_ids.push(v);
        }
        let hm = self.head_max_len as i64;
        let soma = |o: &Vec<Vec<i64>>| o.iter().map(|x| x.len() as i64).sum::<i64>();
        let mut budget = hm - soma(&opt_ids);
        if budget < 16 {
            let per = std::cmp::max(4, (hm - 16) / std::cmp::max(1, opt_ids.len() as i64)) as usize;
            for o in opt_ids.iter_mut() { o.truncate(per); }
            budget = hm - soma(&opt_ids);
        }
        head.truncate(std::cmp::max(8, budget) as usize);
        let mut ids = vec![self.cls];
        ids.extend(head);
        ids.push(self.sep);
        let mut marcadores = vec![];
        for o in opt_ids {
            marcadores.push(ids.len());
            ids.extend(o);
        }
        ids.push(self.sep);
        Ok((ids, marcadores))
    }

    /// `build_sequence` do Laya (estado truncado à direita, sem `truncate_left`).
    fn sequencia(&self, estado: &[i64], qt: &str, ins: &str, opcoes: &[String]) -> Result<(Vec<i64>, Vec<usize>), String> {
        let (mut ids, marcadores) = self.cabeca(qt, ins, opcoes)?;
        let room = self.max_len.saturating_sub(ids.len() + 1);
        ids.extend_from_slice(&estado[..room.min(estado.len())]);
        ids.push(self.sep);
        ids.truncate(self.max_len);
        let marcadores = marcadores.into_iter().filter(|&m| m < self.max_len).collect();
        Ok((ids, marcadores))
    }

    fn opcoes_choice(&self) -> Vec<String> {
        self.tipos.iter().zip(&self.descricoes).map(|(t, d)| if d.is_empty() { t.clone() } else { format!("{t}: {d}") }).collect()
    }
    fn opcoes_noul() -> Vec<String> {
        vec!["false: no, the statement does not hold".into(), "true: yes, the statement holds".into()]
    }

    /// Uma passada do modelo: probabilidades das opções (temperatura do tipo de pergunta).
    fn roda(&mut self, ids: &[i64], marcadores: &[usize], qt: usize) -> Result<Vec<f32>, String> {
        use ort::value::Tensor;
        let l = ids.len();
        let k = marcadores.len();
        if k == 0 { return Err("sem marcadores".into()); }
        let t_ids = Tensor::from_array(([1usize, l], ids.to_vec())).map_err(|e| e.to_string())?;
        let t_att = Tensor::from_array(([1usize, l], vec![1i64; l])).map_err(|e| e.to_string())?;
        let t_pos = Tensor::from_array(([1usize, k], marcadores.iter().map(|&m| m as i64).collect::<Vec<i64>>())).map_err(|e| e.to_string())?;
        let t_msk = Tensor::from_array(([1usize, k], vec![true; k])).map_err(|e| e.to_string())?;
        let t_qt = Tensor::from_array(([1usize], vec![qt as i64])).map_err(|e| e.to_string())?;
        let saidas = self.sessao.run(ort::inputs![
            "input_ids" => t_ids, "attention_mask" => t_att, "marker_pos" => t_pos,
            "marker_mask" => t_msk, "qtype" => t_qt,
        ]).map_err(|e| format!("onnx run: {e}"))?;
        let (_forma, logits) = saidas["logits"].try_extract_tensor::<f32>().map_err(|e| e.to_string())?;
        let temp = self.temp_por_opcoes.get(&bucket(qt, k)).and_then(|v| v.as_f64()).map(clamp_temp)
            .unwrap_or(self.temperatura[qt]);
        let z: Vec<f32> = logits[..k].iter().map(|x| x / temp).collect();
        let m = z.iter().cloned().fold(f32::NEG_INFINITY, f32::max);
        let e: Vec<f32> = z.iter().map(|x| (x - m).exp()).collect();
        let s: f32 = e.iter().sum();
        Ok(e.iter().map(|x| x / s).collect())
    }

    /// Tokens das duas perguntas (para o `--laya-check`) e a decisão.
    pub fn decide_detalhe(&mut self, txt: &str) -> Result<(Decisao, (Vec<i64>, Vec<usize>), (Vec<i64>, Vec<usize>)), String> {
        let txt: String = txt.chars().take(self.max_chars).collect();
        let estado = self.codifica(&estado_body(&txt).replace(&self.mask_token, " "))?;
        let (ids_c, mk_c) = self.sequencia(&estado, "choice", &self.perg_choice.clone(), &self.opcoes_choice())?;
        let p = self.roda(&ids_c, &mk_c, 0)?;
        let (i, &conf) = p.iter().enumerate().fold((0, &f32::MIN), |acc, (i, v)| if *v > *acc.1 { (i, v) } else { acc });
        let tipo = self.tipos[i].clone();
        let ins_s = self.perg_segue.replace("{desc}", &self.descricoes[i]);
        let (ids_s, mk_s) = self.sequencia(&estado, "noul", &ins_s, &Self::opcoes_noul())?;
        let ps = self.roda(&ids_s, &mk_s, 2)?;
        let segue = ps.get(1).copied().unwrap_or(0.0);
        let aceita = self.liberados.iter().any(|l| l == &tipo) && conf >= self.lim_conf && segue >= self.lim_segue;
        Ok((Decisao { tipo, confianca: conf, segue, aceita }, (ids_c, mk_c), (ids_s, mk_s)))
    }

    pub fn decide(&mut self, txt: &str) -> Result<Decisao, String> {
        self.decide_detalhe(txt).map(|(d, _, _)| d)
    }
}

/// Inicializa o ONNX Runtime a partir de uma biblioteca local (uma vez por processo).
pub fn inicia_ort(lib: &str) -> Result<(), String> {
    static FEITO: std::sync::OnceLock<Result<(), String>> = std::sync::OnceLock::new();
    FEITO.get_or_init(|| {
        if !Path::new(lib).exists() { return Err(format!("biblioteca do ONNX Runtime não encontrada: {lib}")); }
        ort::init_from(lib).map_err(|e| e.to_string())?.commit();
        Ok(())
    }).clone()
}

/// `nidhoggd --laya-check <versão>`: compara tokens, marcadores, probabilidades e decisão do Rust
/// com o `paridade.json` gerado pelo Laya oficial. Devolve o relatório e se passou.
pub fn checa_paridade(dir: &Path, threads: usize) -> Result<(String, bool), String> {
    let mut p = Porteira::carrega(dir, threads)?;
    let par = ler_json(&p.dir.join("paridade.json"))?;
    let casos = par["casos"].as_array().ok_or("paridade.json sem casos")?;
    let (mut ids_ok, mut dec_ok, mut dconf, mut dseg) = (0usize, 0usize, 0f32, 0f32);
    let mut falhas = vec![];
    let ids_de = |v: &Value| -> Vec<i64> { v.as_array().map(|a| a.iter().filter_map(|x| x.as_i64()).collect()).unwrap_or_default() };
    let mk_de = |v: &Value| -> Vec<usize> { v.as_array().map(|a| a.iter().filter_map(|x| x.as_u64().map(|n| n as usize)).collect()).unwrap_or_default() };
    let t0 = std::time::Instant::now();
    for c in casos {
        let (d, (ic, mc), (is, ms)) = p.decide_detalhe(c["txt"].as_str().unwrap_or(""))?;
        let ids_iguais = ic == ids_de(&c["seq"]["choice"]["ids"]) && mc == mk_de(&c["seq"]["choice"]["markers"])
            && is == ids_de(&c["seq"]["segue"]["ids"]) && ms == mk_de(&c["seq"]["segue"]["markers"]);
        if ids_iguais { ids_ok += 1; }
        let iguais = d.tipo == c["choice"].as_str().unwrap_or("") && d.aceita == c["aceita"].as_bool().unwrap_or(false);
        if iguais { dec_ok += 1; } else { falhas.push(c["id"].as_str().unwrap_or("?").to_string()); }
        dconf = dconf.max((d.confianca - c["conf"].as_f64().unwrap_or(0.0) as f32).abs());
        dseg = dseg.max((d.segue - c["segue"].as_f64().unwrap_or(0.0) as f32).abs());
    }
    let n = casos.len();
    let ms = t0.elapsed().as_millis() as f64 / n.max(1) as f64;
    let ok = ids_ok == n && dec_ok == n;
    Ok((format!("paridade {} ({}): tokens iguais {ids_ok}/{n} · decisões iguais {dec_ok}/{n} · máx Δconf {dconf:.5} · máx Δsegue {dseg:.5} · {ms:.0} ms/base{}",
                p.versao, if ok { "OK" } else { "FALHOU" },
                if falhas.is_empty() { String::new() } else { format!(" · divergentes: {}", falhas.join(", ")) }), ok))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn estado_igual_ao_json_dumps_do_python() {
        // json.dumps({"body": 'a "b"\n\tç\x01'}, ensure_ascii=False)
        assert_eq!(estado_body("a \"b\"\n\tç\u{1}"), "{\"body\": \"a \\\"b\\\"\\n\\tç\\u0001\"}");
    }

    #[test]
    fn buckets_de_temperatura() {
        assert_eq!(bucket(0, 6), "choice:6-10");
        assert_eq!(bucket(2, 2), "noul:2");
        assert_eq!(bucket(0, 3), "choice:3-5");
        assert_eq!(bucket(0, 11), "choice:11+");
    }

    #[test]
    fn temperatura_limitada() {
        assert_eq!(clamp_temp(0.1), 0.5);
        assert_eq!(clamp_temp(f64::NAN), 1.0);
        assert_eq!(clamp_temp(1.2), 1.2);
    }
}
