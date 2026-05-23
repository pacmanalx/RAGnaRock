//! embed_gen (Rust) — corpus .txt -> base de vetores-histograma em JSON.
//! Porte do embed_gen.py: mesmo schema (campos EN, vec esparso, idf, vocab inline).
use std::collections::HashMap;
use std::time::Instant;
use serde_json::{Map, Value};
use sylkit::{chunk_text, compute_idf, find_chars, histogram, load_vocab, tfidf_norm};

fn help() {
    println!(
"embed_gen (Rust) — gera base de vetores-histograma (JSON) de um corpus.

uso:
  embed_gen <corpus.txt> [opcoes]

opcoes:
  --chunk N        bytes(chars) por chunk (default 2048)
  --saida ARQ      JSON de saida (default {{corpus}}-tokenized.json)
  --tokens ARQ     vocabulario de tokens (default tokens_PTBR.txt)
  --max-chunks N   limita chunks (0 = corpus inteiro)
  --sem-texto      nao grava o texto de cada chunk
  --compacto       JSON minificado (default indentado com TAB)

exemplos:
  embed_gen sda.txt
  embed_gen sda.txt --chunk 1024 --saida sda-1k.json");
}

fn round_to(x: f64, d: i32) -> f64 {
    let f = 10f64.powi(d);
    (x * f).round() / f
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

fn iso_utc_now() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() as i64;
    let days = secs.div_euclid(86400);
    let rem = secs.rem_euclid(86400);
    let (h, mi, s) = (rem / 3600, (rem % 3600) / 60, rem % 60);
    let z = days + 719468;
    let era = (if z >= 0 { z } else { z - 146096 }) / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let mut y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    if m <= 2 { y += 1; }
    format!("{:04}-{:02}-{:02}T{:02}:{:02}:{:02}", y, m, d, h, mi, s)
}

fn basename(p: &str) -> String { p.rsplit('/').next().unwrap_or(p).to_string() }

struct Rec { id: usize, start: usize, len: usize, tokens: usize, oov: usize,
             tf: HashMap<usize, u32>, text: Option<String> }

fn main() {
    let argv: Vec<String> = std::env::args().collect();
    if argv.len() == 1 { help(); return; }

    let (mut corpus, mut chunk, mut saida) = ("sda.txt".to_string(), 2048usize, None::<String>);
    let (mut tokens, mut max_chunks) = ("tokens_PTBR.txt".to_string(), 0usize);
    let (mut sem_texto, mut compacto) = (false, false);
    let mut pos: Vec<String> = vec![];
    let mut it = argv[1..].iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "-h" | "--help" => { help(); return; }
            "--chunk" => chunk = it.next().unwrap().parse().expect("--chunk N"),
            "--saida" | "--out" => saida = Some(it.next().unwrap().clone()),
            "--tokens" => tokens = it.next().unwrap().clone(),
            "--max-chunks" => max_chunks = it.next().unwrap().parse().expect("--max-chunks N"),
            "--sem-texto" => sem_texto = true,
            "--compacto" => compacto = true,
            other => pos.push(other.to_string()),
        }
    }
    if let Some(c) = pos.into_iter().next() { corpus = c; }
    let saida = saida.unwrap_or_else(|| {
        let b = basename(&corpus);
        let stem = b.rsplit_once('.').map(|(s, _)| s).unwrap_or(&b);
        format!("{}-tokenized.json", stem)
    });

    let (vocab, index) = load_vocab(&tokens).unwrap_or_else(|e| {
        eprintln!("erro lendo tokens {tokens:?}: {e}"); std::process::exit(1);
    });
    let text = std::fs::read_to_string(&corpus).unwrap_or_else(|e| {
        eprintln!("erro lendo corpus {corpus:?}: {e}"); std::process::exit(1);
    });

    let t0 = Instant::now();
    let chars: Vec<char> = text.chars().collect();
    let mut pieces = chunk_text(&chars, chunk);
    if max_chunks > 0 { pieces.truncate(max_chunks); }

    // passada 1: histograma + offset (char) de cada chunk
    let mut recs: Vec<Rec> = Vec::with_capacity(pieces.len());
    let mut tfs: Vec<HashMap<usize, u32>> = Vec::with_capacity(pieces.len());
    let (mut tot_tokens, mut tot_oov) = (0usize, 0usize);
    let mut cursor = 0usize;
    for (cid, piece) in pieces.iter().enumerate() {
        let needle: Vec<char> = piece.chars().collect();
        let start = find_chars(&chars, &needle, cursor).unwrap_or(cursor);
        cursor = start + needle.len();
        let (tf, total, oov) = histogram(piece, &index);
        tot_tokens += total; tot_oov += oov;
        tfs.push(tf.clone());
        recs.push(Rec { id: cid, start, len: needle.len(), tokens: total, oov,
                        tf, text: if sem_texto { None } else { Some(piece.clone()) } });
    }

    let idf = compute_idf(&tfs, pieces.len());

    // ---- monta o JSON (ordem de campos = embed_gen.py; vec/idf por dim numerico) ----
    let mut meta = Map::new();
    meta.insert("corpus".into(), Value::from(basename(&corpus)));
    meta.insert("bytes".into(), Value::from(text.len()));
    meta.insert("chunk_size".into(), Value::from(chunk));
    meta.insert("n_chunks".into(), Value::from(pieces.len()));
    meta.insert("vocab_size".into(), Value::from(vocab.len()));
    meta.insert("vocab_used".into(), Value::from(idf.len()));
    meta.insert("tokens_total".into(), Value::from(tot_tokens));
    meta.insert("oov_total".into(), Value::from(tot_oov));
    let coverage = if tot_tokens > 0 { round_to(1.0 - tot_oov as f64 / tot_tokens as f64, 4) } else { 0.0 };
    meta.insert("coverage".into(), Value::from(coverage));
    meta.insert("with_text".into(), Value::from(!sem_texto));
    meta.insert("generator".into(), Value::from("embed_gen (rust)"));
    meta.insert("tokens_file".into(), Value::from(basename(&tokens)));
    meta.insert("built_at".into(), Value::from(iso_utc_now()));
    meta.insert("vocab".into(), Value::from(vocab.clone()));

    let mut idf_keys: Vec<usize> = idf.keys().copied().collect();
    idf_keys.sort_unstable();
    let mut idf_map = Map::new();
    for d in &idf_keys {
        idf_map.insert(d.to_string(), Value::from(round_to(idf[d], 6)));
    }

    let chunks: Vec<Value> = recs.iter().map(|r| {
        let mut o = Map::new();
        o.insert("id".into(), Value::from(r.id));
        o.insert("start".into(), Value::from(r.start));
        o.insert("len".into(), Value::from(r.len));
        o.insert("tokens".into(), Value::from(r.tokens));
        o.insert("oov".into(), Value::from(r.oov));
        let mut dims: Vec<usize> = r.tf.keys().copied().collect();
        dims.sort_unstable();
        let mut vec_map = Map::new();
        for d in &dims { vec_map.insert(d.to_string(), Value::from(r.tf[d])); }
        o.insert("vec".into(), Value::Object(vec_map));
        o.insert("text".into(), match &r.text { Some(t) => Value::from(t.clone()), None => Value::Null });
        o.insert("norm".into(), Value::from(round_to(tfidf_norm(&r.tf, &idf), 6)));
        Value::Object(o)
    }).collect();

    let mut root = Map::new();
    root.insert("meta".into(), Value::Object(meta));
    root.insert("idf".into(), Value::Object(idf_map));
    root.insert("chunks".into(), Value::from(chunks));
    let root = Value::Object(root);

    // grava (TAB indent, UTF-8 sem escape ascii — igual ao Python ensure_ascii=False)
    let out = if compacto {
        serde_json::to_string(&root).unwrap()
    } else {
        let mut buf = Vec::new();
        let fmt = serde_json::ser::PrettyFormatter::with_indent(b"\t");
        let mut ser = serde_json::Serializer::with_formatter(&mut buf, fmt);
        use serde::Serialize;
        root.serialize(&mut ser).unwrap();
        String::from_utf8(buf).unwrap()
    };
    std::fs::write(&saida, &out).unwrap();
    let dt = t0.elapsed().as_secs_f64();
    let size = out.len();

    println!("== EMBEDDINGS GERADOS (rust) ==");
    println!("corpus:    {}  ({} bytes)", corpus, commas(text.len()));
    println!("chunks:    {}  (~{} B cada)  em {:.2}s", pieces.len(), chunk, dt);
    println!("tokens:    {}  ->  {} dims ({} usadas)", basename(&tokens), vocab.len(), idf.len());
    println!("silabas:   {}  (OOV {}  ->  cobertura {:.2}%)",
             commas(tot_tokens), commas(tot_oov), coverage * 100.0);
    println!("saida:     {}  ({} bytes, {} texto)",
             saida, commas(size), if sem_texto { "sem" } else { "com" });
}
