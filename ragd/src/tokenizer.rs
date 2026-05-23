//! sylkit::tokenizer — o silabador (onset/nucleo/coda). Porte FIEL do Python.

#[derive(PartialEq, Clone, Copy)]
enum Kind { V, C, N }

fn is_vowel(c: char) -> bool {
    // Python VOWEL = "aàáâãeéêiíoóôõuúü"
    matches!(c, 'a'|'à'|'á'|'â'|'ã'|'e'|'é'|'ê'|'i'|'í'|'o'|'ó'|'ô'|'õ'|'u'|'ú'|'ü')
}

fn segment(p: &[char]) -> Vec<(Kind, String)> {
    let mut segs = Vec::new();
    let n = p.len();
    let mut i = 0;
    while i < n {
        let c = p[i];
        if is_vowel(c) {
            segs.push((Kind::V, c.to_string())); i += 1; continue;
        }
        if i + 1 < n {
            let a = p[i]; let b = p[i + 1];
            if (a == 'c' && b == 'h') || (a == 'l' && b == 'h') || (a == 'n' && b == 'h') {
                segs.push((Kind::C, format!("{}{}", a, b))); i += 2; continue;
            }
            if a == 'q' && b == 'u' {
                segs.push((Kind::C, "qu".into())); i += 2; continue;
            }
            if a == 'g' && b == 'u' && i + 2 < n
                && matches!(p[i + 2], 'e'|'é'|'ê'|'i'|'í') {
                segs.push((Kind::C, "gu".into())); i += 2; continue;
            }
        }
        segs.push((Kind::C, c.to_string())); i += 1;
    }
    segs
}

fn group_nuclei(vowels: &[char]) -> Vec<String> {
    let weak = |x: char| x == 'i' || x == 'u';
    let weak_acc = |x: char| x == 'í' || x == 'ú';
    let mut nuclei: Vec<String> = Vec::new();
    let mut cur = String::new();
    for &v in vowels {
        if cur.is_empty() { cur.push(v); continue; }
        let prev = cur.chars().last().unwrap();
        let join = if weak_acc(v) || weak_acc(prev) { false }
                   else if weak(v) { true }
                   else if weak(prev) && !weak(v) { true }
                   else { false };
        if join { cur.push(v); } else { nuclei.push(std::mem::take(&mut cur)); cur.push(v); }
    }
    if !cur.is_empty() { nuclei.push(cur); }
    nuclei
}

/// Palavra -> lista de silabas (distribui consoantes entre os nucleos).
pub fn syllabify(word: &str) -> Vec<String> {
    let p: Vec<char> = word.to_lowercase().chars()
        .filter(|c| is_vowel(*c) || c.is_alphabetic()).collect();
    if p.is_empty() { return vec![]; }
    let segs = segment(&p);
    if !segs.iter().any(|(k, _)| *k == Kind::V) {
        return vec![p.iter().collect()];
    }
    // agrupa em blocos: C cluster ou N nucleo
    let mut blocks: Vec<(Kind, String)> = Vec::new();
    let n = segs.len();
    let mut j = 0;
    while j < n {
        if segs[j].0 == Kind::C {
            blocks.push((Kind::C, segs[j].1.clone())); j += 1;
        } else {
            let mut run: Vec<char> = vec![segs[j].1.chars().next().unwrap()]; j += 1;
            while j < n && segs[j].0 == Kind::V {
                run.push(segs[j].1.chars().next().unwrap()); j += 1;
            }
            for nuc in group_nuclei(&run) { blocks.push((Kind::N, nuc)); }
        }
    }
    if !blocks.iter().any(|(k, _)| *k == Kind::N) {
        return vec![p.iter().collect()];
    }
    let mut syl: Vec<String> = Vec::new();
    let mut k = 0;
    let mut pre = String::new();
    while k < blocks.len() && blocks[k].0 == Kind::C { pre.push_str(&blocks[k].1); k += 1; }
    let mut cur = pre + &blocks[k].1; k += 1;
    while k < blocks.len() {
        let mut cc: Vec<String> = Vec::new();
        while k < blocks.len() && blocks[k].0 == Kind::C { cc.push(blocks[k].1.clone()); k += 1; }
        if k >= blocks.len() { for x in &cc { cur.push_str(x); } break; }
        let nxt = blocks[k].1.clone(); k += 1;
        let t = cc.len();
        if t == 0 {
            syl.push(std::mem::take(&mut cur)); cur = nxt;
        } else if t == 1 {
            syl.push(std::mem::take(&mut cur)); cur = format!("{}{}", cc[0], nxt);
        } else {
            let prev = &cc[t - 2]; let last = &cc[t - 1];
            let pair = format!("{}{}", prev, last);
            let onset2 = matches!(pair.as_str(),
                "bl"|"br"|"cl"|"cr"|"dl"|"dr"|"fl"|"fr"|"gl"|"gr"|"pl"|"pr"|"tl"|"tr"|"vl"|"vr");
            let prev_ok = matches!(prev.as_str(), "b"|"c"|"d"|"f"|"g"|"p"|"t"|"v");
            let last_ok = matches!(last.as_str(), "l"|"r");
            let (onset, coda) = if prev_ok && last_ok && onset2 {
                (pair.clone(), cc[..t - 2].concat())
            } else {
                (last.clone(), cc[..t - 1].concat())
            };
            syl.push(format!("{}{}", cur, coda)); cur = format!("{}{}", onset, nxt);
        }
    }
    syl.push(cur);
    syl.into_iter().filter(|s| !s.is_empty()).collect()
}

fn strip_accent(c: char) -> char {
    match c {
        'á'|'à'|'â'|'ã'|'ä'|'å' => 'a',
        'é'|'ê'|'è'|'ë' => 'e',
        'í'|'î'|'ì'|'ï' => 'i',
        'ó'|'ô'|'õ'|'ò'|'ö' => 'o',
        'ú'|'û'|'ù'|'ü' => 'u',
        'ç' => 'c',
        'ñ' => 'n',
        other => other,
    }
}

/// Minuscula + sem acento (chave canonica do vocabulario).
pub fn normalize(s: &str) -> String {
    s.chars().flat_map(|c| c.to_lowercase()).map(strip_accent).collect()
}

fn is_word_char(c: char) -> bool {
    // Python WORD = [a-zàáâãäçéêèíïóôõòúûü] (sobre texto ja' lowercased)
    c.is_ascii_lowercase() || "àáâãäçéêèíïóôõòúûü".contains(c)
}

/// Extrai palavras de um texto JA' lowercased (runs de caracteres de palavra).
pub fn words(lower: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    for c in lower.chars() {
        if is_word_char(c) { cur.push(c); }
        else if !cur.is_empty() { out.push(std::mem::take(&mut cur)); }
    }
    if !cur.is_empty() { out.push(cur); }
    out
}

/// Texto -> SEQUENCIA de silabas normalizadas (preserva ordem).
pub fn syllable_seq(text: &str) -> Vec<String> {
    let lower = text.to_lowercase();
    let mut seq = Vec::new();
    for w in words(&lower) {
        for s in syllabify(&w) {
            let ns = normalize(&s);
            if !ns.is_empty() { seq.push(ns); }
        }
    }
    seq
}
