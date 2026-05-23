//! sylkit::vector — embedding esparso (histograma) e similaridade.
use std::collections::HashMap;
use crate::tokenizer::{normalize, syllabify, words};

/// Texto -> (histograma tf esparso {dim:count}, total de silabas, OOV).
pub fn histogram(text: &str, index: &HashMap<String, usize>) -> (HashMap<usize, u32>, usize, usize) {
    let lower = text.to_lowercase();
    let mut tf: HashMap<usize, u32> = HashMap::new();
    let (mut total, mut oov) = (0usize, 0usize);
    for w in words(&lower) {
        for s in syllabify(&w) {
            let ns = normalize(&s);
            if ns.is_empty() { continue; }
            total += 1;
            match index.get(&ns) {
                Some(&d) => { *tf.entry(d).or_insert(0) += 1; }
                None => { oov += 1; }
            }
        }
    }
    (tf, total, oov)
}

/// idf global = log(N/df).
pub fn compute_idf(tfs: &[HashMap<usize, u32>], n_docs: usize) -> HashMap<usize, f64> {
    let mut df: HashMap<usize, usize> = HashMap::new();
    for tf in tfs {
        for &d in tf.keys() { *df.entry(d).or_insert(0) += 1; }
    }
    let n = if n_docs == 0 { 1.0 } else { n_docs as f64 };
    df.into_iter().map(|(d, dfd)| (d, (n / dfd as f64).ln())).collect()
}

/// Norma L2 do vetor tf-idf (peso = count*idf). 0 -> 1.0 (igual ao Python).
pub fn tfidf_norm(tf: &HashMap<usize, u32>, idf: &HashMap<usize, f64>) -> f64 {
    let mut s = 0.0;
    for (d, c) in tf {
        let w = *c as f64 * idf.get(d).copied().unwrap_or(0.0);
        s += w * w;
    }
    let n = s.sqrt();
    if n == 0.0 { 1.0 } else { n }
}

/// Cosseno de dois vetores esparsos (itera o menor; so dims em comum contam).
pub fn cosine(q: &HashMap<usize, f64>, qn: f64, c: &HashMap<usize, f64>, cn: f64) -> f64 {
    let (small, big) = if q.len() > c.len() { (c, q) } else { (q, c) };
    let mut dot = 0.0;
    for (d, v) in small {
        if let Some(w) = big.get(d) { dot += v * w; }
    }
    dot / (qn * cn)
}
