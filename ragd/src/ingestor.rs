//! ingestor — tokeniza arquivo bruto em JSON de base RAG (mesmo schema do
//! python_concept/embed_gen.py + 3 campos novos no meta: source_file, language,
//! matched_by_ext). Usado pelas rotas POST /ingest (modo raw) e POST /ingest_file.
//!
//! Resolver de driver por extensao: le os campos `# extensoes:` de cada .drv da
//! pasta drivers/ e monta o mapa ext->driver (mesma logica do GET /interpret).
//! Sem match -> fallback PTBR (texto literario).
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
use serde_json::{json, Map, Value};
use crate::{vocab, vector, chunk as syl_chunk};

pub const FALLBACK_DRIVER: &str = "tokens_PTBR.drv";
pub const FALLBACK_LANG: &str = "PTBR";

/// Resultado da escolha de driver pra um arquivo.
pub struct DriverPick {
    pub driver_file: String,    // ex: "tokens_Python_PTBR.drv"
    pub language: String,       // ex: "Python"
    pub matched_by_ext: bool,   // false = fallback PTBR
}

/// Indice ext->(driver_file, language) carregado dos .drv da pasta.
pub struct DriverIndex {
    pub by_ext: HashMap<String, (String, String)>,
    pub drivers_dir: PathBuf,
}

pub fn build_driver_index(drivers_dir: &Path) -> std::io::Result<DriverIndex> {
    let mut entries: Vec<PathBuf> = std::fs::read_dir(drivers_dir)?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().and_then(|x| x.to_str()) == Some("drv"))
        .collect();
    entries.sort();
    let mut by_ext: HashMap<String, (String, String)> = HashMap::new();
    for path in &entries {
        let fname = path.file_name().and_then(|x| x.to_str()).unwrap_or("").to_string();
        let lang = driver_language(&fname);
        let meta = vocab::read_meta(path.to_str().unwrap_or("")).unwrap_or_default();
        for ext in meta.extensions {
            by_ext.insert(ext.to_lowercase(), (fname.clone(), lang.clone()));
        }
    }
    Ok(DriverIndex { by_ext, drivers_dir: drivers_dir.to_path_buf() })
}

/// "tokens_<Lang>_PTBR.drv" -> "<Lang>"; "tokens_PTBR.drv" -> "PTBR".
pub fn driver_language(fname: &str) -> String {
    let stem = fname.strip_suffix(".drv").unwrap_or(fname);
    let stem = stem.strip_prefix("tokens_").unwrap_or(stem);
    stem.strip_suffix("_PTBR").unwrap_or(stem).to_string()
}

pub fn file_extension(path: &Path) -> Option<String> {
    let name = path.file_name()?.to_str()?;
    let dot = name.rfind('.')?;
    if dot == 0 { return None; }
    Some(name[dot..].to_lowercase())
}

/// Escolhe driver por extensao (com fallback PTBR).
pub fn pick_driver(path: &Path, idx: &DriverIndex) -> DriverPick {
    if let Some(ext) = file_extension(path) {
        if let Some((d, l)) = idx.by_ext.get(&ext) {
            return DriverPick {
                driver_file: d.clone(),
                language: l.clone(),
                matched_by_ext: true,
            };
        }
    }
    DriverPick {
        driver_file: FALLBACK_DRIVER.to_string(),
        language: FALLBACK_LANG.to_string(),
        matched_by_ext: false,
    }
}

/// Nome derivado do path pra usar como `base name` quando o cliente nao passar.
/// ./logic_path/01_foo.py -> logic_path__01_foo_py
pub fn derive_base_name(path: &Path) -> String {
    let parts: Vec<String> = path.iter()
        .map(|c| c.to_string_lossy().to_string())
        .filter(|s| s != "." && !s.is_empty() && s != "/")
        .collect();
    let joined = parts.join("__");
    match joined.rfind('.') {
        Some(i) if i > 0 => format!("{}_{}", &joined[..i], &joined[i+1..]),
        _ => joined,
    }
}

fn now_iso() -> String {
    let secs = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let (y, mo, d, h, mi, s) = epoch_to_ymdhms(secs);
    format!("{:04}-{:02}-{:02}T{:02}:{:02}:{:02}", y, mo, d, h, mi, s)
}

fn epoch_to_ymdhms(secs: u64) -> (i32, u32, u32, u32, u32, u32) {
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    let h = (rem / 3600) as u32;
    let mi = ((rem % 3600) / 60) as u32;
    let s = (rem % 60) as u32;
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe/1460 + doe/36524 - doe/146096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365*yoe + yoe/4 - yoe/100);
    let mp = (5*doy + 2)/153;
    let d = (doy - (153*mp + 2)/5 + 1) as u32;
    let mo = (if mp < 10 { mp + 3 } else { mp - 9 }) as u32;
    let y = (y + if mo <= 2 { 1 } else { 0 }) as i32;
    (y, mo, d, h, mi, s)
}

/// Tokeniza CONTEUDO em memoria (sem ler arquivo). Usado pelo upload HTTP.
///
/// Args:
///   text          — conteudo bruto
///   filename      — nome do arquivo de origem (ex "foo.py"); usado pra
///                   pick driver pela extensao e gravado em meta.corpus/source_file
///   source_label  — etiqueta pra meta.source_file (ex caminho absoluto, ou
///                   "<upload>" quando veio pela rede e nao existe local)
///   drivers_dir/driver_override/chunk_size/max_chunks/with_text — iguais ao tokenize_file
pub fn tokenize_content(
    text: &str,
    filename: &str,
    source_label: &str,
    drivers_dir: &Path,
    driver_override: Option<&str>,
    chunk_size: usize,
    max_chunks: usize,
    with_text: bool,
    file: Option<&str>,   // [#8] caminho do arquivo no repo; marca cada chunk + meta.files (None = base 1-arquivo)
) -> Result<Value, String> {
    let idx = build_driver_index(drivers_dir)
        .map_err(|e| format!("erro lendo drivers em {}: {e}", drivers_dir.display()))?;
    let pick = match driver_override {
        Some(forced) => DriverPick {
            driver_file: forced.to_string(),
            language: driver_language(forced),
            matched_by_ext: false,
        },
        None => pick_driver(Path::new(filename), &idx),
    };

    let driver_path = drivers_dir.join(&pick.driver_file);
    let (vocab_list, index_map) = vocab::load_vocab(driver_path.to_str().unwrap_or(""))
        .map_err(|e| format!("erro lendo driver {}: {e}", pick.driver_file))?;

    let chars: Vec<char> = text.chars().collect();
    let mut pieces = syl_chunk::chunk_text(&chars, chunk_size);
    if max_chunks > 0 && pieces.len() > max_chunks { pieces.truncate(max_chunks); }

    let mut chunks_json: Vec<Map<String, Value>> = Vec::with_capacity(pieces.len());
    let mut tfs: Vec<HashMap<usize, u32>> = Vec::with_capacity(pieces.len());
    let mut tot_tokens = 0usize;
    let mut tot_oov = 0usize;
    let mut cursor = 0usize;
    for (cid, piece) in pieces.iter().enumerate() {
        let pc: Vec<char> = piece.chars().collect();
        let start = syl_chunk::find_chars(&chars, &pc, cursor).unwrap_or(cursor);
        let len_chars = piece.chars().count();
        cursor = start + len_chars;
        let (tf, total, oov) = vector::histogram(piece, &index_map);
        tot_tokens += total; tot_oov += oov;
        tfs.push(tf.clone());
        let mut m = Map::new();
        m.insert("id".into(), json!(cid));
        m.insert("start".into(), json!(start));
        m.insert("len".into(), json!(len_chars));
        m.insert("tokens".into(), json!(total));
        m.insert("oov".into(), json!(oov));
        if let Some(f) = file { m.insert("file".into(), json!(f)); }   // [#8] só no modo repo
        m.insert("vec".into(), Value::Null);                 // preenchido na 2a passada
        m.insert("text".into(), if with_text { json!(piece) } else { Value::Null });
        chunks_json.push(m);
    }

    let idf = vector::compute_idf(&tfs, pieces.len());
    for (i, ch) in chunks_json.iter_mut().enumerate() {
        let tf = &tfs[i];
        let norm = vector::tfidf_norm(tf, &idf);
        ch.insert("norm".into(), json!((norm * 1_000_000.0).round() / 1_000_000.0));
        let mut keys: Vec<usize> = tf.keys().copied().collect();
        keys.sort();
        let mut vec_map = Map::new();
        for d in keys { vec_map.insert(d.to_string(), json!(tf[&d])); }
        ch.insert("vec".into(), Value::Object(vec_map));
    }

    let coverage = if tot_tokens > 0 {
        let c = 1.0 - (tot_oov as f64 / tot_tokens as f64);
        (c * 10_000.0).round() / 10_000.0
    } else { 0.0 };
    let vocab_used = idf.len();

    let mut meta = Map::new();
    meta.insert("corpus".into(), json!(filename));
    meta.insert("source_file".into(), json!(source_label));
    if let Some(f) = file {                                   // [#8] mapa arquivo→chunks (formato extensível)
        let ids: Vec<Value> = (0..pieces.len()).map(|i| json!(i)).collect();
        let mut entry = Map::new();
        entry.insert("chunks".into(), Value::Array(ids));
        let mut fmap = Map::new();
        fmap.insert(f.to_string(), Value::Object(entry));
        meta.insert("files".into(), Value::Object(fmap));
    }
    meta.insert("bytes".into(), json!(text.len()));
    meta.insert("chunk_size".into(), json!(chunk_size));
    meta.insert("n_chunks".into(), json!(pieces.len()));
    meta.insert("vocab_size".into(), json!(vocab_list.len()));
    meta.insert("vocab_used".into(), json!(vocab_used));
    meta.insert("tokens_total".into(), json!(tot_tokens));
    meta.insert("oov_total".into(), json!(tot_oov));
    meta.insert("coverage".into(), json!(coverage));
    meta.insert("with_text".into(), json!(with_text));
    meta.insert("generator".into(), json!("ragd-ingest"));
    meta.insert("tokens_file".into(), json!(pick.driver_file));
    meta.insert("language".into(), json!(pick.language));
    meta.insert("matched_by_ext".into(), json!(pick.matched_by_ext));
    meta.insert("built_at".into(), json!(now_iso()));
    meta.insert("vocab".into(), Value::Array(vocab_list.iter().map(|s| json!(s)).collect()));

    let mut idf_keys: Vec<usize> = idf.keys().copied().collect();
    idf_keys.sort();
    let mut idf_map = Map::new();
    for d in idf_keys {
        let v = (idf[&d] * 1_000_000.0).round() / 1_000_000.0;
        idf_map.insert(d.to_string(), json!(v));
    }

    Ok(json!({
        "meta": meta,
        "idf": idf_map,
        "chunks": chunks_json.into_iter().map(Value::Object).collect::<Vec<_>>(),
    }))
}

/// Append INCREMENTAL com CHUNK-PACKING: junta `new_text` a uma base existente
/// "enchendo" o ULTIMO chunk (geralmente parcial) ate `chunk_size` e transbordando o
/// excedente pra chunk(s) novo(s). Assim os chunks crescem ordenados e cheios, em vez de
/// virar muitos chunks pequenos (e com N>1 o idf passa a discriminar de verdade).
/// Os chunks anteriores ao ultimo ficam intactos (reusam o `vec`); so o "rabo" (ultimo
/// chunk + texto novo) e' re-tokenizado. Recomputa `idf` (suavizado) e `norm` de todos.
/// Driver/chunk_size/with_text herdados do meta. Precisa do `text` do ultimo chunk
/// (with_text); sem ele, fallback = anexa o texto novo como chunk(s) apos o fim.
pub fn tokenize_content_append(
    existing: &Value,
    new_text: &str,
    source_label: &str,
    drivers_dir: &Path,
) -> Result<Value, String> {
    let meta = existing.get("meta").and_then(|m| m.as_object())
        .ok_or("base existente sem 'meta'")?;
    let driver_file = meta.get("tokens_file").and_then(|v| v.as_str())
        .unwrap_or(FALLBACK_DRIVER).to_string();
    let chunk_size = meta.get("chunk_size").and_then(|v| v.as_u64()).unwrap_or(2048) as usize;
    let with_text = meta.get("with_text").and_then(|v| v.as_bool()).unwrap_or(true);

    // mesmo driver/vocab da base — indispensavel pro alinhamento de dimensoes
    let driver_path = drivers_dir.join(&driver_file);
    let (vocab_list, index_map) = vocab::load_vocab(driver_path.to_str().unwrap_or(""))
        .map_err(|e| format!("erro lendo driver {driver_file}: {e}"))?;

    let existing_chunks = existing.get("chunks").and_then(|c| c.as_array())
        .ok_or("base existente sem 'chunks'")?;

    // PACKING: reembrulha o ULTIMO chunk junto com o texto novo. So da' se o texto do
    // ultimo chunk existir (with_text); senao, fallback = texto novo apos o fim.
    let last_text: Option<String> = if with_text {
        existing_chunks.last()
            .and_then(|c| c.get("text"))
            .and_then(|t| t.as_str())
            .map(|s| s.to_string())
    } else { None };

    // `kept` = chunks inalterados (reusam vec); `tail_text` = o que sera re-chunkado;
    // `tail_start` = offset (em chars) onde o rabo comeca no corpus.
    let (kept, tail_text, tail_start): (Vec<&Value>, String, usize) = match &last_text {
        Some(lt) => {
            let last = existing_chunks.last().unwrap();
            let last_start = last.get("start").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
            let kept = existing_chunks[..existing_chunks.len() - 1].iter().collect();
            (kept, format!("{lt}{new_text}"), last_start)
        }
        None => {
            let max_end = existing_chunks.iter().map(|c| {
                let s = c.get("start").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
                let l = c.get("len").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
                s + l
            }).max().unwrap_or(0);
            (existing_chunks.iter().collect(), new_text.to_string(), max_end)
        }
    };

    let mut chunks_json: Vec<Map<String, Value>> = Vec::with_capacity(kept.len() + 4);
    let mut tfs: Vec<HashMap<usize, u32>> = Vec::with_capacity(kept.len() + 4);
    let mut next_id: i64 = 0;
    let mut prev_tokens = 0usize;
    let mut prev_oov = 0usize;

    // 1. chunks mantidos: reusa vec (tf), renumera id sequencial
    for ch in &kept {
        let obj = ch.as_object().ok_or("chunk existente invalido")?;
        let mut tf: HashMap<usize, u32> = HashMap::new();
        if let Some(vec) = obj.get("vec").and_then(|v| v.as_object()) {
            for (k, v) in vec {
                if let (Ok(d), Some(c)) = (k.parse::<usize>(), v.as_u64()) {
                    tf.insert(d, c as u32);
                }
            }
        }
        prev_tokens += obj.get("tokens").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
        prev_oov += obj.get("oov").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
        let mut m = obj.clone();
        m.insert("id".into(), json!(next_id));
        tfs.push(tf);
        chunks_json.push(m);
        next_id += 1;
    }

    // 2. "rabo" (ultimo chunk + texto novo) re-chunkado em pedacos de chunk_size
    let chars: Vec<char> = tail_text.chars().collect();
    let pieces = syl_chunk::chunk_text(&chars, chunk_size);
    let mut tail_tokens = 0usize;
    let mut tail_oov = 0usize;
    let mut cursor = 0usize;
    for piece in &pieces {
        let pc: Vec<char> = piece.chars().collect();
        let local = syl_chunk::find_chars(&chars, &pc, cursor).unwrap_or(cursor);
        let len_chars = piece.chars().count();
        cursor = local + len_chars;
        let (tf, total, oov) = vector::histogram(piece, &index_map);
        tail_tokens += total; tail_oov += oov;
        let mut m = Map::new();
        m.insert("id".into(), json!(next_id));
        m.insert("start".into(), json!(tail_start + local));
        m.insert("len".into(), json!(len_chars));
        m.insert("tokens".into(), json!(total));
        m.insert("oov".into(), json!(oov));
        m.insert("vec".into(), Value::Null);                 // preenchido abaixo
        m.insert("text".into(), if with_text { json!(piece) } else { Value::Null });
        tfs.push(tf);
        chunks_json.push(m);
        next_id += 1;
    }

    // 3. idf suavizado + norm de TODOS; escreve vec so dos chunks novos (rabo)
    let total_n = chunks_json.len();
    let idf = vector::compute_idf(&tfs, total_n);
    for (i, ch) in chunks_json.iter_mut().enumerate() {
        let tf = &tfs[i];
        let norm = vector::tfidf_norm(tf, &idf);
        ch.insert("norm".into(), json!((norm * 1_000_000.0).round() / 1_000_000.0));
        if ch.get("vec").map(|v| v.is_null()).unwrap_or(true) {
            let mut keys: Vec<usize> = tf.keys().copied().collect();
            keys.sort();
            let mut vec_map = Map::new();
            for d in keys { vec_map.insert(d.to_string(), json!(tf[&d])); }
            ch.insert("vec".into(), Value::Object(vec_map));
        }
    }

    // 4. meta acumulado (tokens/oov = mantidos + rabo; o rabo ja inclui o antigo ultimo)
    let tot_tokens = prev_tokens + tail_tokens;
    let tot_oov = prev_oov + tail_oov;
    let coverage = if tot_tokens > 0 {
        let c = 1.0 - (tot_oov as f64 / tot_tokens as f64);
        (c * 10_000.0).round() / 10_000.0
    } else { 0.0 };
    let prev_bytes = meta.get("bytes").and_then(|v| v.as_u64()).unwrap_or(0) as usize;

    let mut meta_new = meta.clone();
    meta_new.insert("bytes".into(), json!(prev_bytes + new_text.len()));
    meta_new.insert("n_chunks".into(), json!(total_n));
    meta_new.insert("tokens_total".into(), json!(tot_tokens));
    meta_new.insert("oov_total".into(), json!(tot_oov));
    meta_new.insert("coverage".into(), json!(coverage));
    meta_new.insert("vocab_used".into(), json!(idf.len()));
    meta_new.insert("vocab_size".into(), json!(vocab_list.len()));
    meta_new.insert("built_at".into(), json!(now_iso()));
    meta_new.insert("source_file".into(), json!(source_label));

    let mut idf_keys: Vec<usize> = idf.keys().copied().collect();
    idf_keys.sort();
    let mut idf_map = Map::new();
    for d in idf_keys {
        let v = (idf[&d] * 1_000_000.0).round() / 1_000_000.0;
        idf_map.insert(d.to_string(), json!(v));
    }

    Ok(json!({
        "meta": meta_new,
        "idf": idf_map,
        "chunks": chunks_json.into_iter().map(Value::Object).collect::<Vec<_>>(),
    }))
}

/// Le um arquivo do disco e tokeniza. Wrapper de tokenize_content que resolve
/// source_label como o caminho absoluto e filename como o basename.
pub fn tokenize_file(
    path: &Path,
    drivers_dir: &Path,
    driver_override: Option<&str>,
    chunk_size: usize,
    max_chunks: usize,
    with_text: bool,
    file: Option<&str>,   // [#8] repassado pro tokenize_content
) -> Result<Value, String> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| format!("erro lendo {}: {e}", path.display()))?;
    let filename = path.file_name().and_then(|x| x.to_str()).unwrap_or("");
    let abs = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    tokenize_content(
        &text, filename, &abs.to_string_lossy(),
        drivers_dir, driver_override, chunk_size, max_chunks, with_text, file,
    )
}
