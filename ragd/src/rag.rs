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
fn rerank_score(qt: &QueryTerms, weights: &[f64], words_in_chunk: &[Vec<String>], phonetic: bool) -> (f64, usize) {
    if qt.terms.is_empty() { return (0.0, 0); }
    // indices das palavras onde cada termo esta PRESENTE (casa por fronteira de palavra)
    let mut present_lists: Vec<Vec<usize>> = vec![];
    let mut present_w = 0.0;   // soma dos pesos (idf) dos termos presentes
    for (ti, qs) in qt.terms.iter().enumerate() {
        let (frac, pos) = best_positions(qs, &qt.sx[ti], words_in_chunk, phonetic);
        let matched = (frac * qs.len() as f64).round() as usize;   // silabas casadas
        if frac >= 0.999 || matched >= MIN_SYL {   // termo curto: completo; longo: raiz (>=3)
            present_lists.push(pos);
            present_w += weights[ti];
        }
    }
    // COBERTURA = fracao da MASSA DE IDF da query que o chunk casa. `weights` vem da escala da
    // COLECAO (uidf) quando ha perfil: assim um termo presente na colecao mas ausente NESTA base
    // mantem seu peso no denominador (nao some -> nao crava 1.0 falso) e a escala fica consistente
    // entre bases. Fallback p/ contagem crua se nao ha idf nenhum.
    let total_w: f64 = weights.iter().sum();
    let coverage = if total_w > 0.0 {
        present_w / total_w
    } else {
        present_lists.len() as f64 / qt.terms.len() as f64
    };
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
    /// Timestamp (seg desde epoch) da última ingestão desta base. Usado pelo merge cross-base
    /// pra dar leve boost de recência (sessão nova não perde por empate p/ sessão antiga).
    /// 0 = desconhecido (sem boost; comportamento legado).
    pub mtime: u64,
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
            mtime: 0,   // sem contexto de arquivo aqui; caller (load/ingest) seta depois.
        })
    }

    pub fn load(path: &str) -> Result<RagBase, String> {
        let data = std::fs::read_to_string(path).map_err(|e| format!("erro lendo {path:?}: {e}"))?;
        let mut b = RagBase::from_str(&data)?;
        b.mtime = std::fs::metadata(path).ok()
            .and_then(|m| m.modified().ok())
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs()).unwrap_or(0);
        Ok(b)
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

    pub fn search(&self, query: &str, k: usize, rerank: bool, recall_n: usize, phonetic: bool, weights: Option<&[f64]>) -> (Vec<Hit>, Info) {
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
        let qt = prep_query(query);
        let hits = self.finish(&qt, weights, cand, k, phonetic, &mut info);
        (hits, info)
    }

    /// Estágio 2 compartilhado: rerank (cobertura → proximidade → cos) ou top-k puro.
    /// Usado pelo recall local (`search`) e pelo unificado (`search_unified`) — sem duplicação.
    /// `weights` = peso por termo na escala da COLEÇÃO quando o caller tem perfil (`Some`);
    /// `None` cai no peso LOCAL da base ([term_weights]) — mesma fórmula, fonte de idf diferente.
    fn finish(&self, qt: &QueryTerms, weights: Option<&[f64]>, cand: Vec<(f64, usize)>, k: usize, phonetic: bool, info: &mut Info) -> Vec<Hit> {
        if info.rerank {
            let t1 = std::time::Instant::now();
            // peso por termo: unificado (do caller) ou local (fallback). Hoist 1×, não por candidato.
            let owned = if weights.is_none() { Some(self.term_weights(qt)) } else { None };
            let weights: &[f64] = weights.unwrap_or_else(|| owned.as_ref().unwrap());
            let mut res: Vec<Hit> = cand.iter().map(|&(cos, cid)| {
                let ch = &self.chunks[cid];
                // memory: usa o cache; hybrid: recomputa só este candidato a partir do texto
                let recomputed;
                let words: &[Vec<String>] = if !ch.words.is_empty() {
                    &ch.words
                } else if let Some(t) = &ch.text {
                    recomputed = chunk_words(t); &recomputed
                } else { &[] };
                let (coverage, span) = rerank_score(qt, weights, words, phonetic);
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
        }
    }

    /// Cobertura/span de UMA query (já tokenizada) contra UM chunk pelo id (== índice).
    /// Usado pelo expand pra rerankar o merge contra a INTENÇÃO ORIGINAL — não contra a
    /// cobertura trivial (sempre 1.0) de uma variante de 1 termo. Devolve (0.0, 0) se o
    /// chunk não existe ou não tem texto pra casar.
    pub fn score_chunk(&self, qt: &QueryTerms, weights: Option<&[f64]>, chunk_id: usize, phonetic: bool) -> (f64, usize) {
        let ch = match self.chunks.get(chunk_id) { Some(c) => c, None => return (0.0, 0) };
        let recomputed;
        let words: &[Vec<String>] = if !ch.words.is_empty() {
            &ch.words
        } else if let Some(t) = &ch.text {
            recomputed = chunk_words(t); &recomputed
        } else { &[] };
        let owned = if weights.is_none() { Some(self.term_weights(qt)) } else { None };
        let weights: &[f64] = weights.unwrap_or_else(|| owned.as_ref().unwrap());
        rerank_score(qt, weights, words, phonetic)
    }

    /// Peso de cada termo da query = soma dos idf das suas sílabas presentes no vocab LOCAL.
    /// É o que torna a cobertura PONDERADA: termo raro (Elrond) pesa muito, termo comum
    /// (do/conselho) ou variante-função (to/for) quase nada. Sílaba OOV não soma. Usado como
    /// fallback quando não há perfil de coleção; com perfil, o peso vem de [weighting_unified].
    fn term_weights(&self, qt: &QueryTerms) -> Vec<f64> {
        qt.terms.iter().map(|syls| {
            syls.iter()
                .filter_map(|s| self.index.get(s))
                .map(|d| self.idf.get(d).copied().unwrap_or(0.0))
                .sum()
        }).collect()
    }

    /// Igual ao `search`, mas o RECALL roda no espaço UNIFICADO da coleção: a query já vem
    /// vetorizada (qvec/qnorm globais) e o `vec` de cada chunk é remapeado via `remap`+`unorms`.
    /// O rerank (estágio 2) é idêntico. Caller usa `search` (local) quando não há perfil.
    #[allow(clippy::too_many_arguments)]
    pub fn search_unified(&self, query: &str, k: usize, rerank: bool, recall_n: usize, phonetic: bool,
                          qvec: &HashMap<usize, f64>, qnorm: f64, remap: &[usize], unorms: &[f64],
                          weights: Option<&[f64]>) -> (Vec<Hit>, Info) {
        let mut info = Info { syls: vec![], oov: 0, dims: qvec.len(), n_chunks: self.chunks.len(),
            n_converge: 0, recall_n: 0, rerank: rerank && self.has_text, ms_recall: 0.0, ms_rerank: 0.0 };
        if qvec.is_empty() { return (vec![], info); }
        let t0 = std::time::Instant::now();
        let score_one = |cid: usize, c: &Chunk| -> Option<(f64, usize)> {
            let un = unorms.get(cid).copied().unwrap_or(0.0);
            let s = cosine_unified(qvec, qnorm, c, remap, un);
            if s > 0.0 { Some((s, cid)) } else { None }
        };
        let mut scored: Vec<(f64, usize)> = if self.chunks.len() >= PAR_RECALL_MIN {
            self.chunks.par_iter().enumerate().filter_map(|(cid, c)| score_one(cid, c)).collect()
        } else {
            self.chunks.iter().enumerate().filter_map(|(cid, c)| score_one(cid, c)).collect()
        };
        info.n_converge = scored.len();
        scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap().then(b.1.cmp(&a.1)));
        let rn = if info.rerank { k.max(recall_n) } else { k };
        let cand: Vec<(f64, usize)> = scored.into_iter().take(rn).collect();
        info.recall_n = cand.len();
        info.ms_recall = t0.elapsed().as_secs_f64() * 1000.0;
        let qt = prep_query(query);
        let hits = self.finish(&qt, weights, cand, k, phonetic, &mut info);
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

// --------------------- [#8] perfil unificado por coleção ---------------------
/// Junta os vocabs dos drivers de TODAS as bases de uma coleção num espaço de
/// dimensões GLOBAL e recomputa o idf sobre todos os chunks da coleção (o "idf de
/// repo"). Construído em memória; os JSONs no disco não mudam. O `remap` traduz a
/// dim local de cada base → dim global, pro cosseno remapear on-the-fly na busca.
pub struct CollectionProfile {
    pub uvocab: HashMap<String, usize>,      // sílaba → dim global
    pub uidf: HashMap<usize, f64>,           // dim global → idf unificado (coleção)
    pub remap: HashMap<String, Vec<usize>>,  // base_name → (dim local → dim global)
    pub unorms: HashMap<String, Vec<f64>>,   // base_name → norma tf-idf unificada por chunk
    pub fingerprint: (usize, usize),         // (nº bases, total chunks) — auto-invalida o cache
}

/// Fingerprint barato da coleção pra auto-invalidar o cache do perfil sem rastrear mutação:
/// (nº de bases, total de chunks). Muda quando uma base entra/sai/é re-ingerida com tamanho diferente.
/// Segundos desde epoch (UTC). Usado pra setar `RagBase.mtime` em ingestão nova e pra
/// computar idade no boost de recência do merge cross-base. 0 em falha (sem boost).
pub fn now_secs() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs()).unwrap_or(0)
}

pub fn collection_fingerprint(bases: &HashMap<String, RagBase>) -> (usize, usize) {
    (bases.len(), bases.values().map(|b| b.chunks.len()).sum())
}

/// Constrói o perfil unificado das bases de uma coleção. Determinístico: ordena bases
/// por nome e sílabas por dim local — a mesma coleção gera sempre o mesmo perfil.
pub fn build_collection_profile(bases: &HashMap<String, RagBase>) -> CollectionProfile {
    let mut uvocab: HashMap<String, usize> = HashMap::new();
    let mut remap: HashMap<String, Vec<usize>> = HashMap::new();
    let mut names: Vec<&String> = bases.keys().collect();
    names.sort();
    for name in &names {
        let base = &bases[*name];
        let mut m = vec![0usize; base.index.len()];
        let mut pairs: Vec<(&String, usize)> = base.index.iter().map(|(s, &d)| (s, d)).collect();
        pairs.sort_by_key(|(_, d)| *d);
        for (syl, ld) in pairs {
            let next = uvocab.len();
            let gd = *uvocab.entry(syl.clone()).or_insert(next);
            if ld < m.len() { m[ld] = gd; }
        }
        remap.insert((*name).clone(), m);
    }
    // tfs remapeados por base (pro idf de coleção e pras normas unificadas)
    let mut flat: Vec<HashMap<usize, u32>> = Vec::new();
    let mut per_base: Vec<(String, Vec<HashMap<usize, u32>>)> = Vec::new();
    for name in &names {
        let base = &bases[*name];
        let m = &remap[*name];
        let mut bt: Vec<HashMap<usize, u32>> = Vec::with_capacity(base.chunks.len());
        for ch in &base.chunks {
            let mut tf: HashMap<usize, u32> = HashMap::with_capacity(ch.vec.len());
            for (&ld, &cnt) in &ch.vec {
                if ld < m.len() { tf.insert(m[ld], cnt as u32); }
            }
            flat.push(tf.clone());
            bt.push(tf);
        }
        per_base.push(((*name).clone(), bt));
    }
    let uidf = crate::vector::compute_idf(&flat, flat.len());
    // norma unificada (tf-idf no espaço global) por chunk — denominador do cosseno
    let mut unorms: HashMap<String, Vec<f64>> = HashMap::new();
    for (name, bt) in &per_base {
        let norms = bt.iter().map(|tf| crate::vector::tfidf_norm(tf, &uidf)).collect();
        unorms.insert(name.clone(), norms);
    }
    let fingerprint = collection_fingerprint(bases);
    CollectionProfile { uvocab, uidf, remap, unorms, fingerprint }
}

/// Vetoriza a query no espaço unificado da coleção (mesmo esquema do query_vec: tf*idf).
pub fn query_vec_unified(query: &str, p: &CollectionProfile) -> (HashMap<usize, f64>, f64) {
    let lower = query.to_lowercase();
    let mut tf: HashMap<usize, u32> = HashMap::new();
    for w in words(&lower) {
        for s in syllabify(&w) {
            let ns = normalize(&s);
            if ns.is_empty() { continue; }
            if let Some(&gd) = p.uvocab.get(&ns) { *tf.entry(gd).or_insert(0) += 1; }
        }
    }
    let mut qvec: HashMap<usize, f64> = HashMap::new();
    let mut sum = 0.0;
    for (&gd, &c) in &tf {
        let w = c as f64 * p.uidf.get(&gd).copied().unwrap_or(0.0);
        if w != 0.0 { qvec.insert(gd, w); sum += w * w; }
    }
    let qnorm = sum.sqrt();
    (qvec, if qnorm == 0.0 { 1.0 } else { qnorm })
}

/// Peso por termo na escala da COLEÇÃO (uidf) — mesma fórmula do `term_weights` local, só que
/// a fonte de idf é unificada. É a correção estrutural do #5: como o uidf conhece todos os
/// termos da coleção, um termo presente na coleção mas ausente NUMA base específica mantém seu
/// peso no denominador do rerank (não some → não crava cobertura 1.0 falsa) e a escala fica
/// consistente entre bases (acaba o resíduo cross-base). Sílaba inédita na coleção não soma.
pub fn weighting_unified(qt: &QueryTerms, p: &CollectionProfile) -> Vec<f64> {
    qt.terms.iter().map(|syls| {
        syls.iter()
            .filter_map(|s| p.uvocab.get(s))
            .map(|gd| p.uidf.get(gd).copied().unwrap_or(0.0))
            .sum()
    }).collect()
}

/// Cosseno de um chunk (vec em dims LOCAIS) contra a query (espaço GLOBAL), remapeando
/// on-the-fly via `remap` + idf unificado. Replica o esquema do `cosine` atual: dot da
/// query tf-idf com o tf cru do chunk, sobre qnorm × norma tf-idf unificada do chunk.
pub fn cosine_unified(qvec: &HashMap<usize, f64>, qnorm: f64, chunk: &Chunk, remap: &[usize], unorm: f64) -> f64 {
    if unorm == 0.0 { return 0.0; }
    let mut dot = 0.0;
    for (&ld, &cnt) in &chunk.vec {
        if ld >= remap.len() { continue; }
        if let Some(&wq) = qvec.get(&remap[ld]) { dot += wq * cnt; }
    }
    dot / (qnorm * unorm)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn mk_base(vocab: &[&str], chunks: &[&[usize]]) -> RagBase {
        let index: HashMap<String, usize> =
            vocab.iter().enumerate().map(|(i, s)| (s.to_string(), i)).collect();
        let chunks: Vec<Chunk> = chunks.iter().enumerate().map(|(i, dims)| Chunk {
            id: i, start: 0, len: 0, tokens: 0, oov: 0,
            vec: dims.iter().map(|&d| (d, 1.0)).collect(),
            norm: 1.0, text: None, words: Vec::new(),
        }).collect();
        let n = chunks.len();
        RagBase { index, idf: HashMap::new(), chunks, has_text: false,
                  n_chunks: n, vocab_size: vocab.len(), corpus: "t".into(), generator: "t".into(),
                  mtime: 0 }
    }
    #[test]
    fn unifies_vocabs_across_different_drivers() {
        // base "a" (driver 1): vocab [fro, do]; base "b" (driver 2): vocab [do, ga].
        // "do" tem dim LOCAL diferente em cada (1 em a, 0 em b) — o furo poliglota.
        let mut bases = HashMap::new();
        bases.insert("a".to_string(), mk_base(&["fro", "do"], &[&[0, 1]]));
        bases.insert("b".to_string(), mk_base(&["do", "ga"], &[&[0, 1]]));
        let p = build_collection_profile(&bases);
        assert_eq!(p.uvocab.len(), 3); // união: fro, do, ga
        let g_do = p.uvocab["do"];
        assert_eq!(p.remap["a"][1], g_do); // "do" local 1 em "a" → mesmo dim global
        assert_eq!(p.remap["b"][0], g_do); // "do" local 0 em "b" → mesmo dim global
        // idf de coleção: "do" em 2 chunks de 2 → ln((2+1)/2); "fro" em 1 de 2 → ln(3/1)
        assert!((p.uidf[&g_do] - (3.0_f64 / 2.0).ln()).abs() < 1e-9);
        assert!((p.uidf[&p.uvocab["fro"]] - (3.0_f64 / 1.0).ln()).abs() < 1e-9);
    }

    #[test]
    fn unified_cosine_remaps_local_dims_across_bases() {
        let mut bases = HashMap::new();
        // "a": vocab[fro,do], chunk0=[fro,do], chunk1=[fro]; "b": vocab[do,ga], chunk0=[do,ga]
        bases.insert("a".to_string(), mk_base(&["fro", "do"], &[&[0, 1], &[0]]));
        bases.insert("b".to_string(), mk_base(&["do", "ga"], &[&[0, 1]]));
        let p = build_collection_profile(&bases);
        let g_do = p.uvocab["do"];
        // query (espaço global) = só "do"
        let qw = p.uidf[&g_do];
        let mut qvec = HashMap::new();
        qvec.insert(g_do, qw);
        let qnorm = qw.abs().max(1e-12);
        // chunk de "b" tem "do" no dim LOCAL 0 → remapeado casa a query global
        let s_b = cosine_unified(&qvec, qnorm, &bases["b"].chunks[0], &p.remap["b"], p.unorms["b"][0]);
        // chunk1 de "a" (só "fro") não tem "do" → 0
        let s_a1 = cosine_unified(&qvec, qnorm, &bases["a"].chunks[1], &p.remap["a"], p.unorms["a"][1]);
        assert!(s_b > 0.0, "chunk de outra base/driver deve casar via remap");
        assert_eq!(s_a1, 0.0, "chunk sem o termo não casa");
    }

    #[test]
    fn search_unified_finds_cross_driver_chunk() {
        let mut bases = HashMap::new();
        bases.insert("a".to_string(), mk_base(&["fro", "do"], &[&[0, 1], &[0]]));
        bases.insert("b".to_string(), mk_base(&["do", "ga"], &[&[0, 1]]));
        let p = build_collection_profile(&bases);
        let g_do = p.uvocab["do"];
        let qw = p.uidf[&g_do];
        let mut qvec = HashMap::new();
        qvec.insert(g_do, qw);
        let qnorm = qw.abs().max(1e-12);
        // base "b": chunk0 tem "do" (dim local 0) → casa via remap
        let (hb, _) = bases["b"].search_unified("", 5, false, 20, false, &qvec, qnorm, &p.remap["b"], &p.unorms["b"], None);
        assert_eq!(hb.len(), 1);
        // base "a": só chunk0 (fro,do) casa; chunk1 (só fro) não
        let (ha, _) = bases["a"].search_unified("", 5, false, 20, false, &qvec, qnorm, &p.remap["a"], &p.unorms["a"], None);
        assert_eq!(ha.len(), 1);
        assert_eq!(ha[0].4, 0); // cid do chunk com "do"
    }
}
