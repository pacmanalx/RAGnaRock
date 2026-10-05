//! ragd::rag — motor de busca (RagBase + recall cosseno + rerank matched filter).
//! Cópia EVOLUÍVEL do search_rag da PoC: aqui pode mudar livre (rust_concept congela).
use std::collections::HashMap;
use serde_json::{json, Value};
use rayon::prelude::*;
use crate::tokenizer::{normalize, syllabify, words};
use crate::vector::{cosine_tfidf, SparseVec};
use crate::chunk::find_chars;

const PROX_SCALE: f64 = 8.0;
/// Recall paraleliza (rayon) só a partir deste nº de chunks; abaixo roda sequencial.
const PAR_RECALL_MIN: usize = 512;
/// Modo de armazenamento: true = cacheia `words` no load (rápido, +RAM); false = híbrido
/// (não cacheia; rerank recomputa só os candidatos). Setado no boot a partir do config.
pub static CACHE_WORDS: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);
/// [#41] modo "disk": o TEXTO dos chunks sai da RAM. Na carga, cada base grava o texto num
/// `<base>-tokenized.textblob` ao lado do JSON e o lê por mmap (page-cache do SO). O recall
/// (estágio 1) nunca toca texto — só vetores/idf/índice —, então o caminho quente não muda;
/// texto só é lido no rerank dos candidatos, no snippet, no `/chunk` e no literal-fallback.
/// Implica CACHE_WORDS=false (como o hybrid). O JSON segue sendo a fonte da verdade: o
/// .textblob é regerado a cada carga e nunca é lido sem o JSON correspondente.
pub static TEXT_ON_DISK: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Nome do modo vigente — "memory" | "hybrid" | "disk" — para /config, painel e logs.
pub fn storage_mode() -> &'static str {
    use std::sync::atomic::Ordering::Relaxed;
    if TEXT_ON_DISK.load(Relaxed) { "disk" } else if CACHE_WORDS.load(Relaxed) { "memory" } else { "hybrid" }
}

/// Liga o modo pelo nome. Devolve false (e não mexe em nada) se o nome não existe.
pub fn set_storage_mode(mode: &str) -> bool {
    use std::sync::atomic::Ordering::Relaxed;
    let (cache, disk) = match mode.to_lowercase().as_str() {
        "memory" => (true, false), "hybrid" => (false, false), "disk" => (false, true), _ => return false,
    };
    CACHE_WORDS.store(cache, Relaxed);
    TEXT_ON_DISK.store(disk, Relaxed);
    true
}

/// `<x>-tokenized.json` -> `<x>-tokenized.textblob` (mesma pasta). Não termina em
/// `-tokenized.json`, então o autoload nunca o confunde com uma base.
pub fn blob_path_for(json_path: &str) -> String {
    format!("{}.textblob", json_path.strip_suffix(".json").unwrap_or(json_path))
}

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
    /// [#42] pares (dim, contagem) ORDENADOS por dim — ver `vector::SparseVec`.
    pub vec: SparseVec, pub norm: f64, pub text: Option<String>,
    /// [#41] no modo disk: (offset, tamanho em bytes) do texto dentro do `.textblob` da base;
    /// `text` fica None. Ler sempre por `RagBase::chunk_text`, nunca por `text` direto.
    pub tref: Option<(u64, u32)>,
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
    /// [#41] texto dos chunks mapeado do `.textblob` (modo disk). None nos modos memory/hybrid.
    pub blob: Option<std::sync::Arc<memmap2::Mmap>>,
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
                // [#42] coleta DIRETO no Vec (nada de HashMap intermediário: o pico de RAM
                // do load é o que mais dói com 335 bases) + capacidade exata + ordenação,
                // que é o que o `cosine_tfidf` exige pra buscar binário.
                let vec: SparseVec = c["vec"].as_object().map(|o| {
                    let mut v: SparseVec = Vec::with_capacity(o.len());
                    for (k, val) in o {
                        if let (Ok(d), Some(c)) = (k.parse::<u32>(), val.as_f64()) {
                            v.push((d, c as f32));
                        }
                    }
                    v.sort_unstable_by_key(|&(d, _)| d);
                    v.shrink_to_fit();
                    v
                }).unwrap_or_default();
                let text = c["text"].as_str().map(|s| s.to_string());
                Chunk {
                    id: i,
                    start: c["start"].as_u64().unwrap_or(0) as usize,
                    len: c["len"].as_u64().unwrap_or(0) as usize,
                    tokens: c["tokens"].as_u64().unwrap_or(0) as usize,
                    oov: c["oov"].as_u64().unwrap_or(0) as usize,
                    vec, norm: c["norm"].as_f64().unwrap_or(1.0),
                    text, tref: None, words: Vec::new(),
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
            blob: None,
        })
    }

    /// [#41] O texto do chunk, venha da RAM (memory/hybrid) ou do `.textblob` (disk).
    /// Ponto ÚNICO de leitura de texto — nenhum call site lê `Chunk.text` direto.
    pub fn chunk_text<'a>(&'a self, ch: &'a Chunk) -> Option<&'a str> {
        if let Some(t) = &ch.text { return Some(t.as_str()); }
        let (off, len) = ch.tref?;
        let b = self.blob.as_ref()?;
        let (a, z) = (off as usize, off as usize + len as usize);
        if z > b.len() { return None; }
        std::str::from_utf8(&b[a..z]).ok()
    }

    /// [#41] Tira o texto da RAM: grava todos os textos num `.textblob` (tmp + rename,
    /// atômico), mapeia por mmap e troca cada `text` por `tref`. Base sem texto: no-op.
    /// Erro de I/O devolve Err e a base fica INTACTA na RAM (o caller só avisa).
    pub fn spill_text(&mut self, blob_path: &str) -> Result<(), String> {
        use std::io::Write;
        if !self.chunks.iter().any(|c| c.text.is_some()) { return Ok(()); }
        let tmp = format!("{blob_path}.tmp");
        let mut refs: Vec<Option<(u64, u32)>> = Vec::with_capacity(self.chunks.len());
        {
            let f = std::fs::File::create(&tmp).map_err(|e| format!("criar {tmp:?}: {e}"))?;
            let mut w = std::io::BufWriter::new(f);
            let mut off: u64 = 0;
            for c in &self.chunks {
                match &c.text {
                    Some(t) => {
                        let len = u32::try_from(t.len()).map_err(|_| format!("chunk {} > 4 GB", c.id))?;
                        w.write_all(t.as_bytes()).map_err(|e| format!("gravar {tmp:?}: {e}"))?;
                        refs.push(Some((off, len))); off += len as u64;
                    }
                    None => refs.push(None),
                }
            }
            w.flush().map_err(|e| format!("gravar {tmp:?}: {e}"))?;
        }
        std::fs::rename(&tmp, blob_path).map_err(|e| format!("renomear para {blob_path:?}: {e}"))?;
        let f = std::fs::File::open(blob_path).map_err(|e| format!("abrir {blob_path:?}: {e}"))?;
        // SAFETY: o arquivo é nosso, recém-escrito e trocado por rename atômico; ninguém mais o
        // altera enquanto o daemon roda. Se a base for re-ingerida, um NOVO arquivo substitui
        // este por rename — o mapa antigo segue válido (inode antigo) até a base velha cair.
        let map = unsafe { memmap2::Mmap::map(&f) }.map_err(|e| format!("mmap {blob_path:?}: {e}"))?;
        self.blob = Some(std::sync::Arc::new(map));
        for (c, r) in self.chunks.iter_mut().zip(refs) {
            if r.is_some() { c.tref = r; c.text = None; c.words = Vec::new(); }
        }
        Ok(())
    }

    pub fn load(path: &str) -> Result<RagBase, String> {
        let data = std::fs::read_to_string(path).map_err(|e| format!("erro lendo {path:?}: {e}"))?;
        let mut b = RagBase::from_str(&data)?;
        drop(data);   // [#41] o JSON cru (que inclui o texto) não fica vivo durante o spill
        b.spill_if_disk(&blob_path_for(path));
        b.mtime = std::fs::metadata(path).ok()
            .and_then(|m| m.modified().ok())
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs()).unwrap_or(0);
        Ok(b)
    }

    /// [#41] No modo disk, manda o texto para o `.textblob`; nos outros modos, nada. Falha de
    /// I/O NÃO derruba a base: ela segue com o texto na RAM e o aviso vai pro stderr.
    pub fn spill_if_disk(&mut self, blob_path: &str) {
        if !TEXT_ON_DISK.load(std::sync::atomic::Ordering::Relaxed) || !self.has_text { return; }
        if let Err(e) = self.spill_text(blob_path) {
            eprintln!("storage disk: {e} — base segue com o texto na RAM");
        }
    }

    fn query_vec(&self, query: &str) -> (Vec<(usize, f64)>, f64, Vec<String>, usize) {
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
        // O chunk guarda tf CRU e norma tf-idf. Pra fechar o cosseno de verdade, a query sai
        // daqui com o idf DOBRADO (tf_q·idf²): `Σ (tf_q·idf)(tf_c·idf) = Σ (tf_q·idf²)·tf_c`.
        // A NORMA continua sendo a do vetor tf-idf honesto (‖tf_q·idf‖) — ver `cosine_tfidf`.
        // [#56] em ordem de dim: norma e dot somam sempre na mesma ordem (determinístico).
        let mut dims: Vec<(usize, u32)> = tf.into_iter().collect();
        dims.sort_unstable_by_key(|&(d, _)| d);
        let mut qw2: Vec<(usize, f64)> = Vec::with_capacity(dims.len());
        let mut s = 0.0;
        for (d, c) in dims {
            let idf = self.idf.get(&d).copied().unwrap_or(0.0);
            let w = c as f64 * idf;
            if w != 0.0 { qw2.push((d, w * idf)); s += w * w; }
        }
        let qnorm = if s == 0.0 { 1.0 } else { s.sqrt() };
        (qw2, qnorm, syls, oov)
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
            let s = cosine_tfidf(&qvec, qnorm, &c.vec, c.norm);
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
                } else if let Some(t) = self.chunk_text(ch) {
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
        } else if let Some(t) = self.chunk_text(ch) {
            recomputed = chunk_words(t); &recomputed
        } else { &[] };
        let owned = if weights.is_none() { Some(self.term_weights(qt)) } else { None };
        let weights: &[f64] = weights.unwrap_or_else(|| owned.as_ref().unwrap());
        rerank_score(qt, weights, words, phonetic)
    }

    /// [#44] Cobertura/span de UMA query contra uma PASSAGEM: trechos consecutivos lidos como um
    /// texto só. É o que fecha a co-ocorrência que cruza a fronteira do chunk ("Frodo" no fim de
    /// um, o nome do lugar no começo do seguinte). Mesma régua do rerank (`rerank_score`).
    pub fn score_passage(&self, qt: &QueryTerms, weights: Option<&[f64]>, chunk_ids: &[usize], phonetic: bool) -> (f64, usize) {
        let mut words: Vec<Vec<String>> = vec![];
        for &cid in chunk_ids {
            let ch = match self.chunks.get(cid) { Some(c) => c, None => continue };
            if !ch.words.is_empty() { words.extend(ch.words.iter().cloned()); }
            else if let Some(t) = self.chunk_text(ch) { words.extend(chunk_words(t)); }
        }
        let owned = if weights.is_none() { Some(self.term_weights(qt)) } else { None };
        let weights: &[f64] = weights.unwrap_or_else(|| owned.as_ref().unwrap());
        rerank_score(qt, weights, &words, phonetic)
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
            .map(|c| c.vec.iter().map(|&(d, _)| d as usize).collect()).unwrap_or_default();
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
            .map(|ch| ch.vec.iter().map(|&(d, v)| json!({"dim": d as usize, "c": v})).collect())
            .unwrap_or_default();

        // matched filter: query deslizando sobre a sequência de sílabas do chunk
        // (memory: usa o cache `words`; hybrid: recomputa do texto do chunk)
        let recomputed;
        let words_ref: &[Vec<String>] = match self.chunks.get(cid) {
            Some(c) if !c.words.is_empty() => &c.words,
            Some(c) => match self.chunk_text(c) { Some(t) => { recomputed = chunk_words(t); &recomputed } None => &[] },
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

pub fn collection_fingerprint<B: std::borrow::Borrow<RagBase>>(bases: &HashMap<String, B>) -> (usize, usize) {
    (bases.len(), bases.values().map(|b| b.borrow().chunks.len()).sum())
}

/// Constrói o perfil unificado das bases de uma coleção. Determinístico: ordena bases
/// por nome e sílabas por dim local — a mesma coleção gera sempre o mesmo perfil.
/// [#37] Aceita `RagBase` ou `Arc<RagBase>` (o mapa vivo do daemon guarda `Arc`).
pub fn build_collection_profile<B: std::borrow::Borrow<RagBase>>(bases: &HashMap<String, B>) -> CollectionProfile {
    let mut uvocab: HashMap<String, usize> = HashMap::new();
    let mut remap: HashMap<String, Vec<usize>> = HashMap::new();
    let mut names: Vec<&String> = bases.keys().collect();
    names.sort();
    for name in &names {
        let base = bases[*name].borrow();
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
    // [#57] idf e normas SEM materializar um HashMap por chunk. A versão anterior guardava duas
    // cópias (`flat` e `per_base`, uma clone da outra) de um HashMap<usize,u32> por chunk só para
    // estas duas contas: em 335 livros / 122 mil chunks o pico era +1,47 GB, e o glibc não devolvia
    // ao SO — o RSS ficava ~1,5 GB acima depois da 1ª busca. Agora: df contado direto num vetor
    // pela dim global (cada dim local mapeia para uma global distinta — o remap é injetivo) e a
    // norma de cada chunk calculada em streaming sobre o próprio `vec` (já ordenado).
    let mut df: Vec<u32> = vec![0; uvocab.len()];
    let mut n_docs = 0usize;
    for name in &names {
        let m = &remap[*name];
        for ch in &bases[*name].borrow().chunks {
            n_docs += 1;
            for &(ld, _) in &ch.vec {
                if let Some(&gd) = m.get(ld as usize) { df[gd] += 1; }
            }
        }
    }
    // mesma fórmula de vector::compute_idf: ln((n+1)/df), só para dims com df > 0
    let n = if n_docs == 0 { 1.0 } else { n_docs as f64 };
    let uidf: HashMap<usize, f64> = df.iter().enumerate()
        .filter(|(_, &c)| c > 0).map(|(d, &c)| (d, ((n + 1.0) / c as f64).ln())).collect();
    let idf_dense: Vec<f64> = df.iter().enumerate().map(|(d, _)| uidf.get(&d).copied().unwrap_or(0.0)).collect();
    drop(df);
    // norma unificada (tf-idf no espaço global) por chunk — denominador do cosseno
    let mut unorms: HashMap<String, Vec<f64>> = HashMap::new();
    for name in &names {
        let m = &remap[*name];
        let norms: Vec<f64> = bases[*name].borrow().chunks.iter().map(|ch| {
            let mut s = 0.0;
            for &(ld, cnt) in &ch.vec {
                if let Some(&gd) = m.get(ld as usize) {
                    let w = (cnt as u32) as f64 * idf_dense[gd];
                    s += w * w;
                }
            }
            let nrm = s.sqrt();
            if nrm == 0.0 { 1.0 } else { nrm }
        }).collect();
        unorms.insert((*name).clone(), norms);
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
    // Mesmo esquema do `query_vec`: idf DOBRADO no lado da query (aqui o uidf, da coleção),
    // porque o chunk entra no dot com tf cru e o denominador é a norma tf-idf unificada.
    // [#56] norma somada em ordem de dim global (a ordem do HashMap muda a cada processo).
    // O cosseno unificado já é determinístico: ele percorre o `vec` do chunk, que é ordenado.
    let mut dims: Vec<(usize, u32)> = tf.into_iter().collect();
    dims.sort_unstable_by_key(|&(d, _)| d);
    let mut qw2: HashMap<usize, f64> = HashMap::new();
    let mut sum = 0.0;
    for (gd, c) in dims {
        let uidf = p.uidf.get(&gd).copied().unwrap_or(0.0);
        let w = c as f64 * uidf;
        if w != 0.0 { qw2.insert(gd, w * uidf); sum += w * w; }
    }
    let qnorm = sum.sqrt();
    (qw2, if qnorm == 0.0 { 1.0 } else { qnorm })
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
/// on-the-fly via `remap` + idf unificado. Mesmo esquema do `cosine_tfidf`: o `qvec` chega
/// com o uidf DOBRADO (tf_q·uidf², de `query_vec_unified`) e bate no tf CRU do chunk, sobre
/// qnorm (‖tf_q·uidf‖) × norma tf-idf unificada do chunk. Assim o cosseno fecha em [0,1].
pub fn cosine_unified(qvec: &HashMap<usize, f64>, qnorm: f64, chunk: &Chunk, remap: &[usize], unorm: f64) -> f64 {
    if unorm == 0.0 { return 0.0; }
    let mut dot = 0.0;
    // [#42] este caminho NAO pode iterar a query: ela vive em dims GLOBAIS e o chunk em
    // dims LOCAIS. Segue chunk-driven, remapeando cada par e sondando a query no hash.
    for &(ld, cnt) in &chunk.vec {
        let ld = ld as usize;
        if ld >= remap.len() { continue; }
        if let Some(&wq) = qvec.get(&remap[ld]) { dot += wq * cnt as f64; }
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
            vec: { let mut v: SparseVec = dims.iter().map(|&d| (d as u32, 1.0_f32)).collect(); v.sort_unstable_by_key(|&(d, _)| d); v },
            norm: 1.0, text: None, tref: None, words: Vec::new(),
        }).collect();
        let n = chunks.len();
        RagBase { index, idf: HashMap::new(), chunks, has_text: false,
                  n_chunks: n, vocab_size: vocab.len(), corpus: "t".into(), generator: "t".into(),
                  mtime: 0, blob: None }
    }
    /// [#41] o texto lido do .textblob é byte a byte o original (acentos inclusos) e a RAM larga o texto.
    #[test]
    fn spill_text_roundtrip() {
        let mut b = mk_base(&["a"], &[&[0], &[0], &[0]]);
        let textos = [Some("Frodo Bolseiro saiu do Condado."), None, Some("Ação, ônibus e coração — çãõ!")];
        for (c, t) in b.chunks.iter_mut().zip(textos) { c.text = t.map(String::from); }
        b.has_text = true;
        let dir = std::env::temp_dir().join(format!("ragd-41-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let json = dir.join("x-tokenized.json");
        let blob = blob_path_for(&json.to_string_lossy());
        assert!(blob.ends_with("x-tokenized.textblob"));
        b.spill_text(&blob).unwrap();
        for (c, t) in b.chunks.iter().zip(textos) {
            assert!(c.text.is_none(), "o texto deveria ter saído da RAM");
            assert_eq!(b.chunk_text(c), t);
        }
        assert!(b.chunks[1].tref.is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// [#41] base sem texto não gera arquivo nenhum.
    #[test]
    fn spill_text_sem_texto_e_noop() {
        let mut b = mk_base(&["a"], &[&[0]]);
        let blob = std::env::temp_dir().join(format!("ragd-41-vazio-{}.textblob", std::process::id()));
        b.spill_text(&blob.to_string_lossy()).unwrap();
        assert!(!blob.exists() && b.blob.is_none());
    }

    /// [#57] a montagem ANTIGA do perfil (HashMap por chunk), mantida só como referência.
    fn perfil_referencia(bases: &HashMap<String, RagBase>) -> (HashMap<usize, f64>, HashMap<String, Vec<f64>>) {
        let p = build_collection_profile(bases);   // reaproveita uvocab/remap (não mudaram)
        let mut names: Vec<&String> = bases.keys().collect(); names.sort();
        // tfs remapeados por base (pro idf de coleção e pras normas unificadas)
        let mut flat: Vec<HashMap<usize, u32>> = Vec::new();
        let mut per_base: Vec<(String, Vec<HashMap<usize, u32>>)> = Vec::new();
        for name in &names {
            let base = &bases[*name];
            let m = &p.remap[*name];
            let mut bt: Vec<HashMap<usize, u32>> = Vec::with_capacity(base.chunks.len());
            for ch in &base.chunks {
                let mut tf: HashMap<usize, u32> = HashMap::with_capacity(ch.vec.len());
                for &(ld, cnt) in &ch.vec {
                    let ld = ld as usize;
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
        (uidf, unorms)
    }

    /// [#57] a montagem nova dá o MESMO idf (bit a bit) e as mesmas normas (só a ordem da soma muda).
    #[test]
    fn perfil_sem_hashmap_por_chunk_igual_ao_antigo() {
        let mut bases = HashMap::new();
        let mut a = mk_base(&["fro", "do", "bol"], &[&[0, 1], &[1, 2], &[0, 2]]);
        a.chunks[0].vec = vec![(0, 3.0), (1, 1.0)];
        let mut b = mk_base(&["do", "ga", "fro", "sei"], &[&[0, 1, 3], &[2], &[0, 3]]);
        b.chunks[2].vec = vec![(0, 7.0), (3, 2.0)];
        bases.insert("a".to_string(), a);
        bases.insert("b".to_string(), b);
        let p = build_collection_profile(&bases);
        let (uidf_ref, unorms_ref) = perfil_referencia(&bases);
        assert_eq!(p.uidf, uidf_ref);
        for (nome, ns) in &unorms_ref {
            for (x, y) in p.unorms[nome].iter().zip(ns) { assert!((x - y).abs() < 1e-12, "{nome}: {x} vs {y}"); }
        }
    }

    /// [#56] a mesma busca dá o MESMO score, bit a bit, em todas as execuções. Cada HashMap novo
    /// ganha uma semente própria (RandomState), então repetir a busca no mesmo processo já
    /// sorteia a ordem de iteração — com a soma seguindo essa ordem, o cos variava na 16ª casa.
    #[test]
    fn busca_deterministica_bit_a_bit() {
        let texto = "paralelepipedo borboleta caramelo abacaxi telefone maravilhosa";
        let mut vocab: Vec<String> = vec![];
        for w in words(texto) {
            for sy in syllabify(&w) {
                let ns = normalize(&sy);
                if !ns.is_empty() && !vocab.contains(&ns) { vocab.push(ns); }
            }
        }
        let refs: Vec<&str> = vocab.iter().map(|x| x.as_str()).collect();
        let todas: Vec<usize> = (0..vocab.len()).collect();
        let mut b = mk_base(&refs, &[&todas, &todas[..todas.len() / 2]]);
        b.idf = (0..vocab.len()).map(|d| (d, 0.1 + (d as f64).sqrt() * 0.37)).collect();
        for ch in &mut b.chunks { for (i, x) in ch.vec.iter_mut().enumerate() { x.1 = (i % 5 + 1) as f32; } }
        let mut bases = HashMap::new();
        bases.insert("a".to_string(), b);
        let p = build_collection_profile(&bases);
        let (h0, _) = bases["a"].search(texto, 5, false, 20, false, None);
        let (_, n0) = query_vec_unified(texto, &p);
        assert!(!h0.is_empty());
        for _ in 0..300 {
            let (h, _) = bases["a"].search(texto, 5, false, 20, false, None);
            let got: Vec<(u64, usize)> = h.iter().map(|x| (x.3.to_bits(), x.4)).collect();
            let want: Vec<(u64, usize)> = h0.iter().map(|x| (x.3.to_bits(), x.4)).collect();
            assert_eq!(got, want);
            assert_eq!(query_vec_unified(texto, &p).1.to_bits(), n0.to_bits());
        }
    }

    /// [#44] "frodo" num chunk e "sammath" no vizinho: cada um cobre metade da query; a passagem
    /// dos dois cobre a query inteira.
    #[test]
    fn passagem_fecha_coocorrencia_entre_chunks() {
        let mut b = mk_base(&["fro", "do", "sam", "math"], &[&[0, 1], &[2, 3]]);
        b.chunks[0].text = Some("o hobbit frodo seguiu".into());
        b.chunks[1].text = Some("ate sammath naur".into());
        b.has_text = true;
        b.idf = (0..4).map(|d| (d, 1.0)).collect();
        let qt = prep_query("frodo sammath");
        let (c0, _) = b.score_chunk(&qt, None, 0, false);
        let (c1, _) = b.score_chunk(&qt, None, 1, false);
        let (cp, _) = b.score_passage(&qt, None, &[0, 1], false);
        assert!(c0 < 0.99 && c1 < 0.99, "{c0} {c1}");
        assert!((cp - 1.0).abs() < 1e-9, "passagem cobriu {cp}");
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
        // query (espaço global) = só "do". Contrato do `cosine_unified`: o vetor entra com o
        // uidf DOBRADO (tf·uidf²) e a norma é a do vetor tf-idf honesto (‖tf·uidf‖).
        let qw = p.uidf[&g_do];
        let mut qvec = HashMap::new();
        qvec.insert(g_do, qw * qw);
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
