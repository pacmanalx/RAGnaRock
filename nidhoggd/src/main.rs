//! nidhoggd — Níðhöggr, a camada de INTELIGÊNCIA do RAGnaRock.
//! Worm autônomo (do bem) que "come" o conhecimento das coleções e o destila num
//! conhecimento que SOBREVIVE à deleção da coleção. Roda como processo SEPARADO:
//!  - acessa o corpus SEMPRE pela API do ragd (nunca disco) → independe de localização;
//!  - nasce DESLIGADO (precisa de IA e consome IA);
//!  - dois dials ortogonais: NÍVEL (profundidade) e CADÊNCIA (com que frequência mastiga);
//!  - liga/desliga por COLEÇÃO (não fica re-mastigando a mesma N vezes);
//!  - daemon de MÓDULOS na porta 11497 (vai hospedar N coisas além do Nidhogg).
//! Esqueleto: estrutura + API + keepalive prontos; a inteligência por nível é stub a preencher.

use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use serde_json::{json, Value};
use tiny_http::{Header, Method, Response, Server};

const VERSION: &str = env!("CARGO_PKG_VERSION");
const DEFAULT_PORT: u16 = 11497;
const DEFAULT_RAGD_API: &str = "http://127.0.0.1:11499";
const DEFAULT_DIR: &str = "nidhogg";
const DEFAULT_CADENCE: u64 = 300;   // s entre ciclos do worm (cadência = orçamento de tempo)

// ───────────────────────────── níveis de inteligência (slider) ─────────────────────────────
// Cumulativos. 0 não usa IA; 1+ precisam de provider de IA configurado.
fn level_name(l: u8) -> &'static str {
    match l { 0 => "minerador", 1 => "consciente", 2 => "estrutural", 3 => "propositivo", _ => "minerador" }
}
fn level_num(s: &str) -> u8 {
    match s.trim().to_lowercase().as_str() {
        "consciente" | "1" => 1, "estrutural" | "2" => 2, "propositivo" | "3" => 3,
        // "burro" aceito como sinônimo retrocompatível de "minerador" (nome antigo do nível 0).
        "minerador" | "burro" | "0" | _ => 0,
    }
}
fn levels_json() -> Value {
    json!([
        {"n":0,"name":"minerador","ia":false,"desc":"Só os 3 pilares: índice de raízes, dicionário do corpus, digestão do cache. Zero IA — cava o material bruto."},
        {"n":1,"name":"consciente","ia":true,"desc":"Insights e resumo por coleção — conhecimento que sobrevive à deleção da coleção."},
        {"n":2,"name":"estrutural","ia":true,"desc":"Hierarquia e encaixe de dimensões entre projetos/ingestões — sabe o que encaixa em quê."},
        {"n":3,"name":"propositivo","ia":true,"desc":"Acha furos, sugere, comenta, resume inteligente — código e livros. Aprimora a base de conhecimento."}
    ])
}

// ───────────────────────────── config (nidhogg.cfg) ─────────────────────────────
struct Config {
    port: u16,
    ragd_api: String,
    on: bool,            // OFF por default
    level: u8,           // 0 minerador
    dir: String,         // raiz do conhecimento persistente
    cadence: u64,        // segundos entre ciclos
    cfg_path: String,
}
impl Default for Config {
    fn default() -> Self {
        Config { port: DEFAULT_PORT, ragd_api: DEFAULT_RAGD_API.to_string(), on: false, level: 0,
                 dir: DEFAULT_DIR.to_string(), cadence: DEFAULT_CADENCE, cfg_path: "nidhogg.cfg".to_string() }
    }
}
fn load_cfg(cfg: &mut Config, path: &str) {
    let txt = match std::fs::read_to_string(path) { Ok(t) => t, Err(_) => { eprintln!("config: sem {path:?}, usando defaults"); return; } };
    cfg.cfg_path = path.to_string();
    for raw in txt.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') { continue; }
        let (k, vraw) = match line.split_once('=') { Some(kv) => kv, None => continue };
        let (k, v) = (k.trim(), vraw.split(" #").next().unwrap_or("").trim());
        match k {
            "port"     => if let Ok(p) = v.parse() { cfg.port = p },
            "ragd_api" => cfg.ragd_api = v.to_string(),
            "nidhogg" | "on" => cfg.on = matches!(v, "true" | "1" | "yes" | "on"),
            "level"    => cfg.level = level_num(v),
            "dir"      => cfg.dir = v.to_string(),
            "cadence"  => if let Ok(n) = v.parse() { cfg.cadence = n },
            other => eprintln!("config: chave desconhecida {other:?}"),
        }
    }
    println!("config: carregada de {path:?}");
}
/// Atualiza (ou anexa) `chave = valor` no cfg, preservando o resto.
fn set_cfg_key(path: &str, key: &str, val: &str) {
    let mut lines: Vec<String> = std::fs::read_to_string(path).map(|s| s.lines().map(String::from).collect()).unwrap_or_default();
    let newline = format!("{key} = {val}");
    let mut found = false;
    for l in lines.iter_mut() {
        let t = l.trim_start();
        if t.starts_with(&format!("{key} ")) || t.starts_with(&format!("{key}=")) { *l = newline.clone(); found = true; break; }
    }
    if !found { lines.push(newline); }
    let _ = std::fs::write(path, lines.join("\n") + "\n");
}

// ───────────────────────────── estado compartilhado ─────────────────────────────
struct State {
    on: bool,
    level: u8,
    dir: String,
    cadence: u64,
    ragd_api: String,
    cfg_path: String,
    started: Instant,
    last_cycle: String,
    ragd_online: bool,     // cache do keepalive (atualizado por thread leve) — status NUNCA faz curl ao vivo
    ragd_health: Value,    // último /health do ragd
}

// ───────────────────────────── timestamp (civil, sem dependência) ─────────────────────────────
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = (if mp < 10 { mp + 3 } else { mp - 9 }) as u32;
    (y + if m <= 2 { 1 } else { 0 }, m, d)
}
fn now_stamp() -> String {
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0) - 3 * 3600;
    let (days, tod) = (secs.div_euclid(86400), secs.rem_euclid(86400));
    let (y, mo, d) = civil_from_days(days);
    format!("{y:04}-{mo:02}-{d:02} {:02}:{:02}:{:02}", tod / 3600, (tod % 3600) / 60, tod % 60)
}
fn nlog(line: &str) { println!("[{}] [nidhogg] {line}", now_stamp()); }

// ───────────────────────────── HTTP client (via wget, no espírito do ragd) ─────────────────────────────
fn http_get(url: &str) -> Option<String> {
    // portátil: tenta curl (mac/linux), cai pra wget. Timeout curto (3s).
    for tool in ["curl", "wget"] {
        let mut cmd = std::process::Command::new(tool);
        if tool == "curl" { cmd.args(["-s", "-m", "3", url]); }
        else { cmd.args(["-q", "-O", "-", "--tries=1", "--timeout=3", url]); }
        if let Ok(out) = cmd.output() {
            if out.status.success() && !out.stdout.is_empty() { return Some(String::from_utf8_lossy(&out.stdout).to_string()); }
        }
    }
    None
}
/// Busca o /health do ragd (usado SÓ pela thread de keepalive, nunca no caminho do request).
fn fetch_ragd_health(api: &str) -> Option<Value> {
    http_get(&format!("{api}/health")).and_then(|s| serde_json::from_str(&s).ok())
}
/// Thread leve de keepalive: pinga o ragd periodicamente e cacheia no State.
fn keepalive(state: Arc<Mutex<State>>) {
    loop {
        let api = { state.lock().unwrap().ragd_api.clone() };
        let health = fetch_ragd_health(&api);
        if let Ok(mut s) = state.lock() {
            s.ragd_online = health.is_some();
            s.ragd_health = health.unwrap_or(Value::Null);
        }
        std::thread::sleep(Duration::from_secs(15));
    }
}

// ───────────────────────────── conhecimento persistente (sobrevive à coleção) ─────────────────────────────
// Um arquivo por coleção: estado de DIGESTÃO + conhecimento destilado + PROVENIÊNCIA.
fn safe(name: &str) -> String {
    name.chars().map(|c| if c.is_alphanumeric() || c == '-' || c == '_' { c } else { '_' }).collect()
}
fn knowledge_path(dir: &str, collection: &str) -> std::path::PathBuf {
    Path::new(dir).join(format!("{}.knowledge.json", safe(collection)))
}
fn read_knowledge(dir: &str, collection: &str) -> Value {
    std::fs::read_to_string(knowledge_path(dir, collection)).ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_else(|| json!({
            "collection": collection, "enabled": false, "source_hash": "",
            "saturation": 0.0, "updated": "", "provenance": Value::Null, "knowledge": []
        }))
}
fn write_knowledge(dir: &str, collection: &str, v: &Value) {
    let _ = std::fs::create_dir_all(dir);
    if let Ok(s) = serde_json::to_string_pretty(v) { let _ = std::fs::write(knowledge_path(dir, collection), s); }
}
/// Conta coleções já com algum conhecimento gravado.
fn known_count(dir: &str) -> usize {
    std::fs::read_dir(dir).map(|rd| rd.flatten()
        .filter(|e| e.path().to_string_lossy().ends_with(".knowledge.json")).count()).unwrap_or(0)
}

// ───────────────────────────── API ─────────────────────────────
fn status_json(st: &State) -> Value {
    json!({
        "module": "nidhogg", "version": VERSION,
        "uptime_secs": st.started.elapsed().as_secs(),
        "on": st.on,
        "level": st.level, "level_name": level_name(st.level), "levels": levels_json(),
        "needs_ia": st.level >= 1,
        "cadence_secs": st.cadence,
        "dir": st.dir,
        "collections_known": known_count(&st.dir),
        "last_cycle": st.last_cycle,
        "ragd_api": st.ragd_api,
        "ragd_online": st.ragd_online,   // cache do keepalive (instantâneo)
        "ragd": st.ragd_health.clone(),
    })
}

/// GET /api/nidhogg/collections — lista as coleções do ragd anotadas com o estado de digestão.
fn collections_json(st: &State) -> Value {
    let mut out = vec![];
    if let Some(s) = http_get(&format!("{}/collections", st.ragd_api)) {
        if let Ok(v) = serde_json::from_str::<Value>(&s) {
            let arr = v.get("collections").and_then(|x| x.as_array()).cloned()
                .or_else(|| v.as_array().cloned()).unwrap_or_default();
            for c in arr {
                let name = c.get("collection").and_then(|x| x.as_str()).unwrap_or("").to_string();
                if name.is_empty() { continue; }
                let k = read_knowledge(&st.dir, &name);
                out.push(json!({
                    "collection": name,
                    "bases": c.get("bases").cloned().unwrap_or(Value::Null),
                    "chunks": c.get("chunks").cloned().unwrap_or(Value::Null),
                    "enabled": k["enabled"].as_bool().unwrap_or(false),
                    "saturation": k["saturation"].as_f64().unwrap_or(0.0),
                    "updated": k["updated"].as_str().unwrap_or(""),
                    "has_knowledge": k["knowledge"].as_array().map(|a| !a.is_empty()).unwrap_or(false),
                }));
            }
        }
    }
    json!({"collections": out})
}

fn route(method: &Method, path: &str, body: &str, st: &Arc<Mutex<State>>) -> (u16, String) {
    match (method, path) {
        (Method::Get, "/health") => {
            let s = st.lock().unwrap();
            (200, json!({"status":"ok","module":"nidhogg","version":VERSION,"on":s.on,"level":level_name(s.level)}).to_string())
        }
        (Method::Get, "/api/nidhogg") => { let s = st.lock().unwrap(); (200, status_json(&s).to_string()) }
        (Method::Get, "/api/nidhogg/collections") => { let s = st.lock().unwrap(); (200, collections_json(&s).to_string()) }
        (Method::Post, "/api/nidhogg") => {
            let v: Value = match serde_json::from_str(body) { Ok(v) => v, Err(e) => return (400, json!({"error":format!("JSON inválido: {e}")}).to_string()) };
            let mut s = st.lock().unwrap();
            if let Some(on) = v["on"].as_bool() { s.on = on; let p = s.cfg_path.clone(); set_cfg_key(&p, "nidhogg", if on {"true"} else {"false"}); }
            if let Some(lv) = v["level"].as_str().map(level_num).or_else(|| v["level"].as_u64().map(|n| n as u8)) {
                let lv = lv.min(3); s.level = lv; let p = s.cfg_path.clone(); set_cfg_key(&p, "level", level_name(lv));
            }
            if let Some(c) = v["cadence"].as_u64() { s.cadence = c.max(10); let p = s.cfg_path.clone(); set_cfg_key(&p, "cadence", &s.cadence.to_string()); }
            nlog(&format!("config: on={} nível={} cadência={}s", s.on, level_name(s.level), s.cadence));
            (200, status_json(&s).to_string())
        }
        // liga/desliga o acesso do Nidhogg a UMA coleção (não re-mastiga a mesma N vezes)
        (Method::Post, "/api/nidhogg/collection") => {
            let v: Value = match serde_json::from_str(body) { Ok(v) => v, Err(e) => return (400, json!({"error":format!("JSON inválido: {e}")}).to_string()) };
            let coll = match v["collection"].as_str() { Some(c) if !c.is_empty() => c.to_string(), _ => return (400, json!({"error":"falta 'collection'"}).to_string()) };
            let enabled = v["enabled"].as_bool().unwrap_or(false);
            let s = st.lock().unwrap();
            let mut k = read_knowledge(&s.dir, &coll);
            k["enabled"] = json!(enabled);
            write_knowledge(&s.dir, &coll, &k);
            nlog(&format!("coleção {coll:?} -> acesso {}", if enabled {"LIGADO"} else {"desligado"}));
            (200, json!({"ok":true,"collection":coll,"enabled":enabled}).to_string())
        }
        // dispara um ciclo agora (stub — a inteligência entra aqui)
        (Method::Post, "/api/nidhogg/run") => { nlog("run manual (stub)"); (200, json!({"ok":true,"note":"ciclo stub — inteligência ainda não implementada"}).to_string()) }
        _ => (404, json!({"error":"rota não encontrada","path":path}).to_string()),
    }
}

// ───────────────────────────── worker (esqueleto) ─────────────────────────────
// Acorda na cadência; se ON, faz uma passada. Hoje só registra e checa o ragd.
// Os 3 pilares (nível 0) e a IA (nível >=1) entram aqui nas próximas iterações.
fn worker(state: Arc<Mutex<State>>) {
    loop {
        let cadence = { state.lock().unwrap().cadence.max(10) };
        std::thread::sleep(Duration::from_secs(cadence));
        let (on, level, online) = { let s = state.lock().unwrap(); (s.on, s.level, s.ragd_online) };
        if !on { continue; }
        // TODO(pilares nível 0): índice de raízes · dicionário do corpus · digestão do cache (via API do ragd)
        // TODO(IA nível >=1): por coleção HABILITADA e não-saturada, gerar conhecimento e persistir c/ proveniência
        let stamp = now_stamp();
        nlog(&format!("ciclo (nível={}, ragd={}) — esqueleto: nada a mastigar ainda", level_name(level), if online {"online"} else {"OFFLINE"}));
        if let Ok(mut s) = state.lock() {
            s.last_cycle = format!("{stamp} · nível {} · ragd {} · ciclo vazio (estrutura básica)", level_name(level), if online {"online"} else {"offline"});
        }
    }
}

fn help() {
    println!("nidhoggd {VERSION} — Níðhöggr, camada de inteligência do RAGnaRock (daemon de módulos).
uso:
  nidhoggd [--config <arq>] [--port {DEFAULT_PORT}] [--ragd <url>]
  config: --config <arq>, senão ./nidhogg.cfg, senão defaults.
          chaves: port, ragd_api, nidhogg(on/off), level(minerador|consciente|estrutural|propositivo), dir, cadence
  nasce DESLIGADO (precisa de IA). Liga pelo ValHalla ou pelo cfg.
rotas:
  GET  /health
  GET  /api/nidhogg                 status (nível, cadência, keepalive do ragd, conhecimento)
  GET  /api/nidhogg/collections     coleções do ragd + estado de digestão (liga/desliga por coleção)
  POST /api/nidhogg                 {{\"on\":bool,\"level\":\"minerador|...\",\"cadence\":secs}}
  POST /api/nidhogg/collection      {{\"collection\":\"x\",\"enabled\":bool}}
  POST /api/nidhogg/run             dispara um ciclo agora (stub)");
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--help" || a == "-h") { help(); return; }

    let mut cfg = Config::default();
    // resolve config: --config <arq> senão ./nidhogg.cfg
    let cfg_path = {
        let mut p = "nidhogg.cfg".to_string();
        let mut it = args.iter();
        while let Some(a) = it.next() { if a == "--config" { if let Some(x) = it.next() { p = x.clone(); } } }
        p
    };
    if Path::new(&cfg_path).exists() { load_cfg(&mut cfg, &cfg_path); } else { cfg.cfg_path = cfg_path.clone(); }
    // CLI sobrescreve
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--port" => if let Some(x) = it.next() { if let Ok(p) = x.parse() { cfg.port = p; } },
            "--ragd" => if let Some(x) = it.next() { cfg.ragd_api = x.clone(); },
            _ => {}
        }
    }

    let _ = std::fs::create_dir_all(&cfg.dir);
    let state = Arc::new(Mutex::new(State {
        on: cfg.on, level: cfg.level, dir: cfg.dir.clone(), cadence: cfg.cadence,
        ragd_api: cfg.ragd_api.clone(), cfg_path: cfg.cfg_path.clone(),
        started: Instant::now(), last_cycle: String::new(),
        ragd_online: false, ragd_health: Value::Null,
    }));

    println!("🐉 Níðhöggr {VERSION} — camada de inteligência (daemon de módulos)");
    println!("   estado: {} · nível {} · cadência {}s · ragd {} · conhecimento em {:?}",
             if cfg.on {"LIGADO"} else {"desligado"}, level_name(cfg.level), cfg.cadence, cfg.ragd_api, cfg.dir);

    // keepalive (pinga o ragd a cada 15s, cacheia) + worker (cadência, mastiga)
    let kst = state.clone();
    std::thread::spawn(move || keepalive(kst));
    let wst = state.clone();
    std::thread::spawn(move || worker(wst));

    // servidor HTTP do módulo (porta 11497)
    let addr = format!("0.0.0.0:{}", cfg.port);
    let server = Server::http(&addr).unwrap_or_else(|e| { eprintln!("erro ao subir em {addr}: {e}"); std::process::exit(1); });
    println!("🕸  API do módulo em http://{addr}/  · /health /api/nidhogg /api/nidhogg/collections");

    for mut req in server.incoming_requests() {
        let method = req.method().clone();
        let full = req.url().to_string();
        let path = full.split('?').next().unwrap_or("").to_string();
        let mut body = String::new();
        let _ = req.as_reader().read_to_string(&mut body);
        let (code, payload) = route(&method, &path, &body, &state);
        let mut resp = Response::from_string(payload).with_status_code(code);
        resp.add_header(Header::from_bytes(&b"Content-Type"[..], &b"application/json"[..]).unwrap());
        // CORS liberado (11497 pode ser consumida direto por outras ferramentas)
        resp.add_header(Header::from_bytes(&b"Access-Control-Allow-Origin"[..], &b"*"[..]).unwrap());
        let _ = req.respond(resp);
    }
}
