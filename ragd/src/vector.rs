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

/// idf global SUAVIZADO = log((N+1)/df).
///
/// O `+1` no numerador evita o colapso pra 0 quando N=1 (base de 1 chunk): com o
/// `log(N/df)` clássico, numa base de 1 chunk todo termo tem df=N=1 → idf=0 → vetor
/// tf-idf nulo → a base fica INVISÍVEL na busca por cosseno. Com `(N+1)` uma base de
/// 1 chunk ainda ranqueia (idf=log2≈0.69>0), e termos em todos os chunks (df=N, tipo
/// stopword) viram peso pequeno em vez de exatamente 0 — comportamento à la BM25.
/// (Diverge de propósito do `log(N/df)` dos PoCs python/rust congelados.)
pub fn compute_idf(tfs: &[HashMap<usize, u32>], n_docs: usize) -> HashMap<usize, f64> {
    let mut df: HashMap<usize, usize> = HashMap::new();
    for tf in tfs {
        for &d in tf.keys() { *df.entry(d).or_insert(0) += 1; }
    }
    let n = if n_docs == 0 { 1.0 } else { n_docs as f64 };
    df.into_iter().map(|(d, dfd)| (d, ((n + 1.0) / dfd as f64).ln())).collect()
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

/// Cosseno tf-idf REAL contra um chunk cujo `vec` guarda CONTAGEM crua (tf).
///
/// O chunk guarda `tf_c` (contagem) mas sua norma gravada é `‖tf_c ⊙ idf‖` — escalas
/// diferentes. Bater `tf_q⊙idf · tf_c` contra essa norma NÃO é cosseno e pode passar de
/// 1 (medido: 1,85 em corpus real), inflando o score na proporção das sílabas banais,
/// que é justamente o viés que o idf existe pra eliminar.
///
/// A identidade que conserta sem custo: `Σ (tf_q·idf)(tf_c·idf) = Σ (tf_q·idf²)·tf_c`.
/// Dobramos o idf no lado da QUERY (uma vez por busca, em `query_vec`) e batemos direto
/// no tf cru do chunk. Caminho quente intocado e formato do JSON preservado.
///
/// `qw2` = tf_q ⊙ idf²  ·  `qn` = ‖tf_q ⊙ idf‖  ·  `ctf` = tf_c cru  ·  `cn` = ‖tf_c ⊙ idf‖
pub fn cosine_tfidf(qw2: &HashMap<usize, f64>, qn: f64, ctf: &HashMap<usize, f64>, cn: f64) -> f64 {
    let (small, big) = if qw2.len() > ctf.len() { (ctf, qw2) } else { (qw2, ctf) };
    let mut dot = 0.0;
    for (d, v) in small {
        if let Some(w) = big.get(d) { dot += v * w; }
    }
    dot / (qn * cn)
}

/// Cosseno de dois vetores esparsos JÁ ponderados (itera o menor; so dims em comum contam).
/// Para chunk com `vec` em contagem crua use `cosine_tfidf` — este aqui assume que os dois
/// lados estão na MESMA escala das normas passadas.
pub fn cosine(q: &HashMap<usize, f64>, qn: f64, c: &HashMap<usize, f64>, cn: f64) -> f64 {
    let (small, big) = if q.len() > c.len() { (c, q) } else { (q, c) };
    let mut dot = 0.0;
    for (d, v) in small {
        if let Some(w) = big.get(d) { dot += v * w; }
    }
    dot / (qn * cn)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Monta (qw2, qnorm) do jeito que `query_vec` monta: idf dobrado no dot, norma honesta.
    fn q_from(tf: &HashMap<usize, u32>, idf: &HashMap<usize, f64>) -> (HashMap<usize, f64>, f64) {
        let mut qw2 = HashMap::new();
        let mut s = 0.0;
        for (d, c) in tf {
            let i = idf.get(d).copied().unwrap_or(0.0);
            let w = *c as f64 * i;
            if w != 0.0 { qw2.insert(*d, w * i); s += w * w; }
        }
        (qw2, if s == 0.0 { 1.0 } else { s.sqrt() })
    }

    fn tf(pairs: &[(usize, u32)]) -> HashMap<usize, u32> { pairs.iter().copied().collect() }
    fn cnt(pairs: &[(usize, u32)]) -> HashMap<usize, f64> {
        pairs.iter().map(|&(d, c)| (d, c as f64)).collect()
    }

    /// O invariante que faltava: query IGUAL ao chunk tem de dar cosseno EXATAMENTE 1.
    /// Com o esquema antigo (tf-idf da query · tf cru do chunk) isto dava 1,4+.
    #[test]
    fn self_similarity_is_exactly_one() {
        let idf: HashMap<usize, f64> = [(0, 0.5), (1, 2.0), (2, 0.1)].into_iter().collect();
        let t = tf(&[(0, 5), (1, 3), (2, 7)]);
        let (qw2, qn) = q_from(&t, &idf);
        let cn = tfidf_norm(&t, &idf);
        let cos = cosine_tfidf(&qw2, qn, &cnt(&[(0, 5), (1, 3), (2, 7)]), cn);
        assert!((cos - 1.0).abs() < 1e-12, "auto-similaridade deu {cos}, esperado 1.0");
    }

    /// Sílaba banal (idf baixo) com contagem alta era o que estourava o teto.
    #[test]
    fn common_syllable_never_exceeds_one() {
        // dim 0 = stopword silábica: idf minúsculo, mas aparece 200x no chunk.
        let idf: HashMap<usize, f64> = [(0, 0.01), (1, 3.0)].into_iter().collect();
        let ctf = tf(&[(0, 200), (1, 1)]);
        let cn = tfidf_norm(&ctf, &idf);
        let (qw2, qn) = q_from(&tf(&[(0, 1)]), &idf);
        let cos = cosine_tfidf(&qw2, qn, &cnt(&[(0, 200), (1, 1)]), cn);
        assert!(cos <= 1.0 + 1e-9, "cosseno passou de 1: {cos}");
        assert!(cos > 0.0);
    }

    /// Bate contra o cosseno calculado de forma independente (dois vetores tf-idf explícitos).
    #[test]
    fn matches_explicit_tfidf_cosine() {
        let idf: HashMap<usize, f64> = [(0, 0.5), (1, 2.0), (2, 0.1)].into_iter().collect();
        let qt = tf(&[(0, 2), (1, 1)]);
        let ct = tf(&[(0, 5), (1, 3), (2, 7)]);
        let (qw2, qn) = q_from(&qt, &idf);
        let got = cosine_tfidf(&qw2, qn, &cnt(&[(0, 5), (1, 3), (2, 7)]), tfidf_norm(&ct, &idf));
        // referência: ponderar OS DOIS lados e usar o cosseno genérico
        let wq: HashMap<usize, f64> = qt.iter().map(|(d, c)| (*d, *c as f64 * idf[d])).collect();
        let wc: HashMap<usize, f64> = ct.iter().map(|(d, c)| (*d, *c as f64 * idf[d])).collect();
        let want = cosine(&wq, tfidf_norm(&qt, &idf), &wc, tfidf_norm(&ct, &idf));
        assert!((got - want).abs() < 1e-12, "got {got}, want {want}");
    }
}
