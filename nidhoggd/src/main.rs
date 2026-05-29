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
    cors_origin: String, // CORS: vazio = sem header (same-origin safe); senão ecoa o valor
}
impl Default for Config {
    fn default() -> Self {
        Config { port: DEFAULT_PORT, ragd_api: DEFAULT_RAGD_API.to_string(), on: false, level: 0,
                 dir: DEFAULT_DIR.to_string(), cadence: DEFAULT_CADENCE, cfg_path: "nidhogg.cfg".to_string(),
                 cors_origin: String::new() }
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
        if std::fs::write(&tmp_path, &s).is_ok() {
            let _ = std::fs::rename(&tmp_path, &final_path);
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
        (Method::Post, "/api/nidhogg/run") => { nlog("run manual — forçando ciclo nível 0"); (200, run_cycle(st, true).to_string()) }
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

    // 2) /profile?collection — vocabulário unificado + sílabas salientes (top_uidf).
    let prof: Value = serde_json::from_str(&http_get_t(&format!("{api}/profile?collection={coll}&top=40&vectors=1"), 30)?).ok()?;
    let salient = prof["top_uidf"].as_array().cloned().unwrap_or_default();
    let unified_vocab = prof["unified_vocab_size"].as_u64().unwrap_or(0);
    // dims-por-base (heatmap/dendrograma): vetor tf-idf de cada base nas dims salientes,
    // alinhado 1:1 com `salient` (top_uidf). Vem do /profile&vectors=1.
    let base_vectors = prof["base_vectors"].as_array().cloned().unwrap_or_default();

    // Pilar 1 — RootIndex: as sílabas/dims mais salientes (rankeadas por uidf). É a
    // IDENTIDADE LÉXICA da coleção: o que a distingue das outras.
    let root_index = json!({
        "type": "RootIndex", "level": 0,
        "content": {
            "bases_count": bases.len(),
            "total_chunks": total_chunks,
            "unified_vocab_size": unified_vocab,
            "salient_roots": salient,   // [{dim, syllable, uidf}], ordenado por uidf desc
            "note": "agrupamento por raiz (stem) e ranking idf×freq são [FUTURO]: o /profile expõe uidf, não df/freq por dim"
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
            "bases": per_base,
            "base_vectors": base_vectors,   // [{name,corpus,n_chunks,vec[]}] alinhado às dims salientes (heatmap/dendrograma)
            "note": "base_vectors = tf-idf por base nas dims salientes (alinhado ao salient_roots). vocab completo por base e oov ainda [FUTURO]"
        }
    });

    // CacheDigest: ADIADO — exige um endpoint novo no ragd p/ ler o cache de expansão (o
    // invariante proíbe o nidhoggd ler disco da coleção). Registrado, não fingido.
    Some((source_hash, vec![root_index, corpus_dict], bases.len(), total_chunks))
}

/// Roda UM ciclo. `force=true` (/run manual) re-minera sempre; `force=false` (cadência do
/// worker) pula coleção sem mudança (source_hash igual). NÃO segura o lock durante HTTP/IO.
fn run_cycle(state: &Arc<Mutex<State>>, force: bool) -> Value {
    let (api, dir, level) = { let s = state.lock().unwrap(); (s.ragd_api.clone(), s.dir.clone(), s.level) };
    let colls: Vec<String> = http_get_t(&format!("{api}/collections"), 10)
        .and_then(|s| serde_json::from_str::<Value>(&s).ok())
        .and_then(|v| v["collections"].as_array().map(|a| a.iter()
            .filter_map(|c| c["collection"].as_str().map(String::from)).collect()))
        .unwrap_or_default();
    let (mut mined, mut skipped, mut failed) = (vec![], vec![], vec![]);
    for coll in &colls {
        let mut k = read_knowledge(&dir, coll);
        if !k["enabled"].as_bool().unwrap_or(false) { continue; }   // só coleções HABILITADAS
        match mine_level0(&api, coll) {
            Some((src, pillars, n_bases, total_chunks)) => {
                if !force && k["source_hash"].as_str() == Some(src.as_str()) {
                    skipped.push(coll.clone());   // sem mudança e não forçado → não remastiga
                    continue;
                }
                k["level"] = json!(0);
                k["source_hash"] = json!(src);
                k["updated"] = json!(now_stamp());
                k["knowledge"] = json!(pillars);
                k["provenance"] = json!({
                    "digestion_id": format!("l0-{}", &src[..src.len().min(8)]),
                    "at": now_stamp(), "via": "level0/no-ai",
                    "inputs": {"bases": n_bases, "total_chunks": total_chunks, "source_hash": src},
                });
                write_knowledge(&dir, coll, &k);
                mined.push(coll.clone());
            }
            None => failed.push(coll.clone()),
        }
    }
    if let Ok(mut s) = state.lock() {
        s.last_cycle = format!("{} · nível {} · minou {} · pulou {} · falhou {}{}",
            now_stamp(), level_name(level), mined.len(), skipped.len(), failed.len(),
            if force { " (forçado)" } else { "" });
    }
    json!({"ok": true, "level": level_name(level), "forced": force,
           "mined": mined, "skipped": skipped, "failed": failed, "at": now_stamp()})
}

// ───────────────────────────── worker ─────────────────────────────
// Acorda na cadência; se ON e o ragd online, roda um ciclo do nível 0 (respeitando o
// source_hash: pula coleção sem mudança). A IA (nível >=1) entra aqui nas próximas iterações.
fn worker(state: Arc<Mutex<State>>) {
    loop {
        let cadence = { state.lock().unwrap().cadence.max(10) };
        std::thread::sleep(Duration::from_secs(cadence));
        let (on, online) = { let s = state.lock().unwrap(); (s.on, s.ragd_online) };
        if !on { continue; }
        if !online { nlog("ciclo pulado: ragd OFFLINE"); continue; }
        // TODO(IA nível >=1): por coleção HABILITADA, gerar Summary/Tree/Doc e persistir c/ proveniência.
        let r = run_cycle(&state, false);   // cadência NÃO força: respeita o source_hash
        nlog(&format!("ciclo nível 0 — minou={} pulou={} falhou={}",
            r["mined"].as_array().map(|a| a.len()).unwrap_or(0),
            r["skipped"].as_array().map(|a| a.len()).unwrap_or(0),
            r["failed"].as_array().map(|a| a.len()).unwrap_or(0)));
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
