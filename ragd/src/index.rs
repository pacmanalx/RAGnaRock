//! sylkit::index — indice invertido posicional (postings).
use std::collections::HashMap;

/// Sequencia de tokens -> {token: [posicoes ordenadas]}.
pub fn postings(seq: &[String]) -> HashMap<String, Vec<usize>> {
    let mut pos: HashMap<String, Vec<usize>> = HashMap::new();
    for (i, tok) in seq.iter().enumerate() {
        pos.entry(tok.clone()).or_default().push(i);
    }
    pos
}
