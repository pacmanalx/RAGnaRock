//! search_rag (Rust) — busca na base RAG (fase online). Porte do search_rag.py.
//! recall (cosseno) + rerank (matched filter + span) + snippet, narrando os passos.
use std::collections::HashMap;
use std::io::Write;
use std::time::Instant;
use serde_json::Value;
use sylkit::{cosine, find_chars, normalize, syllabify, syllable_seq, words};

const PROX_SCALE: f64 = 8.0;

// ----------------------------- estagio 2: rerank -----------------------------
fn best_positions(qs: &[String], seq: &[String]) -> (f64, Vec<usize>) {
    let k = qs.len();
    if k == 0 || seq.len() < k { return (0.0, vec![0]); }
    let mut best: i64 = -1;
    let mut pos: Vec<usize> = vec![];
    for p in 0..=(seq.len() - k) {
        let m = (0..k).filter(|&j| seq[p + j] == qs[j]).count() as i64;
        if m > best { best = m; pos = vec![p]; }
        else if m == best { pos.push(p); }
    }
    (best as f64 / k as f64, pos)
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

fn rerank_score(query: &str, seq: &[String]) -> (f64, usize) {
    let lower = query.to_lowercase();
    let mut words_qs: Vec<Vec<String>> = vec![];
    for w in words(&lower) {
        let qs: Vec<String> = syllabify(&w).iter().map(|s| normalize(s))
            .filter(|s| !s.is_empty()).collect();
        if !qs.is_empty() { words_qs.push(qs); }
    }
    if words_qs.is_empty() { return (0.0, 0); }
    let mut fracs = vec![];
    let mut lists = vec![];
    for qs in &words_qs {
        let (frac, pos) = best_positions(qs, seq);
        fracs.push(frac); lists.push(pos);
    }
    let mf = fracs.iter().sum::<f64>() / fracs.len() as f64;
    let span = if lists.len() > 1 { min_span(&lists) } else { 0 };
    (mf, span)
}

fn snippet(text: &str, query: &str) -> String {
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
    // destaca as palavras da query com «»
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
    for (a, b) in marks.into_iter().rev() {
        res.insert(b, '»'); res.insert(a, '«');
    }
    out = res.into_iter().collect();
    out
}

// ------------------------------- a base RAG ----------------------------------
struct Chunk { start: usize, vec: HashMap<usize, f64>, norm: f64, text: Option<String> }

struct RagBase {
    index: HashMap<String, usize>,
    idf: HashMap<usize, f64>,
    chunks: Vec<Chunk>,
    has_text: bool,
    n_chunks: usize, vocab_size: usize, corpus: String, generator: String,
}

struct Info { syls: Vec<String>, oov: usize, dims: usize, n_chunks: usize,
              n_converge: usize, recall_n: usize, rerank: bool, ms_recall: f64, ms_rerank: f64 }
type Hit = (Option<f64>, Option<f64>, Option<usize>, f64, usize); // rr, mf, span, cos, cid

impl RagBase {
    fn load(path: &str) -> RagBase {
        let data = std::fs::read_to_string(path).unwrap();
        let v: Value = serde_json::from_str(&data).unwrap();
        let meta = &v["meta"];
        let vocab: Vec<String> = meta["vocab"].as_array().unwrap().iter()
            .map(|x| x.as_str().unwrap().to_string()).collect();
        let index = vocab.iter().enumerate().map(|(i, t)| (t.clone(), i)).collect();
        let idf: HashMap<usize, f64> = v["idf"].as_object().unwrap().iter()
            .map(|(k, val)| (k.parse().unwrap(), val.as_f64().unwrap())).collect();
        let chunks: Vec<Chunk> = v["chunks"].as_array().unwrap().iter().map(|c| {
            let vec = c["vec"].as_object().unwrap().iter()
                .map(|(k, val)| (k.parse::<usize>().unwrap(), val.as_f64().unwrap())).collect();
            Chunk {
                start: c["start"].as_u64().unwrap() as usize,
                vec, norm: c["norm"].as_f64().unwrap(),
                text: c["text"].as_str().map(|s| s.to_string()),
            }
        }).collect();
        RagBase {
            index, idf,
            n_chunks: meta["n_chunks"].as_u64().unwrap() as usize,
            vocab_size: meta["vocab_size"].as_u64().unwrap() as usize,
            corpus: meta["corpus"].as_str().unwrap_or("?").to_string(),
            generator: meta["generator"].as_str().unwrap_or("?").to_string(),
            has_text: meta["with_text"].as_bool().unwrap_or(false),
            chunks,
        }
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

    fn search(&self, query: &str, k: usize, rerank: bool, recall_n: usize) -> (Vec<Hit>, Info) {
        let (qvec, qnorm, syls, oov) = self.query_vec(query);
        let mut info = Info { syls, oov, dims: qvec.len(), n_chunks: self.chunks.len(),
            n_converge: 0, recall_n: 0, rerank: rerank && self.has_text, ms_recall: 0.0, ms_rerank: 0.0 };
        if qvec.is_empty() { return (vec![], info); }
        // ESTAGIO 1 — recall por cosseno
        let t0 = Instant::now();
        let mut scored: Vec<(f64, usize)> = vec![];
        for (cid, c) in self.chunks.iter().enumerate() {
            let s = cosine(&qvec, qnorm, &c.vec, c.norm);
            if s > 0.0 { scored.push((s, cid)); }
        }
        info.n_converge = scored.len();
        scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap().then(b.1.cmp(&a.1)));
        let rn = if info.rerank { k.max(recall_n) } else { k };
        let cand: Vec<(f64, usize)> = scored.into_iter().take(rn).collect();
        info.recall_n = cand.len();
        info.ms_recall = t0.elapsed().as_secs_f64() * 1000.0;
        // ESTAGIO 2 — rerank
        let hits: Vec<Hit> = if info.rerank {
            let t1 = Instant::now();
            let mut res: Vec<Hit> = cand.iter().map(|&(cos, cid)| {
                let seq = syllable_seq(self.chunks[cid].text.as_deref().unwrap_or(""));
                let (mf, span) = rerank_score(query, &seq);
                let rr = mf / (1.0 + span as f64 / PROX_SCALE);
                (Some(rr), Some(mf), Some(span), cos, cid)
            }).collect();
            res.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap().then(b.3.partial_cmp(&a.3).unwrap()));
            info.ms_rerank = t1.elapsed().as_secs_f64() * 1000.0;
            res.into_iter().take(k).collect()
        } else {
            cand.into_iter().take(k).map(|(cos, cid)| (None, None, None, cos, cid)).collect()
        };
        (hits, info)
    }
}

fn commas(n: usize) -> String {
    let s = n.to_string();
    let len = s.len();
    let mut out = String::new();
    for (i, ch) in s.chars().enumerate() {
        if i > 0 && (len - i) % 3 == 0 { out.push(','); }
        out.push(ch);
    }
    out
}

fn show(base: &RagBase, query: &str, hits: &[Hit], info: &Info, k: usize) {
    let bar = "━".repeat(66);
    println!("\n{bar}\n  BUSCA: {query:?}\n{bar}");
    println!("  [2] HISTOGRAM   query → {}", if info.syls.is_empty() { "(vazio)".into() } else { info.syls.join("-") });
    println!("                  vetor da busca: {} dims ativas, {} OOV", info.dims, info.oov);
    if hits.is_empty() {
        println!("  [3] RECALL      nenhuma sílaba da query no vocabulário → 0 chunks");
        println!("  ⇒ sem resultados.");
        return;
    }
    println!("  [3] RECALL      {} chunks varridos → {} convergem (cosseno > 0)   [{:.1} ms]",
             info.n_chunks, info.n_converge, info.ms_recall);
    println!("                  ↳ mantém os top-{} candidatos pro rerank", info.recall_n);
    if info.rerank {
        println!("  [4] RERANK      filtros: P1 contiguidade (matched filter) · P2 proximidade (span)");
        println!("                  ↳ {} candidatos reordenados por matchpoint   [{:.1} ms]",
                 info.recall_n, info.ms_rerank);
    } else {
        println!("  [4] RERANK      (pulado — só recall por cosseno)");
    }
    println!("  [5] RESULTADOS  top-{k}  (melhor em cima, pior embaixo):");
    for (rank, (rr, mf, span, cos, cid)) in hits.iter().enumerate() {
        let c = &base.chunks[*cid];
        let mp = match rr {
            Some(rr) => format!("matchpoint {:.3}   (mf {:.2} · span {} · cos {:.4})",
                                rr, mf.unwrap(), span.unwrap(), cos),
            None => format!("matchpoint {:.4}   (cosseno puro)", cos),
        };
        println!("\n   #{}  {}", rank + 1, mp);
        println!("        chunk {} @byte {}", cid, commas(c.start));
        if let Some(t) = &c.text {
            println!("        “{}”", snippet(t, query));
        }
    }
}

fn help() {
    println!(
"search_rag (Rust) — busca na base RAG (recall cosseno + rerank matched filter).

uso (a base JSON e' obrigatoria — sempre explicita):
  search_rag <base.json> [queries...] [opcoes]

opcoes:
  -k, --top N      quantos resultados (default 5)
  --recall-n N     candidatos do recall p/ o rerank (default 20)
  --no-rerank      so o estagio 1 (recall cosseno)
  -i               modo interativo (REPL)

exemplos:
  search_rag sda-tokenized.json \"Frodo Bolseiro\"
  search_rag sda-tokenized.json \"anel\" \"montanha\" -k 3
  search_rag sda-tokenized.json -i");
}

fn main() {
    let argv: Vec<String> = std::env::args().collect();
    if argv.len() == 1 { help(); return; }

    let (mut k, mut recall_n) = (5usize, 20usize);
    let (mut no_rerank, mut interactive) = (false, false);
    let mut pos: Vec<String> = vec![];
    let mut it = argv[1..].iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "-h" | "--help" => { help(); return; }
            "-k" | "--top" => k = it.next().unwrap().parse().expect("-k N"),
            "--recall-n" => recall_n = it.next().unwrap().parse().expect("--recall-n N"),
            "--no-rerank" => no_rerank = true,
            "-i" | "--interactive" => interactive = true,
            other => pos.push(other.to_string()),
        }
    }
    if pos.is_empty() {
        eprintln!("erro: base JSON obrigatória.\n      uso: search_rag <base.json> [queries...]");
        std::process::exit(2);
    }
    let base_path = pos.remove(0);
    let queries = pos;
    if !std::path::Path::new(&base_path).exists() {
        eprintln!("erro: base não encontrada: {base_path:?}\n      gere uma com:  embed_gen <corpus.txt>");
        std::process::exit(1);
    }

    let t0 = Instant::now();
    let base = RagBase::load(&base_path);
    println!("  [1] LOAD        base '{}' em RAM   [{:.0} ms]", base_path, t0.elapsed().as_secs_f64() * 1000.0);
    println!("                  {} chunks · vocab {} dims · idf {} · corpus '{}' (por {})",
             commas(base.n_chunks), base.vocab_size, base.idf.len(), base.corpus, base.generator);
    if !base.has_text {
        println!("                  aviso: base com --sem-texto → rerank e snippet desligados");
    }

    let rerank = !no_rerank;
    for q in &queries {
        let (hits, info) = base.search(q, k, rerank, recall_n);
        show(&base, q, &hits, &info, k);
    }

    if interactive {
        println!("\n[modo interativo — digite a busca; ENTER vazio ou 'sair' encerra]");
        loop {
            print!("\nbusca> ");
            std::io::stdout().flush().ok();
            let mut line = String::new();
            if std::io::stdin().read_line(&mut line).unwrap_or(0) == 0 { println!(); break; }
            let q = line.trim();
            if q.is_empty() || matches!(q.to_lowercase().as_str(), "sair" | "quit" | "exit") { break; }
            let (hits, info) = base.search(q, k, rerank, recall_n);
            show(&base, q, &hits, &info, k);
        }
    }
}
