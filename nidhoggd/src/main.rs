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
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use serde_json::{json, Value};
use tiny_http::{Header, Method, Response, Server};

mod db;
mod chdb;

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
        {"n":0,"name":"minerador","ia":false,"desc":"Zero IA. Minera a estrutura do corpus — assinatura léxica (as raízes que só a coleção tem), dicionário e digest do cache. O material bruto sobre o qual todos os níveis de IA trabalham."},
        {"n":1,"name":"consciente","ia":true,"desc":"1º nível com IA. Classifica cada documento em {natureza, tipo} por IA leve (vocabulário editável) e normaliza o dado — a camada de significado que vive no ClickHouse, aponta pro corpus e sobrevive à deleção da coleção."},
        {"n":2,"name":"estrutural","ia":true,"desc":"Grafa as relações sobre o dado já normalizado — entidades, dimensões e como uma coisa encaixa na outra entre coleções. O grafo navegável do conhecimento."},
        {"n":3,"name":"propositivo","ia":true,"desc":"As perguntas que você não está fazendo. Acha lacunas, levanta hipóteses e aponta o que falta sobre o dado dos níveis anteriores — IA cara só nos pontos de decisão."}
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
    cors_origin: String, // CORS: vazio = sem header (same-origin safe); senão ecoa o valor
    llm_url: String,     // endpoint OpenAI-compat da IA (nível >=1). ⚠️ APONTAR PRA IA DA FROTA:
                         // o nível 1 manda CONTEÚDO do corpus (ex. `real`, sensível) pro LLM —
                         // nuvem = conteúdo SAI da frota. Default = llama-server local (Aron).
                         // Independente do provider do ragd de propósito.
    store: String,       // backend do acumulado/classes: "clickhouse" (default) | "sqlite" (rollback)
    ch_url: String,      // endpoint HTTP do ClickHouse (default http://127.0.0.1:8123)
}
impl Default for Config {
    fn default() -> Self {
        Config { port: DEFAULT_PORT, ragd_api: DEFAULT_RAGD_API.to_string(), on: false, level: 0,
                 dir: DEFAULT_DIR.to_string(), cadence: DEFAULT_CADENCE, cfg_path: "nidhogg.cfg".to_string(),
                 cors_origin: String::new(),
                 llm_url: "http://127.0.0.1:8080/v1/chat/completions".to_string(),
                 store: "clickhouse".to_string(),
                 ch_url: "http://127.0.0.1:8123".to_string() }
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
            "cors_origin" => cfg.cors_origin = v.to_string(),
            "llm_url"  => cfg.llm_url = v.to_string(),
            "store"    => cfg.store = v.to_string(),
            "ch_url"   => cfg.ch_url = v.to_string(),
            other => eprintln!("config: chave desconhecida {other:?}"),
        }
    }
    eprintln!("config: carregada de {path:?}");
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
    llm_url: String,       // IA da frota p/ nível >=1 (ver comentário na Config)
    store: String,         // backend do acumulado: "clickhouse" | "sqlite"
    ch_url: String,        // endpoint HTTP do ClickHouse
    cfg_path: String,
    started: Instant,
    last_cycle: String,
    ragd_online: bool,     // cache do keepalive (atualizado por thread leve) — status NUNCA faz curl ao vivo
    ragd_health: Value,    // último /health do ragd
    cycle_running: bool,   // um ciclo em andamento? (worker OU /run async). Impede concorrência.
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
fn nlog(line: &str) { use std::io::Write; println!("[{}] [nidhogg] {line}", now_stamp()); std::io::stdout().flush().ok(); }

// ───────────────────────────── HTTP client (via wget, no espírito do ragd) ─────────────────────────────
fn http_get(url: &str) -> Option<String> { http_get_t(url, 3) }

/// GET com timeout configurável. O keepalive usa 3s (rápido); o worker usa um timeout
/// generoso porque `/profile` unificado numa coleção grande (centenas de bases) pode
/// demorar mais que 3s no ferro modesto da OpenFrame.
fn http_get_t(url: &str, secs: u32) -> Option<String> {
    // portátil: tenta curl (mac/linux), cai pra wget.
    for tool in ["curl", "wget"] {
        let mut cmd = std::process::Command::new(tool);
        if tool == "curl" { cmd.args(["-s", "-m", &secs.to_string(), url]); }
        else { cmd.args(["-q", "-O", "-", "--tries=1", &format!("--timeout={secs}"), url]); }
        if let Ok(out) = cmd.output() {
            if out.status.success() && !out.stdout.is_empty() { return Some(String::from_utf8_lossy(&out.stdout).to_string()); }
        }
    }
    None
}
/// POST JSON (curl/wget). Usado pro /chunk do ragd e pra IA do nível >=1.
fn http_post_t(url: &str, body: &str, secs: u32) -> Option<String> {
    for tool in ["curl", "wget"] {
        let mut cmd = std::process::Command::new(tool);
        if tool == "curl" {
            cmd.args(["-s", "-m", &secs.to_string(), "-H", "Content-Type: application/json", "-d", body, url]);
        } else {
            cmd.args(["-q", "-O", "-", "--tries=1", &format!("--timeout={secs}"),
                      "--header=Content-Type: application/json", &format!("--post-data={body}"), url]);
        }
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
    if let Ok(s) = serde_json::to_string_pretty(v) {
        // Escrita ATÔMICA (tmp + rename): protege contra gravação concorrente (worker × /run)
        // e contra tombo no meio (a lição do journal abortado da OpenFrame). rename() no mesmo
        // FS é atômico — ou o arquivo antigo, ou o novo inteiro, nunca um meio-escrito.
        let final_path = knowledge_path(dir, collection);
        // tmp ÚNICO por escritor (pid+nanos): worker e /run podem gravar a mesma coleção
        // concorrentemente; cada um escreve seu tmp completo e dá rename — o último vence,
        // nunca um arquivo rasgado.
        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos()).unwrap_or(0);
        let tmp_path = final_path.with_extension(format!("json.{}.{}.tmp", std::process::id(), nanos));
        match std::fs::write(&tmp_path, &s) {
            Ok(_) => if let Err(e) = std::fs::rename(&tmp_path, &final_path) {
                nlog(&format!("write_knowledge {collection}: rename falhou ({e}) — conhecimento NÃO persistido"));
            },
            Err(e) => nlog(&format!("write_knowledge {collection}: escrita falhou ({e}) — conhecimento NÃO persistido")),
        }
    }
}
/// Conta coleções já com algum conhecimento gravado.
fn known_count(dir: &str) -> usize {
    std::fs::read_dir(dir).map(|rd| rd.flatten()
        .filter(|e| e.path().to_string_lossy().ends_with(".knowledge.json")).count()).unwrap_or(0)
}
/// Lê TODOS os <coll>.knowledge.json do dir (ignora os .tmp de gravação atômica).
fn list_knowledge(dir: &str) -> Vec<Value> {
    let mut out = vec![];
    if let Ok(rd) = std::fs::read_dir(dir) {
        for e in rd.flatten() {
            let p = e.path();
            if p.to_string_lossy().ends_with(".knowledge.json") {
                if let Ok(s) = std::fs::read_to_string(&p) {
                    if let Ok(v) = serde_json::from_str::<Value>(&s) { out.push(v); }
                }
            }
        }
    }
    out
}
/// Valor de um parâmetro da query string (sem urldecode — chaves do Nidhogg são simples).
fn query_param(query: &str, key: &str) -> Option<String> {
    query.split('&').find_map(|kv| kv.split_once('=').and_then(|(k, v)| (k == key).then(|| v.to_string())))
}

/// [#29] Monta a resposta de leitura do conhecimento, aplicando os filtros opcionais
/// (collection / type / level) sobre os itens de `knowledge[]`.
fn knowledge_query(st: &State, query: &str) -> Value {
    let type_f = query_param(query, "type");
    let level_f = query_param(query, "level").and_then(|s| s.parse::<u64>().ok());
    let filter = |k: &Value| -> Vec<Value> {
        k["knowledge"].as_array().map(|arr| arr.iter().filter(|it| {
            type_f.as_deref().map_or(true, |t| it["type"].as_str() == Some(t))
                && level_f.map_or(true, |l| it["level"].as_u64() == Some(l))
        }).cloned().collect()).unwrap_or_default()
    };
    // ?collection=X → o mapa inteiro daquela coleção (com knowledge[] filtrado).
    if let Some(coll) = query_param(query, "collection") {
        let mut k = read_knowledge(&st.dir, &coll);
        let items = filter(&k);
        k["knowledge"] = json!(items);
        return k;
    }
    // sem collection → todas as coleções conhecidas (cada mapa com knowledge[] filtrado).
    let collections: Vec<Value> = list_knowledge(&st.dir).into_iter().map(|mut k| {
        let items = filter(&k);
        k["knowledge"] = json!(items);
        k
    }).collect();
    json!({"collections": collections})
}

/// [#22] Leitura do Summary (nível 1). O Summary mora em `k["summary"]` (fora do `knowledge[]`
/// dos pilares do nível 0), então tem janela própria. ?collection=X → uma; sem → todas que têm.
fn summary_query(st: &State, query: &str) -> Value {
    if let Some(coll) = query_param(query, "collection") {
        let k = read_knowledge(&st.dir, &coll);
        return json!({"collection": coll, "summary": k.get("summary").cloned().unwrap_or(Value::Null)});
    }
    let summaries: Vec<Value> = list_knowledge(&st.dir).into_iter().filter_map(|k| {
        match k.get("summary") {
            Some(s) if !s.is_null() => Some(json!({"collection": k["collection"], "summary": s})),
            _ => None,
        }
    }).collect();
    json!({"summaries": summaries})
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
        "cycle_running": st.cycle_running,   // um ciclo (worker ou /run async) em andamento
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

fn route(method: &Method, path: &str, query: &str, body: &str, st: &Arc<Mutex<State>>) -> (u16, String) {
    match (method, path) {
        (Method::Get, "/health") => {
            let s = st.lock().unwrap();
            (200, json!({"status":"ok","module":"nidhogg","version":VERSION,"on":s.on,"level":level_name(s.level)}).to_string())
        }
        (Method::Get, "/api/nidhogg") => { let s = st.lock().unwrap(); (200, status_json(&s).to_string()) }
        (Method::Get, "/api/nidhogg/collections") => { let s = st.lock().unwrap(); (200, collections_json(&s).to_string()) }
        // [#29] lê o conhecimento destilado (o que a mineração extraiu). Filtros opcionais
        // por query: ?collection=X (uma só; senão todas) &type=RootIndex|CorpusDict &level=0.
        // SÓ leitura — o ragd nunca consome isto; é a janela pro que o worm colheu.
        (Method::Get, "/api/nidhogg/knowledge") => { let s = st.lock().unwrap(); (200, knowledge_query(&s, query).to_string()) }
        // [#48] CacheDigest — pilar GLOBAL do nível 0 (digest do cache de expansão do ragd).
        (Method::Get, "/api/nidhogg/cachedigest") => { let s = st.lock().unwrap(); (200, read_cachedigest(&s.dir).to_string()) }
        // [#22] Summary — nível 1 (consciente), o resumo destilado por IA.
        (Method::Get, "/api/nidhogg/summary") => { let s = st.lock().unwrap(); (200, summary_query(&s, query).to_string()) }
        // [prompts] biblioteca de prompts nomeados — o que/como cada nível/coleção extrai.
        (Method::Get, "/api/nidhogg/prompts") => { let s = st.lock().unwrap(); (200, read_prompts(&s.dir).to_string()) }
        (Method::Post, "/api/nidhogg/prompts/template") => {
            let v: Value = match serde_json::from_str(body) { Ok(v) => v, Err(e) => return (400, json!({"error":format!("JSON inválido: {e}")}).to_string()) };
            let name = match v["name"].as_str() { Some(n) if !n.trim().is_empty() => n.trim().to_string(), _ => return (400, json!({"error":"falta 'name'"}).to_string()) };
            let system = v["system"].as_str().unwrap_or("").to_string();
            if system.trim().is_empty() { return (400, json!({"error":"falta 'system'"}).to_string()); }
            if system.chars().count() > PROMPT_MAX_CHARS { return (400, json!({"error":format!("system excede {PROMPT_MAX_CHARS} caracteres")}).to_string()); }
            let desc = v["description"].as_str().unwrap_or("").to_string();
            let dir = { st.lock().unwrap().dir.clone() };
            let mut lib = read_prompts(&dir);
            let mut tobj = json!({"description": desc, "system": system, "updated": now_stamp()});
            // #3 max_tokens opcional por template (clampado ao teto); ausente = default global.
            if let Some(mt) = v["max_tokens"].as_u64() { tobj["max_tokens"] = json!(mt.clamp(64, SUMMARY_MAX_TOKENS_CEIL)); }
            lib["templates"][name.as_str()] = tobj;
            write_prompts(&dir, &lib);
            nlog(&format!("prompt template {name:?} salvo ({} chars, max_tokens={})", system.chars().count(),
                          v["max_tokens"].as_u64().map(|m| m.to_string()).unwrap_or_else(|| "default".into())));
            (200, json!({"ok":true,"template":name}).to_string())
        }
        (Method::Post, "/api/nidhogg/prompts/assign") => {
            let v: Value = match serde_json::from_str(body) { Ok(v) => v, Err(e) => return (400, json!({"error":format!("JSON inválido: {e}")}).to_string()) };
            let template = match v["template"].as_str() { Some(t) if !t.is_empty() => t.to_string(), _ => return (400, json!({"error":"falta 'template'"}).to_string()) };
            let dir = { st.lock().unwrap().dir.clone() };
            let mut lib = read_prompts(&dir);
            if !lib["templates"][template.as_str()].is_object() { return (400, json!({"error":format!("template {template:?} não existe")}).to_string()); }
            if let Some(coll) = v["collection"].as_str() { lib["collections"][coll] = json!(template); }
            else if let Some(lv) = v["level"].as_u64() { let ls = lv.to_string(); lib["level_default"][ls.as_str()] = json!(template); }
            else { return (400, json!({"error":"informe 'collection' ou 'level'"}).to_string()); }
            write_prompts(&dir, &lib);
            nlog(&format!("prompt assign: template={template}"));
            (200, json!({"ok":true}).to_string())
        }
        // [Fase 1] classes {natureza,tipo} do banco auxiliar — distribuição por coleção (ou todas).
        (Method::Get, "/api/nidhogg/classes") => {
            let (store, dir, ch_url) = { let s = st.lock().unwrap(); (s.store.clone(), s.dir.clone(), s.ch_url.clone()) };
            let coll = query_param(query, "collection");
            match store_classes_summary(&store, &dir, &ch_url, coll.as_deref()) {
                Ok(v) => (200, v.to_string()),
                Err(e) => (500, json!({"error": format!("store: {e}")}).to_string()),
            }
        }
        // [Fase 2] entidades extraídas (o dump denso) — ClickHouse only, sempre via a view entidade_atual.
        (Method::Get, "/api/nidhogg/entities") => {
            let (store, ch_url) = { let s = st.lock().unwrap(); (s.store.clone(), s.ch_url.clone()) };
            let coll = query_param(query, "collection");
            let base = query_param(query, "base");
            if store != "clickhouse" {
                (200, json!({"count": 0, "note": "extração requer clickhouse"}).to_string())
            } else {
                match chdb::entities_summary(&ch_url, coll.as_deref(), base.as_deref()) {
                    Ok(v) => (200, v.to_string()),
                    Err(e) => (500, json!({"error": format!("store: {e}")}).to_string()),
                }
            }
        }
        // [Fase 3] templates — o registry de moldes por tipo (schema + regras regex + cobertura).
        (Method::Get, "/api/nidhogg/templates") => {
            let (store, ch_url) = { let s = st.lock().unwrap(); (s.store.clone(), s.ch_url.clone()) };
            if store != "clickhouse" {
                (200, json!({"templates": {}, "note": "registry requer clickhouse"}).to_string())
            } else {
                match chdb::get_templates(&ch_url) {
                    Ok(v) => (200, json!({"templates": v}).to_string()),
                    Err(e) => (500, json!({"error": format!("store: {e}")}).to_string()),
                }
            }
        }
        // [Fase 1] doctypes — a lista EDITÁVEL de naturezas/tipos que alimenta o enum do classificador.
        (Method::Get, "/api/nidhogg/doctypes") => {
            let (store, dir, ch_url) = { let s = st.lock().unwrap(); (s.store.clone(), s.dir.clone(), s.ch_url.clone()) };
            let (nat, tip) = store_doctypes(&store, &dir, &ch_url);
            (200, json!({"naturezas": nat, "tipos": tip}).to_string())
        }
        (Method::Post, "/api/nidhogg/doctypes") => {
            let v: Value = match serde_json::from_str(body) { Ok(v) => v, Err(e) => return (400, json!({"error":format!("JSON inválido: {e}")}).to_string()) };
            let to_vec = |k: &str| -> Vec<String> {
                v[k].as_array().map(|a| a.iter().filter_map(|x| x.as_str().map(|s| s.trim().to_string()))
                    .filter(|s| !s.is_empty()).collect()).unwrap_or_default()
            };
            let naturezas = to_vec("naturezas");
            let tipos = to_vec("tipos");
            if naturezas.is_empty() || tipos.is_empty() { return (400, json!({"error":"naturezas e tipos não podem ser vazios"}).to_string()); }
            let (store, dir, ch_url) = { let s = st.lock().unwrap(); (s.store.clone(), s.dir.clone(), s.ch_url.clone()) };
            match store_write_doctypes(&store, &dir, &ch_url, &naturezas, &tipos) {
                Ok(_) => { nlog(&format!("doctypes atualizados: {} naturezas, {} tipos — reclassifica no próximo ciclo", naturezas.len(), tipos.len()));
                           (200, json!({"ok":true,"naturezas":naturezas.len(),"tipos":tipos.len()}).to_string()) }
                Err(e) => (500, json!({"error": format!("store: {e}")}).to_string()),
            }
        }
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
        // dispara um ciclo AGORA, FORÇADO (re-minera o nível 0 ignorando o source_hash).
        // É o "atualiza já" — e o caminho de refresh quando os dados não mudaram.
        // ASSÍNCRONO: spawna o ciclo numa thread e retorna NA HORA — o servidor single-thread
        // não trava mais durante a geração. cycle_running impede disparo concorrente.
        (Method::Post, "/api/nidhogg/run") => {
            if try_start_cycle(st) {
                let st2 = st.clone();
                std::thread::spawn(move || { run_cycle(&st2, true); end_cycle(&st2); });
                nlog("run manual — ciclo FORÇADO iniciado (async)");
                (202, json!({"ok":true,"started":true,"note":"ciclo em andamento — acompanhe cycle_running no status"}).to_string())
            } else {
                (200, json!({"ok":true,"started":false,"reason":"já há um ciclo em andamento"}).to_string())
            }
        }
        _ => (404, json!({"error":"rota não encontrada","path":path}).to_string()),
    }
}

// ───────────────────────────── nível 0: os pilares (zero IA) ─────────────────────────────
// Minera a ESTRUTURA da coleção via API do ragd (nunca disco). É navegação/índice/saúde —
// NÃO é "conhecimento" (esse é o trabalho dos níveis 1-3 com IA). Custa zero IA.
fn hash_hex(s: &str) -> String {
    let mut h = DefaultHasher::new();
    s.hash(&mut h);
    format!("{:016x}", h.finish())
}

/// state_hash de uma base = hash(name, n_chunks, vocab_size, corpus) — NUNCA o path.
/// Renomear o arquivo não muda o hash; só mudança real de conteúdo muda.
fn base_state_hash(b: &Value) -> String {
    hash_hex(&format!("{}|{}|{}|{}",
        b["name"].as_str().unwrap_or(""),
        b["n_chunks"].as_u64().unwrap_or(0),
        b["vocab_size"].as_u64().unwrap_or(0),
        b["corpus"].as_str().unwrap_or("")))
}

/// source_hash da coleção = hash da lista ORDENADA dos state_hash das bases.
/// Mudou/entrou/saiu qualquer base → muda → vale remastigar. (Núcleo do #21 a nível de
/// coleção; o diff fino new/changed/removed por base é do #21 completo, ainda [FUTURO].)
fn collection_source_hash(bases: &[Value]) -> String {
    let mut hs: Vec<String> = bases.iter().map(base_state_hash).collect();
    hs.sort();
    hash_hex(&hs.join(","))
}

/// Minera o nível 0 de UMA coleção (2 chamadas ao ragd: /bases e /profile). Devolve
/// (source_hash, pilares[], n_bases, total_chunks) — ou None se o ragd não responder
/// (não grava dados parciais). Os pilares são DISTINTOS: RootIndex = identidade léxica
/// (sílabas salientes), CorpusDict = anatomia (composição por base).
fn mine_level0(api: &str, coll: &str) -> Option<(String, Vec<Value>, usize, u64)> {
    // 1) /bases?collection — meta por base (alimenta source_hash E o CorpusDict).
    let bases_resp: Value = serde_json::from_str(&http_get_t(&format!("{api}/bases?collection={coll}"), 30)?).ok()?;
    let bases = bases_resp["bases"].as_array()?.clone();
    if bases.is_empty() { return None; }
    let source_hash = collection_source_hash(&bases);
    let total_chunks: u64 = bases.iter().map(|b| b["n_chunks"].as_u64().unwrap_or(0)).sum();

    // 2) /profile?collection — vocabulário unificado + sílabas salientes (top por idf×freq).
    // rank=idffreq (#46): o /profile rankeia as dims por uidf×freq e expõe df/freq por dim.
    // Sem isso o top saía como caça-ao-hapax (só sílabas df=1, uidf máximo empatado).
    let prof: Value = serde_json::from_str(&http_get_t(&format!("{api}/profile?collection={coll}&top=40&vectors=1&rank=idffreq"), 30)?).ok()?;
    let salient = prof["top_uidf"].as_array().cloned().unwrap_or_default();
    let unified_vocab = prof["unified_vocab_size"].as_u64().unwrap_or(0);
    // shared/unique do vocab unificado (#20 restante): dims em >1 base (backbone) vs em 1 só
    // (assinatura exclusiva). Vem do /profile; o coverage/OOV por base vem dentro do base_vectors.
    let shared_vocab = prof["shared_vocab"].as_u64().unwrap_or(0);
    let unique_vocab = prof["unique_vocab"].as_u64().unwrap_or(0);
    // dims-por-base (heatmap/dendrograma): vetor tf-idf de cada base nas dims salientes,
    // alinhado 1:1 com `salient`. Com rank=idffreq as dims salientes deixam de ser hapax
    // (peso ~0 por base) e o base_vectors ganha sinal real (#47). Vem do /profile&vectors=1.
    let base_vectors = prof["base_vectors"].as_array().cloned().unwrap_or_default();

    // Pilar 1 — RootIndex: as sílabas/dims mais salientes (rankeadas por idf×freq). É a
    // IDENTIDADE LÉXICA da coleção: o que a distingue das outras — distintivo E recorrente.
    let root_index = json!({
        "type": "RootIndex", "level": 0,
        "content": {
            "bases_count": bases.len(),
            "total_chunks": total_chunks,
            "unified_vocab_size": unified_vocab,
            "salient_roots": salient,   // [{dim, syllable, uidf, df, freq}], ordenado por idf×freq desc
            "note": "ranking idf×freq ATIVO (#46): privilegia sílaba distintiva E recorrente, não o hapax de OCR. Agrupamento por raiz (stem) segue [FUTURO]."
        }
    });

    // Pilar 2 — CorpusDict: a ANATOMIA do corpus (largura + composição por base). Distinto
    // do RootIndex: aqui é quantas bases, o tamanho/vocab de cada — não as sílabas salientes.
    let per_base: Vec<Value> = bases.iter().map(|b| json!({
        "name": b["name"], "corpus": b["corpus"],
        "n_chunks": b["n_chunks"], "vocab_size": b["vocab_size"]
    })).collect();
    let corpus_dict = json!({
        "type": "CorpusDict", "level": 0,
        "content": {
            "unified_vocab_size": unified_vocab,
            "shared_vocab": shared_vocab,   // dims em >1 base (backbone comum da coleção)
            "unique_vocab": unique_vocab,   // dims em 1 base só (assinatura exclusiva somada)
            "bases": per_base,
            "base_vectors": base_vectors,   // [{name,corpus,n_chunks,dims_used,coverage,unique_dims,shared_dims,vec[]}]
            "note": "base_vectors = tf-idf por base nas dims salientes (alinhado 1:1 ao salient_roots, #47 ok pós-#46). Cada base traz coverage/OOV: dims_used, coverage (fração do vocab unificado), unique_dims (sílabas exclusivas = OOV das outras) e shared_dims. Agrupamento por raiz/stem segue [FUTURO]."
        }
    });

    // CacheDigest: ADIADO — exige um endpoint novo no ragd p/ ler o cache de expansão (o
    // invariante proíbe o nidhoggd ler disco da coleção). Registrado, não fingido.
    Some((source_hash, vec![root_index, corpus_dict], bases.len(), total_chunks))
}

// ───────────────────────────── CacheDigest (#48) — pilar GLOBAL do nível 0 ─────────────────────────────
fn cachedigest_path(dir: &str) -> std::path::PathBuf { Path::new(dir).join("_cachedigest.json") }
/// Escreve o digest global do cache de expansão. Nome `_cachedigest.json` (NÃO `.knowledge.json`)
/// de propósito: `known_count`/`collections_known` só contam `*.knowledge.json`, então o digest
/// global não vira coleção-fantasma. Escrita atômica (tmp + rename), como o write_knowledge.
fn write_cachedigest(dir: &str, v: &Value) {
    let _ = std::fs::create_dir_all(dir);
    if let Ok(s) = serde_json::to_string_pretty(v) {
        let final_path = cachedigest_path(dir);
        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos()).unwrap_or(0);
        let tmp_path = final_path.with_extension(format!("json.{}.{}.tmp", std::process::id(), nanos));
        if std::fs::write(&tmp_path, &s).is_ok() { let _ = std::fs::rename(&tmp_path, &final_path); }
    }
}
fn read_cachedigest(dir: &str) -> Value {
    std::fs::read_to_string(cachedigest_path(dir)).ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_else(|| json!({"type":"CacheDigest","level":0,"scope":"global","updated":"",
            "content":{"n_queries":0,"n_variants_total":0,"avg_variants":0.0,"entries":[],
            "note":"ainda não digerido (rode um ciclo)"}}))
}

/// CacheDigest — o 3º pilar do nível 0 (#48), GLOBAL (não por-coleção): consolida o cache de
/// expansão de query do ragd (`query → variantes`), lido via `GET /expansions` (o invariante
/// proíbe o nidhoggd ler disco da coleção). Cache VAZIO é estado válido → digest com zeros +
/// nota, NUNCA falha por isso. Retorna None só se o ragd não respondeu (não sobrescreve).
/// Clustering de equivalência por "mesmos chunks" é [FUTURO]: o cache não guarda chunk_ids.
fn mine_cachedigest(api: &str) -> Option<Value> {
    let resp: Value = serde_json::from_str(&http_get_t(&format!("{api}/expansions"), 10)?).ok()?;
    let mut entries: Vec<Value> = vec![];
    let mut total_variants = 0u64;
    if let Some(map) = resp["expansions"].as_object() {
        for (q, v) in map {
            let variants = v.as_array().cloned().unwrap_or_default();
            total_variants += variants.len() as u64;
            entries.push(json!({"query": q, "variants": variants, "n_variants": variants.len()}));
        }
    }
    let n = entries.len() as u64;
    Some(json!({
        "type": "CacheDigest", "level": 0, "scope": "global", "updated": now_stamp(),
        "content": {
            "n_queries": n,
            "n_variants_total": total_variants,
            "avg_variants": if n == 0 { 0.0 } else { total_variants as f64 / n as f64 },
            "entries": entries,
            "note": "consolida o cache de expansão do ragd (query→variantes), GLOBAL (não por-coleção — o cache é engine-wide). Clustering de equivalência por 'mesmos chunks' é [FUTURO]: o cache não registra chunk_ids; precisaria o search_expand gravar os hits por variante."
        }
    }))
}

// ───────────────────────────── nível 1 (consciente) — Summary por coleção (#22) ─────────────────────────────
const SUMMARY_CHAR_BUDGET: usize = 20_000;   // janela de FONTE por chamada LLM (~7k tokens PT; cabe no ctx 16384/slot c/ saída)
const BASES_PER_CYCLE: usize = 4;            // bases normalizadas por ciclo (1-4 chamadas LLM cada; o resto fica pros próximos)
const REPAIR_MAX_PASSES: usize = 4;          // teto de passadas por base (gate de escapatória: SEMPRE termina)
const PROMPT_MAX_CHARS: usize = 6000;        // teto do system prompt de um template (cabe no ctx com folga)
const SUMMARY_MAX_TOKENS: u64 = 1500;        // teto DEFAULT de geração do Summary (é resumo, não dump)
const SUMMARY_MAX_TOKENS_CEIL: u64 = 4000;   // teto MÁXIMO configurável por template (trava runaway mesmo custom)
/// Prompt embutido (fallback quando prompts.json some/corrompe, e seed do template "padrão").
/// O daemon NUNCA fica sem prompt. As chaves aqui são as DEFAULT; um template pode definir outras.
const BUILTIN_SUMMARY_PROMPT: &str = "Você é um analista que resume coleções de documentos em português. Seja FIEL ao material amostrado; não invente fatos. Responda APENAS com um objeto JSON com as chaves: \"abstract\" (2 a 4 frases resumindo a coleção), \"themes\" (array de 3 a 6 temas), \"entities\" (array de nomes próprios, organizações e produtos citados) e \"key_points\" (array de 3 a 6 pontos concretos).";

// ───────────────────────────── nível 1 (consciente) — Classificação {natureza,tipo} (Fase 1) ─────────────────────────────
// ADITIVA ao normalizador Summary (coexistem em level>=1): descobre a CLASSE de cada base por
// LLM leve com constrained decoding, persistindo no banco auxiliar (db.rs). É a fundação do
// motor auto-adaptativo — registry determinístico + NQI sobem em cima disto. Endpoint/modelo
// do LLM vem do `nidhogg.cfg` (llm_url); a lista de tipos e o prompt são editáveis no ValHalla.
const CLASSIFY_MAX_CHARS: usize = 1000;   // primeiros N chars do chunk 0 (bate com a calibração 89,5%)
const CLASSIFY_TIMEOUT_S: u32 = 40;       // curto e FIXO (é ~40 tokens; nada do teto 500s do Summary)
const CLASSIFY_MAX_FAILS: usize = 2;      // 2 falhas de LLM consecutivas abortam o lote do ciclo
const CLASSIFY_PER_CYCLE: usize = 40;     // batch por ciclo — classificação é leve (~3s/base), bem além do Summary (4)
// ── Fase 2: extração de entidades (o dump denso no ClickHouse) ──
const EXTRACT_PER_CYCLE: usize = 5;               // extração é PESADA (várias janelas/base) — poucos por ciclo
const EXTRACT_INPUT_CHARS_PER_WINDOW: usize = 1500; // janela por ORÇAMENTO DE CHARS (adaptativo): bases
                                                    // densas pegam menos linhas → não satura o teto de saída
const EXTRACT_MAX_TOKENS: u64 = 1500;
const TEMPLATE_MIN_COVERAGE: f64 = 0.7;   // Fase 3: molde só é gravado se casa ≥70% dos campos na amostra
/// Prompt do extrator (editável no ValHalla como o classificador). `{tipo}` é substituído pela classe.
const BUILTIN_EXTRACT_PROMPT: &str = "Este documento é um {tipo}. Extraia os REGISTROS como um array JSON — um objeto por linha/registro, com os campos relevantes (nomes de campo claros e minúsculos). Responda APENAS com o array JSON, nada além dele.";
/// System prompt calibrado (89,5% no Qwen2.5-7B). Os TIPOS não entram aqui — vêm do `enum` do
/// schema (montado da lista editável de doctypes), então editar a lista muda só o enum.
/// Os EXEMPLOS-âncora entre parênteses e a cláusula "NÃO um documento individual" são o que
/// impede o colapso de comprovante/OC individual em `cadastro` — sem eles a acurácia despenca
/// (medido: 68,8% → 100% de paridade com o baseline). Bate 100% com o baseline nas bases de
/// negócio; a versão acentuada só diverge em narrativos ambíguos do `real` (às vezes melhor).
const BUILTIN_CLASSIFY_PROMPT: &str = "Você classifica um documento em NATUREZA e TIPO.\nNATUREZA: tabular=dados em linhas/colunas, registros, valores (cadastros, notas, comprovantes, balanços, folhas, ordens). narrativo=texto corrido em prosa (livros, artigos, contratos, atas, cartas, currículos). codigo=código-fonte, config, log.\nSe for uma LISTA/TABELA com várias linhas de registros, o TIPO é cadastro ou relatorio, NÃO um documento individual.\nEscolha o TIPO mais específico da lista permitida.";
/// Fase 3 — prompt criador de MOLDE (validado no gate: ancorar no rótulo dá favorecido≠pagador,
/// generalizou 6/6 nos comprovantes, regex Rust-compatível). A âncora no rótulo é OBRIGATÓRIA.
const BUILTIN_TEMPLATE_PROMPT: &str = r#"Você cria um TEMPLATE DE EXTRAÇÃO determinístico para um tipo de documento. Dado UM exemplo, para cada campo rotulado produza um REGEX (sintaxe da crate `regex` do Rust: SEM lookahead/lookbehind, SEM backreference) que:
1. ANCORA no RÓTULO do campo — o rótulo vem ANTES do grupo de captura. OBRIGATÓRIO: sem âncora, 'favorecido' captura o 'pagador' (a primeira ocorrência).
2. tem EXATAMENTE UM grupo de captura () isolando SÓ o valor (sem o rótulo).
3. usa quantificadores GERAIS (\d+, .+?), NUNCA tamanho fixo (\d{8} quebra se variar).
Inclua TODOS os campos rotulados (não omita nenhum). Separe compostos (nome e CNPJ na mesma linha = 2 campos, cada um ancorado no SEU rótulo).
Exemplos de regras BOAS:
  data            -> "Data:\s*(\d{2}/\d{2}/\d{4})"
  favorecido_nome -> "FAVORECIDO:\s*(.+?)\s+CNPJ"
  favorecido_cnpj -> "FAVORECIDO:.*?CNPJ\s*([\d./-]+)"
  valor           -> "VALOR:\s*R\$\s*([\d.,]+)"
'limpar' é só pra FORMATAÇÃO do valor (remover '.', 'R$'), NUNCA o rótulo."#;

// ───────────────────────────── biblioteca de prompts nomeados (por nível/coleção) ─────────────────────────────
fn prompts_path(dir: &str) -> std::path::PathBuf { Path::new(dir).join("prompts.json") }
/// Lê a biblioteca. Ausente/corrompida → seed com o template "padrão" = BUILTIN (nunca sem prompt).
/// Formato: {templates:{name:{description,system,updated}}, level_default:{"1":name}, collections:{coll:name}}
fn read_prompts(dir: &str) -> Value {
    let seed = || json!({
        "templates": {
            "padrão": {
                "description": "Resumo geral: abstract, themes, entities, key_points.",
                "system": BUILTIN_SUMMARY_PROMPT, "updated": "" },
            "classificador": {
                "description": "Classifica {natureza, tipo} na entrada (Fase 1). Os tipos vêm da lista editável de doctypes.",
                "system": BUILTIN_CLASSIFY_PROMPT, "updated": "" },
            "extrator": {
                "description": "Extrai os registros das bases tabulares como array JSON (Fase 2). {tipo} = a classe.",
                "system": BUILTIN_EXTRACT_PROMPT, "updated": "" }
        },
        "level_default": { "1": "padrão" },
        "collections": {}
    });
    match std::fs::read_to_string(prompts_path(dir)).ok().and_then(|s| serde_json::from_str::<Value>(&s).ok()) {
        Some(v) if v["templates"].is_object() => v,
        _ => seed(),
    }
}
fn write_prompts(dir: &str, v: &Value) {
    let _ = std::fs::create_dir_all(dir);
    if let Ok(s) = serde_json::to_string_pretty(v) {
        let final_path = prompts_path(dir);
        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
        let tmp = final_path.with_extension(format!("json.{}.{}.tmp", std::process::id(), nanos));
        if std::fs::write(&tmp, &s).is_ok() { let _ = std::fs::rename(&tmp, &final_path); }
    }
}
/// Resolve o template pra (nível, coleção): override da coleção → default do nível → BUILTIN.
/// Devolve (nome, system, resolved_from) — resolved_from vai na proveniência (debug do 3-camadas).
fn resolve_template(lib: &Value, level: u8, coll: &str) -> (String, String, String, u64) {
    // devolve (nome, system, resolved_from, max_tokens). max_tokens vem do template (#3),
    // com default e teto — clampado pra nunca virar runaway mesmo se o cadastro exagerar.
    let get = |name: &str| {
        let t = &lib["templates"][name];
        t["system"].as_str().map(|s| (s.to_string(),
            t["max_tokens"].as_u64().unwrap_or(SUMMARY_MAX_TOKENS).clamp(64, SUMMARY_MAX_TOKENS_CEIL)))
    };
    if let Some(name) = lib["collections"][coll].as_str() {
        if let Some((sys, mt)) = get(name) { return (name.to_string(), sys, "collection".into(), mt); }
    }
    if let Some(name) = lib["level_default"][level.to_string()].as_str() {
        if let Some((sys, mt)) = get(name) { return (name.to_string(), sys, "level_default".into(), mt); }
    }
    ("padrão".into(), BUILTIN_SUMMARY_PROMPT.to_string(), "builtin".into(), SUMMARY_MAX_TOKENS)
}

/// Extrai um objeto JSON de um texto (tolera fences/prosa em volta). Análogo ao parse_str_array
/// do ragd, mas pra objeto — o Summary é `{abstract, themes, entities, key_points}`.
fn extract_json_object(s: &str) -> Option<Value> {
    let a = s.find('{')?; let b = s.rfind('}')?;
    if a >= b { return None; }
    match serde_json::from_str::<Value>(&s[a..=b]) { Ok(v) if v.is_object() => Some(v), _ => None }
}
/// Extrai um ARRAY JSON de um texto (tolera fences/prosa em volta). Fase 2: os registros extraídos.
fn extract_json_array(s: &str) -> Option<Vec<Value>> {
    let a = s.find('[')?; let b = s.rfind(']')?;
    if a >= b { return None; }
    serde_json::from_str::<Value>(&s[a..=b]).ok().and_then(|v| v.as_array().cloned())
}
/// Resgata um array cortado no teto de tokens: fecha no ÚLTIMO objeto completo e descarta o
/// elemento parcial. Reaproveita a ideia do salvage de objeto — mas devolve o array já fechado.
fn salvage_truncated_array(s: &str) -> Option<Vec<Value>> {
    let a = s.find('[')?;
    let t = &s[a..];
    for (cut, _) in t.rmatch_indices('}').take(1) {
        let candidate = format!("{}]", &t[..=cut]);   // "[{..},{..}" + "]" = array até o último obj completo
        if let Ok(v) = serde_json::from_str::<Value>(&candidate) {
            if let Some(arr) = v.as_array() { return Some(arr.clone()); }
        }
    }
    None
}

// ───────────────────────────── merge genérico do acumulado (normalizado) ─────────────────────────────
/// Campo identificador normalizado de um objeto (comparação de entidade).
fn ent_field(v: &Value, k: &str) -> Option<String> {
    v.get(k).and_then(|x| x.as_str()).map(|s| s.trim().to_lowercase()).filter(|s| !s.is_empty())
}
/// Mesma entidade? Chave FORTE (cnpj/id/código) presente nos DOIS lados decide sozinha —
/// bate = mesma, difere = VETO (nome igual com cnpj diferente são entidades distintas).
/// Sem chave forte comum, decide pelo nome normalizado.
fn same_entity(a: &Value, b: &Value) -> bool {
    for k in ["cnpj", "CNPJ", "id", "codigo"] {
        if let (Some(x), Some(y)) = (ent_field(a, k), ent_field(b, k)) {
            return x == y;   // chave forte decide — inclusive o veto
        }
    }
    for k in ["nome", "name", "titulo"] {
        if let (Some(x), Some(y)) = (ent_field(a, k), ent_field(b, k)) {
            if x == y { return true; }
        }
    }
    false
}
/// Rótulo de exibição de um item (pras passadas de verificação: "já extraído: …").
fn ent_label(v: &Value) -> String {
    for k in ["nome", "name", "titulo", "cnpj", "id", "codigo"] {
        if let Some(s) = v.get(k).and_then(|x| x.as_str()) {
            if !s.trim().is_empty() { return s.trim().to_string(); }
        }
    }
    v.to_string().chars().take(40).collect()
}
/// Funde um valor no acumulado. Regras defensivas (dado de LLM):
/// - null NUNCA substitui nada; array acumulado NUNCA é destruído por não-array;
/// - array acumula: objeto existente (same_entity) é ENRIQUECIDO (campos ausentes
///   preenchidos), objeto novo entra, escalar dedup case/trim-insensitive;
/// - o resto substitui (a última versão vence).
fn merge_value(av: &mut Value, nv: &Value) {
    if nv.is_null() { return; }
    match (&mut *av, nv) {
        (Value::Array(a), Value::Array(n)) => {
            for item in n {
                if item.is_object() {
                    let mut merged = false;
                    for x in a.iter_mut() {
                        if same_entity(x, item) {
                            if let (Some(eo), Some(io)) = (x.as_object_mut(), item.as_object()) {
                                for (k, v) in io {
                                    if eo.get(k).map_or(true, |cur| cur.is_null()) { eo.insert(k.clone(), v.clone()); }
                                }
                            }
                            merged = true;
                            break;
                        }
                    }
                    if !merged { a.push(item.clone()); }
                } else if let Some(is) = item.as_str() {
                    let key = is.trim().to_lowercase();
                    if !a.iter().any(|x| x.as_str().map(|xs| xs.trim().to_lowercase() == key).unwrap_or(false)) {
                        a.push(item.clone());
                    }
                } else if !a.iter().any(|x| x == item) {
                    a.push(item.clone());
                }
            }
        }
        (Value::Array(_), _) => { /* array não se degrada em escalar */ }
        _ => { *av = nv.clone(); }
    }
}
/// Funde um content no acumulado (o template define as chaves — genérico sobre o shape).
fn merge_content(acc: &mut Value, new: &Value) {
    if !acc.is_object() { *acc = new.clone(); return; }
    if let Some(nobj) = new.as_object() {
        for (k, nv) in nobj {
            match acc.get_mut(k.as_str()) {
                Some(av) => merge_value(av, nv),
                None => { if !nv.is_null() { acc[k.as_str()] = nv.clone(); } }
            }
        }
    }
}
/// Soma dos tamanhos de array no content (invariante "arrays não encolhem" + itens no log).
fn array_len_sum(c: &Value) -> usize {
    c.as_object().map(|o| o.values().filter_map(|v| v.as_array().map(|a| a.len())).sum()).unwrap_or(0)
}
/// Maior array do content, qualquer tipo (só pra anotação/log — NÃO prova contagem).
fn max_array_len(c: &Value) -> usize {
    c.as_object().map(|o| o.values().filter_map(|v| v.as_array().map(|a| a.len())).max().unwrap_or(0)).unwrap_or(0)
}
/// O array que PROVA contagem tabular: o maior array de OBJETOS (entidades extraídas).
/// Arrays de strings (themes, key_points…) não contam — inflariam a prova sem mapear linhas.
fn counting_array(c: &Value) -> Option<(String, usize)> {
    c.as_object()?.iter()
        .filter_map(|(k, v)| v.as_array()
            .filter(|a| !a.is_empty() && a.iter().all(|x| x.is_object()))
            .map(|a| (k.clone(), a.len())))
        .max_by_key(|(_, n)| *n)
}
/// Tamanho "acumulável" do content: chaves + itens de array. Strings ficam de fora de
/// propósito — elas SUBSTITUEM no merge, então variação de texto não é novidade. É a régua
/// do gate sem-novidade: passada que não cresce isso não trouxe dado enumerável novo.
fn grow_size(c: &Value) -> usize {
    c.as_object().map(|o| o.len() + o.values().filter_map(|v| v.as_array().map(|a| a.len())).sum::<usize>()).unwrap_or(0)
}

// ───────────────────────────── nível 1 — normalizador (completude por base) ─────────────────────────────
/// Detecção estrutural de documento tabular (CSV/TSV) — determinística e sobre o CONTEÚDO,
/// nunca sobre o nome. Devolve o nº de linhas de DADOS (sem o cabeçalho) se ≥80% das linhas
/// repetem a contagem de delimitadores da primeira.
/// Assinatura tabular: devolve `(delimitador, nº de linhas de DADOS)` quando um delimitador é
/// consistente em ≥80% das linhas não-vazias. É o RECONHECEDOR determinístico — quando casa, a
/// base é um CSV regular e a extração dispensa o LLM (ver `parse_tabular`).
fn tabular_spec(text: &str) -> Option<(char, usize)> {
    let lines: Vec<&str> = text.lines().map(|l| l.trim_end_matches('\r')).filter(|l| !l.trim().is_empty()).collect();
    if lines.len() < 3 { return None; }
    for delim in [',', ';', '\t'] {
        let head = lines[0].matches(delim).count();
        if head == 0 { continue; }
        let consistent = lines.iter().filter(|l| l.matches(delim).count() == head).count();
        if consistent * 100 >= lines.len() * 80 { return Some((delim, lines.len() - 1)); }
    }
    None
}
/// Extração DETERMINÍSTICA de um CSV regular: o CABEÇALHO (linha 0) nomeia os campos e cada linha
/// de dados vira um objeto `{campo: valor}`. Os campos são LIDOS do arquivo, não inferidos — por
/// isso não há LLM aqui, nem schema divergente entre janelas, nem linha pulada. A linha 0 é o
/// schema, logo NUNCA vira registro. Linhas "ragged": células faltantes → `""` (pad), excedentes
/// ignoradas (o objeto tem exatamente as chaves do cabeçalho). Não trata aspas/escape de CSV
/// (RFC 4180) — as bases-alvo são planas; um delimitador dentro de aspas cairia no ragged handling.
fn parse_tabular(text: &str, delim: char) -> (Vec<String>, Vec<Value>) {
    let lines: Vec<&str> = text.lines().map(|l| l.trim_end_matches('\r')).filter(|l| !l.trim().is_empty()).collect();
    if lines.len() < 2 { return (vec![], vec![]); }
    let header: Vec<String> = lines[0].split(delim).map(|c| c.trim().to_string()).collect();
    let mut out = Vec::with_capacity(lines.len() - 1);
    for ln in &lines[1..] {
        let cells: Vec<&str> = ln.split(delim).collect();
        let mut obj = serde_json::Map::new();
        for (i, key) in header.iter().enumerate() {
            let val = cells.get(i).map(|c| c.trim()).unwrap_or("");   // ragged: falta vira ""
            obj.insert(key.clone(), Value::String(val.to_string()));
        }
        out.push(Value::Object(obj));
    }
    (header, out)
}
/// Deriva a NATUREZA semântica do TIPO — determinística. O classificador acerta o tipo (89,5%
/// medido) e a natureza cai por gravidade dele, SEM pedir ao LLM (evita o drift de re-tunar o
/// prompt calibrado). Distingue TABELA (átomo de ANÁLISE, N registros homogêneos) de DOCUMENTO
/// (átomo da PONTA, 1 exemplar individual — comprovante, nota, holerite): a §2 da arquitetura de
/// escala manda NÃO processar a ponta linha-a-linha. Um CSV regular é sempre `tabela` (ver caller).
fn natureza_do_tipo(tipo: &str) -> &'static str {
    match tipo {
        "cadastro" | "relatorio" => "tabela",
        "comprovante" | "nota_fiscal" | "recibo" | "boleto" | "balanco" | "extrato"
            | "dre" | "folha_pagamento" | "ordem_compra" | "cotacao" => "documento",
        "contrato" | "livro" | "artigo" | "ata" | "carta" | "oficio" | "memorial"
            | "curriculo" | "discurso" => "narrativo",
        "codigo_fonte" | "config" | "log" => "codigo",
        _ => "documento",   // fallback conservador: não é tabela → não extrai (o gate csv decide de fato)
    }
}
/// Fase 3 — compila as regras de um molde UMA vez (regex é caro de compilar; reusa nos N documentos
/// do tipo). Regex inválido é DESCARTADO (o caller nota pela diferença de contagem campo↔regra).
fn compile_template(regras: &Value) -> Vec<(String, regex::Regex, Vec<String>)> {
    let mut out = vec![];
    for r in regras.as_array().map(|a| a.as_slice()).unwrap_or(&[]) {
        let campo = match r["campo"].as_str() { Some(c) if !c.is_empty() => c.to_string(), _ => continue };
        let pat = match r["regex"].as_str() { Some(p) => p, _ => continue };
        let re = match regex::Regex::new(pat) { Ok(re) => re, Err(_) => continue };   // regex ruim → pula
        let limpar = r["limpar"].as_array()
            .map(|a| a.iter().filter_map(|s| s.as_str().map(String::from)).collect()).unwrap_or_default();
        out.push((campo, re, limpar));
    }
    out
}
/// Fase 3 — aplica um molde COMPILADO a UM documento → objeto `{campo: valor}`. Cada regra ancora no
/// rótulo e captura o grupo 1 (o valor); as limpezas removem formatação. Determinístico, L0, zero-LLM
/// — o oposto do LLM linha-a-linha. Campo sem match não entra (o caller mede a cobertura).
fn apply_template(text: &str, compiled: &[(String, regex::Regex, Vec<String>)]) -> Value {
    let mut obj = serde_json::Map::new();
    for (campo, re, limpar) in compiled {
        if let Some(caps) = re.captures(text) {
            let val = caps.get(1).or_else(|| caps.get(0)).map(|m| m.as_str()).unwrap_or("");
            let mut v = val.trim().to_string();
            for s in limpar { v = v.replace(s.as_str(), ""); }
            obj.insert(campo.clone(), Value::String(v.trim().to_string()));
        }
    }
    Value::Object(obj)
}
/// Fase 5 — data dd/mm/aaaa ou aaaa-mm-dd (só forma, sem calendário; barato, sem regex).
fn valida_data(v: &str) -> bool {
    let b = v.as_bytes();
    if b.len() != 10 { return false; }
    let dig = |r: &[u8]| r.iter().all(|c| c.is_ascii_digit());
    (b[2] == b'/' && b[5] == b'/' && dig(&b[0..2]) && dig(&b[3..5]) && dig(&b[6..10]))
        || (b[4] == b'-' && b[7] == b'-' && dig(&b[0..4]) && dig(&b[5..7]) && dig(&b[8..10]))
}
/// Fase 5 — validador heurístico por NOME do campo (a PRECISÃO do NQI). Vazio nunca é válido; o
/// nome dá o tipo esperado (cnpj=14díg, cpf=11, data=forma, valor/preço=número), o resto é não-vazio.
fn valida_campo(nome: &str, valor: &str) -> bool {
    let v = valor.trim();
    if v.is_empty() { return false; }
    let n = nome.to_lowercase();
    let ndig = v.chars().filter(|c| c.is_ascii_digit()).count();
    if n.contains("cnpj") { return ndig == 14; }
    if n.contains("cpf") { return ndig == 11; }
    if n.contains("data") || n == "dt" || n.starts_with("dt_") || n.contains("_data") { return valida_data(v); }
    if n.contains("valor") || n.contains("preco") || n.contains("preço") || n.contains("total")
        || n.contains("estoque") || n.contains("qtd") || n.contains("quantidade") || n.contains("multa") {
        return v.chars().any(|c| c.is_ascii_digit())
            && v.chars().all(|c| c.is_ascii_digit() || ".,R$% -".contains(c));
    }
    true   // demais campos: não-vazio já é válido
}
/// Fase 5 — constrói o NQI (cobertura×precisão) + o path-tree AUTOCONTIDO de um registro. `origem`
/// mapeia campo→(via, detalhe) — pra CSV é ("coluna", "col N"); pra template é ("regex", o padrão).
/// O prov guarda por campo {valor, via, origem, válido} + a fonte + o molde — rastreável sem depender
/// do registry (o molde pode mudar depois). NQI agregável; o prov é a auditoria.
fn qualidade_prov(coll: &str, base: &str, modo: &str, molde: Option<(&str, u64)>,
                  origem: &std::collections::HashMap<String, (String, String)>,
                  rec: &Value, n_esperado: usize) -> (f64, String) {
    let obj = rec.as_object().cloned().unwrap_or_default();
    let mut campos = serde_json::Map::new();
    let (mut preench, mut validos) = (0usize, 0usize);
    for (campo, valor) in &obj {
        let v = valor.as_str().unwrap_or("");
        if !v.trim().is_empty() { preench += 1; }
        let valido = valida_campo(campo, v);
        if valido { validos += 1; }
        let (via, det) = origem.get(campo).cloned().unwrap_or_else(|| ("?".into(), String::new()));
        campos.insert(campo.clone(), json!({"valor": v, "via": via, "origem": det, "valido": valido}));
    }
    let cob = if n_esperado > 0 { preench as f64 / n_esperado as f64 } else { 0.0 };
    let prec = if preench > 0 { validos as f64 / preench as f64 } else { 0.0 };
    let nqi = cob * prec;
    let r2 = |x: f64| (x * 100.0).round() / 100.0;
    let mut prov = json!({
        "modo": modo, "fonte": {"coll": coll, "base": base},
        "cob": r2(cob), "prec": r2(prec), "campos": campos,
    });
    if let Some((t, ver)) = molde { prov["molde"] = json!({"tipo": t, "version": ver}); }
    (nqi, prov.to_string())
}
/// Janelas de extração por ORÇAMENTO DE CARACTERES (adaptativo): bases densas pegam menos linhas
/// por janela, estreitas pegam mais — dimensiona a SAÍDA do LLM pra não estourar o teto de tokens
/// (a causa da sub-extração medida em bases com muitos registros). Ignora linhas vazias (mesma
/// população que `tabular_spec` conta). Uma linha maior que o orçamento vira a própria janela.
fn extract_windows(text: &str, budget: usize) -> Vec<String> {
    let mut wins: Vec<String> = vec![];
    let mut cur: Vec<&str> = vec![];
    let mut chars = 0usize;
    for ln in text.lines().filter(|l| !l.trim().is_empty()) {
        if !cur.is_empty() && chars + ln.len() + 1 > budget {
            wins.push(cur.join("\n"));
            cur.clear(); chars = 0;
        }
        cur.push(ln);
        chars += ln.len() + 1;
    }
    if !cur.is_empty() { wins.push(cur.join("\n")); }
    wins
}
/// Texto INTEIRO de uma base via /chunk (id 0 + after gigante = todos os chunks). Truncar a
/// fonte é furo de completude — o normalizador lê tudo e janela depois, se precisar.
fn fetch_base_text(api: &str, coll: &str, name: &str) -> Option<String> {
    let req = json!({"collection": coll, "base": name, "id": 0, "after": 999_999}).to_string();
    let v: Value = serde_json::from_str(&http_post_t(&format!("{api}/chunk"), &req, 30)?).ok()?;
    let mut out = String::new();
    for c in v["chunks"].as_array()? {
        if let Some(t) = c["text"].as_str() { out.push_str(t); }
    }
    if out.trim().is_empty() { None } else { Some(out) }
}
/// Lista compacta do já-extraído (só arrays), pro prompt de verificação. Capada.
fn extracted_labels(acc: &Value) -> String {
    let mut out: Vec<String> = vec![];
    let mut chars = 0usize;
    let mut skipped = 0usize;
    if let Some(o) = acc.as_object() {
        for (k, v) in o {
            if let Some(arr) = v.as_array() {
                for it in arr {
                    let lbl = format!("{k}:{}", ent_label(it));
                    if out.len() >= 120 || chars > 3000 { skipped += 1; continue; }
                    chars += lbl.len() + 3;
                    out.push(lbl);
                }
            }
        }
    }
    if skipped > 0 { out.push(format!("(+{skipped} não listados)")); }
    if out.is_empty() { "(nada)".into() } else { out.join(" · ") }
}
/// Resgata um JSON cortado no teto de tokens: corta no último '}' e tenta fechar as
/// estruturas abertas. Transforma truncamento em CONTINUAÇÃO (o parcial entra no merge e a
/// passada seguinte pede o que falta) em vez de jogar a passada fora.
fn salvage_truncated_json(s: &str) -> Option<Value> {
    let a = s.find('{')?;
    let t = &s[a..];
    for (cut, _) in t.rmatch_indices('}').take(8) {
        let base = &t[..=cut];
        for close in ["", "]", "]}", "]}]", "]}}", "}}", "}]", "}]}"] {
            if let Ok(v) = serde_json::from_str::<Value>(&format!("{base}{close}")) {
                if v.is_object() { return Some(v); }
            }
        }
    }
    None
}
/// Uma chamada de extração. Err carrega o MOTIVO (timeout/não-JSON/cortado) — falha de LLM
/// nunca é None mudo. Ok devolve (content, truncado?).
fn llm_extract(llm_url: &str, sys: &str, user: &str, max_tokens: u64) -> Result<(Value, bool), String> {
    let body = json!({"messages":[{"role":"system","content":sys},{"role":"user","content":user}],
                      "temperature":0.2, "max_tokens":max_tokens}).to_string();
    // timeout: prompt-eval (~30 tok/s no pior caso medido sob contenção) + geração + folga
    let to = ((user.len() / 90) + (max_tokens as usize / 10) + 90).min(500) as u32;
    let resp = http_post_t(llm_url, &body, to).ok_or(format!("sem resposta (timeout {to}s)"))?;
    let rv: Value = serde_json::from_str(&resp).map_err(|_| format!("resposta não-JSON ({} bytes)", resp.len()))?;
    let content = match rv["choices"][0]["message"]["content"].as_str() {
        Some(c) => c.to_string(),
        None => return Err(format!("sem content (err={})", rv["error"].to_string().chars().take(120).collect::<String>())),
    };
    let truncated = rv["choices"][0]["finish_reason"].as_str() == Some("length");
    match extract_json_object(&content) {
        Some(v) => Ok((v, truncated)),
        None if truncated => match salvage_truncated_json(&content) {
            Some(v) => Ok((v, true)),   // parcial resgatado — a próxima passada continua dele
            None => Err("JSON cortado no teto de tokens (finish=length), irrecuperável".into()),
        },
        None => Err("não devolveu JSON válido".into()),
    }
}
/// Normaliza UMA base até PROVA de completude (ou esgotar as passadas — gate de escapatória).
/// Provas: "contagem" (tabular: array de OBJETOS ≥ linhas da fonte) · "sem-novidade" (uma
/// verificação que não acrescenta nada; anotada com o gap quando tabular) · "janelas:N".
/// `infra:true` no registro = falha de infraestrutura (ragd/LLM fora) — o chamador NÃO deve
/// gravar veredito: a base re-tenta no próximo ciclo.
fn normalize_base(api: &str, llm_url: &str, coll: &str, name: &str, sys: &str,
                  max_tokens: u64, state_hash: &str) -> (Value, Value) {
    let text = match fetch_base_text(api, coll, name) {
        Some(t) => t,
        None => return (json!({}), json!({"h": state_hash, "ok": false, "proof": "sem-texto",
                                          "passes": 0, "itens": 0, "linhas": null, "trunc": false, "infra": true})),
    };
    let rows = tabular_spec(&text).map(|(_, n)| n);
    let tab_hint = rows.map(|n| format!("\nDocumento TABULAR com {n} linha(s) de dados — o resultado deve cobrir TODAS as linhas."))
                       .unwrap_or_default();
    // janelas por linha até o orçamento — a fonte nunca é truncada
    let mut windows: Vec<String> = vec![];
    let mut cur = String::new();
    for line in text.lines() {
        if !cur.is_empty() && cur.len() + line.len() + 1 > SUMMARY_CHAR_BUDGET { windows.push(std::mem::take(&mut cur)); }
        cur.push_str(line);
        cur.push('\n');
    }
    if !cur.is_empty() { windows.push(cur); }

    let mut acc = json!({});
    let mut trunc_any = false;
    let mut calls = 0usize;

    if windows.len() > 1 {
        // base maior que uma janela: uma extração por janela + merge. O tab_hint global NÃO
        // entra (a contagem total confundiria cada janela parcial). Gate de escapatória:
        // 2 falhas LLM consecutivas abortam a base (infra) — nunca fica presa horas.
        let nw = windows.len();
        let mut fails = 0usize;
        let mut aborted = false;
        for (i, w) in windows.iter().enumerate() {
            let user = format!("COLEÇÃO: {coll} — documento \"{name}\" (janela {}/{nw} de um documento maior).\n\nCONTEÚDO:\n{w}", i + 1);
            calls += 1;
            match llm_extract(llm_url, sys, &user, max_tokens) {
                Ok((c, t)) => { fails = 0; trunc_any |= t; merge_content(&mut acc, &c); }
                Err(e) => {
                    fails += 1;
                    nlog(&format!("norm {coll}/{name}: janela {}/{nw} falhou ({e})", i + 1));
                    if fails >= 2 { aborted = true; break; }
                }
            }
        }
        // a contagem também vale aqui — a fonte inteira foi lida através das janelas
        let (ok, proof, infra) = if aborted {
            (false, format!("janelas:{nw} abortada (2 falhas LLM seguidas)"), true)
        } else if let (Some(r), Some((key, got))) = (rows, counting_array(&acc)) {
            if got >= r { (true, format!("contagem {key}:{got}≥{r}"), false) }
            else { (true, format!("janelas:{nw} ({key}:{got}/{r} linhas)"), false) }
        } else {
            (true, format!("janelas:{nw}"), false)
        };
        let rec = json!({"h": state_hash, "ok": ok, "proof": proof, "passes": calls,
                         "itens": array_len_sum(&acc), "linhas": rows, "trunc": trunc_any, "infra": infra});
        return (acc, rec);
    }

    let text1 = &windows[0];
    let mut ok = false;
    let mut proof = String::new();
    let mut fails = 0usize;
    let mut infra = false;
    while calls < REPAIR_MAX_PASSES {
        let user = if calls == 0 {
            format!("COLEÇÃO: {coll} — documento \"{name}\".{tab_hint}\n\nCONTEÚDO:\n{text1}")
        } else {
            let got_now = counting_array(&acc).map(|(_, n)| n).unwrap_or_else(|| max_array_len(&acc));
            let tab_gap = match rows {
                Some(r) if got_now < r => format!("\nA fonte tem {r} linha(s) de dados e o extraído cobre {got_now} item(ns) — verifique LINHA A LINHA o que falta; se a diferença é legítima (linhas fora do escopo do pedido), confirme devolvendo {{}}."),
                _ => String::new(),
            };
            format!("COLEÇÃO: {coll} — documento \"{name}\" (verificação {calls}).{tab_hint}{tab_gap}\n\
                     Já extraído (NÃO repita): {}\n\
                     Re-examine o documento e devolva APENAS o que FALTA, no MESMO formato JSON \
                     (mesmas chaves). Responda SOMENTE com JSON. Se não falta nada, devolva {{}}.\n\nCONTEÚDO:\n{text1}",
                    extracted_labels(&acc))
        };
        calls += 1;
        match llm_extract(llm_url, sys, &user, max_tokens) {
            Err(e) => {
                fails += 1;
                nlog(&format!("norm {coll}/{name}: passada {calls} falhou ({e})"));
                if fails >= 2 { proof = format!("llm-falhou: {e}"); infra = true; break; }
            }
            Ok((c, trunc)) => {
                fails = 0;
                trunc_any |= trunc;
                let before = grow_size(&acc);
                merge_content(&mut acc, &c);
                let after = grow_size(&acc);
                // gate CONTAGEM: só array de OBJETOS prova (strings não mapeiam linhas)
                if let (Some(r), Some((key, got))) = (rows, counting_array(&acc)) {
                    if got >= r { ok = true; proof = format!("contagem {key}:{got}≥{r}"); break; }
                }
                // uma VERIFICAÇÃO (não a 1ª passada) que não cresce nada e não truncou = completa;
                // em tabular sem contagem fechada, a proof carrega o gap (auditável na UI)
                if calls > 1 && after == before && !trunc {
                    ok = true;
                    proof = match rows {
                        Some(r) => {
                            let g = counting_array(&acc).map(|(_, n)| n).unwrap_or_else(|| max_array_len(&acc));
                            if g < r { format!("sem-novidade ({g}/{r} linhas)") } else { "sem-novidade".into() }
                        }
                        None => "sem-novidade".into(),
                    };
                    break;
                }
            }
        }
    }
    if !ok && proof.is_empty() { proof = format!("passadas-esgotadas ({REPAIR_MAX_PASSES})"); }
    let rec = json!({"h": state_hash, "ok": ok, "proof": proof, "passes": calls,
                     "itens": array_len_sum(&acc), "linhas": rows, "trunc": trunc_any, "infra": infra});
    (acc, rec)
}
/// Base precisa (re)processar? Nunca vista · fonte mudou (state_hash) · ou, sob `force`,
/// vista mas SEM prova de completude (o "regenerar" dá nova chance só às incompletas).
fn base_needs(rec: Option<&Value>, h: &str, force: bool) -> bool {
    match rec {
        Some(r) => r["h"].as_str() != Some(h) || (force && !r["ok"].as_bool().unwrap_or(false)),
        None => true,
    }
}

/// Nível 1 — o NORMALIZADOR. Transforma cada base em dado normalizado COMPLETO, base a base,
/// ciclo a ciclo, com PROVA de completude por base (contagem estrutural quando tabular;
/// passada-sem-novidade quando não). Sem heurística de nome: TODA base recebe extração
/// focada (diluir 60 documentos num prompt era o bug dos 7/15 fornecedores). O content da
/// coleção é DERIVADO de `bases_norm` por merge determinístico — a heurística (dedup,
/// enriquecimento de entidade) roda sobre o dado JÁ normalizado, nunca sobre a fonte crua;
/// melhorar o dedup no futuro re-deriva tudo sem custar 1 token de inferência.
/// Falha de INFRA (ragd/LLM fora) não grava veredito nem destrói dado bom: pula e re-tenta.
/// ⚠️ conteúdo do corpus sai da caixa (vai pro llm_url) — llm_url deve ser IA da frota.
fn mine_summary(api: &str, llm_url: &str, lib: &Value, level: u8, coll: &str, src: &str,
                prev: &Value, force: bool) -> Option<Value> {
    let (tname, sys, resolved_from, max_tokens) = resolve_template(lib, level, coll);
    let phash = hash_hex(&format!("v2|{sys}|mt={max_tokens}"));
    // RESET quando o prompt/teto mudam ("v2" no hash também descarta o formato stopgap antigo)
    let reset = prev["prompt_hash"].as_str() != Some(phash.as_str());
    let mut processed: serde_json::Map<String, Value> =
        if reset { serde_json::Map::new() } else { prev["processed"].as_object().cloned().unwrap_or_default() };
    processed.retain(|_, v| v.is_object());   // entrada legada (string) não tem prova → reprocessa
    let mut bases_norm: serde_json::Map<String, Value> =
        if reset { serde_json::Map::new() } else { prev["bases_norm"].as_object().cloned().unwrap_or_default() };

    let bases_resp: Value = serde_json::from_str(&http_get_t(&format!("{api}/bases?collection={coll}"), 30)?).ok()?;
    let bases = bases_resp["bases"].as_array()?.clone();
    let total = bases.len();

    // GC de fantasmas: base deletada/renomeada sai do checkpoint e do normalizado — senão
    // trava completude_provada pra sempre e dado de base morta polui o content derivado
    let current: std::collections::HashSet<&str> = bases.iter().filter_map(|b| b["name"].as_str()).collect();
    let before_gc = processed.len() + bases_norm.len();
    processed.retain(|k, _| current.contains(k.as_str()));
    bases_norm.retain(|k, _| current.contains(k.as_str()));
    let gc_removed = before_gc - (processed.len() + bases_norm.len());
    if gc_removed > 0 { nlog(&format!("summary {coll}: GC removeu {gc_removed} registro(s) de base(s) que não existem mais")); }

    // fila determinística por nome — SEM prioridade heurística (doutrina: completude p/ todas)
    let mut queue: Vec<(String, String)> = bases.iter().filter_map(|b| {
        let name = b["name"].as_str().filter(|n| !n.is_empty())?.to_string();
        let h = base_state_hash(b);
        if base_needs(processed.get(&name), &h, force) { Some((name, h)) } else { None }
    }).collect();
    queue.sort();
    if queue.is_empty() && gc_removed == 0 { return None; }   // nada novo neste ciclo — acumulado mantido

    let mut inserted = 0usize;
    let batch_n = queue.len().min(BASES_PER_CYCLE);
    for (name, h) in queue.iter().take(BASES_PER_CYCLE) {
        let (content, rec) = normalize_base(api, llm_url, coll, name, &sys, max_tokens, h);
        if rec["infra"].as_bool().unwrap_or(false) {
            // infra fora do ar: sem veredito, sem sobrescrever dado bom — re-tenta no próximo
            // ciclo; e aborta o lote (insistir agora só queima timeout atrás de timeout)
            nlog(&format!("norm {coll}/{name}: infra ({}) — lote abortado, re-tenta no próximo ciclo",
                rec["proof"].as_str().unwrap_or("?")));
            break;
        }
        let status = if rec["ok"].as_bool().unwrap_or(false) {
            format!("COMPLETA ({})", rec["proof"].as_str().unwrap_or("?"))
        } else {
            format!("INCOMPLETA ({})", rec["proof"].as_str().unwrap_or("?"))
        };
        nlog(&format!("norm {coll}/{name}: {} passada(s) · {} item(ns) · {status}", rec["passes"], rec["itens"]));
        bases_norm.insert(name.clone(), content);
        processed.insert(name.clone(), rec);
        inserted += 1;
    }
    if inserted == 0 && gc_removed == 0 { return None; }   // lote todo abortado por infra — nada mudou

    // content DERIVADO do normalizado, do zero, em ordem de nome — determinístico e re-derivável
    let mut content = json!({});
    let mut names: Vec<String> = bases_norm.keys().cloned().collect();
    names.sort();
    for n in &names {
        if let Some(c) = bases_norm.get(n.as_str()) { merge_content(&mut content, c); }
    }

    let remaining = bases.iter().filter(|b| {
        b["name"].as_str().filter(|n| !n.is_empty())
            .map(|n| base_needs(processed.get(n), &base_state_hash(b), false))
            .unwrap_or(false)
    }).count();
    let cobertura_completa = remaining == 0;                 // todas vistas com a fonte atual
    let completude_provada = cobertura_completa &&           // ... e cada uma com PROVA
        processed.values().all(|r| r["ok"].as_bool().unwrap_or(false));
    let incompletas: Vec<String> = processed.iter()
        .filter(|(_, r)| !r["ok"].as_bool().unwrap_or(false))
        .map(|(k, _)| k.clone()).take(60).collect();
    // truncamento é estado PERSISTENTE do acumulado (qualquer base com passada cortada), não do último lote
    let truncated_any = processed.values().any(|r| r["trunc"].as_bool().unwrap_or(false));
    let done = processed.len();
    nlog(&format!("summary {coll}: lote {batch_n} · {done}/{total} base(s) · {} item(ns) · {}{}",
        array_len_sum(&content),
        if completude_provada { "COMPLETUDE PROVADA ✓" }
        else if cobertura_completa { "coberta — prova pendente" } else { "varrendo" },
        if incompletas.is_empty() { String::new() } else { format!(" · {} incompleta(s)", incompletas.len()) }));

    Some(json!({
        "type": "Summary", "level": 1, "updated": now_stamp(), "source_hash": src, "prompt_hash": phash,
        "truncated": truncated_any,
        "cobertura_completa": cobertura_completa, "completude_provada": completude_provada,
        "processed": Value::Object(processed), "bases_norm": Value::Object(bases_norm),
        "provenance": {"via": "level1/normalizador", "template": tname, "resolved_from": resolved_from,
                       "prompt_hash": phash, "max_tokens": max_tokens,
                       "processadas": done, "total_bases": total,
                       "cobertura_completa": cobertura_completa, "completude_provada": completude_provada,
                       "incompletas": incompletas, "batch": batch_n, "at": now_stamp()},
        "content": content
    }))
}

/// Cadeado de ciclo: impede que worker e /run rodem ao mesmo tempo (ou dois /run). Devolve
/// true se ESTE chamador pegou o cadeado; false se já havia um ciclo. Pareado com end_cycle.
fn try_start_cycle(st: &Arc<Mutex<State>>) -> bool {
    let mut s = st.lock().unwrap();
    if s.cycle_running { false } else { s.cycle_running = true; true }
}
fn end_cycle(st: &Arc<Mutex<State>>) { if let Ok(mut s) = st.lock() { s.cycle_running = false; } }

/// Roda UM ciclo. `force=true` (/run manual) re-minera sempre; `force=false` (cadência do
/// worker) pula coleção sem mudança (source_hash igual). NÃO segura o lock durante HTTP/IO.
/// System do classificador: template "classificador" da biblioteca (editável no ValHalla) ou o
/// BUILTIN. Devolve (system, resolved_from).
fn classify_system(lib: &Value) -> (String, String) {
    match lib["templates"]["classificador"]["system"].as_str() {
        Some(s) if !s.trim().is_empty() => (s.to_string(), "template".into()),
        _ => (BUILTIN_CLASSIFY_PROMPT.to_string(), "builtin".into()),
    }
}
/// System do extrator (Fase 2): template "extrator" editável ou o BUILTIN. `{tipo}` é substituído
/// pela classe no momento da chamada.
fn extract_system(lib: &Value) -> (String, String) {
    match lib["templates"]["extrator"]["system"].as_str() {
        Some(s) if !s.trim().is_empty() => (s.to_string(), "template".into()),
        _ => (BUILTIN_EXTRACT_PROMPT.to_string(), "builtin".into()),
    }
}
/// System do criador de MOLDE (Fase 3): template "modelador" editável ou o BUILTIN.
fn template_system(lib: &Value) -> (String, String) {
    match lib["templates"]["modelador"]["system"].as_str() {
        Some(s) if !s.trim().is_empty() => (s.to_string(), "template".into()),
        _ => (BUILTIN_TEMPLATE_PROMPT.to_string(), "builtin".into()),
    }
}
/// Fase 3 — o L1 cria o molde de um tipo a partir de UMA amostra. Structured output força
/// `{schema:[...], regras:[{campo,regex,limpar}]}`. Devolve (schema_json, regras_json) como strings.
fn llm_make_template(llm_url: &str, sys: &str, tipo: &str, amostra: &str) -> Result<(String, String), String> {
    let schema = json!({
        "type": "object",
        "properties": {
            "schema": {"type": "array", "items": {"type": "string"}},
            "regras": {"type": "array", "items": {"type": "object", "properties": {
                "campo": {"type": "string"}, "regex": {"type": "string"},
                "limpar": {"type": "array", "items": {"type": "string"}}
            }, "required": ["campo", "regex"]}}
        },
        "required": ["schema", "regras"]
    });
    let body = json!({
        "messages": [
            {"role": "system", "content": sys},
            {"role": "user", "content": format!("TIPO: {tipo}\nDOCUMENTO EXEMPLO:\n{amostra}")}
        ],
        "temperature": 0, "max_tokens": 1200,
        "response_format": {"type": "json_schema", "json_schema": {"schema": schema}}
    }).to_string();
    let resp = http_post_t(llm_url, &body, 180).ok_or_else(|| "sem resposta (template)".to_string())?;
    let rv: Value = serde_json::from_str(&resp).map_err(|_| format!("resposta não-JSON ({} bytes)", resp.len()))?;
    let content = rv["choices"][0]["message"]["content"].as_str()
        .ok_or_else(|| format!("sem content (err={})", rv["error"].to_string().chars().take(120).collect::<String>()))?;
    let obj = extract_json_object(content).ok_or_else(|| "molde não é JSON válido".to_string())?;
    let regras = obj["regras"].clone();
    if !regras.is_array() || regras.as_array().map(|a| a.is_empty()).unwrap_or(true) {
        return Err("molde sem regras".into());
    }
    Ok((obj["schema"].to_string(), regras.to_string()))
}
/// Fase 3 — cria moldes pros tipos NÃO-CSV que ainda não têm template (1 tipo por ciclo, o mais
/// populoso primeiro = mais valor). Pega 1 amostra, o L1 cria o molde, valida a cobertura aplicando
/// nela mesma, e grava no registry. O L0 (mine_entities) aplica depois aos N documentos.
fn mine_templates(api: &str, llm_url: &str, ch_url: &str, lib: &Value, coll: &str) -> Value {
    if let Err(e) = chdb::ensure_template_schema(ch_url) {
        return json!({"ok": false, "collection": coll, "error": format!("schema template: {e}")});
    }
    let templates = chdb::get_templates(ch_url).unwrap_or_else(|_| json!({}));
    let bases: Vec<Value> = chdb::classes_summary(ch_url, Some(coll)).ok()
        .and_then(|v| v["bases"].as_array().cloned()).unwrap_or_default();
    // tipos NÃO-CSV, sem molde ainda → escolhe o mais populoso (name = 1 amostra por tipo)
    let mut por_tipo: std::collections::HashMap<String, (usize, String)> = std::collections::HashMap::new();
    for b in &bases {
        let is_csv = b["csv"].as_i64() == Some(1) || b["csv"].as_u64() == Some(1);
        let natureza = b["natureza"].as_str().unwrap_or("");
        let tipo = b["tipo"].as_str().unwrap_or("");
        let name = b["name"].as_str().unwrap_or("");
        // SÓ natureza=documento (átomos da ponta ESTRUTURADOS: comprovante/nota/holerite/OC). Narrativo
        // (contrato, memorial) e código NÃO têm campos rotulados → molde regex não serve (viraria lixo).
        if is_csv || natureza != "documento" || tipo.is_empty() || tipo == "sem-texto" || name.is_empty() { continue; }
        if templates.get(tipo).is_some() { continue; }   // já tem molde
        let e = por_tipo.entry(tipo.to_string()).or_insert((0, name.to_string()));
        e.0 += 1;
    }
    let (tipo, count, aname) = match por_tipo.into_iter().max_by_key(|(_, (c, _))| *c) {
        Some((t, (c, n))) => (t, c, n),
        None => return json!({"ok": true, "collection": coll, "criados": 0, "note": "tipos não-CSV já têm molde (ou não há)"}),
    };
    let amostra = match fetch_base_text(api, coll, &aname) { Some(t) => t, None => return json!({"ok": false, "collection": coll, "error": "sem amostra"}) };
    let (sys, _from) = template_system(lib);
    let (schema, regras) = match llm_make_template(llm_url, &sys, &tipo, &amostra) {
        Ok(x) => x,
        Err(e) => { nlog(&format!("template {coll}/{tipo}: {e}")); return json!({"ok": false, "collection": coll, "tipo": tipo, "error": e}); }
    };
    // valida cobertura aplicando o molde na própria amostra (% de campos que casaram)
    let regras_v: Value = serde_json::from_str(&regras).unwrap_or_else(|_| json!([]));
    let compiled = compile_template(&regras_v);
    let rec = apply_template(&amostra, &compiled);
    let n_campos = regras_v.as_array().map(|a| a.len()).unwrap_or(0);
    let n_ok = rec.as_object().map(|o| o.values().filter(|v| !v.as_str().unwrap_or("").is_empty()).count()).unwrap_or(0);
    let cobertura = if n_campos > 0 { n_ok as f64 / n_campos as f64 } else { 0.0 };
    // GATE de cobertura: só grava se o molde CASA a maioria dos campos na amostra. Cobertura baixa =
    // documento sem estrutura extraível (o regex não ancorou) → NÃO vira molde (nem lixo no dump).
    if cobertura < TEMPLATE_MIN_COVERAGE {
        nlog(&format!("molde REJEITADO: tipo={tipo} cobertura={:.0}% < {:.0}% (sem estrutura extraível)",
            cobertura * 100.0, TEMPLATE_MIN_COVERAGE * 100.0));
        return json!({"ok": true, "collection": coll, "criados": 0, "tipo": tipo,
                      "cobertura": cobertura, "rejeitado": "cobertura baixa"});
    }
    let row = chdb::TemplateRow {
        tipo: tipo.clone(), schema, regras, cobertura, origem: "llm".into(),
        created_at: now_stamp(), version: chdb::now_version(),
    };
    if let Err(e) = chdb::upsert_template(ch_url, &row) {
        return json!({"ok": false, "collection": coll, "tipo": tipo, "error": format!("upsert: {e}")});
    }
    nlog(&format!("molde criado: tipo={tipo} campos={n_campos} cobertura={:.0}% ({count} exemplares aguardando)", cobertura * 100.0));
    json!({"ok": true, "collection": coll, "criados": 1, "tipo": tipo, "campos": n_campos, "cobertura": cobertura})
}

/// Garante que os templates "classificador" e "extrator" existem na biblioteca (pra aparecerem no
/// editor do ValHalla). Idempotente: só grava o que faltava.
fn ensure_prompt_templates(dir: &str) {
    let mut lib = read_prompts(dir);
    let mut changed = false;
    if !lib["templates"]["classificador"].is_object() {
        lib["templates"]["classificador"] = json!({
            "description": "Classifica {natureza, tipo} na entrada (Fase 1). Os tipos vêm da lista editável de doctypes.",
            "system": BUILTIN_CLASSIFY_PROMPT, "updated": now_stamp() });
        changed = true;
    }
    if !lib["templates"]["extrator"].is_object() {
        lib["templates"]["extrator"] = json!({
            "description": "Extrai os registros das bases tabulares como array JSON (Fase 2). {tipo} = a classe.",
            "system": BUILTIN_EXTRACT_PROMPT, "updated": now_stamp() });
        changed = true;
    }
    if changed { write_prompts(dir, &lib); }
}

/// Texto do chunk 0 de uma base (primeiros CLASSIFY_MAX_CHARS chars). Leve — não puxa a base
/// inteira como o normalizador. None se a base não tem texto.
fn fetch_chunk0(api: &str, coll: &str, name: &str) -> Option<String> {
    let req = json!({"collection": coll, "base": name, "id": 0}).to_string();
    let v: Value = serde_json::from_str(&http_post_t(&format!("{api}/chunk"), &req, 20)?).ok()?;
    let t = v["chunks"][0]["text"].as_str()?;
    if t.trim().is_empty() { return None; }
    Some(t.chars().take(CLASSIFY_MAX_CHARS).collect())
}

/// Uma classificação por LLM com CONSTRAINED DECODING (json_schema/enum). temperature 0.
/// Err carrega o motivo. Devolve (natureza, tipo).
fn llm_classify(llm_url: &str, sys: &str, text: &str, naturezas: &[String], tipos: &[String])
    -> Result<(String, String), String>
{
    let schema = json!({
        "type": "object",
        "properties": {
            "natureza": {"type": "string", "enum": naturezas},
            "tipo": {"type": "string", "enum": tipos}
        },
        "required": ["natureza", "tipo"]
    });
    let body = json!({
        "messages": [
            {"role": "system", "content": sys},
            {"role": "user", "content": format!("DOCUMENTO:\n{text}")}
        ],
        "temperature": 0, "max_tokens": 40,
        "response_format": {"type": "json_schema", "json_schema": {"schema": schema}}
    }).to_string();
    let resp = http_post_t(llm_url, &body, CLASSIFY_TIMEOUT_S)
        .ok_or(format!("sem resposta (timeout {CLASSIFY_TIMEOUT_S}s)"))?;
    let rv: Value = serde_json::from_str(&resp).map_err(|_| format!("resposta não-JSON ({} bytes)", resp.len()))?;
    let content = rv["choices"][0]["message"]["content"].as_str()
        .ok_or_else(|| format!("sem content (err={})", rv["error"].to_string().chars().take(120).collect::<String>()))?;
    let obj = extract_json_object(content).ok_or_else(|| "não devolveu JSON válido".to_string())?;
    let nat = obj["natureza"].as_str().unwrap_or("").to_string();
    let tip = obj["tipo"].as_str().unwrap_or("").to_string();
    if nat.is_empty() || tip.is_empty() { return Err(format!("classe incompleta: {obj}")); }
    Ok((nat, tip))
}

/// Classifica UMA base: chunk 0 + LLM constrangido. Devolve `(natureza_llm, tipo, csv)`, onde
/// `csv` = reconhecedor determinístico (tabular_spec sobre o chunk 0 — o delimitador consistente
/// já aparece no início). O chamador IGNORA `natureza_llm` e deriva a natureza do tipo/csv.
/// Err("sem-texto") se a base não tem texto.
fn classify_base(api: &str, llm_url: &str, sys: &str, coll: &str, name: &str,
                 naturezas: &[String], tipos: &[String]) -> Result<(String, String, bool), String> {
    let text = fetch_chunk0(api, coll, name).ok_or_else(|| "sem-texto".to_string())?;
    let csv = tabular_spec(&text).is_some();
    let (nat, tip) = llm_classify(llm_url, sys, &text, naturezas, tipos)?;
    Ok((nat, tip, csv))
}

// ── dispatch do STORE do acumulado: ClickHouse (default) ou SQLite (rollback via cfg store=sqlite) ──
// ClickHouse é o caminho ativo (decidido 09/ago). O SQLite fica como rede de segurança — trocar é
// uma linha no nidhogg.cfg (store=sqlite), sem reverter binário.
fn store_ensure(store: &str, dir: &str, ch_url: &str) {
    if store == "clickhouse" {
        match chdb::ensure_schema(ch_url) {
            Ok(_) => println!("   🗄  store = ClickHouse ({ch_url}) — db nidhogg pronto"),
            Err(e) => eprintln!("   ⚠ ClickHouse ensure falhou: {e} — classificação indisponível"),
        }
        if let Err(e) = chdb::ensure_entidade_schema(ch_url) { eprintln!("   ⚠ entidade (Fase 2) schema falhou: {e}"); }
    } else {
        match db::open(dir) {
            Ok(_) => println!("   🗄  store = SQLite ({:?})", db::db_path(dir)),
            Err(e) => eprintln!("   ⚠ SQLite falhou: {e}"),
        }
    }
}
fn store_doctypes(store: &str, dir: &str, ch_url: &str) -> (Vec<String>, Vec<String>) {
    if store == "clickhouse" { chdb::doctypes(ch_url).unwrap_or_default() }
    else { db::open(dir).and_then(|c| db::doctypes(&c)).unwrap_or_default() }
}
/// Hash da CONFIG de vocabulário — determinístico das listas (bate nos dois backends).
fn store_doctypes_hash(store: &str, dir: &str, ch_url: &str) -> String {
    let (nat, tip) = store_doctypes(store, dir, ch_url);
    hash_hex(&format!("nat:{}|tip:{}", nat.join(","), tip.join(",")))
}
fn store_needs_class(store: &str, dir: &str, ch_url: &str, coll: &str, name: &str, sh: &str, cfg: &str) -> bool {
    if store == "clickhouse" { chdb::needs_class(ch_url, coll, name, sh, cfg).unwrap_or(true) }
    else { db::open(dir).and_then(|c| db::needs_class(&c, coll, name, sh, cfg)).unwrap_or(true) }
}
fn store_insert_classes(store: &str, dir: &str, ch_url: &str, rows: &[chdb::ClassRow]) -> Result<(), String> {
    if rows.is_empty() { return Ok(()); }
    if store == "clickhouse" { chdb::insert_classes(ch_url, rows) }
    else {
        let conn = db::open(dir).map_err(|e| e.to_string())?;
        for r in rows {
            db::upsert_class(&conn, &r.collection, &r.name, &r.state_hash, &r.cfg_hash,
                             &r.natureza, &r.tipo, r.confianca, &r.classified_at).map_err(|e| e.to_string())?;
        }
        Ok(())
    }
}
fn store_write_doctypes(store: &str, dir: &str, ch_url: &str, nat: &[String], tip: &[String]) -> Result<(), String> {
    if store == "clickhouse" { chdb::write_doctypes(ch_url, nat, tip) }
    else { let conn = db::open(dir).map_err(|e| e.to_string())?; db::write_doctypes(&conn, nat, tip).map_err(|e| e.to_string()) }
}
fn store_classes_summary(store: &str, dir: &str, ch_url: &str, coll: Option<&str>) -> Result<Value, String> {
    if store == "clickhouse" { chdb::classes_summary(ch_url, coll) }
    else { let conn = db::open(dir).map_err(|e| e.to_string())?; db::classes_summary(&conn, coll).map_err(|e| e.to_string()) }
}

/// Ciclo de classificação de UMA coleção (Fase 1). Reconcilia /bases do ragd com o STORE:
/// classifica só as bases NOVAS/mudadas (state_hash) ou afetadas por edição de vocabulário/prompt
/// (cfg_hash). CLASSIFY_PER_CYCLE por ciclo; aborta em 2 falhas de LLM seguidas. As classes vão num
/// ÚNICO INSERT em lote no fim (ClickHouse detesta inserts unitários). O corpus fica no ragd.
fn mine_classes(api: &str, llm_url: &str, store: &str, dir: &str, ch_url: &str, lib: &Value, coll: &str, force: bool) -> Value {
    let (naturezas, tipos) = store_doctypes(store, dir, ch_url);
    if naturezas.is_empty() || tipos.is_empty() {
        return json!({"ok": false, "error": "doctypes vazios"});
    }
    let (sys, _from) = classify_system(lib);
    // checkpoint = doctypes + prompt. Editar a lista OU o prompt reclassifica; state_hash cobre o
    // corpus. `force` re-minera L0/Summary, mas a classificação SEGUE o checkpoint (não refaz as
    // provadas). Prune de fantasmas PULADO de propósito (mutation é caro no CH; ghosts são inócuos).
    let _ = force;
    let cfg_hash = hash_hex(&format!("{}|{}", store_doctypes_hash(store, dir, ch_url), sys));

    let bases: Vec<Value> = match http_get_t(&format!("{api}/bases?collection={coll}"), 30)
        .and_then(|s| serde_json::from_str::<Value>(&s).ok())
    {
        Some(v) => v["bases"].as_array().cloned().unwrap_or_default(),
        None => return json!({"ok": false, "error": "ragd /bases sem resposta"}),
    };
    let mut queue: Vec<Value> = bases.into_iter().filter(|b| {
        let name = b["name"].as_str().unwrap_or("");
        !name.is_empty() && store_needs_class(store, dir, ch_url, coll, name, &base_state_hash(b), &cfg_hash)
    }).collect();
    queue.sort_by(|a, b| a["name"].as_str().unwrap_or("").cmp(b["name"].as_str().unwrap_or("")));

    let pending_before = queue.len();
    let (mut classified, mut no_text, mut fails_total) = (0usize, 0usize, 0usize);
    let mut consecutive_fails = 0usize;
    let at = now_stamp();
    let mut rows: Vec<chdb::ClassRow> = vec![];   // acumula o lote (1 INSERT no fim)
    for b in queue.iter().take(CLASSIFY_PER_CYCLE) {
        let name = b["name"].as_str().unwrap_or("");
        let sh = base_state_hash(b);
        let has_text = b["has_text"].as_bool().unwrap_or(true);
        let mkrow = |nat: &str, tip: &str, csv: bool, conf: f64| chdb::ClassRow {
            collection: coll.to_string(), name: name.to_string(), state_hash: sh.clone(),
            cfg_hash: cfg_hash.clone(), natureza: nat.to_string(), tipo: tip.to_string(),
            csv, confianca: conf, classified_at: at.clone(), version: chdb::now_version(),
        };
        if !has_text {
            rows.push(mkrow("?", "sem-texto", false, 0.0)); no_text += 1; continue;
        }
        match classify_base(api, llm_url, &sys, coll, name, &naturezas, &tipos) {
            // a natureza do LLM é IGNORADA — derivamos do tipo (89,5%) + csv (determinístico).
            // um CSV regular é sempre `tabela`; o resto cai por gravidade do tipo.
            Ok((_nat_llm, tip, csv)) => {
                let nat = if csv { "tabela" } else { natureza_do_tipo(&tip) };
                rows.push(mkrow(nat, &tip, csv, 1.0)); classified += 1; consecutive_fails = 0;
            }
            Err(e) if e == "sem-texto" => { rows.push(mkrow("?", "sem-texto", false, 0.0)); no_text += 1; consecutive_fails = 0; }
            Err(e) => {
                nlog(&format!("classify {coll}/{name} falhou: {e}"));
                fails_total += 1; consecutive_fails += 1;
                if consecutive_fails >= CLASSIFY_MAX_FAILS {
                    nlog(&format!("classify: {CLASSIFY_MAX_FAILS} falhas seguidas em {coll} — aborta lote"));
                    break;
                }
            }
        }
    }
    if let Err(e) = store_insert_classes(store, dir, ch_url, &rows) {
        nlog(&format!("classify {coll}: gravação no store falhou: {e}"));
        return json!({"ok": false, "collection": coll, "error": format!("store: {e}")});
    }
    let pending = pending_before.saturating_sub(classified + no_text);
    json!({"ok": true, "collection": coll, "classified": classified,
           "no_text": no_text, "fails": fails_total, "pending": pending})
}

/// Modo CLI de teste (portão de paridade): classifica uma LISTA de bases e imprime JSONL em
/// stdout, sem daemon e sem Summary. Reusa exatamente as funções da Fase 1 — é o que confere o
/// port Rust contra o baseline Python. Aceita `[["coll","base"],...]` ou `[{"coll","base"},...]`.
fn classify_list_cli(cfg: &Config, path: &str) {
    store_ensure(&cfg.store, &cfg.dir, &cfg.ch_url);
    let (naturezas, tipos) = store_doctypes(&cfg.store, &cfg.dir, &cfg.ch_url);
    let lib = read_prompts(&cfg.dir);
    let (sys, from) = classify_system(&lib);
    let arr: Value = serde_json::from_str(&std::fs::read_to_string(path).expect("ler lista")).expect("json");
    let items = arr.as_array().cloned().unwrap_or_default();
    eprintln!("classify-list: {} itens · prompt={from} · {} naturezas · {} tipos · llm={}",
              items.len(), naturezas.len(), tipos.len(), cfg.llm_url);
    for item in items {
        let (coll, base) = if let Some(pair) = item.as_array() {
            (pair.first().and_then(|v| v.as_str()).unwrap_or("").to_string(),
             pair.get(1).and_then(|v| v.as_str()).unwrap_or("").to_string())
        } else {
            (item["coll"].as_str().unwrap_or("").to_string(),
             item["base"].as_str().unwrap_or("").to_string())
        };
        match classify_base(&cfg.ragd_api, &cfg.llm_url, &sys, &coll, &base, &naturezas, &tipos) {
            Ok((nat_llm, tip, csv)) => {
                let nat = if csv { "tabela".to_string() } else { natureza_do_tipo(&tip).to_string() };
                println!("{}", json!({"coll": coll, "base": base, "natureza": nat, "tipo": tip, "csv": csv, "nat_llm": nat_llm}));
            }
            Err(e) => println!("{}", json!({"coll": coll, "base": base, "natureza": "?", "tipo": format!("ERR:{e}")})),
        }
    }
}

/// Extrai os registros de UMA janela como array JSON. temperature 0. Reusa o salvage pra truncado.
/// O chamador valida cada elemento (all-or-nothing).
fn llm_extract_records(llm_url: &str, sys: &str, tipo: &str, text: &str) -> Result<Vec<Value>, String> {
    let sys_r = sys.replace("{tipo}", tipo);
    let body = json!({
        "messages": [{"role":"system","content":sys_r},{"role":"user","content":format!("DOCUMENTO:\n{text}")}],
        "temperature": 0, "max_tokens": EXTRACT_MAX_TOKENS
    }).to_string();
    let to = ((text.len() / 90) + (EXTRACT_MAX_TOKENS as usize / 10) + 90).min(400) as u32;
    let resp = http_post_t(llm_url, &body, to).ok_or(format!("sem resposta (timeout {to}s)"))?;
    let rv: Value = serde_json::from_str(&resp).map_err(|_| format!("resposta não-JSON ({} bytes)", resp.len()))?;
    let truncated = rv["choices"][0]["finish_reason"].as_str() == Some("length");
    let content = rv["choices"][0]["message"]["content"].as_str()
        .ok_or_else(|| format!("sem content (err={})", rv["error"].to_string().chars().take(100).collect::<String>()))?;
    extract_json_array(content)
        .or_else(|| if truncated { salvage_truncated_array(content) } else { None })
        .ok_or_else(|| if truncated { "array cortado no teto (finish=length)".to_string() } else { "não devolveu array JSON".to_string() })
}

/// Ciclo de EXTRAÇÃO (Fase 2) de UMA coleção — só ClickHouse. Extrai os registros das bases
/// TABULARES (natureza vem do doc_class) como array JSON, incremental por base (janela por Nº de
/// linhas → saída bounded), ALL-OR-NOTHING (janela falha ⇒ descarta a base), 1 INSERT em lote por
/// base (mesmo version). O acumulado vira o dump denso; a completude é COUNT ≥ linhas da fonte.
fn mine_entities(api: &str, store: &str, ch_url: &str, lib: &Value, coll: &str) -> Value {
    if store != "clickhouse" {
        return json!({"ok": false, "collection": coll, "error": "extração requer clickhouse"});
    }
    if let Err(e) = chdb::ensure_entidade_schema(ch_url) {
        return json!({"ok": false, "collection": coll, "error": format!("schema: {e}")});
    }
    if let Err(e) = chdb::ensure_template_schema(ch_url) {
        return json!({"ok": false, "collection": coll, "error": format!("schema template: {e}")});
    }
    let (sys, _from) = extract_system(lib);
    let ext_cfg_csv = hash_hex(&format!("extrator|v4nqi|{sys}|win{}|tok{}",
        EXTRACT_INPUT_CHARS_PER_WINDOW, EXTRACT_MAX_TOKENS));
    let templates = chdb::get_templates(ch_url).unwrap_or_else(|_| json!({}));   // registry (Fase 3)

    // O GATE: extrai só quem tem COMO extrair DETERMINISTICAMENTE —
    //  · csv=1 → parse_tabular (cabeçalho = schema)
    //  · csv=0 mas o TIPO tem molde no registry → apply_template (regex ancorado, 1 doc = 1 registro)
    // A natureza do LLM não manda; o determinístico sim. `ext_cfg` é POR-BASE: CSV inclui a config do
    // parser; template inclui o HASH DO MOLDE do tipo — mudar um molde re-extrai só o tipo dele.
    let bases_class: Vec<Value> = chdb::classes_summary(ch_url, Some(coll)).ok()
        .and_then(|v| v["bases"].as_array().cloned()).unwrap_or_default();
    let is_csv = |b: &Value| b["csv"].as_i64() == Some(1) || b["csv"].as_u64() == Some(1) || b["csv"].as_str() == Some("1");
    let mut extraiveis: std::collections::HashMap<String, (String, bool, String)> = std::collections::HashMap::new();
    for b in &bases_class {
        let name = match b["name"].as_str() { Some(n) if !n.is_empty() => n.to_string(), _ => continue };
        let tipo = b["tipo"].as_str().unwrap_or("registro").to_string();
        if is_csv(b) {
            extraiveis.insert(name, (tipo, true, ext_cfg_csv.clone()));
        } else if let Some(t) = templates.get(tipo.as_str()) {
            let ecfg = hash_hex(&format!("template|v2nqi|{tipo}|{}", hash_hex(&t["regras"].to_string())));
            extraiveis.insert(name, (tipo, false, ecfg));
        }
    }
    // ponto cego: natureza=tabela sem csv E sem molde → ninguém extrai (VISÍVEL, não silencioso)
    let blind = bases_class.iter()
        .filter(|b| b["natureza"].as_str() == Some("tabela") && !is_csv(b)
                && !templates.get(b["tipo"].as_str().unwrap_or("")).is_some()).count();
    if blind > 0 { nlog(&format!("extract {coll}: {blind} base(s) natureza=tabela sem csv nem molde — não extraídas")); }
    if extraiveis.is_empty() {
        return json!({"ok": true, "collection": coll, "extracted": 0, "pending": 0, "note": "nada extraível (sem CSV nem molde)"});
    }

    let bases: Vec<Value> = match http_get_t(&format!("{api}/bases?collection={coll}"), 30)
        .and_then(|s| serde_json::from_str::<Value>(&s).ok()) {
        Some(v) => v["bases"].as_array().cloned().unwrap_or_default(),
        None => return json!({"ok": false, "collection": coll, "error": "ragd /bases sem resposta"}),
    };
    let mut queue: Vec<Value> = bases.into_iter().filter(|b| {
        let name = b["name"].as_str().unwrap_or("");
        match extraiveis.get(name) {
            Some((_, _, ecfg)) => chdb::needs_extract(ch_url, coll, name, &base_state_hash(b), ecfg).unwrap_or(true),
            None => false,
        }
    }).collect();
    queue.sort_by(|a, b| a["name"].as_str().unwrap_or("").cmp(b["name"].as_str().unwrap_or("")));

    let pending_before = queue.len();
    let (mut extracted, mut incompletas, mut fails, mut det_bases, mut tpl_bases) = (0usize, 0usize, 0usize, 0usize, 0usize);
    let mut compiled_cache: std::collections::HashMap<String, Vec<(String, regex::Regex, Vec<String>)>> = std::collections::HashMap::new();
    let at = now_stamp();
    for b in queue.iter().take(EXTRACT_PER_CYCLE) {
        let name = b["name"].as_str().unwrap_or("");
        let sh = base_state_hash(b);
        let (tipo, _bcsv, ecfg) = match extraiveis.get(name) { Some(x) => x.clone(), None => continue };
        let text = match fetch_base_text(api, coll, name) { Some(t) => t, None => continue };
        let spec = tabular_spec(&text);
        let rows_src = spec.map(|(_, n)| n);
        let version = chdb::now_version();
        let mut ents: Vec<chdb::EntidadeRow> = vec![];
        let modo: &str;
        if let Some((delim, _)) = spec {
            // CSV → determinístico: cabeçalho = schema, cada linha = registro.
            modo = "det";
            let (header, registros) = parse_tabular(&text, delim);
            // path-tree (Fase 5): cada campo veio de uma COLUNA do cabeçalho
            let origem: std::collections::HashMap<String, (String, String)> = header.iter().enumerate()
                .map(|(i, c)| (c.clone(), ("coluna".to_string(), format!("col {i}")))).collect();
            let mut idx: u32 = 0;
            for rec in &registros {
                let (nqi, prov) = qualidade_prov(coll, name, "det", None, &origem, rec, header.len());
                ents.push(chdb::EntidadeRow {
                    collection: coll.to_string(), base: name.to_string(), tipo: tipo.clone(),
                    idx, dado: rec.to_string(), modo: "det".to_string(), nqi, prov,
                    state_hash: sh.clone(), ext_cfg_hash: ecfg.clone(), version, extracted_at: at.clone(),
                });
                idx += 1;
            }
        } else {
            // Fase 3: aplica o MOLDE do tipo (regex ancorado). 1 documento = 1 registro. Zero LLM.
            modo = "template";
            let molde_ver = templates[tipo.as_str()]["version"].as_u64().unwrap_or(0);
            let compiled = compiled_cache.entry(tipo.clone())
                .or_insert_with(|| compile_template(&templates[tipo.as_str()]["regras"]));
            // path-tree (Fase 5): cada campo veio de um REGEX ancorado no rótulo
            let origem: std::collections::HashMap<String, (String, String)> = compiled.iter()
                .map(|(c, re, _)| (c.clone(), ("regex".to_string(), re.as_str().to_string()))).collect();
            let n_esperado = compiled.len();
            let rec = apply_template(&text, &compiled[..]);
            let (nqi, prov) = qualidade_prov(coll, name, "template", Some((&tipo, molde_ver)), &origem, &rec, n_esperado);
            ents.push(chdb::EntidadeRow {
                collection: coll.to_string(), base: name.to_string(), tipo: tipo.clone(),
                idx: 0, dado: rec.to_string(), modo: "template".to_string(), nqi, prov,
                state_hash: sh.clone(), ext_cfg_hash: ecfg.clone(), version, extracted_at: at.clone(),
            });
        }
        match chdb::insert_entities(ch_url, &ents) {
            Ok(_) => { extracted += 1;
                       if modo == "det" { det_bases += 1; if let Some(r) = rows_src { if ents.len() < r { incompletas += 1; } } }
                       else { tpl_bases += 1; } }
            Err(e) => { nlog(&format!("extract {coll}/{name}: insert falhou ({e})")); fails += 1; }
        }
    }
    let pending = pending_before.saturating_sub(extracted);
    json!({"ok": true, "collection": coll, "extracted": extracted, "deterministicas": det_bases,
           "templates": tpl_bases, "incompletas": incompletas, "fails": fails, "pending": pending})
}

fn run_cycle(state: &Arc<Mutex<State>>, force: bool) -> Value {
    let (api, dir, level, llm_url, store, ch_url) = { let s = state.lock().unwrap();
        (s.ragd_api.clone(), s.dir.clone(), s.level, s.llm_url.clone(), s.store.clone(), s.ch_url.clone()) };
    let lib = read_prompts(&dir);   // biblioteca de prompts (uma leitura por ciclo)
    let colls: Vec<String> = http_get_t(&format!("{api}/collections"), 10)
        .and_then(|s| serde_json::from_str::<Value>(&s).ok())
        .and_then(|v| v["collections"].as_array().map(|a| a.iter()
            .filter_map(|c| c["collection"].as_str().map(String::from)).collect()))
        .unwrap_or_default();
    let (mut mined, mut skipped, mut failed) = (vec![], vec![], vec![]);
    let mut classified: Vec<String> = vec![];
    let mut extracted_ents: Vec<String> = vec![];
    let mut det_total = 0u64;   // bases extraídas DETERMINISTICAMENTE (CSV, zero LLM) no ciclo
    for coll in &colls {
        let mut k = read_knowledge(&dir, coll);
        if !k["enabled"].as_bool().unwrap_or(false) { continue; }   // só coleções HABILITADAS
        match mine_level0(&api, coll) {
            Some((src, pillars, n_bases, total_chunks)) => {
                let l0_same = k["source_hash"].as_str() == Some(src.as_str());
                if level >= 1 {
                    // Fase 1: classifica {natureza,tipo} das bases novas/mudadas (doc_class no ClickHouse).
                    let cl = mine_classes(&api, &llm_url, &store, &dir, &ch_url, &lib, coll, force);
                    if cl["classified"].as_u64().unwrap_or(0) > 0 || cl["no_text"].as_u64().unwrap_or(0) > 0 {
                        classified.push(coll.clone());
                    }
                    // Fase 3: o L1 cria/mantém os MOLDES dos tipos não-CSV (1 tipo por ciclo). Roda
                    // ANTES da extração pra o molde já estar no registry quando o L0 for aplicar.
                    let _tpl = mine_templates(&api, &llm_url, &ch_url, &lib, coll);
                    // Fase 2/4/3: extrai DETERMINISTICAMENTE — csv=1 via parse_tabular, csv=0-com-molde
                    // via apply_template (regex). Zero LLM aqui (o LLM só criou o molde acima).
                    let ex = mine_entities(&api, &store, &ch_url, &lib, coll);
                    if ex["extracted"].as_u64().unwrap_or(0) > 0 { extracted_ents.push(coll.clone()); }
                    det_total += ex["deterministicas"].as_u64().unwrap_or(0) + ex["templates"].as_u64().unwrap_or(0);
                    // Summary d00a009 APOSENTADO (10/ago): o normalizador aberto (1 LLM pesado/base, extração
                    // às cegas + agregação) foi substituído pela Fase 1 (classifica) + Fase 2/4 (extrai
                    // determinístico). mine_summary/normalize_base ficam no código, reservados pra Fase 5
                    // (validador de amostra dos verdes, doc §5) — não rodam mais no ciclo.
                }
                // grava se o nível 0 mudou/forçado (o Summary não gera mais escrita).
                if force || !l0_same {
                    // o ciclo pode demorar minutos: reler o `enabled` do disco antes de gravar —
                    // um POST /collection {enabled:false} no meio do ciclo não pode ser perdido
                    let cur = read_knowledge(&dir, coll);
                    k["enabled"] = cur["enabled"].clone();
                    k["level"] = json!(if level >= 1 { 1 } else { 0 });
                    k["source_hash"] = json!(src);
                    k["updated"] = json!(now_stamp());
                    k["knowledge"] = json!(pillars);
                    k["provenance"] = json!({
                        "digestion_id": format!("l0-{}", &src[..src.len().min(8)]),
                        "at": now_stamp(), "via": "level0/no-ai",
                        "inputs": {"bases": n_bases, "total_chunks": total_chunks, "source_hash": src},
                    });
                    write_knowledge(&dir, coll, &k);   // nível 0 (léxico) numa escrita atômica
                    if !l0_same || force { mined.push(coll.clone()); }
                } else {
                    skipped.push(coll.clone());
                }
            }
            None => failed.push(coll.clone()),
        }
    }
    // CacheDigest (#48): 3º pilar do nível 0, GLOBAL — UMA vez por ciclo, FORA do loop de
    // coleções (senão reescreveria N×). Cache vazio é válido; só não grava se o ragd caiu.
    let cache_queries = match mine_cachedigest(&api) {
        Some(cd) => { write_cachedigest(&dir, &cd); cd["content"]["n_queries"].as_u64().unwrap_or(0) }
        None => u64::MAX,   // sentinela: ragd não respondeu /expansions
    };
    if let Ok(mut s) = state.lock() {
        s.last_cycle = format!("{} · nível {} · minou {} · pulou {} · falhou {} · classificou {} · extraiu {}{}{}",
            now_stamp(), level_name(level), mined.len(), skipped.len(), failed.len(), classified.len(), extracted_ents.len(),
            if det_total > 0 { format!(" · det {det_total}") } else { String::new() },
            if force { " (forçado)" } else { "" });
    }
    json!({"ok": true, "level": level_name(level), "forced": force,
           "mined": mined, "skipped": skipped, "failed": failed,
           "classified": classified, "extracted": extracted_ents,
           "cache_digest_queries": if cache_queries == u64::MAX { Value::Null } else { json!(cache_queries) },
           "at": now_stamp()})
}

// ───────────────────────────── worker ─────────────────────────────
// Back-off adaptativo: a cadência é o gate "TEM TRABALHO?" — quando um ciclo PROGRIDE (classificou/
// extraiu/minou/resumiu algo), EMENDA o próximo imediatamente (sem dormir), processando a fila em
// rajada. Só dorme a `cadence` quando IDLE (nada a fazer), desligado, ou ragd offline. Isso corta a
// ociosidade entre ciclos quando há backlog (a máquina não fica esperando 5min à toa) e preserva o
// gate leve quando não há o que fazer. On/online/nível são relidos a cada volta → reativo a pause.
fn worker(state: Arc<Mutex<State>>) {
    const SHORT_NAP: u64 = 8;   // re-checagem rápida: ragd offline (boot) ou ciclo ocupado (/run)
    loop {
        let (on, online, cadence) = { let s = state.lock().unwrap(); (s.on, s.ragd_online, s.cadence.max(10)) };
        // nap = quanto dormir ANTES de reavaliar. cadence = gate normal (idle/desligado);
        // SHORT_NAP = condição transitória (ragd subindo no boot, ou /run ocupando) — re-checa logo,
        // pra um restart não custar 5min ociosos esperando o ragd health-check.
        let nap = if !on {
            cadence   // desligado: gate normal
        } else if !online {
            nlog("ciclo pulado: ragd OFFLINE (re-checa em breve)");
            SHORT_NAP.min(cadence)
        } else if try_start_cycle(&state) {
            let r = run_cycle(&state, false);   // cadência NÃO força: respeita o source_hash
            end_cycle(&state);
            // PROGRESSO = alguma coleção avançou a fila (base minada/classificada/extraída).
            // Só `skipped`/`failed` = nada avançou → idle. `failed` NÃO conta como progresso pra não
            // emendar em loop quente quando o LLM está caindo (aí o back-off da cadência protege).
            let worked = ["mined", "classified", "extracted"].iter()
                .any(|k| r[k].as_array().map(|a| !a.is_empty()).unwrap_or(false));
            nlog(&format!("ciclo — minou={} pulou={} falhou={} classificou={} extraiu={} → {}",
                r["mined"].as_array().map(|a| a.len()).unwrap_or(0),
                r["skipped"].as_array().map(|a| a.len()).unwrap_or(0),
                r["failed"].as_array().map(|a| a.len()).unwrap_or(0),
                r["classified"].as_array().map(|a| a.len()).unwrap_or(0),
                r["extracted"].as_array().map(|a| a.len()).unwrap_or(0),
                if worked { "EMENDA (tem backlog)" } else { "IDLE (dorme a cadência)" }));
            if worked { continue; }   // EMENDA: volta JÁ, sem dormir
            cadence                   // idle: gate normal
        } else {
            nlog("ciclo pulado: já há um ciclo (ex. /run) em andamento");
            SHORT_NAP.min(cadence)
        };
        std::thread::sleep(Duration::from_secs(nap));
    }
}

fn help() {
    println!("nidhoggd {VERSION} — Níðhöggr, camada de inteligência do RAGnaRock (daemon de módulos).
uso:
  nidhoggd [--config <arq>] [--port {DEFAULT_PORT}] [--ragd <url>]
  config: --config <arq>, senão ./nidhogg.cfg, senão defaults.
          chaves: port, ragd_api, nidhogg(on/off), level(minerador|consciente|estrutural|propositivo), dir, cadence, cors_origin
  nasce DESLIGADO (precisa de IA). Liga pelo ValHalla ou pelo cfg.
rotas:
  GET  /health
  GET  /api/nidhogg                 status (nível, cadência, keepalive do ragd, conhecimento)
  GET  /api/nidhogg/collections     coleções do ragd + estado de digestão (liga/desliga por coleção)
  GET  /api/nidhogg/knowledge       conhecimento destilado (?collection=&type=&level=) — só leitura
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

    // modo teste (portão de paridade): classifica uma lista de bases e imprime JSONL, sem daemon.
    if let Some(pos) = args.iter().position(|a| a == "--classify-list") {
        let path = args.get(pos + 1).cloned().unwrap_or_default();
        classify_list_cli(&cfg, &path);
        return;
    }

    let _ = std::fs::create_dir_all(&cfg.dir);
    // store do acumulado/classes: ClickHouse (default) ou SQLite (rollback via cfg store=sqlite)
    store_ensure(&cfg.store, &cfg.dir, &cfg.ch_url);
    ensure_prompt_templates(&cfg.dir);   // garante os templates classificador+extrator editáveis no ValHalla
    let state = Arc::new(Mutex::new(State {
        on: cfg.on, level: cfg.level, dir: cfg.dir.clone(), cadence: cfg.cadence,
        ragd_api: cfg.ragd_api.clone(), llm_url: cfg.llm_url.clone(),
        store: cfg.store.clone(), ch_url: cfg.ch_url.clone(), cfg_path: cfg.cfg_path.clone(),
        started: Instant::now(), last_cycle: String::new(),
        ragd_online: false, ragd_health: Value::Null, cycle_running: false,
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

    // CORS: vazio (default) = same-origin, nenhum header emitido. ValHalla fala com o
    // nidhoggd via proxy server-side no ragd, então o browser nunca bate aqui direto.
    // Setar `cors_origin` no cfg só se for expor a 11497 a um front em outra origem.
    let cors_origin = cfg.cors_origin.clone();
    let cors_header = |resp: &mut Response<std::io::Cursor<Vec<u8>>>| {
        if !cors_origin.is_empty() {
            resp.add_header(Header::from_bytes(&b"Access-Control-Allow-Origin"[..], cors_origin.as_bytes()).unwrap());
        }
    };

    for mut req in server.incoming_requests() {
        let method = req.method().clone();
        let full = req.url().to_string();
        let path = full.split('?').next().unwrap_or("").to_string();
        // preflight CORS: só responde quando habilitado; senão segue o fluxo normal
        if method == Method::Options && !cors_origin.is_empty() {
            let mut resp = Response::from_string("").with_status_code(204);
            cors_header(&mut resp);
            resp.add_header(Header::from_bytes(&b"Access-Control-Allow-Methods"[..], &b"GET, POST, OPTIONS"[..]).unwrap());
            resp.add_header(Header::from_bytes(&b"Access-Control-Allow-Headers"[..], &b"Content-Type"[..]).unwrap());
            let _ = req.respond(resp);
            continue;
        }
        let query = full.splitn(2, '?').nth(1).unwrap_or("").to_string();
        let mut body = String::new();
        let _ = req.as_reader().read_to_string(&mut body);
        let (code, payload) = route(&method, &path, &query, &body, &state);
        let mut resp = Response::from_string(payload).with_status_code(code);
        resp.add_header(Header::from_bytes(&b"Content-Type"[..], &b"application/json"[..]).unwrap());
        cors_header(&mut resp);
        let _ = req.respond(resp);
    }
}
