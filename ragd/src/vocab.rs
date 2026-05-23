//! sylkit::vocab — o vocabulario (matriz de tokens em ordem FIXA) + metadata do .drv.
//!
//! Formato .drv (texto, 1 token por linha):
//!   - linha "# RAGnaRock driver: <Lang>"  -> header principal
//!   - linha "# descricao: <texto>"        -> descricao curta
//!   - linha "# extensoes: .ext1 .ext2 ..."-> extensoes que o driver cobre
//!   - linha "# ..." (qualquer outra)      -> comentario/cabecalho secundario
//!   - linha "=palavra"                    -> KEYWORD atomica (Jeito B)
//!   - qualquer outra                       -> silaba do vocabulario
//! Tanto silabas quanto keywords ocupam dimensoes na ordem do arquivo.
//! Aceita tambem .txt legado (sem '#'/'=').
use std::collections::{HashMap, HashSet};

/// Metadata extraida do cabecalho '#' de um .drv.
#[derive(Debug, Default, Clone)]
pub struct DriverMeta {
    pub header: String,           // "RAGnaRock driver: <Lang>"
    pub description: String,      // texto da linha "# descricao:"
    pub extensions: Vec<String>,  // [".py", ".pyw", ...] da linha "# extensoes:"
}

/// Le o driver -> (vocab, keywords). keywords ⊆ vocab; sao as linhas '=palavra'.
pub fn load_driver(path: &str) -> std::io::Result<(Vec<String>, HashSet<String>)> {
    let content = std::fs::read_to_string(path)?;
    let mut vocab = Vec::new();
    let mut keywords = HashSet::new();
    for line in content.lines() {
        let s = line.trim();
        if s.is_empty() || s.starts_with('#') { continue; }
        if let Some(kw) = s.strip_prefix('=') {
            if kw.is_empty() { continue; }
            keywords.insert(kw.to_string());
            vocab.push(kw.to_string());
        } else {
            vocab.push(s.to_string());
        }
    }
    Ok((vocab, keywords))
}

/// Le o vocabulario e devolve (lista ordenada, indice token->dim). Wrapper de load_driver.
pub fn load_vocab(path: &str) -> std::io::Result<(Vec<String>, HashMap<String, usize>)> {
    let (vocab, _) = load_driver(path)?;
    let index = vocab.iter().enumerate().map(|(i, t)| (t.clone(), i)).collect();
    Ok((vocab, index))
}

/// Le APENAS o bloco de cabecalho '#' do topo (parando na primeira linha non-#),
/// extraindo header / descricao / extensoes. Nao carrega o vocab.
pub fn read_meta(path: &str) -> std::io::Result<DriverMeta> {
    let content = std::fs::read_to_string(path)?;
    let mut meta = DriverMeta::default();
    let mut first_header_set = false;
    for line in content.lines() {
        let s = line.trim();
        if s.is_empty() { continue; }
        let Some(rest) = s.strip_prefix('#') else { break; };
        let rest = rest.trim();
        // primeiro comentario (sem prefixo conhecido) vira o header principal
        if let Some(desc) = strip_ci_prefix(rest, "descricao:") {
            meta.description = desc.trim().to_string();
        } else if let Some(exts) = strip_ci_prefix(rest, "extensoes:") {
            meta.extensions = exts.split_whitespace()
                .filter(|t| t.starts_with('.'))
                .map(|t| t.to_lowercase())
                .collect();
        } else if !first_header_set {
            meta.header = rest.to_string();
            first_header_set = true;
        }
    }
    Ok(meta)
}

/// Le so a primeira linha '#' (compat com a versao anterior; usado em poucos lugares).
pub fn read_header(path: &str) -> std::io::Result<String> {
    Ok(read_meta(path)?.header)
}

/// Strip case-insensitive de um prefixo. Devolve o restante se casar.
fn strip_ci_prefix<'a>(s: &'a str, prefix: &str) -> Option<&'a str> {
    if s.len() >= prefix.len() && s[..prefix.len()].eq_ignore_ascii_case(prefix) {
        Some(&s[prefix.len()..])
    } else {
        None
    }
}
