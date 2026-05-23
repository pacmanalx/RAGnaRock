//! sylkit::chunk — fatiamento do corpus (chunking) por CHAR (igual ao Python str).

/// Fatia em pedacos de ~size chars, cortando no ultimo espaco antes do limite.
/// Cada piece vem trimado; pieces vazios sao descartados.
pub fn chunk_text(chars: &[char], size: usize) -> Vec<String> {
    let mut chunks = Vec::new();
    let n = chars.len();
    let mut i = 0;
    while i < n {
        let mut end = (i + size).min(n);
        if end < n {
            // rfind ' ' em [i, end): pega o espaco mais a' direita
            let mut k = end;
            while k > i {
                k -= 1;
                if chars[k] == ' ' {
                    if k > i { end = k; }
                    break;
                }
            }
        }
        let piece: String = chars[i..end].iter().collect::<String>().trim().to_string();
        if !piece.is_empty() { chunks.push(piece); }
        i = end;
    }
    chunks
}

/// Acha a subsequencia `needle` em `hay` a partir de `from` (indices de char).
pub fn find_chars(hay: &[char], needle: &[char], from: usize) -> Option<usize> {
    let (n, m) = (hay.len(), needle.len());
    if m == 0 { return Some(from); }
    if m > n { return None; }
    let mut i = from;
    while i + m <= n {
        if hay[i..i + m] == needle[..] { return Some(i); }
        i += 1;
    }
    None
}
