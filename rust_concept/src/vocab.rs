//! sylkit::vocab — o vocabulario (matriz de tokens em ordem FIXA).
use std::collections::HashMap;

/// Le o vocabulario (1 token por linha) -> (lista ordenada, indice token->dim).
pub fn load_vocab(path: &str) -> std::io::Result<(Vec<String>, HashMap<String, usize>)> {
    let content = std::fs::read_to_string(path)?;
    let vocab: Vec<String> = content.lines()
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty())
        .collect();
    let index = vocab.iter().enumerate().map(|(i, t)| (t.clone(), i)).collect();
    Ok((vocab, index))
}
