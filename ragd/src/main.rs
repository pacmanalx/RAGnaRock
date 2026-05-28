//! ragd — RAGnaRock daemon. Segura N bases RAG em memoria e atende busca/ingest
//! via HTTP JSON (curl, MCP, ...). Reusa a sylkit interna (src/lib.rs).
#![allow(dead_code, unused_imports)]

use std::collections::{HashMap, HashSet};
use std::io::Read;
use std::path::Path;
use std::sync::Arc;
use parking_lot::{Mutex, RwLock};   // [#6] sem PoisonError, fair, sem .unwrap() em locks
use std::thread;
use std::time::Instant;
use rayon::prelude::*;
use serde_json::{json, Map, Value};
use tiny_http::{Header, Method, Request, Response, Server};
use ragd::{vocab, rag, ingestor, multipart, tokenizer};
use ragd::rag::RagBase;
use ragd::ingestor::DriverPick;

const DEFAULT_PORT: u16 = 11499;            // API
const DEFAULT_DASH_PORT: u16 = 11498;       // dashboard / supervisório
const DEFAULT_DRIVERS_DIR: &str = "drivers";
const DEFAULT_THESAURUS_DIR: &str = "thesaurus";
const DEFAULT_RAGFILES_DIR: &str = "ragfiles";
const DEFAULT_MAX_UPLOAD: usize = 1024 * 1024 * 1024;   // 1 GB
const VERSION: &str = env!("CARGO_PKG_VERSION");
/// Offset de fuso (horas) pros timestamps do log. Default -3 (Brasil). Atômico p/ ler sem lock.
static LOG_OFFSET: std::sync::atomic::AtomicI64 = std::sync::atomic::AtomicI64::new(-3);
/// Caminhos onde procuramos ragnarock.cfg se --config não for passado.
const CONFIG_PATHS: [&str; 2] = ["/etc/ragnarock/ragnarock.cfg", "ragnarock.cfg"];
/// Página do dashboard embutida no binário (servida na porta de controle).
const DASHBOARD_HTML: &str = include_str!("dashboard.html");
const DEFAULT_COLLECTION: &str = "default";

/// Saneia o nome da base p/ virar arquivo seguro. Tira ponto(s) inicial(is): senão o
/// `<name>-tokenized.json` vira dotfile e SOME do glob de preload `*/*-tokenized.json`
/// (foi o caso dos `.vscode/` na ingestão do Eduxe_Microservices).
fn safe_name(n: &str) -> String {
    let s = n.trim_start_matches('.').trim();
    if s.is_empty() { "base".to_string() } else { s.to_string() }
}

/// Mapa de bases agrupadas por coleção: collection -> name -> RagBase.
type Bases = HashMap<String, HashMap<String, RagBase>>;

/// Retorna true se user/pass são as credenciais padrão não alteradas.
fn is_default_creds(user: &str, pass: &str) -> bool {
    user == "admin" && pass == "admin"
}

/// Estado compartilhado entre as rotas (API + dashboard, via Arc<RwLock<State>>).
/// [#6] **Outer lock = parking_lot::RwLock<State>** (fair, sem PoisonError):
///   - rotas read-only (search/chunk/list/health/profile/stats) pegam `.read()` → N paralelas;
///   - rotas write (ingest/delete/config/login/toggle) pegam `.write()` → exclusivo.
/// **Interior mutability** em `collection_profiles` e `expansions`: são caches que `search` e
/// `search_expand` precisam mutar mesmo sob outer-read — `Mutex` interno permite isso sem
/// pedir outer-write (que serializaria com as outras searches em paralelo).
struct State {
    bases: Bases,
    drivers_dir: String,
    ragfiles_dir: String,
    max_upload: usize,
    started: Instant,
    admin_user: String,
    admin_pass: String,
    dev: bool,         // true = aceita admin/admin; false = recusa credenciais padrão
    log_file: String,
    config_path: String,        // cfg de onde lemos / onde persistimos mudanças do painel
    anthropic_key: String,      // chave p/ acoplar Claude (vazio = não cadastrada)
    openai_key: String,         // chave p/ acoplar OpenAI/Codex
    active_provider: String,    // "none" | "anthropic" | "openai" — SÓ UM ativo por vez
    cache_dir: String,          // pasta do cache por-QUERY (sinônimos consultados antes da IA)
    expansions: RwLock<HashMap<String, Vec<String>>>, // [#6] interior mut RW: search_expand cacheia sob outer-read; N readers
    thesaurus_dir: String,      // pasta dos dicionários por-PALAVRA (subdir/CODE com inuse.flag)
    word_syn: HashMap<String, Vec<String>>,   // palavra -> sinônimos (união dos dicionários ATIVOS)
    nidhogg_url: String,        // URL do daemon de módulos (nidhoggd:11497) — só pro proxy do console
    sessions: HashMap<String, Instant>,   // token de sessão -> criado em (TTL via SESSION_TTL)
    collection_profiles: RwLock<HashMap<String, rag::CollectionProfile>>, // [#6] interior mut RW: search cacheia sob outer-read; N readers
}

/// Configuração do daemon. Vem de ragnarock.cfg (chave = valor) e/ou CLI (CLI vence).
struct Config {
    api_port: u16,
    dash_port: u16,
    drivers_dir: String,
    ragfiles_dir: String,
    max_upload: usize,
    autoload: bool,
    admin_user: String,
    admin_pass: String,
    log_file: String,
    log_utc_offset: i64,
    storage: String,   // "memory" (default) | "hybrid"
    anthropic_key: String,
    openai_key: String,
    active_provider: String,
    cache_dir: String,
    thesaurus_dir: String,
    nidhogg_url: String,
    dev: bool,         // --dev: aceita credenciais padrão admin/admin (só pra desenvolvimento)
}
impl Default for Config {
    fn default() -> Self {
        Config {
            api_port: DEFAULT_PORT, dash_port: DEFAULT_DASH_PORT,
            drivers_dir: DEFAULT_DRIVERS_DIR.to_string(),
            ragfiles_dir: DEFAULT_RAGFILES_DIR.to_string(),
            max_upload: DEFAULT_MAX_UPLOAD, autoload: true,
            admin_user: "admin".to_string(), admin_pass: "admin".to_string(),
            log_file: "/tmp/ragd-all.log".to_string(),
            log_utc_offset: -3,
            storage: "memory".to_string(),
            anthropic_key: String::new(),
            openai_key: String::new(),
            active_provider: "none".to_string(),
            cache_dir: "cache".to_string(),
            thesaurus_dir: DEFAULT_THESAURUS_DIR.to_string(),
            nidhogg_url: "http://127.0.0.1:11497".to_string(),
            dev: false,
        }
    }
}

/// Lê ragnarock.cfg: linhas `chave = valor`, `#` comenta. Chaves desconhecidas: aviso.
fn load_config_file(cfg: &mut Config, path: &str) {
    let txt = match std::fs::read_to_string(path) {
        Ok(t) => t, Err(e) => { eprintln!("config: não li {path:?}: {e}"); return; }
    };
    for (lineno, raw) in txt.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') { continue; }
        let (k, vraw) = match line.split_once('=') {
            Some((k, v)) => (k.trim(), v),
            None => { eprintln!("config:{}: linha sem '=': {line:?}", lineno + 1); continue; }
        };
        // tira comentário inline (apenas '#' precedido de espaço, p/ preservar ex. senha "#x")
        let v = match vraw.find(" #").or_else(|| vraw.find("\t#")) {
            Some(p) => vraw[..p].trim(), None => vraw.trim(),
        };
        match k {
            "api_port"    => if let Ok(p) = v.parse() { cfg.api_port = p },
            "dash_port"   => if let Ok(p) = v.parse() { cfg.dash_port = p },
            "drivers_dir" => cfg.drivers_dir = v.to_string(),
            "ragfiles_dir"=> cfg.ragfiles_dir = v.to_string(),
            "max_upload"  => if let Ok(n) = v.parse() { cfg.max_upload = n },
            "autoload"    => cfg.autoload = matches!(v, "true" | "1" | "yes" | "on"),
            "admin_user"  => cfg.admin_user = v.to_string(),
            "admin_pass"  => cfg.admin_pass = v.to_string(),
            "log_file"    => cfg.log_file = v.to_string(),
            "log_utc_offset" => if let Ok(n) = v.parse() { cfg.log_utc_offset = n },
            "storage"     => cfg.storage = v.to_string(),
            "anthropic_key" => cfg.anthropic_key = v.to_string(),
            "openai_key"  => cfg.openai_key = v.to_string(),
            "active_provider" => cfg.active_provider = v.to_string(),
            "cache_dir"   => cfg.cache_dir = v.to_string(),
            "thesaurus_dir" => cfg.thesaurus_dir = v.to_string(),
            "nidhogg_url" => cfg.nidhogg_url = v.to_string(),
            other => eprintln!("config:{}: chave desconhecida {other:?}", lineno + 1),
        }
    }
    println!("config: carregada de {path:?}");
}

fn total_chunks(b: &Bases) -> usize {
    b.values().flat_map(|m| m.values()).map(|x| x.n_chunks).sum()
}

/// RSS do processo em MB (Linux: /proc/self/statm, campo 2 = páginas residentes).
fn read_rss_mb() -> Option<f64> {
    let s = std::fs::read_to_string("/proc/self/statm").ok()?;
    let resident: f64 = s.split_whitespace().nth(1)?.parse().ok()?;
    Some(resident * 4096.0 / 1_048_576.0)   // assume página de 4K
}

/// Memória do sistema (MemTotal, MemAvailable) em MB via /proc/meminfo (Linux).
fn read_sys_mem_mb() -> (Option<f64>, Option<f64>) {
    let s = match std::fs::read_to_string("/proc/meminfo") { Ok(s) => s, Err(_) => return (None, None) };
    let get = |k: &str| s.lines().find(|l| l.starts_with(k))
        .and_then(|l| l.split_whitespace().nth(1)).and_then(|v| v.parse::<f64>().ok()).map(|kb| kb / 1024.0);
    (get("MemTotal:"), get("MemAvailable:"))
}

/// Pressão de memória: RSS real (Linux) + memória do sistema + estimativa de onde o
/// RAG gasta RAM (texto dos chunks, vetores esparsos, tokens cacheados em `words`).
fn mem_stats(b: &Bases) -> Value {
    let (mut text, mut vec_entries, mut word_tokens) = (0usize, 0usize, 0usize);
    for m in b.values() { for base in m.values() { for c in &base.chunks {
        text += c.text.as_ref().map(|t| t.len()).unwrap_or(0);
        vec_entries += c.vec.len();
        word_tokens += c.words.iter().map(|w| w.len()).sum::<usize>();
    }}}
    let mb = |bytes: usize| bytes as f64 / 1_048_576.0;
    let (sys_total, sys_avail) = read_sys_mem_mb();
    let hybrid = !rag::CACHE_WORDS.load(std::sync::atomic::Ordering::Relaxed);
    json!({
        "storage": if hybrid { "hybrid" } else { "memory" },
        "rss_mb": read_rss_mb(),
        "sys_total_mb": sys_total,
        "sys_avail_mb": sys_avail,
        // estimativa grosseira por estrutura (texto domina; vec ~16B/entrada; words ~24B/token)
        "est_text_mb": mb(text),
        "est_vec_mb": mb(vec_entries * 16),
        "est_words_mb": mb(word_tokens * 24),
    })
}

/// Data civil (ano, mês, dia) a partir de dias desde a época Unix (algoritmo de Hinnant).
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719468;
    let era = (if z >= 0 { z } else { z - 146096 }) / 146097;
    let doe = (z - era * 146097) as i64;            // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;   // [0, 399]
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);  // [0, 365]
    let mp = (5 * doy + 2) / 153;                    // [0, 11]
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;   // [1, 31]
    let m = (if mp < 10 { mp + 3 } else { mp - 9 }) as u32;
    (y + if m <= 2 { 1 } else { 0 }, m, d)
}

/// Timestamp "YYYY-MM-DD HH:MM:SS" no fuso de LOG_OFFSET (sem dependência externa).
fn now_stamp() -> String {
    let off = LOG_OFFSET.load(std::sync::atomic::Ordering::Relaxed);
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64).unwrap_or(0) + off * 3600;
    let (days, tod) = (secs.div_euclid(86400), secs.rem_euclid(86400));
    let (y, mo, d) = civil_from_days(days);
    format!("{y:04}-{mo:02}-{d:02} {:02}:{:02}:{:02}", tod / 3600, (tod % 3600) / 60, tod % 60)
}

/// Detalhe por rota pro log (query+hits numa busca, arquivo+chunks numa ingestão, etc).
fn req_extra(path: &str, body: &[u8], payload: &str) -> String {
    let pj: Value = serde_json::from_str(payload).unwrap_or(Value::Null);
    let bj = || serde_json::from_slice::<Value>(body).unwrap_or(Value::Null);
    // Erros têm prioridade — sem o err o resto vira lixo (campos vazios).
    if let Some(err) = pj["error"].as_str() { return format!("err={err}"); }

    // ── busca ──
    if path.ends_with("search_expand") {
        let q = bj()["query"].as_str().unwrap_or("").to_string();
        let hits = pj["hits"].as_array().map(|a| a.len()).unwrap_or(0);
        let dropped = pj["dropped"].as_array().map(|a| a.len()).unwrap_or(0);
        let abs = if pj["absent"].as_bool().unwrap_or(false) { " AUSENTE" } else { "" };
        return format!("q={q:?} source={} hits={hits} dropped={dropped}{abs}",
                       pj["source"].as_str().unwrap_or("?"));
    }
    if path.ends_with("/search") {
        let b = bj();
        let q = b["query"].as_str().unwrap_or("").to_string();
        let unified = b["unified"].as_bool().unwrap_or(false);
        let hits = pj["hits"].as_array().map(|a| a.len()).unwrap_or(0);
        let scope = pj["scope"].as_array().map(|a| a.len()).unwrap_or(0);
        let conv: u64 = pj["searched"].as_array()
            .map(|a| a.iter().map(|x| x["n_converge"].as_u64().unwrap_or(0)).sum()).unwrap_or(0);
        return format!("q={q:?} hits={hits} scope={scope} convergem={conv}{}", if unified { " unified" } else { "" });
    }

    // ── ingestão (todas as variantes: /ingest, /ingest_file, /ingest_upload) ──
    if path.contains("ingest") {
        let coll = pj["collection"].as_str().unwrap_or("?");
        let name = pj["name"].as_str().unwrap_or("?");
        let chunks = pj["n_chunks"].as_u64().unwrap_or(0);
        let mut s = format!("{coll}/{name} chunks={chunks}");
        if let Some(b) = pj["bytes"].as_u64() { s.push_str(&format!(" bytes={b}")); }
        if let Some(v) = pj["via"].as_str() { s.push_str(&format!(" via={v}")); }
        if pj["appended"].as_bool().unwrap_or(false) { s.push_str(" append"); }
        if let Some(b) = pj["bases"].as_u64() { s.push_str(&format!(" total_bases={b}")); }
        return s;
    }

    // ── chunk lookup ──
    if path.ends_with("/chunk") {
        let b = bj();
        let base = b["base"].as_str().unwrap_or("?");
        let target = if b["ids"].is_array() {
            format!("ids={}", b["ids"].as_array().map(|a| a.len()).unwrap_or(0))
        } else { format!("id={}", b["id"]) };
        let before = b["before"].as_u64().unwrap_or(0);
        let after = b["after"].as_u64().unwrap_or(0);
        let win = if before > 0 || after > 0 { format!(" before={before} after={after}") } else { String::new() };
        let returned = pj["chunks"].as_array().map(|a| a.len()).unwrap_or(
            if pj["chunk"].is_object() { 1 } else { 0 });
        return format!("base={base} {target}{win} -> {returned}");
    }

    // ── dashboard / inspeção ──
    if path.ends_with("/histogram") {
        return format!("q={:?} -> {}/{}", bj()["query"].as_str().unwrap_or(""),
                       pj["base"].as_str().unwrap_or("?"), pj["chunk_id"]);
    }

    // ── listagens ──
    if path.ends_with("/collections") {
        return format!("coleções={} bases_total={}",
                       pj["count"].as_u64().unwrap_or(0),
                       pj["total_bases"].as_u64().unwrap_or(0));
    }
    if path.ends_with("/bases") {
        return format!("escopo={}/{} -> {}",
                       pj["collection"].as_str().unwrap_or("*"),
                       pj["match"].as_str().unwrap_or("*"),
                       pj["count"].as_u64().unwrap_or(0));
    }
    if path.ends_with("/drivers") || path.ends_with("/drivers_out") {
        let c = pj["count"].as_u64().unwrap_or_else(|| pj["drivers"].as_array().map(|a| a.len() as u64).unwrap_or(0));
        return format!("count={c}");
    }
    if path.ends_with("/thesaurus") {
        return format!("dicts={} ativos={}",
                       pj["count"].as_u64().unwrap_or(0),
                       pj["active"].as_u64().unwrap_or(0));
    }
    if path.ends_with("/interpret") {
        return format!("{} -> {} ({})",
                       pj["file"].as_str().or(pj["ext"].as_str()).unwrap_or("?"),
                       pj["driver"].as_str().unwrap_or("?"),
                       pj["language"].as_str().unwrap_or("?"));
    }

    // ── ações ──
    if path.ends_with("driver_move") {
        return format!("{} {}", pj["action"].as_str().unwrap_or("?"), pj["file"].as_str().unwrap_or("?"));
    }
    if path.ends_with("thesaurus_toggle") {
        let b = bj();
        return format!("{} {} -> ativos={} palavras={}",
                       b["code"].as_str().unwrap_or("?"),
                       b["action"].as_str().unwrap_or("?"),
                       pj["active_count"].as_u64().or(pj["active"].as_u64()).unwrap_or(0),
                       pj["entries"].as_u64().or(pj["words"].as_u64()).unwrap_or(0));
    }

    // ── config / chaves ──
    if path.ends_with("/api/config") {
        return format!("provider={}", pj["active_provider"].as_str().or(pj["provider"].as_str()).unwrap_or("?"));
    }
    if path.ends_with("test_key") {
        let ok = pj["ok"].as_bool().unwrap_or(false);
        return format!("{} {}", pj["provider"].as_str().unwrap_or("?"), if ok { "OK" } else { "FALHA" });
    }

    // ── observabilidade ──
    if path.ends_with("/api/logs") {
        let lines = pj["lines"].as_array().map(|a| a.len() as u64).unwrap_or(pj["count"].as_u64().unwrap_or(0));
        return format!("linhas={lines}");
    }
    if path.ends_with("/api/stats") || path == "/stats" {
        return format!("bases={} cols={} chunks={}",
                       pj["bases"].as_u64().unwrap_or(0),
                       pj["collections"].as_u64().unwrap_or(0),
                       pj["chunks"].as_u64().unwrap_or(0));
    }

    // ── /profile (#1) ──
    if path == "/profile" {
        let scope = pj["scope"].as_str().unwrap_or("?");
        let coll = pj["collection"].as_str().unwrap_or("?");
        if scope == "base" {
            return format!("{coll}/{} chunks={} vocab={}",
                           pj["base"].as_str().unwrap_or("?"),
                           pj["n_chunks"].as_u64().unwrap_or(0),
                           pj["vocab_size"].as_u64().unwrap_or(0));
        }
        return format!("{coll} (unified) bases={} chunks={} vocab={}",
                       pj["bases"].as_u64().unwrap_or(0),
                       pj["chunks"].as_u64().unwrap_or(0),
                       pj["unified_vocab_size"].as_u64().unwrap_or(0));
    }

    // ── DELETE /collections/{name} (#2) ──
    if path.starts_with("/collections/") {
        return format!("removida={} bases_removed={} purged={}",
                       pj["collection"].as_str().unwrap_or("?"),
                       pj["bases_removed"].as_u64().unwrap_or(0),
                       pj["purged"].as_bool().unwrap_or(false));
    }

    // ── /bases/{coll}/{name} (GET, #4) ou DELETE /bases/{name} ──
    if path.starts_with("/bases/") {
        // GET com coll/name traz a meta; DELETE traz removed
        if let Some(corpus) = pj["corpus"].as_str() {
            return format!("{}/{} corpus={corpus} chunks={}",
                           pj["collection"].as_str().unwrap_or("?"),
                           pj["name"].as_str().unwrap_or("?"),
                           pj["n_chunks"].as_u64().unwrap_or(0));
        }
        return format!("removida={}", pj["removed"].as_str().or(pj["name"].as_str()).unwrap_or("?"));
    }

    String::new()
}

/// Linha de log padronizada: [ts] [tag] ip METHOD path?query -> code (Xms) · extra
fn log_line(tag: &str, ip: &str, method: &Method, path: &str, query: &str, code: u16, ms: f64, extra: &str) {
    let q = if query.is_empty() { String::new() } else { format!("?{query}") };
    let ex = if extra.is_empty() { String::new() } else { format!("  ·  {extra}") };
    println!("[{}] [{tag}] {ip} {method:?} {path}{q} -> {code} ({ms:.1}ms){ex}", now_stamp());
}

/// Trace hierárquico de pipeline (mesmo stdout, tag livre). Os handlers rodam sob o lock do
/// State (serializados), então as linhas de um trace não se intercalam com as de outra
/// requisição. Use `pfx` pra desenhar a árvore (├─, │, └─). `slog` mantém a tag [search]
/// (compat); novos pipelines (ingest, etc.) usam [tlog] direto com a tag deles.
fn tlog(tag: &str, line: &str) { println!("[{}] [{tag}] {line}", now_stamp()); }
fn slog(line: &str) { tlog("search", line); }

/// Últimas `n` linhas de um arquivo de log (pro painel de Logs do ValHalla).
fn tail_lines(path: &str, n: usize) -> String {
    match std::fs::read_to_string(path) {
        Ok(s) => {
            let lines: Vec<&str> = s.lines().collect();
            let start = lines.len().saturating_sub(n);
            lines[start..].join("\n")
        }
        Err(e) => format!("(sem log em {path}: {e})"),
    }
}

// ----------------- helpers de Bases (coleção -> nome -> base) -----------------

fn total_bases(b: &Bases) -> usize { b.values().map(|m| m.len()).sum() }

fn get_base<'a>(b: &'a Bases, coll: &str, name: &str) -> Option<&'a RagBase> {
    b.get(coll)?.get(name)
}

fn insert_base(b: &mut Bases, coll: &str, name: String, base: RagBase) {
    b.entry(coll.to_string()).or_default().insert(name, base);
}

/// Carrega um `<name>-tokenized.json` na coleção `coll`. Devolve true se entrou.
fn load_base_file(bases: &mut Bases, coll: &str, path: &Path) -> bool {
    let fname = match path.file_name().and_then(|x| x.to_str()) { Some(f) => f, None => return false };
    let name = match fname.strip_suffix("-tokenized.json") { Some(s) => s, None => return false };
    match RagBase::load(path.to_str().unwrap_or("")) {
        Ok(b) => { insert_base(bases, coll, name.to_string(), b); true }
        Err(e) => { eprintln!("autoload: '{coll}/{name}' falhou: {e}"); false }
    }
}

/// Auto-load do boot: varre ragfiles_dir e carrega TODAS as bases. Cada subdir = uma
/// coleção; cada `*-tokenized.json` dentro = uma base. Arquivos soltos direto na raiz
/// caem na coleção default. Usa read_dir (enxerga até dotfiles, ao contrário do glob).
fn autoload_ragfiles(dir: &str, bases: &mut Bases) {
    let root = Path::new(dir);
    if !root.is_dir() {
        eprintln!("autoload: ragfiles-dir {dir:?} não existe — subindo vazio");
        return;
    }
    let entries = match std::fs::read_dir(root) {
        Ok(e) => e, Err(e) => { eprintln!("autoload: erro lendo {dir:?}: {e}"); return; }
    };
    let mut n = 0;
    for entry in entries.flatten() {
        let p = entry.path();
        if p.is_dir() {
            let coll = entry.file_name().to_string_lossy().to_string();
            if let Ok(sub) = std::fs::read_dir(&p) {
                for f in sub.flatten() {
                    let fp = f.path();
                    if fp.is_file() && load_base_file(bases, &coll, &fp) { n += 1; }
                }
            }
        } else if p.is_file() && load_base_file(bases, DEFAULT_COLLECTION, &p) {
            n += 1;
        }
    }
    println!("autoload: {n} base(s) de {dir:?} em {} coleção(ões)", bases.len());
}

fn remove_base(b: &mut Bases, coll: &str, name: &str) -> bool {
    let removed = b.get_mut(coll).and_then(|m| m.remove(name)).is_some();
    // se a coleção ficou vazia, descarta a entrada também (sem coleções fantasmas)
    if removed {
        if let Some(m) = b.get(coll) {
            if m.is_empty() { b.remove(coll); }
        }
    }
    removed
}

/// Resolve escopo (coleções + bases) a partir de wildcards.
/// - `coll_pat = "*"` ou None → todas as coleções; senão exata
/// - `base_pat = "sda"` exata; `"sd*"` prefixo; `"*"` todas
/// Devolve lista de (coll, name) ordenada por (coll, name).
fn resolve_scope(b: &Bases, coll_pat: Option<&str>, base_pat: &str) -> Vec<(String, String)> {
    let mut out = vec![];
    let colls: Vec<&String> = match coll_pat {
        None | Some("*") => b.keys().collect(),
        Some(exact) => b.keys().filter(|k| k.as_str() == exact).collect(),
    };
    let base_prefix = base_pat.strip_suffix('*');
    for c in colls {
        if let Some(inner) = b.get(c) {
            for n in inner.keys() {
                let ok = base_pat == "*"
                    || base_prefix.map(|p| n.starts_with(p)).unwrap_or(false)
                    || n == base_pat;
                if ok { out.push((c.clone(), n.clone())); }
            }
        }
    }
    out.sort();
    out
}

fn main() {
    let argv: Vec<String> = std::env::args().collect();
    if argv.iter().any(|a| a == "-h" || a == "--help") { help(); return; }

    // 1) config: --config <path>, senão CONFIG_PATHS, senão defaults embutidos
    let mut cfg = Config::default();
    let cli_config = argv.iter().position(|a| a == "--config").and_then(|i| argv.get(i + 1)).cloned();
    let cfg_path = cli_config.or_else(||
        CONFIG_PATHS.iter().find(|p| Path::new(p).is_file()).map(|s| s.to_string()));
    if let Some(p) = &cfg_path { load_config_file(&mut cfg, p); }

    // 2) CLI sobrescreve a config
    let mut no_autoload = !cfg.autoload;
    let mut preload: Vec<(String, String)> = vec![];
    let mut it = argv[1..].iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--config" => { it.next(); }   // já tratado acima
            "--port" => cfg.api_port = it.next().expect("--port N").parse().expect("porta inválida"),
            "--dash-port" => cfg.dash_port = it.next().expect("--dash-port N").parse().expect("porta inválida"),
            "--drivers-dir" => cfg.drivers_dir = it.next().expect("--drivers-dir <path>").clone(),
            "--ragfiles-dir" => cfg.ragfiles_dir = it.next().expect("--ragfiles-dir <path>").clone(),
            "--max-upload" => cfg.max_upload = it.next().expect("--max-upload N").parse().expect("--max-upload N"),
            "--no-autoload" => no_autoload = true,
            "--dev" => cfg.dev = true,
            "--storage" => cfg.storage = it.next().expect("--storage memory|hybrid").clone(),
            "--preload" => {
                if let Some((n, p)) = it.next().expect("--preload nome=caminho").split_once('=') {
                    preload.push((n.to_string(), p.to_string()));
                }
            }
            other => eprintln!("aviso: argumento ignorado: {other:?}"),
        }
    }

    LOG_OFFSET.store(cfg.log_utc_offset, std::sync::atomic::Ordering::Relaxed);
    let hybrid = cfg.storage.eq_ignore_ascii_case("hybrid");
    rag::CACHE_WORDS.store(!hybrid, std::sync::atomic::Ordering::Relaxed);
    println!("storage: {} (cache de tokens {})", cfg.storage, if hybrid { "DESLIGADO — recomputa candidatos" } else { "ligado" });

    // 3) bases: autoload (default) + preload aditivo
    let mut bases: Bases = HashMap::new();
    if !no_autoload { autoload_ragfiles(&cfg.ragfiles_dir, &mut bases); }
    for (raw_name, path) in &preload {
        let (coll, name) = match raw_name.split_once('/') {
            Some((c, n)) if !c.is_empty() && !n.is_empty() => (c.to_string(), n.to_string()),
            _ => (DEFAULT_COLLECTION.to_string(), raw_name.clone()),
        };
        match RagBase::load(path) {
            Ok(b) => { println!("preload '{coll}/{name}' <- {path}  ({} chunks)", b.n_chunks);
                       insert_base(&mut bases, &coll, name, b); }
            Err(e) => eprintln!("preload '{coll}/{raw_name}' falhou: {e}"),
        }
    }

    let n_drivers = count_drivers(&cfg.drivers_dir);
    println!("🤘 RAGnaRock {VERSION}  ({} base(s) em {} coleção(ões), {} driver(s), ragfiles em {:?}, max upload {} MB)",
             total_bases(&bases), bases.len(), n_drivers, cfg.ragfiles_dir, cfg.max_upload / (1024 * 1024));
    let max_upload = cfg.max_upload;   // local p/ limitar leitura sem travar o Mutex
    let state = Arc::new(RwLock::new(State {
        bases, drivers_dir: cfg.drivers_dir.clone(), ragfiles_dir: cfg.ragfiles_dir.clone(),
        max_upload, started: Instant::now(),
        admin_user: cfg.admin_user.clone(), admin_pass: cfg.admin_pass.clone(),
        dev: cfg.dev,
        log_file: cfg.log_file.clone(),
        config_path: cfg_path.clone().unwrap_or_else(|| "ragnarock.cfg".to_string()),
        anthropic_key: cfg.anthropic_key.clone(),
        openai_key: cfg.openai_key.clone(),
        active_provider: cfg.active_provider.clone(),
        cache_dir: cfg.cache_dir.clone(),
        expansions: RwLock::new({
            let t = load_expansions(&cfg.cache_dir);
            println!("cache de expansões: {} entrada(s) em {:?}", t.len(), expansions_file(&cfg.cache_dir));
            t
        }),
        thesaurus_dir: cfg.thesaurus_dir.clone(),
        word_syn: {
            let m = load_active_dicts(&cfg.thesaurus_dir);
            let n_act = dict_dirs(&cfg.thesaurus_dir).iter().filter(|p| p.join("inuse.flag").exists()).count();
            println!("dicionários: {} dir(s) em {:?}, {} ativo(s) -> {} palavra(s)",
                     dict_dirs(&cfg.thesaurus_dir).len(), cfg.thesaurus_dir, n_act, m.len());
            m
        },
        nidhogg_url: cfg.nidhogg_url.clone(),
        collection_profiles: RwLock::new(HashMap::new()),
        sessions: HashMap::new(),
    }));

    // 4) dashboard / supervisório numa thread separada (porta de controle)
    let dash_addr = format!("0.0.0.0:{}", cfg.dash_port);
    match Server::http(&dash_addr) {
        Ok(dash) => {
            if cfg.dev {
                println!("⚔  ValHalla (console) em http://{dash_addr}/  (login: {} / ****) [MODO DEV]", cfg.admin_user);
            } else {
                println!("⚔  ValHalla (console) em http://{dash_addr}/  (login: {} / ****)", cfg.admin_user);
            }
            if is_default_creds(&cfg.admin_user, &cfg.admin_pass) && !cfg.dev {
                eprintln!("⚠  ATENÇÃO: credenciais padrão admin/admin detectadas.");
                eprintln!("   Login no ValHalla será RECUSADO até que você altere admin_user/admin_pass");
                eprintln!("   no ragnarock.cfg (ou via painel Config) — ou suba com --dev p/ desenvolvimento.");
            }
            let st = state.clone();
            thread::spawn(move || { for req in dash.incoming_requests() { handle_dashboard(req, &st); } });
        }
        Err(e) => eprintln!("aviso: dashboard não subiu em {dash_addr}: {e}"),
    }

    // 5) API na thread principal
    let api_addr = format!("0.0.0.0:{}", cfg.api_port);
    let server = Server::http(&api_addr).unwrap_or_else(|e| {
        eprintln!("erro ao subir em {api_addr}: {e}"); std::process::exit(1);
    });
    println!("🤘 API em http://{api_addr}/  · /health /bases /collections /drivers /thesaurus /interpret /search /chunk /ingest*");
    for mut req in server.incoming_requests() {
        let method = req.method().clone();
        let full = req.url().to_string();
        let (path, query) = match full.split_once('?') {
            Some((p, q)) => (p.to_string(), q.to_string()),
            None => (full, String::new()),
        };
        let ip = req.remote_addr().map(|a| a.ip().to_string()).unwrap_or_else(|| "?".into());
        let headers: Vec<(String, String)> = req.headers().iter()
            .map(|h| (h.field.as_str().as_str().to_string(), h.value.as_str().to_string()))
            .collect();
        let mut body_bytes: Vec<u8> = Vec::new();
        if method == Method::Post {
            let max_read = max_upload.saturating_add(1);
            req.as_reader().take(max_read as u64).read_to_end(&mut body_bytes).ok();
        }
        let t0 = Instant::now();
        // [#6] Classifica antes de pegar o lock: rotas que mutam o State (ingest, delete,
        // toggle) pegam write() exclusivo; o resto pega read() — N searches em paralelo.
        // Caches que search/search_expand precisam mutar (collection_profiles, expansions)
        // têm interior mutability (Mutex<>) e funcionam sob read() outer.
        let (code, payload) = if is_write_route(&method, &path) {
            let mut st = state.write();
            route(&method, &path, &query, &headers, &body_bytes, &mut *st)
        } else {
            let st = state.read();
            route_ro(&method, &path, &query, &headers, &body_bytes, &*st)
        };
        let ms = t0.elapsed().as_secs_f64() * 1000.0;
        log_line("api", &ip, &method, &path, &query, code, ms, &req_extra(&path, &body_bytes, &payload));
        let header = Header::from_bytes(&b"Content-Type"[..],
                                        &b"application/json; charset=utf-8"[..]).unwrap();
        let _ = req.respond(Response::from_string(payload).with_status_code(code).with_header(header));
    }
}

// ----------------------------- dashboard (porta de controle) -----------------------------

const SESSION_TTL: u64 = 12 * 3600;   // sessão válida por 12h

/// Gera um token de sessão (16 bytes de /dev/urandom em hex; fallback temporal).
fn gen_token() -> String {
    let mut buf = [0u8; 16];
    if let Ok(mut f) = std::fs::File::open("/dev/urandom") {
        if f.read_exact(&mut buf).is_ok() {
            return buf.iter().map(|b| format!("{b:02x}")).collect();
        }
    }
    let n = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos()).unwrap_or(0);
    format!("{n:032x}")
}

/// Lê o valor de um cookie pelo nome no header Cookie.
fn cookie_val(headers: &[(String, String)], key: &str) -> Option<String> {
    let c = headers.iter().find(|(k, _)| k.eq_ignore_ascii_case("cookie")).map(|(_, v)| v.as_str())?;
    for part in c.split(';') {
        let p = part.trim();
        if let Some(v) = p.strip_prefix(&format!("{key}=")) { return Some(v.to_string()); }
    }
    None
}

/// Sessão válida = cookie vh_session presente e não expirado.
fn session_ok(headers: &[(String, String)], sessions: &HashMap<String, Instant>) -> bool {
    match cookie_val(headers, "vh_session") {
        Some(t) => sessions.get(&t).map(|created| created.elapsed().as_secs() < SESSION_TTL).unwrap_or(false),
        None => false,
    }
}

fn stats_json(st: &State) -> String {
    let detail: Vec<Value> = st.bases.iter().map(|(c, m)| json!({
        "collection": c, "bases": m.len(),
        "chunks": m.values().map(|x| x.n_chunks).sum::<usize>(),
    })).collect();
    json!({
        "version": VERSION,
        "uptime_secs": st.started.elapsed().as_secs(),
        "collections": st.bases.len(),
        "bases": total_bases(&st.bases),
        "chunks": total_chunks(&st.bases),
        "drivers": count_drivers(&st.drivers_dir),
        "dicts_active": dict_dirs(&st.thesaurus_dir).iter().filter(|p| p.join("inuse.flag").exists()).count(),
        "word_syn_entries": st.word_syn.len(),
        "ragfiles_dir": st.ragfiles_dir,
        "collections_detail": detail,
        "mem": mem_stats(&st.bases),
    }).to_string()
}

/// Move um .drv entre `drivers/` (instalado) e `drivers.out/` (disponível). Como os
/// drivers são lidos do disco a cada request, instalar/desinstalar reflete na hora.
fn driver_move(body: &str, drivers_dir: &str) -> (u16, String) {
    let v: Value = match serde_json::from_str(body) {
        Ok(v) => v, Err(e) => return (400, json!({"error": format!("JSON inválido: {e}")}).to_string()),
    };
    let file = match v["file"].as_str() {
        Some(f) => f, None => return (400, json!({"error": "falta 'file'"}).to_string()),
    };
    let action = v["action"].as_str().unwrap_or("");
    // segurança: só nome de arquivo .drv, sem path traversal
    if file.contains('/') || file.contains('\\') || file.contains("..") || !file.ends_with(".drv") {
        return (400, json!({"error": "nome de driver inválido"}).to_string());
    }
    let out_dir = format!("{}.out", drivers_dir);
    let (from, to) = match action {
        "install"   => (out_dir.clone(), drivers_dir.to_string()),       // drivers.out -> drivers
        "uninstall" => {
            if file == ingestor::FALLBACK_DRIVER {
                return (400, json!({"error": format!("{file} é o fallback — não desinstalar")}).to_string());
            }
            (drivers_dir.to_string(), out_dir.clone())                   // drivers -> drivers.out
        }
        _ => return (400, json!({"error": "action deve ser 'install' ou 'uninstall'"}).to_string()),
    };
    std::fs::create_dir_all(&to).ok();
    let src = Path::new(&from).join(file);
    let dst = Path::new(&to).join(file);
    if !src.is_file() { return (404, json!({"error": format!("{file} não está em {from}")}).to_string()); }
    match std::fs::rename(&src, &dst) {
        Ok(_) => (200, json!({"ok": true, "file": file, "action": action,
                              "installed": count_drivers(drivers_dir)}).to_string()),
        Err(e) => (500, json!({"error": format!("mover {file}: {e}")}).to_string()),
    }
}

/// POST /api/histogram — roda a busca, pega o hit #1 e devolve os histogramas
/// (query + chunk mais próximo) pro painel de visualização do ValHalla.
fn histogram(body: &str, bases: &Bases) -> (u16, String) {
    let v: Value = match serde_json::from_str(body) {
        Ok(v) => v, Err(e) => return (400, json!({"error": format!("JSON inválido: {e}")}).to_string()),
    };
    let query = match v["query"].as_str() {
        Some(q) => q, None => return (400, json!({"error": "falta 'query'"}).to_string()),
    };
    let tmp_profiles = RwLock::new(HashMap::new());   // histograma é visualização pontual; não usa cache unificado
    let (code, sres) = search(body, bases, &tmp_profiles);
    if code != 200 { return (code, sres); }
    let sv: Value = serde_json::from_str(&sres).unwrap_or_else(|_| json!({}));
    let top = match sv["hits"].as_array().and_then(|h| h.first()) {
        Some(t) => t,
        None => return (200, json!({"found": false, "query_syllables": sv["query_syllables"]}).to_string()),
    };
    let coll = top["collection"].as_str().unwrap_or("");
    let bname = top["base"].as_str().unwrap_or("");
    let cid = top["chunk"].as_u64().unwrap_or(0) as usize;
    let base = match get_base(bases, coll, bname) {
        Some(b) => b, None => return (404, json!({"error": "base do hit não encontrada"}).to_string()),
    };
    let mut data = base.hist_data(query, cid);
    if let Some(o) = data.as_object_mut() {
        o.insert("found".into(), json!(true));
        o.insert("collection".into(), json!(coll));
        o.insert("base".into(), json!(bname));
        o.insert("chunk_id".into(), json!(cid));   // o array do histograma já é "chunk"
        o.insert("coverage".into(), top.get("coverage").cloned().unwrap_or(json!(null)));
        o.insert("cos".into(), top.get("cos").cloned().unwrap_or(json!(null)));
        o.insert("query_syllables".into(), sv["query_syllables"].clone());
    }
    (200, data.to_string())
}

/// Reinicia o próprio processo: re-exec com a mesma linha de comando (relê o cfg, zera a RAM).
/// Os sockets do Rust são CLOEXEC → fecham no exec → a nova instância faz bind limpo.
#[cfg(unix)]
fn restart_self() -> ! {
    use std::os::unix::process::CommandExt;
    let exe = std::env::current_exe().unwrap_or_else(|_| std::path::PathBuf::from("ragd"));
    let args: Vec<String> = std::env::args().skip(1).collect();
    let e = std::process::Command::new(exe).args(&args).exec();   // substitui o processo
    eprintln!("[restart] exec falhou: {e}");
    std::process::exit(1);
}
#[cfg(not(unix))]
fn restart_self() -> ! {
    let exe = std::env::current_exe().unwrap_or_else(|_| std::path::PathBuf::from("ragd"));
    let args: Vec<String> = std::env::args().skip(1).collect();
    let _ = std::process::Command::new(exe).args(&args).spawn();
    std::process::exit(0);
}

/// Atualiza (ou anexa) `chave = valor` no arquivo de config, preservando o resto.
fn set_cfg_key(path: &str, key: &str, val: &str) {
    let mut lines: Vec<String> = std::fs::read_to_string(path)
        .map(|s| s.lines().map(|l| l.to_string()).collect()).unwrap_or_default();
    let newline = format!("{key} = {val}");
    let mut found = false;
    for l in lines.iter_mut() {
        let t = l.trim_start();
        if t.starts_with(&format!("{key} ")) || t.starts_with(&format!("{key}=")) || t.starts_with(&format!("{key}\t")) {
            *l = newline.clone(); found = true; break;
        }
    }
    if !found { lines.push(newline); }
    let _ = std::fs::write(path, lines.join("\n") + "\n");
}

/// Mascara uma chave pra exibição (início…fim).
fn mask_key(k: &str) -> String {
    let n = k.chars().count();
    if n == 0 { String::new() }
    else if n <= 10 { "•".repeat(n) }
    else { format!("{}…{}", &k.chars().take(6).collect::<String>(), &k.chars().skip(n - 4).collect::<String>()) }
}

/// Normaliza a query pra chave do cache de expansões: minúsculas + espaços colapsados.
fn normalize_query(q: &str) -> String {
    q.to_lowercase().split_whitespace().collect::<Vec<_>>().join(" ")
}
fn expansions_file(dir: &str) -> std::path::PathBuf { Path::new(dir).join("expansions.json") }
fn load_expansions(dir: &str) -> HashMap<String, Vec<String>> {
    std::fs::read_to_string(expansions_file(dir)).ok()
        .and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
}
fn save_expansions(dir: &str, map: &HashMap<String, Vec<String>>) {
    let _ = std::fs::create_dir_all(dir);
    if let Ok(s) = serde_json::to_string_pretty(map) { let _ = std::fs::write(expansions_file(dir), s); }
}

/// Extrai um array JSON de strings de um texto (tolera prosa/fences ```json em volta).
fn parse_str_array(s: &str) -> Vec<String> {
    if let (Some(a), Some(b)) = (s.find('['), s.rfind(']')) {
        if a < b {
            if let Ok(Value::Array(arr)) = serde_json::from_str::<Value>(&s[a..=b]) {
                return arr.iter().filter_map(|x| x.as_str().map(|t| t.trim().to_string()))
                    .filter(|t| !t.is_empty()).collect();
            }
        }
    }
    vec![]
}

/// Chama o LLM ativo (via wget POST) pra expandir a query em sinônimos/reformulações.
/// O LLM toca SÓ a query (uma frase) — o corpus nunca passa por aqui.
fn llm_expand(provider: &str, key: &str, query: &str) -> Result<Vec<String>, String> {
    let prompt = format!(
        "Você é um expansor de consulta para um motor de busca LÉXICO (casamento de palavras) de \
         código-fonte e documentos em português. Dada a CONSULTA, gere de 4 a 6 reformulações curtas: \
         sinônimos, termos técnicos relacionados e variações que ajudem a achar o MESMO conteúdo. \
         Não explique. Responda APENAS com um array JSON de strings.\n\nCONSULTA: {query}");
    let (url, model, headers, body) = match provider {
        "openai" => ("https://api.openai.com/v1/chat/completions", "gpt-4o-mini",
            vec![format!("Authorization: Bearer {key}")],
            json!({"model": "gpt-4o-mini", "temperature": 0.3,
                   "messages": [{"role": "user", "content": prompt}]}).to_string()),
        "anthropic" => ("https://api.anthropic.com/v1/messages", "claude-3-5-haiku-20241022",
            vec![format!("x-api-key: {key}"), "anthropic-version: 2023-06-01".into()],
            json!({"model": "claude-3-5-haiku-20241022", "max_tokens": 300,
                   "messages": [{"role": "user", "content": prompt}]}).to_string()),
        _ => return Err("provider inválido".into()),
    };
    // trace: prompt enviado (uma linha, quebras viram ⏎ pra não poluir o log)
    slog(&format!("   │     prompt→{provider}/{model} ({} chars): {}",
                  prompt.chars().count(),
                  prompt.replace('\n', " ⏎ ").chars().take(180).collect::<String>()));
    let mut cmd = std::process::Command::new("wget");
    cmd.args(["-q", "-O", "-", "--content-on-error", "--timeout=30", "--tries=1"]);
    cmd.arg("--header=Content-Type: application/json");
    for h in &headers { cmd.arg(format!("--header={h}")); }
    cmd.arg(format!("--post-data={body}")).arg(url);
    let t0 = Instant::now();
    let out = cmd.output().map_err(|e| format!("wget: {e}"))?;
    let infer_ms = t0.elapsed().as_secs_f64() * 1000.0;
    let resp = String::from_utf8_lossy(&out.stdout);
    let rv: Value = serde_json::from_str(&resp).unwrap_or(Value::Null);
    let content = match provider {
        "openai" => rv["choices"][0]["message"]["content"].as_str(),
        "anthropic" => rv["content"][0]["text"].as_str(),
        _ => None,
    }.unwrap_or("");
    if content.is_empty() {
        slog(&format!("   │     inferência FALHOU ({infer_ms:.0}ms): {}", resp.chars().take(120).collect::<String>().replace('\n', " ")));
        return Err(format!("resposta inesperada: {}", resp.chars().take(160).collect::<String>().replace('\n', " ")));
    }
    let arr: Vec<String> = parse_str_array(content).into_iter().take(6).collect();
    if arr.is_empty() { return Err(format!("não extraí sinônimos de: {}", content.chars().take(120).collect::<String>())); }
    slog(&format!("   │     inferência←{provider} ({infer_ms:.0}ms): {arr:?}"));
    Ok(arr)
}

/// Sílabas normalizadas de um termo, no MESMO esquema das chaves do índice das bases
/// (lowercase → silabador → normalize/strip-accent). É assim que casamos variante×corpus.
fn term_syllables(term: &str) -> Vec<String> {
    let mut out = vec![];
    for w in tokenizer::words(&term.to_lowercase()) {
        for s in tokenizer::syllabify(&w) {
            let ns = tokenizer::normalize(&s);
            if !ns.is_empty() { out.push(ns); }
        }
    }
    out
}

/// Nº de sílabas-âncora. A busca casa o termo ao INÍCIO de uma palavra e prioriza
/// "termos-chave" de ≥2 sílabas; 2 é discriminante sem cortar variante morfológica
/// (ex.: "aparecimento" e "apareceu" compartilham a chave "a·pa").
const KEY_SYL: usize = 2;

/// Chave de uma palavra = suas 1ª(s) sílaba(s) (até KEY_SYL) já normalizadas, juntadas.
fn word_key(syls: &[String]) -> Option<String> {
    if syls.is_empty() { return None; }
    Some(syls[..syls.len().min(KEY_SYL)].join("\u{1}"))
}

/// Conjunto de CHAVES de prefixo das palavras do corpus no escopo. A busca casa um termo
/// ao começo de uma palavra, então uma variante só pode gerar hit se sua chave existir aqui.
/// Usa o cache `Chunk.words` (sílabas por palavra, modo memory); cai pro texto no híbrido.
fn scope_word_keys(bases: &Bases, coll: Option<&str>, base_pat: &str) -> HashSet<String> {
    const SCAN_BUDGET: usize = 400_000;       // teto de palavras varridas (escopo GLOBAL)
    let mut set = HashSet::new();
    let mut budget = SCAN_BUDGET;
    'scan: for (c, n) in resolve_scope(bases, coll, base_pat) {
        let b = match get_base(bases, &c, &n) { Some(b) => b, None => continue };
        for ch in &b.chunks {
            if !ch.words.is_empty() {
                for w in &ch.words {
                    if budget == 0 { break 'scan; } budget -= 1;
                    if let Some(k) = word_key(w) { set.insert(k); }
                }
            } else if let Some(t) = &ch.text {
                for w in tokenizer::words(&t.to_lowercase()) {
                    if budget == 0 { break 'scan; } budget -= 1;
                    let syl: Vec<String> = tokenizer::syllabify(&w).iter()
                        .map(|s| tokenizer::normalize(s)).filter(|s| !s.is_empty()).collect();
                    if let Some(k) = word_key(&syl) { set.insert(k); }
                }
            }
        }
    }
    set
}

/// Um termo "casa o corpus" se ALGUMA de suas palavras tem chave de prefixo presente no
/// escopo (recall: basta uma palavra ancorar). Abaixo disso a busca é garantidamente nula.
fn term_in_corpus(term: &str, keys: &HashSet<String>) -> bool {
    tokenizer::words(&term.to_lowercase()).iter().any(|w| {
        let syl: Vec<String> = tokenizer::syllabify(w).iter()
            .map(|s| tokenizer::normalize(s)).filter(|s| !s.is_empty()).collect();
        word_key(&syl).map(|k| keys.contains(&k)).unwrap_or(false)
    })
}

/// "Aqui está o que o corpus TEM que se parece": did-you-mean fonético. Varre as palavras
/// dos chunks do escopo (teto de WORD_SCAN_BUDGET) e rankeia por soundex igual ao termo
/// ausente (reusa a MESMA máquina fonética do rerank) e, em desempate, por sílaba comum.
/// Só roda no caminho de AUSÊNCIA (raro), então o custo da varredura é aceitável.
fn suggest_terms(bases: &Bases, coll: Option<&str>, base_pat: &str, missing: &[String], limit: usize) -> Vec<String> {
    const WORD_SCAN_BUDGET: usize = 60_000;
    let want_sx: HashSet<String> = missing.iter().map(|t| rag::soundex(t)).filter(|s| !s.is_empty()).collect();
    let want_syl: HashSet<String> = missing.iter().flat_map(|t| term_syllables(t)).collect();
    let want_norm: Vec<String> = missing.iter().map(|t| tokenizer::normalize(t)).filter(|s| !s.is_empty()).collect();
    let mut seen: HashSet<String> = HashSet::new();
    // (soundex_hit, sílabas em comum, -distância_edição_mín) — maior é melhor
    let mut scored: Vec<(i32, i32, i32, String)> = vec![];
    let mut budget = WORD_SCAN_BUDGET;
    'scan: for (c, n) in resolve_scope(bases, coll, base_pat) {
        let b = match get_base(bases, &c, &n) { Some(b) => b, None => continue };
        for ch in &b.chunks {
            let text = match &ch.text { Some(t) => t, None => continue };
            for w in tokenizer::words(&text.to_lowercase()) {
                if budget == 0 { break 'scan; }
                budget -= 1;
                if w.chars().count() < 3 { continue; }
                if !seen.insert(w.clone()) { continue; }
                let sx_hit = { let sx = rag::soundex(&w); !sx.is_empty() && want_sx.contains(&sx) };
                let shared = term_syllables(&w).iter().filter(|s| want_syl.contains(*s)).count() as i32;
                if sx_hit || shared > 0 {
                    let wn = tokenizer::normalize(&w);
                    let dist = want_norm.iter().map(|t| edit_distance(&wn, t)).min().unwrap_or(99) as i32;
                    scored.push((if sx_hit { 1 } else { 0 }, shared, -dist, w));
                }
            }
        }
    }
    scored.sort_by(|a, b| b.0.cmp(&a.0).then(b.1.cmp(&a.1)).then(b.2.cmp(&a.2)).then(a.3.chars().count().cmp(&b.3.chars().count())));
    scored.into_iter().take(limit).map(|(_, _, _, w)| w).collect()
}

/// Distância de Levenshtein (sobre chars) — desempate do did-you-mean: "frudo"→"frodo"(1).
fn edit_distance(a: &str, b: &str) -> usize {
    let (a, b): (Vec<char>, Vec<char>) = (a.chars().collect(), b.chars().collect());
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut cur = vec![0usize; b.len() + 1];
    for (i, &ca) in a.iter().enumerate() {
        cur[0] = i + 1;
        for (j, &cb) in b.iter().enumerate() {
            let cost = if ca == cb { 0 } else { 1 };
            cur[j + 1] = (prev[j] + cost).min(prev[j + 1] + 1).min(cur[j] + 1);
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    prev[b.len()]
}

// ============================================================================
// LITERAL FALLBACK (Issue #38) — quarto estágio da cascata do search_expand.
// Dispara quando dict/cache/IA todos retornam vazio E a query tem tokens com
// dígitos (códigos de ticket, normas, SKUs, preços). O motor silábico é cego
// pra identificadores alfanuméricos; este fallback varre `chunk.text` em RAM
// procurando as needles literais. Sem indexação extra, custo só quando ativa.
// ============================================================================

/// Extrai tokens alfanuméricos QUE CONTÊM AO MENOS UM DÍGITO. Esse é o gate
/// que diferencia "oe-6016", "RFC1918", "30012", "R$1500" de palavras naturais.
/// Palavra pura (sem dígito) NÃO vira needle — pra essas o pipeline silábico
/// já é o caminho certo, e dispará-las aqui seria ruído.
fn extract_alnum_needles(q: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut cur = String::new();
    let mut has_digit = false;
    let push_if = |cur: &mut String, has_digit: &mut bool, out: &mut Vec<String>| {
        if *has_digit && cur.chars().count() >= 2 {
            let lc = cur.to_lowercase();
            if !out.contains(&lc) { out.push(lc); }
        }
        cur.clear(); *has_digit = false;
    };
    for c in q.chars() {
        if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
            cur.push(c);
            if c.is_ascii_digit() { has_digit = true; }
        } else {
            push_if(&mut cur, &mut has_digit, &mut out);
        }
    }
    push_if(&mut cur, &mut has_digit, &mut out);
    out
}

/// Faz busca literal `text.to_lowercase().contains(needle)` em todos os chunks
/// do escopo. Score = nº de needles distintos casados / total de needles.
/// Retorna hits no shape do /search (matchpoint, coverage, chunk, start, snippet,
/// rank, via=literal_fallback). Vazio se nenhum chunk casar nenhum needle.
fn literal_fallback(needles: &[String], bases: &Bases, coll: Option<&str>, base_pat: &str, k: usize) -> Vec<Value> {
    if needles.is_empty() { return vec![]; }
    let mut hits: Vec<(f64, usize, Value)> = Vec::new();
    let scope = resolve_scope(bases, coll, base_pat);
    for (cn, bn) in scope {
        let b = match get_base(bases, &cn, &bn) { Some(b) => b, None => continue };
        if !b.has_text { continue; }
        for ch in &b.chunks {
            let text = match &ch.text { Some(t) => t, None => continue };
            let lc = text.to_lowercase();
            let matched: Vec<&String> = needles.iter().filter(|n| lc.contains(n.as_str())).collect();
            if matched.is_empty() { continue; }
            let score = matched.len() as f64 / needles.len() as f64;
            // snippet: janela ±100 chars em torno da primeira ocorrência
            let needle = matched[0];
            let pos = lc.find(needle.as_str()).unwrap_or(0);
            let start = pos.saturating_sub(100);
            let end = (pos + needle.len() + 100).min(text.len());
            // ajusta start/end pra fronteira de char UTF-8 (text.find dá byte index)
            let start = text.char_indices().take_while(|(i,_)| *i <= start).last().map(|(i,_)| i).unwrap_or(0);
            let end = text.char_indices().find(|(i,_)| *i >= end).map(|(i,_)| i).unwrap_or(text.len());
            let snippet = format!("…{}…", &text[start..end]);
            hits.push((score, matched.len(), json!({
                "collection": cn,
                "base": bn,
                "corpus": b.corpus,
                "matchpoint": score,
                "coverage": score,
                "span": matched.len(),
                "cos": 0.0,
                "chunk": ch.id,
                "start": ch.start,
                "snippet": snippet,
                "needles_matched": matched.iter().map(|s| (**s).clone()).collect::<Vec<_>>(),
            })));
        }
    }
    // ordem: score ↓ · nº de needles distintos ↓
    hits.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap().then(b.1.cmp(&a.1)));
    hits.into_iter().take(k).enumerate().map(|(i, (.., mut h))| {
        if let Some(o) = h.as_object_mut() {
            o.insert("rank".into(), json!(i + 1));
            o.insert("via".into(), json!("literal_fallback"));
        }
        h
    }).collect()
}

/// POST /api/search_expand — busca COM expansão por IA: expande a query, roda a busca léxica
/// pra original + variantes, e mescla por (coleção,base,chunk) com peso maior no termo original.
fn search_expand(body: &str, st: &State) -> (u16, String) {
    // [#6] aceita &State (não mut) — os caches que precisa mutar (expansions, collection_profiles)
    // têm interior mutability (RwLock<>), o que permite search_expand rodar sob outer-read.
    let v: Value = match serde_json::from_str(body) {
        Ok(v) => v, Err(e) => return (400, json!({"error": format!("JSON inválido: {e}")}).to_string()),
    };
    let query = match v["query"].as_str() { Some(q) if !q.trim().is_empty() => q.to_string(), _ => return (400, json!({"error": "falta 'query'"}).to_string()) };
    let query = query.as_str();
    // ── TRACE: cabeçalho da árvore de busca ──
    slog(&format!("┌─ search_expand q={query:?} escopo={}/{} k={}",
                  v["collection"].as_str().unwrap_or("*"), v["base"].as_str().unwrap_or("*"),
                  v["k"].as_u64().unwrap_or(8)));
    // Cascata de expansão (do mais barato pro mais caro):
    //   1) DICIONÁRIOS por-palavra ATIVOS (instantâneo, zero custo) — se houver, encerra aqui.
    //   2) CACHE por-query (cache/expansions.json) — hit instantâneo.
    //   3) LLM ativo — só quando 1 e 2 não deram nada; grava no cache.
    let nkey = normalize_query(query);
    let dict_exps = if st.word_syn.is_empty() { vec![] } else { expand_with_dicts(query, &st.word_syn) };
    // [#6 fix] Extrai o cache hit pra um let ANTES do if/else: garante que o read lock de
    // `expansions` é dropado ANTES da arm `else` poder tentar `expansions.write()` (senão
    // mesmo-thread read+write deadlocka em parking_lot/std RwLock).
    let cache_hit: Option<Vec<String>> = st.expansions.read().get(&nkey).cloned();
    let (exps, source): (Vec<String>, &str) = if !dict_exps.is_empty() {
        slog(&format!("   ├─ cascata: 📚 dicionário ({} palavras ativas) → {} variante(s) · ENCERRA (sem cache/IA)",
                      st.word_syn.len(), dict_exps.len()));
        (dict_exps, "dict")
    } else if let Some(c) = cache_hit {
        slog(&format!("   ├─ cascata: 📚 dict=∅ → 📖 cache HIT ({} variante(s)) · sem IA", c.len()));
        (c, "cache")
    } else {
        let provider = st.active_provider.clone();
        let key = match provider.as_str() { "anthropic" => st.anthropic_key.clone(), "openai" => st.openai_key.clone(), _ => String::new() };
        if provider == "none" || key.is_empty() {
            // [Issue #38] Quarto estágio: literal_fallback. Se a query tem tokens
            // alfanuméricos com dígito (ticket OE-6016, norma RFC1918, preço 1500),
            // o motor silábico é cego mas o grep literal acha. Só dispara aqui,
            // depois que dict/cache/IA falharam — não substitui a busca silábica
            // pra texto natural, complementa onde ela é estruturalmente cega.
            let needles = extract_alnum_needles(query);
            if !needles.is_empty() {
                let k_lit = v["k"].as_u64().unwrap_or(8) as usize;
                let base_lit = v["base"].as_str().unwrap_or("*").to_string();
                let coll_lit = v["collection"].as_str().map(|s| s.to_string());
                let lit_hits = literal_fallback(&needles, &st.bases, coll_lit.as_deref(), &base_lit, k_lit);
                if !lit_hits.is_empty() {
                    slog(&format!("   └─ cascata: 📚 dict=∅ → 📖 cache MISS → 🧠 IA=∅ → 🔎 literal {needles:?} · {} hit(s)", lit_hits.len()));
                    return (200, json!({
                        "query": query,
                        "source": "literal_fallback",
                        "needles": needles,
                        "expansions": [],
                        "hits": lit_hits,
                    }).to_string());
                }
                slog(&format!("   └─ cascata: 📚 dict=∅ → 📖 cache MISS → 🧠 IA=∅ → 🔎 literal {needles:?} · 0 hit(s) · 400"));
            } else {
                slog("   └─ cascata: 📚 dict=∅ → 📖 cache MISS → 🧠 IA indisponível (sem provider) · 400");
            }
            return (400, json!({"error": "nenhum dicionário ativo, sem cache e sem provider de IA — ative um dicionário na aba Dicionários ou um provider na aba Config (ou semeie cache/expansions.json)"}).to_string());
        }
        slog(&format!("   ├─ cascata: 📚 dict=∅ → 📖 cache MISS → 🧠 aciona IA ({provider})"));
        match llm_expand(&provider, &key, query) {
            Ok(e) => {
                let mut m = st.expansions.write();
                m.insert(nkey.clone(), e.clone());
                save_expansions(&st.cache_dir, &m);
                drop(m);
                slog(&format!("   ├─ IA → {} variante(s) · gravado no cache", e.len()));
                (e, "llm")
            }
            Err(e) => { slog(&format!("   └─ IA FALHOU: {e} · 502")); return (502, json!({"error": format!("expansão falhou: {e}")}).to_string()); }
        }
    };
    let provider = st.active_provider.clone();
    let k = v["k"].as_u64().unwrap_or(8) as usize;
    let base = v["base"].as_str().unwrap_or("*").to_string();
    let phon = v["phonetic"].as_bool().unwrap_or(false);
    let coll = v["collection"].as_str().map(|s| s.to_string());
    // FILTRO POR VOCAB: variante que não ancora em nenhuma palavra do corpus do escopo é
    // busca garantidamente nula — corta ANTES de rodar. O que ficou de fora vira transparência.
    let keys = scope_word_keys(&st.bases, coll.as_deref(), &base);
    let orig_in = term_in_corpus(query, &keys);
    let mut kept: Vec<String> = vec![];
    let mut dropped: Vec<String> = vec![];
    for e in &exps {
        if term_in_corpus(e, &keys) { kept.push(e.clone()); } else { dropped.push(e.clone()); }
    }
    slog(&format!("   ├─ filtro vocab ({} chaves no escopo): original {} · {} mantida(s), {} cortada(s){}",
                  keys.len(), if orig_in { "ancora ✓" } else { "FORA ✗" }, kept.len(), dropped.len(),
                  if dropped.is_empty() { String::new() } else { format!(" → {dropped:?}") }));
    // MODELO DE FALHA EXPLÍCITO: nem a consulta nem nenhum sinônimo casa o corpus →
    // não devolve "0 hits" mudo; devolve a AUSÊNCIA + o que o corpus tem de mais parecido.
    if !orig_in && kept.is_empty() {
        let mut missing = vec![query.to_string()];
        missing.extend(exps.iter().cloned());
        let did_you_mean = suggest_terms(&st.bases, coll.as_deref(), &base, &missing, 6);
        slog(&format!("   └─ AUSENTE: nada ancora no corpus · did-you-mean={did_you_mean:?}"));
        return (200, json!({
            "query": query, "provider": provider, "source": source,
            "expansions": exps, "absent": true, "dropped": dropped,
            "reason": "nem a consulta nem os sinônimos têm sílabas neste escopo do corpus",
            "did_you_mean": did_you_mean, "hits": []
        }).to_string());
    }
    let mut variants = vec![query.to_string()];
    variants.extend(kept.iter().cloned());
    // merge por (coll,base,chunk) -> melhor cobertura (original ganha leve desempate)
    let mut best: HashMap<(String, String, u64), (f64, Value, usize)> = HashMap::new();
    for (qi, q) in variants.iter().enumerate() {
        let mut qb = Map::new();
        if let Some(c) = &coll { qb.insert("collection".into(), json!(c)); }
        qb.insert("base".into(), json!(base));
        qb.insert("query".into(), json!(q));
        qb.insert("k".into(), json!(k));
        qb.insert("phonetic".into(), json!(phon));
        let (code, res) = search(&Value::Object(qb).to_string(), &st.bases, &st.collection_profiles);
        if code != 200 { slog(&format!("   │  ├ {} {q:?} → erro {code}", if qi == 0 { "orig" } else { "var " })); continue; }
        let rv: Value = serde_json::from_str(&res).unwrap_or(Value::Null);
        let nh = rv["hits"].as_array().map(|a| a.len()).unwrap_or(0);
        let conv: u64 = rv["searched"].as_array()
            .map(|a| a.iter().map(|x| x["n_converge"].as_u64().unwrap_or(0)).sum()).unwrap_or(0);
        let glyph = if qi + 1 == variants.len() { "└" } else { "├" };
        slog(&format!("   │  {glyph} {} {q:?} → {nh} hit(s), {conv} chunk(s) convergem o cosseno",
                      if qi == 0 { "orig" } else { "var " }));
        if let Some(hits) = rv["hits"].as_array() {
            for h in hits {
                let kk = (h["collection"].as_str().unwrap_or("").to_string(),
                          h["base"].as_str().unwrap_or("").to_string(),
                          h["chunk"].as_u64().unwrap_or(0));
                let cov = h["coverage"].as_f64().or_else(|| h["matchpoint"].as_f64()).unwrap_or(0.0);
                let weighted = if qi == 0 { cov + 0.001 } else { cov };
                let e = best.entry(kk).or_insert((-1.0, Value::Null, qi));
                if weighted > e.0 { *e = (weighted, h.clone(), qi); }
            }
        }
    }
    // RERANK contra a INTENÇÃO ORIGINAL (fix #1). A cobertura de uma variante de 1 termo é
    // sempre 1.0 — rankear por ela fazia o lixo subir ao topo ("seguir" casando "Pippin
    // exausto", hits de outra coleção com matchpoint 1.0). Aqui cada candidato é rescorado
    // com a query ORIGINAL; esse vira o matchpoint EXIBIDO (inspecionável a olho nu). A
    // cobertura da variante (`var_cov`) só desempata, premiando o sinônimo que de fato ajudou.
    let qt = rag::prep_query(query);
    // [#5] peso unificado por coleção pro rescore (perfis já construídos pelas buscas das
    // variantes via `search`); coleção sem perfil cai no peso local dentro do score_chunk.
    let exp_weightings: HashMap<String, Vec<f64>> = {
        let p = st.collection_profiles.read();
        p.iter().map(|(c, prof)| (c.clone(), rag::weighting_unified(&qt, prof))).collect()
    };
    let mut rows: Vec<(f64, f64, i64, usize, Value)> = best.into_values()
        .map(|(var_cov, mut h, via)| {
            let coll_h = h["collection"].as_str().unwrap_or("").to_string();
            let base_h = h["base"].as_str().unwrap_or("").to_string();
            let cid = h["chunk"].as_u64().unwrap_or(0) as usize;
            let w = exp_weightings.get(&coll_h).map(|v| v.as_slice());
            let (orig_cov, orig_span) = st.bases.get(&coll_h)
                .and_then(|m| m.get(&base_h))
                .map(|b| b.score_chunk(&qt, w, cid, phon))
                .unwrap_or((0.0, 0));
            if let Some(o) = h.as_object_mut() {
                o.insert("matchpoint".into(), json!(orig_cov));
                o.insert("coverage".into(), json!(orig_cov));
                o.insert("span".into(), json!(orig_span));
                o.insert("var_cov".into(), json!(var_cov));
            }
            (orig_cov, var_cov, -(orig_span as i64), via, h)
        }).collect();
    // cobertura ORIGINAL ↓ · cobertura de variante ↓ · span ↑ · original desempata
    rows.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap()
        .then(b.1.partial_cmp(&a.1).unwrap())
        .then(b.2.cmp(&a.2))
        .then((b.3 == 0).cmp(&(a.3 == 0))));
    let hits: Vec<Value> = rows.into_iter().take(k).enumerate().map(|(i, (.., via, mut h))| {
        if let Some(o) = h.as_object_mut() {
            o.insert("rank".into(), json!(i + 1));
            o.insert("via".into(), json!(if via == 0 { "original".to_string() } else { variants[via].clone() }));
        }
        h
    }).collect();
    let top = hits.first().map(|h| format!(" · top cov={:.2} {}/{}#{}",
        h["coverage"].as_f64().or_else(|| h["matchpoint"].as_f64()).unwrap_or(0.0),
        h["collection"].as_str().unwrap_or("?"), h["base"].as_str().unwrap_or("?"),
        h["chunk"].as_u64().unwrap_or(0))).unwrap_or_default();
    slog(&format!("   └─ merge ({} variante(s), peso no original) → {} hit(s) por matchpoint{}",
                  variants.len(), hits.len(), top));
    (200, json!({"query": query, "provider": provider, "source": source,
                 "expansions": exps, "absent": false, "dropped": dropped, "hits": hits}).to_string())
}

/// Proxy HTTP do console (ValHalla) pra um MÓDULO externo (nidhoggd na 11497, etc.) via wget.
/// O ragd não depende do código do módulo — só conhece a URL. Se o módulo não responder, devolve
/// {"online":false} com 200 pra UI degradar graciosa (keepalive embutido). Online → injeta online:true.
fn module_proxy(url: &str, post_body: Option<&str>) -> (u16, String) {
    // portátil: tenta curl (mac/linux), cai pra wget. Ambos por subprocess (sem dep nova).
    let run = |tool: &str| -> Option<String> {
        let mut cmd = std::process::Command::new(tool);
        if tool == "curl" {
            cmd.args(["-s", "-m", "5"]);
            if let Some(b) = post_body { cmd.args(["-H", "Content-Type: application/json", "-d", b]); }
            cmd.arg(url);
        } else {
            cmd.args(["-q", "-O", "-", "--tries=1", "--timeout=5"]);
            if let Some(b) = post_body { cmd.arg("--header=Content-Type: application/json").arg(format!("--post-data={b}")); }
            cmd.arg(url);
        }
        let out = cmd.output().ok()?;
        if out.status.success() && !out.stdout.is_empty() { Some(String::from_utf8_lossy(&out.stdout).to_string()) } else { None }
    };
    match run("curl").or_else(|| run("wget")) {
        Some(txt) => match serde_json::from_str::<Value>(&txt) {
            Ok(Value::Object(mut m)) => { m.insert("online".into(), json!(true)); (200, Value::Object(m).to_string()) }
            _ => (200, txt),
        },
        None => (200, json!({"online": false, "module_url": url}).to_string()),
    }
}

/// GET /api/config — estado da configuração (chaves mascaradas, nunca o valor cru).
fn config_json(st: &State) -> String {
    let hybrid = !rag::CACHE_WORDS.load(std::sync::atomic::Ordering::Relaxed);
    json!({
        "storage": if hybrid { "hybrid" } else { "memory" },
        "config_path": st.config_path,
        "drivers_dir": st.drivers_dir, "ragfiles_dir": st.ragfiles_dir,
        "max_upload_mb": st.max_upload / (1024 * 1024),
        "admin_user": st.admin_user,
        "admin_is_default": is_default_creds(&st.admin_user, &st.admin_pass),
        "dev_mode": st.dev,
        "anthropic_key_set": !st.anthropic_key.is_empty(), "anthropic_key": st.anthropic_key,
        "openai_key_set": !st.openai_key.is_empty(), "openai_key": st.openai_key,
        "active_provider": st.active_provider,
        "cache_dir": st.cache_dir,
        "expansions_entries": st.expansions.read().len(),
        "thesaurus_dir": st.thesaurus_dir,
        "dicts_active": dict_dirs(&st.thesaurus_dir).iter().filter(|p| p.join("inuse.flag").exists()).count(),
        "word_syn_entries": st.word_syn.len(),
    }).to_string()
}

/// Testa uma chave de API com uma chamada real (lista de modelos) via `wget` — sem
/// dependência nova de cliente HTTP. Devolve (ok, mensagem).
fn test_provider_key(provider: &str, key: &str) -> (bool, String) {
    if key.trim().is_empty() { return (false, "chave não cadastrada".into()); }
    let (url, headers): (&str, Vec<String>) = match provider {
        "openai" => ("https://api.openai.com/v1/models", vec![format!("Authorization: Bearer {key}")]),
        "anthropic" => ("https://api.anthropic.com/v1/models",
                        vec![format!("x-api-key: {key}"), "anthropic-version: 2023-06-01".into()]),
        _ => return (false, "provider inválido".into()),
    };
    let mut cmd = std::process::Command::new("wget");
    cmd.args(["-S", "-O", "-", "--content-on-error", "--timeout=12", "--tries=1"]);
    for h in &headers { cmd.arg(format!("--header={h}")); }
    cmd.arg(url);
    match cmd.output() {
        Ok(out) => {
            let body = String::from_utf8_lossy(&out.stdout);
            let err = String::from_utf8_lossy(&out.stderr);   // -S joga os headers HTTP aqui
            // status HTTP da última linha "HTTP/x yyy"
            let status: u16 = err.lines().rev()
                .find_map(|l| l.trim().strip_prefix("HTTP/")
                    .and_then(|s| s.split_whitespace().nth(1)).and_then(|c| c.parse().ok()))
                .unwrap_or(0);
            if status == 200 && body.contains("\"data\"") {
                (true, "chave válida ✓".into())
            } else if status == 0 {
                (false, format!("sem conexão: {}", err.lines().rev().find(|l| l.contains("rror") || l.contains("ailed")).unwrap_or("wget falhou").trim()))
            } else {
                // mensagem do corpo de erro, se houver
                let detail = if !body.trim().is_empty() { body.chars().take(140).collect::<String>().replace('\n', " ") }
                             else { "chave rejeitada".into() };
                (false, format!("HTTP {status} — {detail}"))
            }
        }
        Err(e) => (false, format!("wget indisponível no host: {e}")),
    }
}

/// POST /api/config — aplica e PERSISTE mudanças do painel (storage, chaves de API).
/// Trocar storage recarrega as bases (pra liberar/recachear a RAM de fato).
fn set_config(body: &str, st: &mut State) -> (u16, String) {
    let v: Value = match serde_json::from_str(body) {
        Ok(v) => v, Err(e) => return (400, json!({"error": format!("JSON inválido: {e}")}).to_string()),
    };
    let mut notes: Vec<String> = vec![];
    let mut reload = false;
    if let Some(u) = v["admin_user"].as_str() { let u = u.trim(); if !u.is_empty() {
        st.admin_user = u.to_string(); set_cfg_key(&st.config_path, "admin_user", u);
        notes.push(format!("admin_user → {u}"));
    }}
    if let Some(p) = v["admin_pass"].as_str() { let p = p.trim(); if !p.is_empty() {
        st.admin_pass = p.to_string(); set_cfg_key(&st.config_path, "admin_pass", p);
        notes.push("admin_pass atualizada".into());
    }}
    if let Some(s) = v["storage"].as_str() {
        let s = s.to_lowercase();
        if s != "memory" && s != "hybrid" {
            return (400, json!({"error": "storage deve ser 'memory' ou 'hybrid'"}).to_string());
        }
        let want_hybrid = s == "hybrid";
        if want_hybrid != !rag::CACHE_WORDS.load(std::sync::atomic::Ordering::Relaxed) {
            rag::CACHE_WORDS.store(!want_hybrid, std::sync::atomic::Ordering::Relaxed);
            set_cfg_key(&st.config_path, "storage", &s);
            reload = true;
            notes.push(format!("storage → {s} (recarregado)"));
            notes.push("obs: RAM só é devolvida ao SO no próximo restart (of_launch / @reboot); o cfg já está salvo".into());
        }
    }
    if let Some(k) = v["anthropic_key"].as_str() { if !k.trim().is_empty() {
        st.anthropic_key = k.trim().to_string(); set_cfg_key(&st.config_path, "anthropic_key", k.trim());
        notes.push("anthropic_key salva".into());
    }}
    if let Some(k) = v["openai_key"].as_str() { if !k.trim().is_empty() {
        st.openai_key = k.trim().to_string(); set_cfg_key(&st.config_path, "openai_key", k.trim());
        notes.push("openai_key salva".into());
    }}
    if v["clear_anthropic"].as_bool() == Some(true) {
        st.anthropic_key.clear(); set_cfg_key(&st.config_path, "anthropic_key", "");
        if st.active_provider == "anthropic" { st.active_provider = "none".into(); set_cfg_key(&st.config_path, "active_provider", "none"); }
        notes.push("anthropic_key removida".into());
    }
    if v["clear_openai"].as_bool() == Some(true) {
        st.openai_key.clear(); set_cfg_key(&st.config_path, "openai_key", "");
        if st.active_provider == "openai" { st.active_provider = "none".into(); set_cfg_key(&st.config_path, "active_provider", "none"); }
        notes.push("openai_key removida".into());
    }
    // SÓ UM provider ativo por vez: ativar um desativa o outro
    if let Some(p) = v["active_provider"].as_str() {
        let p = p.to_lowercase();
        match p.as_str() {
            "none" => { st.active_provider = "none".into(); set_cfg_key(&st.config_path, "active_provider", "none"); notes.push("LLM desativado (nenhum provider)".into()); }
            "anthropic" | "openai" => {
                let has = if p == "anthropic" { !st.anthropic_key.is_empty() } else { !st.openai_key.is_empty() };
                if !has { return (400, json!({"error": format!("não dá pra ativar '{p}': chave não cadastrada")}).to_string()); }
                st.active_provider = p.clone(); set_cfg_key(&st.config_path, "active_provider", &p);
                notes.push(format!("provider ativo → {p} (o outro fica inativo)"));
            }
            _ => return (400, json!({"error": "active_provider deve ser none|anthropic|openai"}).to_string()),
        }
    }
    if reload {
        st.bases.clear();
        autoload_ragfiles(&st.ragfiles_dir, &mut st.bases);
    }
    if notes.is_empty() { notes.push("nada mudou".into()); }
    (200, json!({"ok": true, "notes": notes, "reloaded": reload, "config": serde_json::from_str::<Value>(&config_json(st)).unwrap_or(Value::Null)}).to_string())
}

/// Responde JSON com status e (opcional) um Set-Cookie.
fn respond_json(req: Request, code: u16, payload: String, set_cookie: Option<&str>) {
    let mut resp = Response::from_string(payload).with_status_code(code).with_header(
        Header::from_bytes(&b"Content-Type"[..], &b"application/json; charset=utf-8"[..]).unwrap());
    if let Some(c) = set_cookie {
        resp.add_header(Header::from_bytes(&b"Set-Cookie"[..], c.as_bytes()).unwrap());
    }
    let _ = req.respond(resp);
}

/// Servidor da porta de controle (ValHalla): serve o shell HTML (público — sem segredos)
/// + endpoints /api/* atrás de SESSÃO POR COOKIE (login/logout reais). Reusa o motor da API.
fn handle_dashboard(mut req: Request, state: &Arc<RwLock<State>>) {
    let method = req.method().clone();
    let full = req.url().to_string();
    let (path, query) = match full.split_once('?') {
        Some((p, q)) => (p.to_string(), q.to_string()),
        None => (full, String::new()),
    };
    let ip = req.remote_addr().map(|a| a.ip().to_string()).unwrap_or_else(|| "?".into());
    let headers: Vec<(String, String)> = req.headers().iter()
        .map(|h| (h.field.as_str().as_str().to_string(), h.value.as_str().to_string()))
        .collect();
    let t0 = Instant::now();

    // shell HTML é público (não tem dado; os /api é que exigem sessão)
    if method == Method::Get && (path == "/" || path == "/index.html") {
        let resp = Response::from_string(DASHBOARD_HTML)
            .with_header(Header::from_bytes(&b"Content-Type"[..], &b"text/html; charset=utf-8"[..]).unwrap())
            .with_header(Header::from_bytes(&b"Cache-Control"[..], &b"no-store"[..]).unwrap());
        let _ = req.respond(resp);
        return;
    }

    // ping público (loader do restart usa pra saber quando o daemon voltou)
    if method == Method::Get && path == "/api/ping" {
        respond_json(req, 200, json!({"ok": true, "version": VERSION}).to_string(), None);
        return;
    }

    // corpo (POST)
    let mut body = Vec::new();
    if method == Method::Post { req.as_reader().take(8 * 1024 * 1024).read_to_end(&mut body).ok(); }
    let body_str = std::str::from_utf8(&body).unwrap_or("").to_string();

    // login: valida credenciais -> cria sessão -> Set-Cookie
    if method == Method::Post && path == "/api/login" {
        let v: Value = serde_json::from_str(&body_str).unwrap_or(json!({}));
        let (u, p) = (v["user"].as_str().unwrap_or(""), v["pass"].as_str().unwrap_or(""));
        let mut st = state.write();
        if is_default_creds(&st.admin_user, &st.admin_pass) && !st.dev {
            println!("[{}] [valhalla] {ip} login RECUSADO — credenciais padrão fora do --dev (user={u})", now_stamp());
            respond_json(req, 403, json!({"error": "credenciais padrão não permitidas fora do modo dev. Altere admin_user/admin_pass no ragnarock.cfg ou inicie com --dev."}).to_string(), None);
        } else if u == st.admin_user && p == st.admin_pass {
            let tok = gen_token();
            st.sessions.retain(|_, t| t.elapsed().as_secs() < SESSION_TTL);   // limpa expiradas
            st.sessions.insert(tok.clone(), Instant::now());
            let cookie = format!("vh_session={tok}; HttpOnly; Path=/; Max-Age={SESSION_TTL}; SameSite=Strict");
            println!("[{}] [valhalla] {ip} login OK (user={u})", now_stamp());
            respond_json(req, 200, json!({"ok": true, "user": u}).to_string(), Some(&cookie));
        } else {
            println!("[{}] [valhalla] {ip} login FALHOU (user={u})", now_stamp());
            respond_json(req, 401, json!({"error": "credenciais inválidas"}).to_string(), None);
        }
        return;
    }

    // logout: descarta a sessão + limpa o cookie
    if method == Method::Post && path == "/api/logout" {
        if let Some(t) = cookie_val(&headers, "vh_session") { state.write().sessions.remove(&t); }
        let cookie = "vh_session=; HttpOnly; Path=/; Max-Age=0; SameSite=Strict";
        println!("[{}] [valhalla] {ip} logout", now_stamp());
        respond_json(req, 200, json!({"ok": true}).to_string(), Some(cookie));
        return;
    }

    // demais /api/* exigem sessão válida
    let authed = { let st = state.read(); session_ok(&headers, &st.sessions) };
    if !authed {
        println!("[{}] [valhalla] {ip} {method:?} {path} -> 401 (sem sessão)", now_stamp());
        respond_json(req, 401, json!({"error": "sessão necessária", "login": true}).to_string(), None);
        return;
    }

    // restart/aplicar: responde, dá um tempo pro browser receber, e re-exec o daemon
    if method == Method::Post && path == "/api/restart" {
        println!("[{}] [valhalla] {ip} RESTART solicitado — re-exec em 400ms", now_stamp());
        respond_json(req, 200, json!({"ok": true, "restarting": true}).to_string(), None);
        thread::spawn(|| { thread::sleep(std::time::Duration::from_millis(400)); restart_self(); });
        return;
    }

    // MÓDULOS externos (Nidhogg etc.): o console agrega via proxy HTTP. CRÍTICO: fazer FORA
    // do lock do State — o módulo chama a API do ragd de volta e deadlockaria no mesmo Mutex.
    if path.starts_with("/api/nidhogg") {
        let url = { state.read().nidhogg_url.clone() };
        let q = if query.is_empty() { String::new() } else { format!("?{query}") };
        let (code, payload) = module_proxy(&format!("{url}{path}{q}"), if method == Method::Post { Some(&body_str) } else { None });
        log_line("valhalla", &ip, &method, &path, &query, code, t0.elapsed().as_secs_f64() * 1000.0, &req_extra(&path, &body, &payload));
        respond_json(req, code, payload, None);
        return;
    }

    // [#6] write apenas pras rotas que mutam o State (config POST, thesaurus_toggle, ingest_upload);
    // o resto roda sob read() — N polls de stats/logs/search do ValHalla em paralelo com a API.
    let is_w = matches!((&method, path.as_str()),
        (Method::Post, "/api/config") | (Method::Post, "/api/thesaurus_toggle")
        | (Method::Post, "/api/ingest_upload"));
    let (code, payload) = if is_w {
        let mut st = state.write();
        match (&method, path.as_str()) {
            (Method::Post, "/api/config")           => set_config(&body_str, &mut *st),
            (Method::Post, "/api/thesaurus_toggle") => dict_toggle(&body_str, &mut *st),
            (Method::Post, "/api/ingest_upload")    => ingest_upload(&query, &headers, &body, &mut *st),
            _ => unreachable!(),
        }
    } else {
        let st = state.read();
        match (&method, path.as_str()) {
            (Method::Get, "/api/stats")       => (200, stats_json(&st)),
            (Method::Get, "/api/config")      => (200, config_json(&st)),
            (Method::Post, "/api/test_key")   => {
                let pv: Value = serde_json::from_str(&body_str).unwrap_or(Value::Null);
                let prov = pv["provider"].as_str().unwrap_or("");
                let key = if prov == "anthropic" { &st.anthropic_key } else if prov == "openai" { &st.openai_key } else { "" };
                let (ok, msg) = test_provider_key(prov, key);
                (if ok { 200 } else { 400 }, json!({"ok": ok, "provider": prov, "message": msg}).to_string())
            }
            (Method::Get, "/api/logs")        => {
                let n = query_param(&query, "n").and_then(|s| s.parse().ok()).unwrap_or(300usize);
                (200, json!({"file": st.log_file, "log": tail_lines(&st.log_file, n)}).to_string())
            }
            (Method::Get, "/api/collections") => list_collections(&st.bases),
            (Method::Get, "/api/bases")       => list_bases(&query, &st.bases),
            (Method::Get, "/api/drivers")     => list_drivers(&query, &st.drivers_dir),
            (Method::Get, "/api/drivers_out") => {
                let out = format!("{}.out", st.drivers_dir);
                if Path::new(&out).is_dir() { list_drivers(&query, &out) }
                else { (200, json!({"drivers_dir": out, "match": "*", "count": 0, "drivers": []}).to_string()) }
            }
            (Method::Post, "/api/driver_move")   => driver_move(&body_str, &st.drivers_dir),
            (Method::Get,  "/api/thesaurus")     => list_dicts(&query, &st.thesaurus_dir),
            (Method::Post, "/api/search")        => search(&body_str, &st.bases, &st.collection_profiles),
            (Method::Post, "/api/search_expand") => search_expand(&body_str, &*st),
            (Method::Post, "/api/histogram")     => histogram(&body_str, &st.bases),
            (Method::Post, "/api/chunk")         => fetch_chunk(&body_str, &st.bases),
            _ => (404, json!({"error": "rota dashboard não encontrada", "path": path}).to_string()),
        }
    };
    // loga ações no dashboard (menos os pollers stats/logs, pra não poluir)
    if path != "/api/stats" && path != "/api/logs" {
        let ms = t0.elapsed().as_secs_f64() * 1000.0;
        log_line("valhalla", &ip, &method, &path, &query, code, ms, &req_extra(&path, &body, &payload));
    }
    respond_json(req, code, payload, None);
}

/// [#6] Decide o lock antes do dispatch: rotas que mutam o State pedem write(); o resto read().
fn is_write_route(method: &Method, path: &str) -> bool {
    matches!((method, path),
        (Method::Post, "/ingest") | (Method::Post, "/ingest_file") | (Method::Post, "/ingest_upload"))
    || matches!(method, Method::Delete)   // /bases/{name}, /collections/{name}
}

/// [#6] Dispatch READ-ONLY: roda sob `state.read()`, N requests em paralelo. Os caches que
/// search/search_expand precisam mutar (collection_profiles, expansions) têm interior
/// mutability (RwLock<>) — sob outer-read continuam funcionando.
fn route_ro(method: &Method, path: &str, query: &str, _headers: &[(String, String)],
            body_bytes: &[u8], state: &State) -> (u16, String) {
    let body_str = || std::str::from_utf8(body_bytes).unwrap_or("");
    match (method, path) {
        (Method::Get, "/health") =>
            (200, json!({"status": "ok", "bases": total_bases(&state.bases),
                         "collections": state.bases.len(),
                         "drivers": count_drivers(&state.drivers_dir)}).to_string()),
        (Method::Get, "/bases") => list_bases(query, &state.bases),
        (Method::Get, p) if p.starts_with("/bases/") && p[7..].contains('/') => {   // [#4]
            let rest = &p[7..];
            match rest.split_once('/') {
                Some((coll, name)) => base_meta(coll, name, &state.bases),
                None => (404, json!({"error": "uso: GET /bases/{coll}/{name}"}).to_string()),
            }
        }
        (Method::Get, "/collections") => list_collections(&state.bases),
        (Method::Get, "/profile") => profile(query, &state.bases),                  // [#1]
        (Method::Get, "/stats") => (200, stats_json(state)),                        // [#3]
        (Method::Get, "/drivers") => list_drivers(query, &state.drivers_dir),
        (Method::Get, "/thesaurus") => list_dicts(query, &state.thesaurus_dir),
        (Method::Get, "/interpret") => interpret(query, &state.drivers_dir),
        (Method::Post, "/search") => search(body_str(), &state.bases, &state.collection_profiles),
        (Method::Post, "/search_expand") => search_expand(body_str(), state),
        (Method::Post, "/chunk") => fetch_chunk(body_str(), &state.bases),
        _ => (404, json!({"error": "rota não encontrada", "path": path}).to_string()),
    }
}

/// [#6] Dispatch READ-WRITE: roda sob `state.write()`, exclusivo. Só ingest e delete entram aqui.
fn route(method: &Method, path: &str, query: &str, headers: &[(String, String)],
         body_bytes: &[u8], state: &mut State) -> (u16, String) {
    let body_str = || std::str::from_utf8(body_bytes).unwrap_or("");
    match (method, path) {
        (Method::Post, "/ingest") => ingest(body_str(), &state.drivers_dir, &state.ragfiles_dir, &mut state.bases),
        (Method::Post, "/ingest_file") => ingest_file(body_str(), &state.drivers_dir, &state.ragfiles_dir, &mut state.bases),
        (Method::Post, "/ingest_upload") => ingest_upload(query, headers, body_bytes, state),
        (Method::Delete, p) if p.starts_with("/bases/") => {
            let name = &p["/bases/".len()..];
            let coll = query_param(query, "collection").unwrap_or_else(|| DEFAULT_COLLECTION.to_string());
            if remove_base(&mut state.bases, &coll, name) {
                (200, json!({"ok": true, "removed": name, "collection": coll,
                             "bases": total_bases(&state.bases)}).to_string())
            } else {
                (404, json!({"error": format!("base '{coll}/{name}' não encontrada")}).to_string())
            }
        }
        (Method::Delete, p) if p.starts_with("/collections/") => {                  // [#2]
            let name = &p["/collections/".len()..];
            drop_collection(name, query, state)
        }
        _ => (404, json!({"error": "rota write não encontrada", "path": path}).to_string()),
    }
}

/// POST /ingest — 3 modos:
///   1) {name, path}            -> path aponta pra JSON tokenizado (como antes)
///   2) {name, data:<base>}     -> base inline JSON tokenizada (como antes)
///   3) {name, path, raw:true,  -> path e' arquivo BRUTO (codigo/texto). Daemon
///       chunk?, driver?, with_text?, max_chunks?}     tokeniza usando o driver
///                                                      apontado por 'driver' ou
///                                                      auto-detectado pela ext.
fn ingest(body: &str, drivers_dir: &str, ragfiles_dir: &str, bases: &mut Bases) -> (u16, String) {
    let v: Value = match serde_json::from_str(body) {
        Ok(v) => v,
        Err(e) => return (400, json!({"error": format!("body JSON inválido: {e}")}).to_string()),
    };
    let name = match v["name"].as_str() {
        Some(n) => safe_name(n),
        None => return (400, json!({"error": "falta 'name'"}).to_string()),
    };
    let collection = v["collection"].as_str().unwrap_or(DEFAULT_COLLECTION).to_string();
    let raw_mode = v["raw"].as_bool().unwrap_or(false);

    let mut saved_to: Option<String> = None;
    let res: Result<RagBase, String> = if raw_mode {
        let path = match v["path"].as_str() {
            Some(p) => p, None => return (400, json!({"error": "raw=true exige 'path' (arquivo bruto)"}).to_string()),
        };
        ingest_raw_to_base(path, drivers_dir, ragfiles_dir, &collection, &name, &v, &mut saved_to)
    } else if let Some(p) = v["path"].as_str() {
        RagBase::load(p)
    } else if !v["data"].is_null() {
        RagBase::from_str(&v["data"].to_string()).map(|mut b| { b.mtime = rag::now_secs(); b })
    } else {
        return (400, json!({"error": "forneça 'path', 'data' (JSON tokenizado) ou {path, raw:true} (arquivo bruto)"}).to_string());
    };
    match res {
        Ok(b) => {
            let n = b.n_chunks;
            insert_base(bases, &collection, name.clone(), b);
            let mut r = Map::new();
            r.insert("ok".into(), json!(true));
            r.insert("collection".into(), json!(collection));
            r.insert("name".into(), json!(name));
            r.insert("n_chunks".into(), json!(n));
            r.insert("bases".into(), json!(total_bases(bases)));
            r.insert("raw".into(), json!(raw_mode));
            if let Some(p) = saved_to { r.insert("saved_to".into(), json!(p)); }
            (200, Value::Object(r).to_string())
        }
        Err(e) => (400, json!({"error": e}).to_string()),
    }
}

/// POST /ingest_file — atalho dedicado pra ingerir arquivo BRUTO.
///   {path, name?, chunk?, driver?, with_text?, max_chunks?}
/// 'name' default = derive_base_name(path) (path achatado, ex: logic_path__03_histogram_py).
/// 'driver' default = auto pela extensao (fallback PTBR).
/// Sempre grava o JSON tokenizado em ragfiles_dir/<name>-tokenized.json.
fn ingest_file(body: &str, drivers_dir: &str, ragfiles_dir: &str, bases: &mut Bases) -> (u16, String) {
    let v: Value = match serde_json::from_str(body) {
        Ok(v) => v,
        Err(e) => return (400, json!({"error": format!("body JSON inválido: {e}")}).to_string()),
    };
    let path = match v["path"].as_str() {
        Some(p) => p, None => return (400, json!({"error": "falta 'path'"}).to_string()),
    };
    let collection = v["collection"].as_str().unwrap_or(DEFAULT_COLLECTION).to_string();
    let name = safe_name(&v["name"].as_str().map(|s| s.to_string())
        .unwrap_or_else(|| ingestor::derive_base_name(Path::new(path))));

    let mut saved_to: Option<String> = None;
    match ingest_raw_to_base(path, drivers_dir, ragfiles_dir, &collection, &name, &v, &mut saved_to) {
        Ok(b) => {
            let n = b.n_chunks;
            let corpus = b.corpus.clone();
            insert_base(bases, &collection, name.clone(), b);
            let mut r = Map::new();
            r.insert("ok".into(), json!(true));
            r.insert("collection".into(), json!(collection));
            r.insert("name".into(), json!(name));
            r.insert("corpus".into(), json!(corpus));
            r.insert("n_chunks".into(), json!(n));
            r.insert("bases".into(), json!(total_bases(bases)));
            if let Some(p) = saved_to { r.insert("saved_to".into(), json!(p)); }
            (200, Value::Object(r).to_string())
        }
        Err(e) => (400, json!({"error": e}).to_string()),
    }
}

/// POST /ingest_upload — recebe o ARQUIVO via HTTP (sem precisar do path local).
///
/// Dois modos via Content-Type:
///   a) multipart/form-data: campo 'file' (arquivo), demais campos textuais
///      (name, filename, chunk, driver, with_text, max_chunks)
///   b) qualquer outro (application/octet-stream, text/plain, etc): body inteiro
///      e' o arquivo; metadados via query string (?filename=foo.py&name=hist&chunk=...)
///
/// Em ambos os modos: grava ragfiles_dir/<name>-tokenized.json e carrega em memoria.
/// Limite de tamanho: --max-upload (default 1 GB). Estoura -> HTTP 413.
fn ingest_upload(query: &str, headers: &[(String, String)], body: &[u8], state: &mut State) -> (u16, String) {
    let t0 = std::time::Instant::now();
    if body.len() > state.max_upload {
        return (413, json!({"error": format!("upload excede limite de {} bytes (--max-upload)", state.max_upload)}).to_string());
    }
    // helper: case-insensitive lookup de header
    let header = |name: &str| -> Option<String> {
        headers.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.clone())
    };
    let ct = header("Content-Type").unwrap_or_default();
    let is_multipart = ct.to_lowercase().starts_with("multipart/form-data");

    // resolve content + metadados
    let (filename, content_bytes, fields): (String, Vec<u8>, HashMap<String, String>) = if is_multipart {
        let boundary = match multipart::extract_boundary(&ct) {
            Some(b) => b, None => return (400, json!({"error": "Content-Type multipart sem boundary"}).to_string()),
        };
        let parts = match multipart::parse(body, &boundary) {
            Ok(p) => p,
            Err(e) => return (400, json!({"error": format!("multipart inválido: {e}")}).to_string()),
        };
        let mut file_bytes: Option<Vec<u8>> = None;
        let mut fname_from_part: Option<String> = None;
        let mut fields: HashMap<String, String> = HashMap::new();
        for part in parts {
            if part.name == "file" {
                fname_from_part = part.filename.clone();
                file_bytes = Some(part.bytes);
            } else {
                let v = String::from_utf8(part.bytes).unwrap_or_default();
                fields.insert(part.name, v);
            }
        }
        let content = match file_bytes {
            Some(b) => b, None => return (400, json!({"error": "multipart sem campo 'file'"}).to_string()),
        };
        // filename: campo explicito > filename do part > erro
        let filename = fields.get("filename").cloned()
            .or(fname_from_part)
            .unwrap_or_else(|| "upload.bin".to_string());
        (filename, content, fields)
    } else {
        // raw body — pega metadados da query string
        let mut fields: HashMap<String, String> = HashMap::new();
        for kv in query.split('&').filter(|s| !s.is_empty()) {
            if let Some((k, v)) = kv.split_once('=') {
                fields.insert(percent_decode(k), percent_decode(v));
            }
        }
        let filename = fields.get("filename").cloned().unwrap_or_else(|| "upload.bin".to_string());
        (filename, body.to_vec(), fields)
    };

    let via = if is_multipart { "multipart" } else { "raw" };
    tlog("ingest", &format!("┌─ ingest_upload via={via} filename={filename:?} bytes={}", content_bytes.len()));

    // conteudo precisa ser UTF-8 pra tokenizar como texto
    let text = match std::str::from_utf8(&content_bytes) {
        Ok(s) => s.to_string(),
        Err(e) => {
            tlog("ingest", &format!("   └─ FALHOU: arquivo não é UTF-8 ({e})"));
            return (400, json!({"error": format!("arquivo nao e' UTF-8: {e}")}).to_string());
        }
    };

    let name = safe_name(&fields.get("name").cloned()
        .unwrap_or_else(|| ingestor::derive_base_name(Path::new(&filename))));
    let collection = fields.get("collection").cloned()
        .unwrap_or_else(|| DEFAULT_COLLECTION.to_string());
    let chunk_size: usize = fields.get("chunk").and_then(|s| s.parse().ok()).unwrap_or(2048);
    let max_chunks: usize = fields.get("max_chunks").and_then(|s| s.parse().ok()).unwrap_or(0);
    let with_text: bool = fields.get("with_text").map(|s| s != "false" && s != "0").unwrap_or(true);
    let driver_override = fields.get("driver").map(|s| s.as_str());
    let source_label = format!("<upload:{filename}>");
    // append=true (query/field): acumula na base existente em vez de sobrescrever.
    let append: bool = fields.get("append").map(|s| s != "false" && s != "0").unwrap_or(false);
    tlog("ingest", &format!("   ├─ destino: {collection}/{name} driver={} chunk={chunk_size}{}{}",
                            driver_override.unwrap_or("auto"),
                            if max_chunks > 0 { format!(" max_chunks={max_chunks}") } else { String::new() },
                            if append { " append" } else { "" }));

    // persiste em ragfiles_dir/<collection>/<name>-tokenized.json
    let rag_dir = Path::new(&state.ragfiles_dir).join(&collection);
    if let Err(e) = std::fs::create_dir_all(&rag_dir) {
        return (500, json!({"error": format!("nao criou {}: {e}", rag_dir.display())}).to_string());
    }
    let out_path = rag_dir.join(format!("{name}-tokenized.json"));

    // append so' faz sentido se a base ja existe; senao cria normal (sem erro).
    let did_append = append && out_path.exists();
    let tk = std::time::Instant::now();
    let value = if did_append {
        let existing_str = match std::fs::read_to_string(&out_path) {
            Ok(s) => s,
            Err(e) => { tlog("ingest", &format!("   └─ FALHOU: ler base p/ append: {e}"));
                        return (500, json!({"error": format!("ler base p/ append {}: {e}", out_path.display())}).to_string()); }
        };
        let existing: Value = match serde_json::from_str(&existing_str) {
            Ok(v) => v,
            Err(e) => { tlog("ingest", &format!("   └─ FALHOU: base existente não é JSON válido: {e}"));
                        return (400, json!({"error": format!("base existente nao e' JSON valido: {e}")}).to_string()); }
        };
        match ingestor::tokenize_content_append(&existing, &text, &source_label, Path::new(&state.drivers_dir)) {
            Ok(v) => v,
            Err(e) => { tlog("ingest", &format!("   └─ FALHOU na tokenização: {e}"));
                        return (400, json!({"error": e}).to_string()); }
        }
    } else {
        match ingestor::tokenize_content(
            &text, &filename, &source_label,
            Path::new(&state.drivers_dir), driver_override, chunk_size, max_chunks, with_text,
        ) {
            Ok(v) => v,
            Err(e) => { tlog("ingest", &format!("   └─ FALHOU na tokenização: {e}"));
                        return (400, json!({"error": e}).to_string()); }
        }
    };
    tlog("ingest", &format!("   ├─ tokenizado em {:.0}ms (texto {} bytes)",
                            tk.elapsed().as_secs_f64() * 1000.0, text.len()));
    let mut buf = Vec::new();
    let fmt = serde_json::ser::PrettyFormatter::with_indent(b"\t");
    let mut ser = serde_json::Serializer::with_formatter(&mut buf, fmt);
    if let Err(e) = serde::Serialize::serialize(&value, &mut ser) {
        return (500, json!({"error": format!("serialize JSON: {e}")}).to_string());
    }
    let json_bytes = buf.len();
    if let Err(e) = std::fs::write(&out_path, &buf) {
        tlog("ingest", &format!("   └─ FALHOU ao gravar {}: {e}", out_path.display()));
        return (500, json!({"error": format!("gravar {}: {e}", out_path.display())}).to_string());
    }
    let saved_to = std::fs::canonicalize(&out_path).map(|p| p.display().to_string())
        .unwrap_or_else(|_| out_path.display().to_string());
    tlog("ingest", &format!("   ├─ salvo: {saved_to} ({json_bytes} bytes)"));

    let mut base = match RagBase::from_str(&String::from_utf8(buf).unwrap_or_default()) {
        Ok(b) => b,
        Err(e) => { tlog("ingest", &format!("   └─ FALHOU ao carregar como RagBase: {e}"));
                    return (400, json!({"error": e}).to_string()); }
    };
    base.mtime = rag::now_secs();   // ingestão recém-feita → "agora" pra boost de recência
    let n = base.n_chunks;
    let corpus = base.corpus.clone();
    insert_base(&mut state.bases, &collection, name.clone(), base);
    let total = total_bases(&state.bases);
    tlog("ingest", &format!("   └─ carregado: {collection}/{name} ({n} chunks) · total={total} bases ({:.0}ms)",
                            t0.elapsed().as_secs_f64() * 1000.0));
    (200, json!({"ok": true, "collection": collection, "name": name, "filename": filename,
                 "corpus": corpus, "n_chunks": n, "bytes": content_bytes.len(),
                 "appended": did_append,
                 "bases": total, "saved_to": saved_to,
                 "via": via}).to_string())
}

/// Decodificacao MINIMA de percent-encoding pra query string (RFC 3986).
fn percent_decode(s: &str) -> String {
    let mut out = Vec::with_capacity(s.len());
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let (Some(h), Some(l)) = (hex_val(bytes[i+1]), hex_val(bytes[i+2])) {
                out.push((h << 4) | l); i += 3; continue;
            }
        }
        if bytes[i] == b'+' { out.push(b' '); } else { out.push(bytes[i]); }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}
fn hex_val(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

/// Helper: tokeniza arquivo bruto -> JSON -> grava em ragfiles_dir/<collection>/<name>-tokenized.json
/// -> carrega como RagBase. saved_to recebe o path absoluto do JSON gravado.
fn ingest_raw_to_base(
    path: &str,
    drivers_dir: &str,
    ragfiles_dir: &str,
    collection: &str,
    name: &str,
    body: &Value,
    saved_to: &mut Option<String>,
) -> Result<RagBase, String> {
    let chunk_size = body["chunk"].as_u64().unwrap_or(2048) as usize;
    let max_chunks = body["max_chunks"].as_u64().unwrap_or(0) as usize;
    let with_text = body["with_text"].as_bool().unwrap_or(true);
    let driver_override = body["driver"].as_str();
    let value = ingestor::tokenize_file(
        Path::new(path),
        Path::new(drivers_dir),
        driver_override,
        chunk_size,
        max_chunks,
        with_text,
    )?;
    // persiste em ragfiles_dir/<collection>/<name>-tokenized.json
    let rag_dir = Path::new(ragfiles_dir).join(collection);
    std::fs::create_dir_all(&rag_dir)
        .map_err(|e| format!("não criou {}: {e}", rag_dir.display()))?;
    let out_path = rag_dir.join(format!("{name}-tokenized.json"));
    // indentado com TAB pra bater com embed_gen.py (legivel em editor)
    let mut buf = Vec::new();
    let fmt = serde_json::ser::PrettyFormatter::with_indent(b"\t");
    let mut ser = serde_json::Serializer::with_formatter(&mut buf, fmt);
    use serde::Serialize;
    value.serialize(&mut ser).map_err(|e| format!("serialize JSON: {e}"))?;
    std::fs::write(&out_path, &buf).map_err(|e| format!("gravar {}: {e}", out_path.display()))?;
    *saved_to = Some(std::fs::canonicalize(&out_path)
        .map(|p| p.display().to_string())
        .unwrap_or_else(|_| out_path.display().to_string()));
    RagBase::from_str(&String::from_utf8(buf).unwrap_or_default())
        .map(|mut b| { b.mtime = rag::now_secs(); b })
}

fn query_param(query: &str, key: &str) -> Option<String> {
    query.split('&').find_map(|kv| {
        let (k, v) = kv.split_once('=')?;
        if k == key { Some(percent_decode(v)) } else { None }
    })
}

/// GET /bases — lista bases. Filtros opcionais:
///   ?collection=X (default = todas) — '*' explícito também = todas
///   ?match=sd*    (wildcard sobre o nome dentro da coleção; default "*" = todas)
fn list_bases(query: &str, bases: &Bases) -> (u16, String) {
    let coll_q = query_param(query, "collection");
    let coll_pat = coll_q.as_deref();
    let name_pat = query_param(query, "match").unwrap_or_else(|| "*".to_string());
    let pairs = resolve_scope(bases, coll_pat, &name_pat);
    let list: Vec<Value> = pairs.iter().filter_map(|(c, n)| get_base(bases, c, n).map(|b| json!({
        "collection": c, "name": n,
        "n_chunks": b.n_chunks, "vocab_size": b.vocab_size,
        "corpus": b.corpus, "generator": b.generator, "has_text": b.has_text
    }))).collect();
    let mut resp = Map::new();
    resp.insert("collection".into(), match coll_pat { Some(c) => json!(c), None => json!("*") });
    resp.insert("match".into(), json!(name_pat));
    resp.insert("count".into(), json!(list.len()));
    resp.insert("bases".into(), Value::Array(list));
    (200, Value::Object(resp).to_string())
}

/// GET /collections — lista coleções existentes com contagem de bases.
fn list_collections(bases: &Bases) -> (u16, String) {
    let mut colls: Vec<(&String, usize)> = bases.iter().map(|(c, m)| (c, m.len())).collect();
    colls.sort_by(|a, b| a.0.cmp(b.0));
    let list: Vec<Value> = colls.iter().map(|(c, n)| json!({"collection": c, "bases": n})).collect();
    (200, json!({"count": list.len(), "total_bases": total_bases(bases),
                 "collections": list}).to_string())
}

/// GET /bases/{coll}/{name} — só a meta da base (sem chunks). [#4]
fn base_meta(coll: &str, name: &str, bases: &Bases) -> (u16, String) {
    let base = match bases.get(coll).and_then(|m| m.get(name)) {
        Some(b) => b,
        None => return (404, json!({"error": format!("base '{coll}/{name}' não encontrada")}).to_string()),
    };
    let vocab_used = base.idf.iter().filter(|(_, &v)| v > 0.0).count();
    (200, json!({
        "collection": coll, "name": name,
        "corpus": base.corpus, "generator": base.generator,
        "n_chunks": base.n_chunks, "vocab_size": base.vocab_size, "vocab_used": vocab_used,
        "has_text": base.has_text, "mtime": base.mtime,
    }).to_string())
}

/// GET /profile?collection=&base=&top=N — perfil léxico inspecionável. [#1]
/// Dois modos: com `base` → idf POR-BASE; sem `base` → idf UNIFICADO da coleção (constrói o
/// perfil on-the-fly — operação read-only, sem cachear no State).
fn profile(query: &str, bases: &Bases) -> (u16, String) {
    let coll = match query_param(query, "collection") {
        Some(c) => c, None => return (400, json!({"error": "falta 'collection'"}).to_string()),
    };
    let top_n: usize = query_param(query, "top").and_then(|s| s.parse().ok()).unwrap_or(20);

    // ── modo BASE ─────────────────────────────────────────────────────────────
    if let Some(name) = query_param(query, "base") {
        let base = match bases.get(&coll).and_then(|m| m.get(&name)) {
            Some(b) => b,
            None => return (404, json!({"error": format!("base '{coll}/{name}' não encontrada")}).to_string()),
        };
        let mut dim2syl: HashMap<usize, &str> = HashMap::with_capacity(base.index.len());
        for (s, &d) in &base.index { dim2syl.insert(d, s.as_str()); }
        let vocab_used = base.idf.iter().filter(|(_, &v)| v > 0.0).count();
        let mut idfs: Vec<(usize, f64)> = base.idf.iter().map(|(&d, &v)| (d, v)).collect();
        idfs.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());
        let top_idf: Vec<Value> = idfs.into_iter().take(top_n).map(|(d, v)| {
            json!({"dim": d, "syllable": dim2syl.get(&d).copied().unwrap_or("?"), "idf": v})
        }).collect();
        return (200, json!({
            "scope": "base",
            "collection": coll, "base": name,
            "corpus": base.corpus, "n_chunks": base.n_chunks,
            "vocab_size": base.vocab_size, "vocab_used": vocab_used,
            "has_text": base.has_text, "mtime": base.mtime,
            "top_idf": top_idf,
        }).to_string());
    }

    // ── modo COLEÇÃO (unificado, refs #5/#8) ──────────────────────────────────
    let inner = match bases.get(&coll) {
        Some(m) if !m.is_empty() => m,
        _ => return (404, json!({"error": format!("coleção '{coll}' vazia ou não encontrada")}).to_string()),
    };
    let prof = rag::build_collection_profile(inner);
    let mut udim2syl: HashMap<usize, &str> = HashMap::with_capacity(prof.uvocab.len());
    for (s, &d) in &prof.uvocab { udim2syl.insert(d, s.as_str()); }
    let mut uidfs: Vec<(usize, f64)> = prof.uidf.iter().map(|(&d, &v)| (d, v)).collect();
    uidfs.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());
    let top_uidf: Vec<Value> = uidfs.into_iter().take(top_n).map(|(d, v)| {
        json!({"dim": d, "syllable": udim2syl.get(&d).copied().unwrap_or("?"), "uidf": v})
    }).collect();
    let total_chunks: usize = inner.values().map(|b| b.n_chunks).sum();
    (200, json!({
        "scope": "collection",
        "collection": coll,
        "bases": inner.len(), "chunks": total_chunks,
        "unified_vocab_size": prof.uvocab.len(),
        "top_uidf": top_uidf,
    }).to_string())
}

/// DELETE /collections/{name}?purge=true — apaga uma coleção inteira da memória.
/// Com `purge=true` também remove `ragfiles/<name>/` do disco. [#2]
fn drop_collection(name: &str, query: &str, st: &mut State) -> (u16, String) {
    let purge = query_param(query, "purge").map(|s| s == "true" || s == "1").unwrap_or(false);
    let removed = match st.bases.remove(name) {
        Some(m) => m.len(),
        None => return (404, json!({"error": format!("coleção '{name}' não encontrada")}).to_string()),
    };
    st.collection_profiles.write().remove(name);   // invalida o perfil cacheado
    let mut purged = false;
    if purge {
        let dir = Path::new(&st.ragfiles_dir).join(name);
        if dir.exists() {
            if let Err(e) = std::fs::remove_dir_all(&dir) {
                return (500, json!({"error": format!("removida da memória mas falhou apagar {}: {e}", dir.display()),
                                    "collection": name, "bases_removed": removed}).to_string());
            }
            purged = true;
        }
    }
    (200, json!({"ok": true, "collection": name, "bases_removed": removed,
                 "purged": purged, "bases": total_bases(&st.bases),
                 "collections": st.bases.len()}).to_string())
}

fn search(body: &str, bases: &Bases, profiles: &RwLock<HashMap<String, rag::CollectionProfile>>) -> (u16, String) {
    let v: Value = match serde_json::from_str(body) {
        Ok(v) => v,
        Err(e) => return (400, json!({"error": format!("body JSON inválido: {e}")}).to_string()),
    };
    let pattern = match v["base"].as_str() {
        Some(n) => n, None => return (400, json!({"error": "falta 'base' (nome, 'pref*' ou '*')"}).to_string()),
    };
    let query = match v["query"].as_str() {
        Some(q) => q, None => return (400, json!({"error": "falta 'query'"}).to_string()),
    };
    let k = v["k"].as_u64().unwrap_or(5) as usize;
    let recall_n = v["recall_n"].as_u64().unwrap_or(20) as usize;
    let rerank = v["rerank"].as_bool().unwrap_or(true);
    let phonetic = v["phonetic"].as_bool().unwrap_or(false);
    // escopo: collection ausente ou "*" => todas
    let coll_pat = v["collection"].as_str();

    let pairs = resolve_scope(bases, coll_pat, pattern);
    if pairs.is_empty() {
        return (404, json!({"error": format!("nenhuma base casa com '{}/{pattern}'",
            coll_pat.unwrap_or("*"))}).to_string());
    }

    // [#8] modo unificado por coleção (opt-in via body.unified): vocab+idf de repo, em cache,
    // auto-invalidado por fingerprint (nº bases, total chunks). Sem ele, recall local de sempre.
    let unified = v["unified"].as_bool().unwrap_or(false);
    // [#5] PESO unificado no rerank: coleção com >1 base NO ESCOPO ganha perfil (idf de coleção,
    // cacheado por fingerprint) pra que o peso por termo use a escala da COLEÇÃO — termo ausente
    // numa base não some do denominador (corrige o "1.0 falso" per-arquivo) e a escala fica
    // consistente entre bases. Busca de 1 base usa peso local (fallback no finish). O recall
    // unificado (opt-in) reusa o mesmo perfil.
    let mut scope_count: HashMap<&str, usize> = HashMap::new();
    for (c, _) in &pairs { *scope_count.entry(c.as_str()).or_insert(0) += 1; }
    let build_colls: Vec<String> = scope_count.iter()
        .filter(|(_, n)| **n > 1).map(|(c, _)| c.to_string()).collect();
    // [#6] check fingerprint sob READ primeiro (N searches paralelas não esperam aqui); só pega
    // WRITE rapidinho se precisa rebuild — minimiza tempo de exclusão.
    let stale: Vec<String> = {
        let p = profiles.read();
        build_colls.iter().filter(|c| {
            bases.get(*c).map(|inner| {
                let fp = rag::collection_fingerprint(inner);
                p.get(*c).map(|x| x.fingerprint) != Some(fp)
            }).unwrap_or(false)
        }).cloned().collect()
    };
    if !stale.is_empty() {
        let mut p = profiles.write();
        for c in &stale {
            if let Some(inner) = bases.get(c) {
                let fp = rag::collection_fingerprint(inner);
                if p.get(c).map(|x| x.fingerprint) != Some(fp) {
                    p.insert(c.clone(), rag::build_collection_profile(inner));
                }
            }
        }
    }
    let qt = rag::prep_query(query);
    // segura UM read-lock durante todo o scatter-gather: outras searches lêem em paralelo
    let profiles_guard = profiles.read();
    let weightings: HashMap<String, Vec<f64>> = build_colls.iter()
        .filter_map(|c| profiles_guard.get(c).map(|p| (c.clone(), rag::weighting_unified(&qt, p))))
        .collect();
    let qvecs: HashMap<String, (HashMap<usize, f64>, f64)> = if unified {
        build_colls.iter()
            .filter_map(|c| profiles_guard.get(c).map(|p| (c.clone(), rag::query_vec_unified(query, p))))
            .collect()
    } else { HashMap::new() };
    let profiles_ref: &HashMap<String, rag::CollectionProfile> = &*profiles_guard;

    // scatter-gather: busca em cada base. Paraleliza com rayon quando há mais de uma
    // base no escopo (caso GLOBAL/coleção); cada base é independente, merge no fim.
    type BaseResult = (Value, String, Vec<(f64, u64, i64, f64, Map<String, Value>)>);
    let search_one = |coll: &String, name: &String| -> Option<BaseResult> {
        let base = get_base(bases, coll, name)?;
        let w = weightings.get(coll).map(|v| v.as_slice());   // peso unificado da coleção (ou None)
        let (hits, info) = if unified {
            // perfil + query vetorizada da coleção + remap/normas desta base → recall unificado;
            // fallback pro recall local se faltar qualquer peça (robustez)
            match (profiles_ref.get(coll), qvecs.get(coll)) {
                (Some(p), Some((qv, qn))) => match (p.remap.get(name), p.unorms.get(name)) {
                    (Some(remap), Some(unorms)) =>
                        base.search_unified(query, k, rerank, recall_n, phonetic, qv, *qn, remap, unorms, w),
                    _ => base.search(query, k, rerank, recall_n, phonetic, w),
                },
                _ => base.search(query, k, rerank, recall_n, phonetic, w),
            }
        } else {
            base.search(query, k, rerank, recall_n, phonetic, w)
        };
        let entry = json!({"collection": coll, "base": name,
                           "n_chunks": info.n_chunks, "n_converge": info.n_converge,
                           "dims": info.dims, "oov": info.oov,
                           "ms_recall": info.ms_recall, "ms_rerank": info.ms_rerank});
        let mut local: Vec<(f64, u64, i64, f64, Map<String, Value>)> = vec![];
        for (rr, cov, span, cos, cid) in hits {
            let c = &base.chunks[cid];
            let coverage = rr.unwrap_or(cos);
            let sp = span.unwrap_or(0);
            let mut o = Map::new();
            o.insert("collection".into(), json!(coll));
            o.insert("base".into(), json!(name));
            o.insert("corpus".into(), json!(base.corpus.clone()));   // nome do arquivo (com extensão)
            // `path`: caminho relativo reconstruído pra IA ir DIRETO no arquivo (uso central
            // do RAG de código). Diretório vem do `name` codificado (__→/, sem o último
            // segmento) + nome real (corpus, com extensão). Sem "__" (base de 1 segmento) = só corpus.
            let path = match name.rsplit_once("__") {
                Some((dir, _)) => format!("{}/{}", dir.replace("__", "/"), base.corpus),
                None => base.corpus.clone(),
            };
            o.insert("path".into(), json!(path));
            o.insert("matchpoint".into(), json!(coverage));
            if cov.is_some() { o.insert("coverage".into(), json!(coverage)); }
            if span.is_some() { o.insert("span".into(), json!(sp)); }
            o.insert("cos".into(), json!(cos));
            o.insert("chunk".into(), json!(c.id));
            o.insert("start".into(), json!(c.start));
            if let Some(t) = &c.text { o.insert("snippet".into(), json!(rag::snippet(t, query))); }
            // tuple: (coverage_honesta, mtime_base, neg_span, cos, hit). mtime entra pro boost
            // de recência no merge cross-base (sessão nova não perde pra antiga em quase-empate).
            local.push((coverage, base.mtime, -(sp as i64), cos, o));
        }
        Some((entry, info.syls.join("-"), local))
    };
    let per_base: Vec<BaseResult> = if pairs.len() > 1 {
        pairs.par_iter().filter_map(|(c, n)| search_one(c, n)).collect()
    } else {
        pairs.iter().filter_map(|(c, n)| search_one(c, n)).collect()
    };
    // merge: concatena hits e monta o relatório por base (ordem = ordem do escopo)
    let mut merged: Vec<(f64, u64, i64, f64, Map<String, Value>)> = vec![];
    let mut searched: Vec<Value> = vec![];
    let mut syllables = String::new();
    for (entry, syls, local) in per_base {
        if syllables.is_empty() && !syls.is_empty() { syllables = syls; }
        searched.push(entry);
        merged.extend(local);
    }
    // RECÊNCIA COMO DESEMPATE (não multiplicador). A relevância (coverage honesta) é a
    // chave PRIMÁRIA; a recência só decide quando coverage/span/cos empatam — aí o mais
    // novo ganha. Motivo: o multiplicador antigo (coverage × fator) fazia uma base recente
    // de coverage MENOR superar conteúdo mais relevante de outra coleção (a memória de
    // sessão, regravada a cada turno, vencia livros/código). Como tie-breaker puro, a
    // recência resolve o caso pra que foi criada (turnos de mesma coverage: novo > velho)
    // sem distorcer o ranking entre coleções. mtime=0 (desconhecido) ordena por último.
    let now = rag::now_secs();
    const REC_HALF_LIFE: f64 = 7.0 * 86400.0;
    let recency = |mtime: u64| -> f64 {   // só p/ exibir o fator no hit (transparência)
        if mtime == 0 { return 1.0; }
        let age = (now.saturating_sub(mtime)) as f64;
        1.0 + 0.10 * (-age / REC_HALF_LIFE).exp()
    };
    merged.sort_by(|a, b| {
        b.0.partial_cmp(&a.0).unwrap()                 // coverage PURA desc (relevância manda)
            .then(a.2.cmp(&b.2))                        // span asc (proximidade)
            .then(b.3.partial_cmp(&a.3).unwrap())       // cos desc
            .then(b.1.cmp(&a.1))                        // mtime desc (recência só desempata)
    });
    let hits: Vec<Value> = merged.into_iter().take(k).enumerate().map(|(i, (_cov, mt, _sp, _cos, mut o))| {
        o.insert("recency".into(), json!(format!("{:.3}", recency(mt))));
        o.insert("rank".into(), json!(i + 1));
        Value::Object(o)
    }).collect();

    let scope_label: Vec<Value> = pairs.iter().map(|(c, n)| json!(format!("{c}/{n}"))).collect();
    let resp = json!({
        "query": query, "query_syllables": syllables,
        "scope": scope_label, "searched": searched, "hits": hits
    });
    (200, resp.to_string())
}

/// Retorna o(s) chunk(s) inteiro(s) por id — pra montar contexto (vizinhos, etc).
fn fetch_chunk(body: &str, bases: &Bases) -> (u16, String) {
    let v: Value = match serde_json::from_str(body) {
        Ok(v) => v,
        Err(e) => return (400, json!({"error": format!("body JSON inválido: {e}")}).to_string()),
    };
    let name = match v["base"].as_str() {
        Some(n) => n, None => return (400, json!({"error": "falta 'base'"}).to_string()),
    };
    let collection = v["collection"].as_str().unwrap_or(DEFAULT_COLLECTION);
    let base = match get_base(bases, collection, name) {
        Some(b) => b,
        None => return (404, json!({"error": format!("base '{collection}/{name}' não carregada")}).to_string()),
    };
    let n = base.chunks.len();
    // ids: lista explícita OU janela id ± before/after
    let ids: Vec<usize> = if let Some(arr) = v["ids"].as_array() {
        arr.iter().filter_map(|x| x.as_u64().map(|n| n as usize)).collect()
    } else if let Some(id) = v["id"].as_u64() {
        let id = id as usize;
        let before = v["before"].as_u64().unwrap_or(0) as usize;
        let after = v["after"].as_u64().unwrap_or(0) as usize;
        let lo = id.saturating_sub(before);
        let hi = (id + after).min(n.saturating_sub(1));
        if lo <= hi { (lo..=hi).collect() } else { vec![] }
    } else {
        return (400, json!({"error": "forneça 'id' (com 'before'/'after' opcionais) ou 'ids'"}).to_string());
    };
    let chunks: Vec<Value> = ids.iter().filter_map(|&i| base.chunks.get(i).map(|c| json!({
        "id": c.id, "start": c.start, "len": c.len, "tokens": c.tokens, "oov": c.oov,
        "norm": c.norm, "text": c.text
    }))).collect();
    (200, json!({"collection": collection, "base": name, "corpus": base.corpus,
                 "n_chunks": base.n_chunks, "chunks": chunks}).to_string())
}

/// Conta quantos .drv ha em drivers_dir (silencioso em erro).
fn count_drivers(dir: &str) -> usize {
    scan_drivers(dir).map(|v| v.len()).unwrap_or(0)
}

/// Lista os arquivos .drv da pasta (ordenado por nome). Retorna paths absolutos.
fn scan_drivers(dir: &str) -> std::io::Result<Vec<std::path::PathBuf>> {
    let mut out: Vec<std::path::PathBuf> = std::fs::read_dir(dir)?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().and_then(|x| x.to_str()) == Some("drv"))
        .collect();
    out.sort();
    Ok(out)
}

/// Resolve nome do driver: tokens_<Lang>_PTBR.drv -> Lang (strip prefixo/sufixo).
fn driver_language(fname: &str) -> String {
    let stem = fname.strip_suffix(".drv").unwrap_or(fname);
    let stem = stem.strip_prefix("tokens_").unwrap_or(stem);
    stem.strip_suffix("_PTBR").unwrap_or(stem).to_string()
}

/// GET /drivers  — lista os drivers .drv instalados. ?match=ASP* (wildcard, default todos).
/// Cada item traz header / description / extensions extraidos do cabecalho do .drv.
fn list_drivers(query: &str, drivers_dir: &str) -> (u16, String) {
    let pattern = query_param(query, "match").unwrap_or_else(|| "*".to_string());
    let abs_dir = std::fs::canonicalize(drivers_dir)
        .map(|p| p.display().to_string())
        .unwrap_or_else(|_| drivers_dir.to_string());

    let files = match scan_drivers(drivers_dir) {
        Ok(v) => v,
        Err(e) => return (500, json!({"error": format!("scan {drivers_dir}: {e}")}).to_string()),
    };

    let prefix = pattern.strip_suffix('*');
    let mut drivers: Vec<Value> = vec![];
    for path in &files {
        let fname = path.file_name().and_then(|x| x.to_str()).unwrap_or("").to_string();
        let lang = driver_language(&fname);
        let matches = if pattern == "*" { true }
                      else if let Some(p) = prefix { lang.starts_with(p) }
                      else { lang == pattern };
        if !matches { continue; }
        let p = path.to_string_lossy().to_string();
        let (vocab, keywords) = match vocab::load_driver(&p) {
            Ok(t) => t,
            Err(e) => return (500, json!({"error": format!("load {fname}: {e}")}).to_string()),
        };
        let meta = vocab::read_meta(&p).unwrap_or_default();
        let kw = keywords.len();
        let syl = vocab.len().saturating_sub(kw);
        drivers.push(json!({
            "name": fname,
            "language": lang,
            "description": meta.description,
            "extensions": meta.extensions,
            "syllables": syl,
            "keywords": kw,
            "vocab_size": vocab.len(),
            "header": meta.header,
        }));
    }
    (200, json!({"drivers_dir": abs_dir, "match": pattern,
                 "count": drivers.len(), "drivers": drivers}).to_string())
}

/// GET /interpret?file=foo.py  (ou ?ext=.py)
/// Mapeia arquivo->driver pela extensao usando o resolver do ingestor (mesma
/// logica usada pelo POST /ingest_file). Sem match: fallback PTBR.
fn interpret(query: &str, drivers_dir: &str) -> (u16, String) {
    let file_q = query_param(query, "file");
    let ext_q = query_param(query, "ext").map(|e| if e.starts_with('.') { e } else { format!(".{e}") });
    if file_q.is_none() && ext_q.is_none() {
        return (400, json!({"error": "forneça ?file=foo.py ou ?ext=.py"}).to_string());
    }
    let idx = match ingestor::build_driver_index(Path::new(drivers_dir)) {
        Ok(i) => i,
        Err(e) => return (500, json!({"error": format!("scan {drivers_dir}: {e}")}).to_string()),
    };

    // resolve extensao a partir de ?file= OU ?ext=
    let ext: Option<String> = match (&file_q, &ext_q) {
        (Some(f), _) => ingestor::file_extension(Path::new(f)),
        (_, Some(e)) => Some(e.to_lowercase()),
        _ => None,
    };

    let mut resp = Map::new();
    if let Some(f) = &file_q { resp.insert("file".into(), json!(f)); }
    resp.insert("extension".into(), match &ext {
        Some(e) => json!(e), None => Value::Null,
    });
    resp.insert("drivers_dir".into(), json!(drivers_dir));
    resp.insert("drivers_scanned".into(), json!(idx.by_ext.len()));

    match ext.as_deref().and_then(|e| idx.by_ext.get(e)) {
        Some((driver, lang)) => {
            resp.insert("matched".into(), json!(true));
            resp.insert("driver".into(), json!(driver));
            resp.insert("language".into(), json!(lang));
        }
        None => {
            resp.insert("matched".into(), json!(false));
            resp.insert("fallback".into(), json!(ingestor::FALLBACK_LANG));
            resp.insert("driver".into(), json!(ingestor::FALLBACK_DRIVER));
            resp.insert("language".into(), json!(ingestor::FALLBACK_LANG));
        }
    }
    (200, Value::Object(resp).to_string())
}

// ============================ Dicionários (thesaurus por-PALAVRA) ============================
// Cada subdir de `thesaurus/` é um dicionário (CODE = nome do dir) com `synonyms.jsonl`:
//   linha 0 = {"meta": {...}} · demais = {"w": palavra, "s": [sinônimos]}.
// Ligar/desligar NÃO move o arquivo (são grandes, 4–14 MB): cria/remove `inuse.flag`
// dentro do dir. Só os dicionários ATIVOS entram no mapa por-palavra (st.word_syn).

/// Lista os subdirs que são dicionários válidos (têm synonyms.jsonl), ordenados.
fn dict_dirs(dir: &str) -> Vec<std::path::PathBuf> {
    let mut out: Vec<std::path::PathBuf> = match std::fs::read_dir(dir) {
        Ok(rd) => rd.filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.is_dir() && p.join("synonyms.jsonl").is_file())
            .collect(),
        Err(_) => vec![],
    };
    out.sort();
    out
}

/// Lê só a 1a linha do synonyms.jsonl (o objeto `meta`).
fn read_dict_meta(p: &Path) -> Value {
    use std::io::{BufRead, BufReader};
    let f = match std::fs::File::open(p.join("synonyms.jsonl")) { Ok(f) => f, Err(_) => return Value::Null };
    let mut first = String::new();
    if BufReader::new(f).read_line(&mut first).is_ok() {
        if let Ok(v) = serde_json::from_str::<Value>(first.trim()) {
            return v.get("meta").cloned().unwrap_or(Value::Null);
        }
    }
    Value::Null
}

/// GET /thesaurus — lista os dicionários com meta (origem/licença/entradas) e estado (ativo).
fn list_dicts(_query: &str, dir: &str) -> (u16, String) {
    let abs = std::fs::canonicalize(dir).map(|p| p.display().to_string()).unwrap_or_else(|_| dir.to_string());
    let mut dicts: Vec<Value> = vec![];
    let mut n_active = 0usize;
    for p in dict_dirs(dir) {
        let code = p.file_name().and_then(|x| x.to_str()).unwrap_or("").to_string();
        let active = p.join("inuse.flag").exists();
        if active { n_active += 1; }
        let meta = read_dict_meta(&p);
        let size = std::fs::metadata(p.join("synonyms.jsonl")).map(|m| m.len()).unwrap_or(0);
        let g = |k: &str| meta.get(k).cloned().unwrap_or(Value::Null);
        dicts.push(json!({
            "code": code, "active": active,
            "entries": g("entries"), "source": g("source"), "source_url": g("source_url"),
            "license": g("license"), "kind": g("kind"),
            "lang_query": g("lang_query"), "lang_target": g("lang_target"),
            "size_bytes": size,
        }));
    }
    (200, json!({"thesaurus_dir": abs, "count": dicts.len(), "active": n_active, "dicts": dicts}).to_string())
}

/// Monta o mapa palavra->sinônimos da UNIÃO dos dicionários ATIVOS (com inuse.flag).
fn load_active_dicts(dir: &str) -> HashMap<String, Vec<String>> {
    let mut map: HashMap<String, Vec<String>> = HashMap::new();
    for p in dict_dirs(dir) {
        if !p.join("inuse.flag").exists() { continue; }
        let content = match std::fs::read_to_string(p.join("synonyms.jsonl")) { Ok(c) => c, Err(_) => continue };
        for (i, line) in content.lines().enumerate() {
            if i == 0 { continue; }                 // pula o meta
            let line = line.trim();
            if line.is_empty() { continue; }
            if let Ok(v) = serde_json::from_str::<Value>(line) {
                let w = v["w"].as_str().unwrap_or("").to_lowercase();
                if w.is_empty() { continue; }
                let syns: Vec<String> = v["s"].as_array()
                    .map(|a| a.iter().filter_map(|x| x.as_str().map(|s| s.to_string())).collect())
                    .unwrap_or_default();
                if syns.is_empty() { continue; }
                map.entry(w).or_default().extend(syns);
            }
        }
    }
    map
}

/// POST /api/thesaurus_toggle {"code":"PTBR","action":"enable"|"disable"} — cria/remove
/// o inuse.flag e RECARREGA o mapa por-palavra. Reflete na hora, sem reiniciar.
fn dict_toggle(body: &str, st: &mut State) -> (u16, String) {
    let v: Value = match serde_json::from_str(body) {
        Ok(v) => v, Err(e) => return (400, json!({"error": format!("JSON inválido: {e}")}).to_string()),
    };
    let code = match v["code"].as_str() { Some(c) => c, None => return (400, json!({"error": "falta 'code'"}).to_string()) };
    let action = v["action"].as_str().unwrap_or("");
    if code.is_empty() || code.contains('/') || code.contains('\\') || code.contains("..") {
        return (400, json!({"error": "code inválido"}).to_string());
    }
    let dirp = Path::new(&st.thesaurus_dir).join(code);
    if !dirp.join("synonyms.jsonl").is_file() {
        return (404, json!({"error": format!("dicionário '{code}' não existe em {:?}", st.thesaurus_dir)}).to_string());
    }
    let flag = dirp.join("inuse.flag");
    match action {
        "enable"  => if let Err(e) = std::fs::write(&flag, b"1\n") {
            return (500, json!({"error": format!("criar flag: {e}")}).to_string());
        },
        "disable" => if flag.exists() {
            if let Err(e) = std::fs::remove_file(&flag) {
                return (500, json!({"error": format!("remover flag: {e}")}).to_string());
            }
        },
        _ => return (400, json!({"error": "action deve ser 'enable' ou 'disable'"}).to_string()),
    }
    st.word_syn = load_active_dicts(&st.thesaurus_dir);
    let n_act = dict_dirs(&st.thesaurus_dir).iter().filter(|p| p.join("inuse.flag").exists()).count();
    (200, json!({"ok": true, "code": code, "action": action,
                 "active": n_act, "word_entries": st.word_syn.len()}).to_string())
}

/// Expansão POR-PALAVRA: cada palavra da query vira suas variantes (sinônimos dos dicts ativos).
/// O casamento real (e o corte de polissemia) acontece no search por sílaba + merge por cobertura.
fn expand_with_dicts(query: &str, map: &HashMap<String, Vec<String>>) -> Vec<String> {
    let mut out: Vec<String> = vec![];
    let mut seen: HashSet<String> = HashSet::new();
    let lower = query.to_lowercase();
    let all: Vec<&str> = lower.split_whitespace().collect();
    // MESMO critério do prep_query (rerank): só palavra de CONTEÚDO (>=2 sílabas) vira chave de
    // expansão. Palavra-função/stopword ("do","de","da","a","o") NÃO expande — senão colide com
    // palavra EN nos dicts ingleses ativos (do→inglês "do": *act/accomplish* + festa *mardi
    // gras/saturnalia*; da→"D.A." District Attorney) e polui o recall. Fallback igual ao
    // prep_query: se TODAS forem monossílabas, usa todas.
    let is_content = |w: &str| tokenizer::syllabify(w).iter()
        .filter(|s| !tokenizer::normalize(s).is_empty()).count() >= 2;
    let content: Vec<&str> = all.iter().copied().filter(|w| is_content(w)).collect();
    let keys: &[&str] = if content.is_empty() { &all } else { &content };
    for &w in keys {
        if let Some(syns) = map.get(w) {
            for s in syns {
                let s = s.trim();
                if s.is_empty() { continue; }
                let low = s.to_lowercase();
                if low == w { continue; }
                if seen.insert(low) { out.push(s.to_string()); }
            }
        }
    }
    out.truncate(12);
    out
}

fn help() {
    println!(
"ragd — RAGnaRock daemon (segura N bases RAG em memoria, HTTP JSON).

uso:
  ragd [--config <arq>] [--port {DEFAULT_PORT}] [--dash-port {DEFAULT_DASH_PORT}]
       [--drivers-dir {DEFAULT_DRIVERS_DIR}] [--ragfiles-dir {DEFAULT_RAGFILES_DIR}]
       [--max-upload {DEFAULT_MAX_UPLOAD}] [--no-autoload] [--dev]
       [--preload nome=caminho.json ...]

  config: --config <arq>, senao /etc/ragnarock/ragnarock.cfg, senao ./ragnarock.cfg, senao defaults.
          (chaves: api_port, dash_port, drivers_dir, ragfiles_dir, max_upload, autoload, admin_user, admin_pass)
  duas portas: API (default {DEFAULT_PORT}) + dashboard/supervisorio (default {DEFAULT_DASH_PORT}, login por sessao).
  seguranca: credenciais admin/admin sao recusadas a menos que --dev seja passado. Troque no .cfg ou pelo painel.
  por padrao carrega TODAS as bases de ragfiles-dir no boot (cada subdir = colecao).
  --no-autoload sobe vazio (p/ ingerir do zero); --preload adiciona bases por cima.

rotas:
  GET    /health
  GET    /bases                     ?collection=X (default todas) e/ou ?match=sd* (wildcard no nome)
  GET    /collections               lista coleções com contagem de bases
  GET    /drivers                   (todos)  ou  /drivers?match=ASP* (filtra por wildcard)
  GET    /thesaurus                  lista dicionarios por-palavra (origem/licenca/entradas/ativo)
  GET    /interpret?file=foo.py     mapeia arquivo->driver pela extensao (fallback PTBR)
  POST   /ingest         {{\"name\":\"sda\",\"path\":\"ragfiles/sda-tokenized.json\"}}                 (JSON tokenizado)
                         {{\"name\":\"sda\",\"data\":<base>}}                                          (base embedded)
                         {{\"name\":\"py01\",\"path\":\"logic_path/01_foo.py\",\"raw\":true,\"chunk\":2048}}  (arquivo bruto)
  POST   /ingest_file    {{\"path\":\"logic_path/01_foo.py\",\"name\":?,\"chunk\":?,\"driver\":?}}        (atalho bruto)
  POST   /ingest_upload  curl -F file=@local.py -F name=hist $H/ingest_upload                    (upload via multipart)
                          curl --data-binary @local.py \"$H/ingest_upload?filename=local.py&name=hist\"   (raw body)
  POST   /search    {{\"base\":\"sda\"|\"sd*\"|\"*\",\"query\":\"anel\",\"k\":5,\"rerank\":true}}
  POST   /chunk     {{\"base\":\"sda\",\"id\":87,\"before\":1,\"after\":1}}  ou  {{\"base\":\"sda\",\"ids\":[1,87]}}
  DELETE /bases/{{nome}}");
}
