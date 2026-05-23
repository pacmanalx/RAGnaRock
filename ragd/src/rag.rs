//! ragd::rag — motor de busca (RagBase + recall cosseno + rerank matched filter).
//! Cópia EVOLUÍVEL do search_rag da PoC: aqui pode mudar livre (rust_concept congela).
use std::collections::HashMap;
use serde_json::{json, Value};
use rayon::prelude::*;
use crate::tokenizer::{normalize, syllabify, words};
use crate::vector::cosine;
use crate::chunk::find_chars;

const PROX_SCALE: f64 = 8.0;
/// Recall paraleliza (rayon) só a partir deste nº de chunks; abaixo roda sequencial.
const PAR_RECALL_MIN: usize = 512;
/// Modo de armazenamento: true = cacheia `words` no load (rápido, +RAM); false = híbrido
/// (não cacheia; rerank recomputa só os candidatos). Setado no boot a partir do config.
pub static CACHE_WORDS: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);

// ----------------------------- rerank (estagio 2) ----------------------------
/// Codigo fonetico estilo SOUNDEX (1a letra + 3 digitos de grupos de consoantes).
/// Palavras que SOAM parecido compartilham o codigo — Aslan/Aslam -> "A245".
fn sound_code(c: char) -> u8 {
    match c {
        'b' | 'f' | 'p' | 'v' => b'1',
        'c' | 'g' | 'j' | 'k' | 'q' | 's' | 'x' | 'z' => b'2',
        'd' | 't' => b'3',
        'l' => b'4',
        'm' | 'n' => b'5',
        'r' => b'6',
        _ => b'0', // vogais, h, w, y
    }
}
pub fn soundex(word: &str) -> String {
    let w: Vec<char> = normalize(word).chars().filter(|c| c.is_ascii_alphabetic()).collect();
    if w.is_empty() { return String::new(); }
    let mut out = vec![w[0].to_ascii_uppercase() as u8];
    let mut prev = sound_code(w[0]);
    for &c in &w[1..] {
        let code = sound_code(c);
        if code != b'0' && code != prev {
            out.push(code);
            if out.len() == 4 { break; }
        }
        if c != 'h' && c != 'w' { prev = code; }   // h/w nao resetam (regra classica)
    }
    while out.len() < 4 { out.push(b'0'); }
    String::from_utf8(out).unwrap()
}

/// Chunk -> silabas agrupadas por PALAVRA (cada item = silabas de uma palavra).
fn chunk_words(text: &str) -> Vec<Vec<String>> {
    let lower = text.to_lowercase();
    let mut out = vec![];
    for w in words(&lower) {
        let syls: Vec<String> = syllabify(&w).iter().map(|s| normalize(s))
            .filter(|s| !s.is_empty()).collect();
        if !syls.is_empty() { out.push(syls); }
    }
    out
}

/// Query pré-tokenizada UMA vez por busca (hoist): sílabas dos termos-chave + o
/// soundex de cada termo já calculado. Antes isso era refeito a CADA chunk candidato.
pub struct QueryTerms {
    terms: Vec<Vec<String>>,   // sílabas por termo-chave (palavras >= 2 sílabas, ou todas)
    sx: Vec<String>,           // soundex por termo ("" se termo longo, len > 3)
}

/// Tokeniza a query em termos-chave 1× (chamada fora do loop de candidatos).
pub fn prep_query(query: &str) -> QueryTerms {
    let lower = query.to_lowercase();
    let mut all: Vec<Vec<String>> = vec![];
    for w in words(&lower) {
        let qs: Vec<String> = syllabify(&w).iter().map(|s| normalize(s))
            .filter(|s| !s.is_empty()).collect();
        if !qs.is_empty() { all.push(qs); }
    }
    // termos-chave = palavras com >= 2 sílabas; se todas forem monossílabas, usa todas
    let terms: Vec<Vec<String>> = {
        let multi: Vec<Vec<String>> = all.iter().filter(|qs| qs.len() >= 2).cloned().collect();
        if multi.is_empty() { all } else { multi }
    };
    // SOUNDEX só p/ termos CURTOS (nomes tipo Aslan/Aslam); palavra longa colide demais
    // ("ressurreição" ~ "rigorosa" = R262) e já tem raiz silábica discriminante
    let sx: Vec<String> = terms.iter()
        .map(|qs| if qs.len() <= 3 { soundex(&qs.concat()) } else { String::new() }).collect();
    QueryTerms { terms, sx }
}

/// Casa o termo `qs` contra CADA palavra do chunk, alinhado ao INICIO (prefixo) —
/// nunca cruza fronteira de palavra. Devolve (melhor fracao casada, indices das
/// palavras com esse melhor casamento). Assim "Aslan"(as-lan) NAO casa "as lanças",
/// e "aparecimento" casa "apareceu" (raiz a-pa-re) mas nao "desapareciam" (prefixo des).
/// `q_sx` = soundex do termo, pré-calculado em prep_query.
fn best_positions(qs: &[String], q_sx: &str, words_in_chunk: &[Vec<String>], phonetic: bool) -> (f64, Vec<usize>) {
    let k = qs.len();
    if k == 0 || words_in_chunk.is_empty() { return (0.0, vec![0]); }
    let mut best = -1.0f64;
    let mut pos: Vec<usize> = vec![];
    for (wi, w) in words_in_chunk.iter().enumerate() {
        let lim = k.min(w.len());
        // prefixo CONTIGUO (raiz): para na 1a divergencia — evita casar "ressurreição"
        // com "respiração" (que coincidem só em res…ção, espalhado)
        let mut m = 0;
        while m < lim && w[m] == qs[m] { m += 1; }
        let mut frac = m as f64 / k as f64;
        // SOUNDEX: se a palavra SOA igual ao termo, casa total (Aslan ~ Aslam)
        if phonetic && !q_sx.is_empty() && soundex(&w.concat()) == q_sx {
            frac = 1.0;
        }
        if frac > best + 1e-9 { best = frac; pos = vec![wi]; }
        else if (frac - best).abs() <= 1e-9 { pos.push(wi); }
    }
    (best.max(0.0), pos)
}

fn min_span(lists: &[Vec<usize>]) -> usize {
    use std::cmp::Reverse;
    use std::collections::BinaryHeap;
    let mut heap: BinaryHeap<Reverse<(usize, usize, usize)>> = BinaryHeap::new();
    let mut cur_max = 0usize;
    for (i, lst) in lists.iter().enumerate() {
        heap.push(Reverse((lst[0], i, 0)));
        cur_max = cur_max.max(lst[0]);
    }
    let mut best = cur_max - heap.peek().unwrap().0 .0;
    loop {
        let Reverse((mn, i, j)) = heap.pop().unwrap();
        if cur_max - mn < best { best = cur_max - mn; }
        if j + 1 == lists[i].len() { return best; }
        let nxt = lists[i][j + 1];
        cur_max = cur_max.max(nxt);
        heap.push(Reverse((nxt, i, j + 1)));
    }
}

const MIN_SYL: usize = 3;   // termo "presente": casamento COMPLETO (curtos) ou >= 3 silabas (raiz)

/// Rerank por PROXIMIDADE DE TERMOS: ignora monossilabos (stopwords), exige
/// co-ocorrencia dos termos-chave no chunk e pontua pela proximidade entre eles.
/// Devolve (cobertura_dos_termos, span minimo entre os termos presentes).
/// `qt` = query já tokenizada (prep_query, 1×); `words_in_chunk` = cache do chunk.
fn rerank_score(qt: &QueryTerms, words_in_chunk: &[Vec<String>], phonetic: bool) -> (f64, usize) {
    if qt.terms.is_empty() { return (0.0, 0); }
    // indices das palavras onde cada termo esta PRESENTE (casa por fronteira de palavra)
    let mut present_lists: Vec<Vec<usize>> = vec![];
    for (ti, qs) in qt.terms.iter().enumerate() {
        let (frac, pos) = best_positions(qs, &qt.sx[ti], words_in_chunk, phonetic);
        let matched = (frac * qs.len() as f64).round() as usize;   // silabas casadas
        if frac >= 0.999 || matched >= MIN_SYL {   // termo curto: completo; longo: raiz (>=3)
            present_lists.push(pos);
        }
    }
    let coverage = present_lists.len() as f64 / qt.terms.len() as f64;
    // span agora em PALAVRAS (proximidade entre os termos presentes)
    let span = if present_lists.len() > 1 { min_span(&present_lists) } else { 0 };
    (coverage, span)
}

pub fn snippet(text: &str, query: &str) -> String {
    let width = 140usize;
    let flat: String = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let fchars: Vec<char> = flat.chars().collect();
    let fna: Vec<char> = normalize(&flat).chars().collect();
    let n = fchars.len();
    let lowq = query.to_lowercase();
    let qwords = words(&lowq);
    let mut pos: Option<usize> = None;
    for w in &qwords {
        let wn: Vec<char> = normalize(w).chars().collect();
        if !wn.is_empty() {
            if let Some(i) = find_chars(&fna, &wn, 0) { pos = Some(i); break; }
        }
    }
    let mut out: String = match pos {
        None => {
            if n > width { let mut s: String = fchars[..width].iter().collect(); s.push('…'); s }
            else { flat.clone() }
        }
        Some(p) => {
            let a = p.saturating_sub(35);
            let b = (a + width).min(n);
            let mid: String = fchars[a..b].iter().collect::<String>().trim().to_string();
            let mut s = String::new();
            if a > 0 { s.push('…'); }
            s.push_str(&mid);
            if b < n { s.push('…'); }
            s
        }
    };
    let ochars: Vec<char> = out.chars().collect();
    let ona: Vec<char> = normalize(&out).chars().collect();
    let mut marks: Vec<(usize, usize)> = vec![];
    for w in &qwords {
        let wn: Vec<char> = normalize(w).chars().collect();
        if wn.is_empty() { continue; }
        let mut start = 0;
        while let Some(i) = find_chars(&ona, &wn, start) {
            marks.push((i, i + wn.len())); start = i + wn.len();
        }
    }
    marks.sort_unstable(); marks.dedup();
    let mut res = ochars;
    for (a, b) in marks.into_iter().rev() { res.insert(b, '»'); res.insert(a, '«'); }
    out = res.into_iter().collect();
    out
}

// ------------------------------- a base RAG ----------------------------------
pub struct Chunk {
    pub id: usize, pub start: usize, pub len: usize, pub tokens: usize, pub oov: usize,
    /// [#8] arquivo de origem (modo repo). None em base de 1 arquivo → omitido no JSON (preserva equivalência).
    pub file: Option<String>,
    pub vec: HashMap<usize, f64>, pub norm: f64, pub text: Option<String>,
    /// cache: sílabas por palavra do chunk (pro rerank). Calculado 1× no load —
    /// antes era refeito (re-silabado) a CADA query. Vazio quando o chunk não tem texto.
    pub words: Vec<Vec<String>>,
}

pub struct RagBase {
    pub index: HashMap<String, usize>,
    pub idf: HashMap<usize, f64>,
    pub chunks: Vec<Chunk>,
    pub has_text: bool,
    pub n_chunks: usize,
    pub vocab_size: usize,
    pub corpus: String,
    pub generator: String,
}

pub struct Info {
    pub syls: Vec<String>, pub oov: usize, pub dims: usize, pub n_chunks: usize,
    pub n_converge: usize, pub recall_n: usize, pub rerank: bool,
    pub ms_recall: f64, pub ms_rerank: f64,
}
pub type Hit = (Option<f64>, Option<f64>, Option<usize>, f64, usize); // rr, mf, span, cos, cid

impl RagBase {
    pub fn from_str(data: &str) -> Result<RagBase, String> {
        let v: Value = serde_json::from_str(data).map_err(|e| format!("JSON inválido: {e}"))?;
        let meta = v.get("meta").ok_or("falta 'meta'")?;
        let vocab: Vec<String> = meta.get("vocab").and_then(|x| x.as_array())
            .ok_or("falta 'meta.vocab'")?
            .iter().map(|x| x.as_str().unwrap_or("").to_string()).collect();
        let index = vocab.iter().enumerate().map(|(i, t)| (t.clone(), i)).collect();
        let idf: HashMap<usize, f64> = v.get("idf").and_then(|x| x.as_object())
            .ok_or("falta 'idf'")?
            .iter().filter_map(|(k, val)| Some((k.parse().ok()?, val.as_f64()?))).collect();
        let chunks: Vec<Chunk> = v.get("chunks").and_then(|x| x.as_array())
            .ok_or("falta 'chunks'")?
            .iter().enumerate().map(|(i, c)| {
                let vec = c["vec"].as_object().map(|o| o.iter()
                    .filter_map(|(k, val)| Some((k.parse::<usize>().ok()?, val.as_f64()?)))
                    .collect()).unwrap_or_default();
                let text = c["text"].as_str().map(|s| s.to_string());
                Chunk {
                    id: i,
                    start: c["start"].as_u64().unwrap_or(0) as usize,
                    len: c["len"].as_u64().unwrap_or(0) as usize,
                    tokens: c["tokens"].as_u64().unwrap_or(0) as usize,
                    oov: c["oov"].as_u64().unwrap_or(0) as usize,
                    file: c["file"].as_str().map(|s| s.to_string()),
                    vec, norm: c["norm"].as_f64().unwrap_or(1.0),
                    text, words: Vec::new(),
                }
            }).collect();
        // modo "memory" (default): tokeniza os chunks UMA vez no load (rápido, +RAM).
        // modo "hybrid": NÃO cacheia (libera RAM); o rerank recomputa só os candidatos.
        let mut chunks = chunks;
        if CACHE_WORDS.load(std::sync::atomic::Ordering::Relaxed) {
            chunks.par_iter_mut().for_each(|c| {
                if let Some(t) = &c.text { c.words = chunk_words(t); }
            });
        }
        Ok(RagBase {
            index, idf,
            n_chunks: meta["n_chunks"].as_u64().unwrap_or(chunks.len() as u64) as usize,
            vocab_size: meta["vocab_size"].as_u64().unwrap_or(vocab.len() as u64) as usize,
            corpus: meta["corpus"].as_str().unwrap_or("?").to_string(),
            generator: meta["generator"].as_str().unwrap_or("?").to_string(),
            has_text: meta["with_text"].as_bool().unwrap_or(false),
            chunks,
        })
    }

    pub fn load(path: &str) -> Result<RagBase, String> {
        let data = std::fs::read_to_string(path).map_err(|e| format!("erro lendo {path:?}: {e}"))?;
        RagBase::from_str(&data)
    }

    fn query_vec(&self, query: &str) -> (HashMap<usize, f64>, f64, Vec<String>, usize) {
        let lower = query.to_lowercase();
        let mut tf: HashMap<usize, u32> = HashMap::new();
        let mut syls = vec![];
        let mut oov = 0;
        for w in words(&lower) {
            for s in syllabify(&w) {
                let ns = normalize(&s);
                if ns.is_empty() { continue; }
                syls.push(ns.clone());
                match self.index.get(&ns) {
                    Some(&d) => *tf.entry(d).or_insert(0) += 1,
                    None => oov += 1,
                }
            }
        }
        let mut qvec: HashMap<usize, f64> = HashMap::new();
        for (d, c) in &tf {
            let w = *c as f64 * self.idf.get(d).copied().unwrap_or(0.0);
            if w != 0.0 { qvec.insert(*d, w); }
        }
        let s: f64 = qvec.values().map(|v| v * v).sum();
        let qnorm = if s == 0.0 { 1.0 } else { s.sqrt() };
        (qvec, qnorm, syls, oov)
    }

    pub fn search(&self, query: &str, k: usize, rerank: bool, recall_n: usize, phonetic: bool) -> (Vec<Hit>, Info) {
        let (qvec, qnorm, syls, oov) = self.query_vec(query);
        let mut info = Info { syls, oov, dims: qvec.len(), n_chunks: self.chunks.len(),
            n_converge: 0, recall_n: 0, rerank: rerank && self.has_text, ms_recall: 0.0, ms_rerank: 0.0 };
        if qvec.is_empty() { return (vec![], info); }
        let t0 = std::time::Instant::now();
        // recall (estágio 1): cosseno em todos os chunks. Paraleliza com rayon só em
        // base grande — em base pequena o overhead de fan-out não compensa.
        let score_one = |cid: usize, c: &Chunk| -> Option<(f64, usize)> {
            let s = cosine(&qvec, qnorm, &c.vec, c.norm);
            if s > 0.0 { Some((s, cid)) } else { None }
        };
        let mut scored: Vec<(f64, usize)> = if self.chunks.len() >= PAR_RECALL_MIN {
            self.chunks.par_iter().enumerate()
                .filter_map(|(cid, c)| score_one(cid, c)).collect()
        } else {
            self.chunks.iter().enumerate()
                .filter_map(|(cid, c)| score_one(cid, c)).collect()
        };
        info.n_converge = scored.len();
        scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap().then(b.1.cmp(&a.1)));
        let rn = if info.rerank { k.max(recall_n) } else { k };
        let cand: Vec<(f64, usize)> = scored.into_iter().take(rn).collect();
        info.recall_n = cand.len();
        info.ms_recall = t0.elapsed().as_secs_f64() * 1000.0;
        let hits: Vec<Hit> = if info.rerank {
            let t1 = std::time::Instant::now();
            let qt = prep_query(query);   // tokeniza a query 1× (hoist), não por candidato
            let mut res: Vec<Hit> = cand.iter().map(|&(cos, cid)| {
                let ch = &self.chunks[cid];
                // memory: usa o cache; hybrid: recomputa só este candidato a partir do texto
                let recomputed;
                let words: &[Vec<String>] = if !ch.words.is_empty() {
                    &ch.words
                } else if let Some(t) = &ch.text {
                    recomputed = chunk_words(t); &recomputed
                } else { &[] };
                let (coverage, span) = rerank_score(&qt, words, phonetic);
                (Some(coverage), Some(coverage), Some(span), cos, cid)
            }).collect();
            // COBERTURA (quantos termos co-ocorrem) domina; span (proximidade) e cos só desempatam
            res.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap()
                .then(a.2.unwrap().cmp(&b.2.unwrap()))
                .then(b.3.partial_cmp(&a.3).unwrap()));
            info.ms_rerank = t1.elapsed().as_secs_f64() * 1000.0;
            res.into_iter().take(k).collect()
        } else {
            cand.into_iter().take(k).map(|(cos, cid)| (None, None, None, cos, cid)).collect()
        };
        (hits, info)
    }

    /// Dados pros gráficos (estilo logic_path/matched_filter.png):
    /// - painel de baixo: query (sílaba→dim→contagem, com flag `hit`=converge no cosseno) +
    ///   embedding do chunk `cid` (dim→contagem)
    /// - painel de cima: matched filter (cada palavra da query deslizando sobre a sequência
    ///   de sílabas do chunk; `peak`=ponto de convergência)
    pub fn hist_data(&self, query: &str, cid: usize) -> Value {
        // dim → sílaba (inverte o index)
        let mut dim2syl: HashMap<usize, &str> = HashMap::with_capacity(self.index.len());
        for (s, &d) in &self.index { dim2syl.insert(d, s.as_str()); }
        // dimensões presentes no chunk (= as que podem convergir no cosseno)
        let chunk_dims: std::collections::HashSet<usize> = self.chunks.get(cid)
            .map(|c| c.vec.keys().copied().collect()).unwrap_or_default();
        // histograma da query (contagem por dimensão) + flag de convergência
        let lower = query.to_lowercase();
        let mut qc: HashMap<usize, u32> = HashMap::new();
        let mut oov = 0u32;
        for w in words(&lower) {
            for s in syllabify(&w) {
                let ns = normalize(&s);
                if ns.is_empty() { continue; }
                match self.index.get(&ns) { Some(&d) => *qc.entry(d).or_insert(0) += 1, None => oov += 1 }
            }
        }
        let q: Vec<Value> = qc.iter().map(|(d, c)| json!({
            "dim": d, "syl": dim2syl.get(d).copied().unwrap_or(""), "c": c,
            "hit": chunk_dims.contains(d),   // dim também no chunk → contribui pro cosseno
        })).collect();
        let chunk: Vec<Value> = self.chunks.get(cid)
            .map(|ch| ch.vec.iter().map(|(d, v)| json!({"dim": d, "c": *v})).collect())
            .unwrap_or_default();

        // matched filter: query deslizando sobre a sequência de sílabas do chunk
        // (memory: usa o cache `words`; hybrid: recomputa do texto do chunk)
        let recomputed;
        let words_ref: &[Vec<String>] = match self.chunks.get(cid) {
            Some(c) if !c.words.is_empty() => &c.words,
            Some(c) => match &c.text { Some(t) => { recomputed = chunk_words(t); &recomputed } None => &[] },
            None => &[],
        };
        let seq: Vec<&str> = words_ref.iter().flatten().map(|s| s.as_str()).collect();
        let n = seq.len();
        let mut mf: Vec<Value> = vec![];
        for w in words(&lower) {
            let qs: Vec<String> = syllabify(&w).iter().map(|s| normalize(s))
                .filter(|s| !s.is_empty()).collect();
            let k = qs.len();
            if k == 0 || n < k { continue; }
            let mut points: Vec<Value> = vec![];
            let (mut peak_pos, mut peak) = (0usize, 0.0f64);
            for p in 0..=(n - k) {
                let m = (0..k).filter(|&j| seq[p + j] == qs[j]).count();
                if m > 0 {
                    let frac = m as f64 / k as f64;
                    points.push(json!([p, frac]));
                    if frac > peak { peak = frac; peak_pos = p; }
                }
            }
            mf.push(json!({"term": qs.join("-"), "k": k, "peak_pos": peak_pos, "peak": peak, "points": points}));
        }

        json!({"vocab_size": self.index.len(), "query": q, "query_oov": oov,
               "chunk": chunk, "seq_len": n, "mf": mf})
    }
}
