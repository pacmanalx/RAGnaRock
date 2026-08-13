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

/// percent-decode mínimo (pra parâmetros com texto livre, ex.: ?q= da busca da árvore).
fn pdec(s: &str) -> String {
    let b = s.as_bytes();
    let hex = |c: u8| (c as char).to_digit(16).map(|d| d as u8);
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            if let (Some(h), Some(l)) = (hex(b[i + 1]), hex(b[i + 2])) { out.push((h << 4) | l); i += 3; continue; }
        }
        out.push(if b[i] == b'+' { b' ' } else { b[i] });
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// [FORMA] Assinatura ESTRUTURAL do documento (zero-IA): o esqueleto de rótulos.
/// Extrai os rótulos "Nome:" no início de linha (ordem preservada, dedup) e hasheia.
/// Documentos irmãos de forma (100 mil PIX do mesmo template) compartilham a assinatura;
/// formas DIFERENTES do mesmo tipo ganham moldes separados — é o agrupador que faz o
/// "1 IA por FORMA" escalar. "" = sem estrutura rotulada (narrativo/código não têm forma).
fn form_signature(text: &str) -> String {
    let mut labels: Vec<String> = vec![];
    for line in text.lines().take(400) {
        let t = line.trim_start();
        if let Some(p) = t.find(':') {
            if (2..=28).contains(&p) {
                let lab = &t[..p];
                if lab.chars().any(|c| c.is_alphabetic())
                    && lab.chars().all(|c| c.is_alphanumeric() || matches!(c, ' ' | '_' | '/' | '-' | '.')) {
                    let norm = lab.trim().to_uppercase();
                    if !labels.contains(&norm) { labels.push(norm); }
                }
            }
        }
    }
    if labels.len() < 2 { return String::new(); }   // 1 rótulo solto não é estrutura
    hash_hex(&labels.join("|"))[..8].to_string()
}

/// [L2] Normaliza um valor de campo pra virar CHAVE DE LIGAÇÃO do KnowledgeTree.
/// Dois registros que compartilham a chave estão LIGADOS (a aresta implícita).
/// None = valor que não identifica nada (data, dinheiro, texto curto) — ligar por ele é ruído.
fn norm_valor(campo: &str, v: &str) -> Option<String> {
    let c = campo.to_lowercase();
    // campos de medida/tempo não identificam entidades (mas "nome_*" sempre vale)
    if !c.contains("nome") && ["data", "valor", "total", "preco", "qtd", "quant"].iter().any(|k| c.contains(k)) {
        return None;
    }
    let t = v.trim();
    if t.is_empty() || t.to_lowercase().starts_with("r$") { return None; }
    let digits: String = t.chars().filter(|ch| ch.is_ascii_digit()).collect();
    // data dd/mm/aaaa (8 dígitos com separador) não é identidade
    if t.contains('/') && digits.len() == 8 && t.chars().count() <= 10 { return None; }
    // numérico longo (CNPJ/CPF/conta/ticket): a chave são os dígitos crus
    if digits.len() >= 8 && digits.len() * 2 >= t.chars().count() { return Some(digits); }
    // texto: precisa de corpo (≥5 chars, com letra) — chave = minúsculo sem acento
    if t.chars().count() < 5 || !t.chars().any(|ch| ch.is_alphabetic()) { return None; }
    use unicode_normalization::UnicodeNormalization;
    let folded: String = t.nfd()
        .filter(|ch| !unicode_normalization::char::is_combining_mark(*ch))
        .collect::<String>().to_lowercase().split_whitespace().collect::<Vec<_>>().join(" ");
    Some(folded)
}

// [L2] Fichas narrativas: o LLM local lê JANELAS espalhadas da base narrativa e produz
// fichas {nome, atributos, relações} que entram no MESMO dump — o mine_links liga os
// personagens entre bases/registros como liga CNPJs entre comprovantes.
const FICHA_BASES_PER_CYCLE: usize = 1;   // 1 base narrativa por ciclo (LLM é o caro)
const FICHA_WINDOWS: usize = 4;           // janelas espalhadas pelo documento
const FICHA_WIN_CHARS: usize = 2000;

fn mine_fichas(api: &str, llm_url: &str, ch_url: &str, lib: &Value, coll: &str) -> Value {
    let sys = lib["templates"]["fichas"]["system"].as_str().unwrap_or(BUILTIN_FICHA_PROMPT).to_string();
    let ecfg = hash_hex(&format!("ficha|v1|{}", hash_hex(&sys)));
    let bases: Vec<Value> = chdb::classes_summary(ch_url, Some(coll)).ok()
        .and_then(|v| v["bases"].as_array().cloned()).unwrap_or_default();
    let mut feitas = 0usize;
    let mut fichas_total = 0usize;
    for b in &bases {
        if feitas >= FICHA_BASES_PER_CYCLE { break; }
        if b["natureza"].as_str() != Some("narrativo") { continue; }
        let name = match b["name"].as_str() { Some(n) if !n.is_empty() => n, _ => continue };
        let (sh, _) = chdb::get_class_hashes(ch_url, coll, name).unwrap_or_default();
        if !chdb::needs_extract(ch_url, coll, name, &sh, &ecfg).unwrap_or(true) { continue; }
        let text = match fetch_base_text(api, coll, name) { Some(t) => t, None => continue };
        // janelas espalhadas: início + pontos internos (um livro não cabe no LLM; amostramos)
        let chars: Vec<char> = text.chars().collect();
        let n = chars.len();
        let wins: Vec<String> = (0..FICHA_WINDOWS).map(|i| {
            let start = if FICHA_WINDOWS == 1 { 0 } else { i * n.saturating_sub(FICHA_WIN_CHARS) / (FICHA_WINDOWS - 1) };
            chars[start.min(n)..(start + FICHA_WIN_CHARS).min(n)].iter().collect()
        }).collect();
        // merge por nome folded: atributos acumulam (dedup), relações acumulam (dedup)
        let mut fichas: std::collections::BTreeMap<String, (String, Vec<String>, Vec<String>)> = std::collections::BTreeMap::new();
        let mut janelas_ok = 0usize;
        for w in &wins {
            if w.trim().len() < 200 { continue; }
            match llm_extract_records(llm_url, &sys, "narrativa", w) {
                Ok(regs) => {
                    janelas_ok += 1;
                    for r in regs {
                        let nome = r["nome"].as_str().unwrap_or("").trim().to_string();
                        if nome.chars().count() < 3 { continue; }
                        let key = norm_valor("personagem", &nome).unwrap_or_else(|| nome.to_lowercase());
                        let e = fichas.entry(key).or_insert((nome.clone(), vec![], vec![]));
                        for a in r["atributos"].as_array().map(|x| x.as_slice()).unwrap_or(&[]) {
                            if let Some(s) = a.as_str() { let s = s.trim().to_string();
                                if !s.is_empty() && !e.1.contains(&s) && e.1.len() < 12 { e.1.push(s); } }
                        }
                        for rel in r["relacoes"].as_array().map(|x| x.as_slice()).unwrap_or(&[]) {
                            if let Some(s) = rel.as_str() { let s = s.trim().to_string();
                                if s.chars().count() >= 3 && !e.2.contains(&s) && e.2.len() < 8 { e.2.push(s); } }
                        }
                    }
                }
                Err(e) => nlog(&format!("ficha {coll}/{name}: janela falhou ({e})")),
            }
        }
        if janelas_ok == 0 { continue; }   // LLM fora do ar — não grava checkpoint, tenta no próximo
        let version = chdb::now_version();
        let at = now_stamp();
        let mut rows: Vec<chdb::EntidadeRow> = vec![];
        for (idx, (_k, (nome, attrs, rels))) in fichas.iter().enumerate() {
            let mut dado = serde_json::Map::new();
            dado.insert("personagem".into(), json!(nome));
            if !attrs.is_empty() { dado.insert("atributos".into(), json!(attrs.join("; "))); }
            for (i, r) in rels.iter().enumerate() { dado.insert(format!("rel_{}", i + 1), json!(r)); }
            let nqi = 0.5 + if attrs.is_empty() { 0.0 } else { 0.25 } + if rels.is_empty() { 0.0 } else { 0.25 };
            rows.push(chdb::EntidadeRow {
                collection: coll.to_string(), base: name.to_string(), tipo: "ficha".to_string(),
                idx: idx as u32, dado: Value::Object(dado).to_string(), modo: "ficha".to_string(),
                nqi, prov: json!({"via": "ficha-llm", "janelas": janelas_ok, "modelo": "local"}).to_string(),
                state_hash: sh.clone(), ext_cfg_hash: ecfg.clone(), version, extracted_at: at.clone(),
            });
        }
        match chdb::insert_entities(ch_url, &rows) {
            Ok(_) => { feitas += 1; fichas_total += rows.len();
                       nlog(&format!("fichas {coll}/{name}: {} entidade(s) de {janelas_ok} janela(s)", rows.len())); }
            Err(e) => nlog(&format!("fichas {coll}/{name}: insert falhou ({e})")),
        }
    }
    json!({"ok": true, "collection": coll, "bases": feitas, "fichas": fichas_total})
}

/// [L2] Liga o dump denso de UMA coleção: cada valor-chave dos registros vira nó em
/// nidhogg.no_valor. Incremental por fingerprint (max version do dump) guardado no
/// knowledge.json — religa SÓ quando a extração produziu algo novo. Zero IA.
fn mine_links(ch_url: &str, dir: &str, coll: &str) -> Value {
    if let Err(e) = chdb::ensure_no_schema(ch_url) {
        return json!({"ok": false, "collection": coll, "error": format!("schema no_valor: {e}")});
    }
    let fp = chdb::max_entity_version(ch_url, coll).unwrap_or(0);
    let k = read_knowledge(dir, coll);
    if fp == 0 { return json!({"ok": true, "collection": coll, "linked": 0, "note": "dump vazio"}); }
    if k["link_src"].as_u64() == Some(fp) {
        return json!({"ok": true, "collection": coll, "linked": 0, "note": "dump inalterado"});
    }
    let regs = match chdb::entities_dump(ch_url, coll) {
        Ok(r) => r, Err(e) => return json!({"ok": false, "collection": coll, "error": e}),
    };
    let version = chdb::now_version();
    let at = now_stamp();
    let mut rows: Vec<chdb::NoValorRow> = vec![];
    for r in &regs {
        let dado: Value = serde_json::from_str(r["dado"].as_str().unwrap_or("{}")).unwrap_or(json!({}));
        let obj = match dado.as_object() { Some(o) => o, None => continue };
        for (campo, val) in obj {
            let vs = match val.as_str() { Some(s) => s, None => continue };
            if let Some(norm) = norm_valor(campo, vs) {
                rows.push(chdb::NoValorRow {
                    collection: coll.to_string(), valor_norm: norm, valor: vs.trim().to_string(),
                    campo: campo.clone(), tipo: r["tipo"].as_str().unwrap_or("?").to_string(),
                    base: r["base"].as_str().unwrap_or("").to_string(),
                    idx: r["idx"].as_u64().unwrap_or(0) as u32,
                    nqi: r["nqi"].as_f64().unwrap_or(0.0), version, linked_at: at.clone(),
                });
            }
        }
    }
    if let Err(e) = chdb::insert_nos(ch_url, &rows) {
        return json!({"ok": false, "collection": coll, "error": format!("insert nós: {e}")});
    }
    // persiste o fingerprint (escrita leve — mesmo padrão da saturação)
    let mut cur = read_knowledge(dir, coll);
    cur["link_src"] = json!(fp);
    write_knowledge(dir, coll, &cur);
    nlog(&format!("L2 {coll}: {} nó(s) de valor ligados de {} registro(s)", rows.len(), regs.len()));
    json!({"ok": true, "collection": coll, "linked": rows.len(), "registros": regs.len()})
}

/// Normalização Unicode NFC — mesmo fix do ragd: o macOS entrega nomes em NFD e o mesmo
/// documento virava DUAS classes no doc_class (NFD picada + NFC limpa). Aplicar em TODO
/// nome de base/coleção que entra (API própria e /bases do ragd).
fn nfc(s: &str) -> String {
    use unicode_normalization::UnicodeNormalization;
    s.nfc().collect()
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
            if let Some(mt) = v["max_tokens"].as_u64() { tobj["max_tokens"] = json!(mt.clamp(64, PROMPT_MAX_TOKENS_CEIL)); }
            lib["templates"][name.as_str()] = tobj;
            write_prompts(&dir, &lib);
            nlog(&format!("prompt template {name:?} salvo ({} chars, max_tokens={})", system.chars().count(),
                          v["max_tokens"].as_u64().map(|m| m.to_string()).unwrap_or_else(|| "default".into())));
            (200, json!({"ok":true,"template":name}).to_string())
        }
        // [Fase 1] classes {natureza,tipo} do banco auxiliar — distribuição por coleção (ou todas).
        (Method::Get, "/api/nidhogg/classes") => {
            let (store, dir, ch_url) = { let s = st.lock().unwrap(); (s.store.clone(), s.dir.clone(), s.ch_url.clone()) };
            let coll = query_param(query, "collection").map(|c| nfc(&c));
            match store_classes_summary(&store, &dir, &ch_url, coll.as_deref()) {
                Ok(v) => (200, v.to_string()),
                Err(e) => (500, json!({"error": format!("store: {e}")}).to_string()),
            }
        }
        // [Fase 2] entidades extraídas (o dump denso) — ClickHouse only, sempre via a view entidade_atual.
        (Method::Get, "/api/nidhogg/entities") => {
            let (store, ch_url) = { let s = st.lock().unwrap(); (s.store.clone(), s.ch_url.clone()) };
            let coll = query_param(query, "collection").map(|c| nfc(&c));
            let base = query_param(query, "base").map(|b| nfc(&b));
            if store != "clickhouse" {
                (200, json!({"count": 0, "note": "extração requer clickhouse"}).to_string())
            } else {
                match chdb::entities_summary(&ch_url, coll.as_deref(), base.as_deref()) {
                    Ok(v) => (200, v.to_string()),
                    Err(e) => (500, json!({"error": format!("store: {e}")}).to_string()),
                }
            }
        }
        // [L2] KnowledgeTree — a árvore de assuntos: nós de valor (≥2 participações) →
        // ramos por tipo → registros. ?collection= obrigatório, ?q= busca por assunto.
        (Method::Get, "/api/nidhogg/tree") => {
            let (store, ch_url) = { let s = st.lock().unwrap(); (s.store.clone(), s.ch_url.clone()) };
            let coll = query_param(query, "collection").map(|c| nfc(&pdec(&c))).unwrap_or_default();
            let q = query_param(query, "q").map(|v| pdec(&v)).unwrap_or_default();
            if store != "clickhouse" {
                (200, json!({"nodes": [], "note": "KnowledgeTree requer clickhouse"}).to_string())
            } else if coll.is_empty() {
                (400, json!({"error": "falta ?collection="}).to_string())
            } else {
                match chdb::tree_json(&ch_url, &coll, &q, 100) {
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
        // [Fase 6/cockpit] rejeitados — os documentos que o motor não conseguiu processar (sem molde,
        // NQI baixo, tabela não-CSV). O humano vê aqui e ajusta a ingestão específica.
        (Method::Get, "/api/nidhogg/rejeitados") => {
            let (store, ch_url) = { let s = st.lock().unwrap(); (s.store.clone(), s.ch_url.clone()) };
            if store != "clickhouse" {
                (200, json!({"count": 0, "rejeitados": [], "note": "requer clickhouse"}).to_string())
            } else {
                match chdb::rejeitados_summary(&ch_url) {
                    Ok(v) => (200, v.to_string()),
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
        // [Rejeitados/cockpit] re-tipagem MANUAL de uma base. O operador escolhe o tipo certo; gravamos
        // com origem='humano' → o LLM NUNCA sobrescreve (needs_class curto-circuita). natureza deriva do
        // tipo, csv é determinístico (tabular_spec no texto real). Re-extrai no próximo ciclo (o novo tipo
        // muda o ext_cfg → needs_extract dispara). Corrige os mal-tipados e o COMPARATIVO nqi-baixo.
        (Method::Post, "/api/nidhogg/reclass") => {
            let v: Value = match serde_json::from_str(body) { Ok(v) => v, Err(e) => return (400, json!({"error":format!("JSON inválido: {e}")}).to_string()) };
            let coll = nfc(v["collection"].as_str().unwrap_or("").trim());
            let base = nfc(v["base"].as_str().unwrap_or("").trim());
            let tipo = v["tipo"].as_str().unwrap_or("").trim().to_string();
            if coll.is_empty() || base.is_empty() || tipo.is_empty() {
                return (400, json!({"error":"faltam 'collection', 'base' e 'tipo'"}).to_string());
            }
            let (api, store, dir, ch_url) = { let s = st.lock().unwrap(); (s.ragd_api.clone(), s.store.clone(), s.dir.clone(), s.ch_url.clone()) };
            if store != "clickhouse" { return (400, json!({"error":"re-tipagem requer clickhouse"}).to_string()); }
            if let Err(e) = chdb::ensure_schema(&ch_url) { return (500, json!({"error": format!("schema: {e}")}).to_string()); }
            let (_nat, tipos) = store_doctypes(&store, &dir, &ch_url);
            if !tipos.iter().any(|t| t == &tipo) {
                return (400, json!({"error": format!("tipo desconhecido: {tipo}"), "tipos": tipos}).to_string());
            }
            // csv DETERMINÍSTICO (tabular_spec no texto real); natureza deriva do tipo (ou 'tabela' se csv)
            let texto = fetch_base_text(&api, &coll, &base);
            let csv = texto.as_deref().map(|t| tabular_spec(t).is_some()).unwrap_or(false);
            let forma = texto.as_deref().map(form_signature).unwrap_or_default();
            let natureza = if csv { "tabela".to_string() } else { natureza_do_tipo(&tipo).to_string() };
            // extraível AGORA? csv (det) OU já existe molde pro tipo (puro ou tipo@forma). Se não
            // (documento sem molde ainda, ou narrativo/código que nunca gera registro), NÃO há
            // re-extração — e uma extração ANTIGA desta base (se houver) PERMANECE no dump sob o
            // tipo velho até um molde existir. Avisamos.
            let tem_molde = chdb::get_templates(&ch_url).ok().map(|t|
                t.get(tipo.as_str()).is_some()
                || (!forma.is_empty() && t.get(&format!("{tipo}@{forma}")).is_some())).unwrap_or(false);
            let extraivel = csv || tem_molde;
            let nota = if extraivel { "re-extrai no próximo ciclo" }
                       else if natureza == "documento" { "sem molde — extração antiga purgada; dê um molde dirigido pra re-extrair" }
                       else { "natureza não gera registro — extração antiga purgada do dump" };
            let (sh, ch) = chdb::get_class_hashes(&ch_url, &coll, &base).unwrap_or_default();
            let row = chdb::ClassRow {
                collection: coll.clone(), name: base.clone(), state_hash: sh, cfg_hash: ch,
                natureza: natureza.clone(), tipo: tipo.clone(), forma, csv, origem: "humano".to_string(),
                confianca: 1.0, classified_at: now_stamp(), version: chdb::now_version(),
            };
            match chdb::insert_classes(&ch_url, &[row]) {
                Ok(_) => {
                    // base NÃO-extraível (narrativo/código, ou documento sem molde): a extração velha (do
                    // tipo antigo) vira lixo no dump → PURGA. Extraível: a re-extração do próximo ciclo
                    // supersede pela version (a view entidade_atual já mostra só a mais recente).
                    let purgadas = if !extraivel {
                        chdb::delete_entities(&ch_url, &coll, &base).unwrap_or_else(|e| {
                            nlog(&format!("re-tipado {coll}/{base}: purge de entidades falhou ({e})")); 0 })
                    } else { 0 };
                    let suf = if purgadas > 0 { format!(" ({purgadas} entidade(s) purgada(s))") } else { String::new() };
                    nlog(&format!("re-tipado (humano): {coll}/{base} → tipo={tipo} natureza={natureza} csv={csv} — LLM não sobrescreve; {nota}{suf}"));
                    (200, json!({"ok":true,"collection":coll,"base":base,"tipo":tipo,"natureza":natureza,"csv":csv,"extraivel":extraivel,"nota":nota,"purgadas":purgadas}).to_string())
                }
                Err(e) => (500, json!({"error": format!("store: {e}")}).to_string()),
            }
        }
        // [Rejeitados/cockpit] molde DIRIGIDO: o operador escreve O QUE extrair (instrucao) + aponta 1
        // amostra (collection/base); o L1 cria o molde regex ancorado. Destrava os 'sem molde'. NÃO
        // aplica o gate de cobertura (confia no operador) — só reporta a cobertura medida na amostra.
        (Method::Post, "/api/nidhogg/molde") => {
            let v: Value = match serde_json::from_str(body) { Ok(v) => v, Err(e) => return (400, json!({"error":format!("JSON inválido: {e}")}).to_string()) };
            let tipo = v["tipo"].as_str().unwrap_or("").trim().to_string();
            let instrucao = v["instrucao"].as_str().unwrap_or("").trim().to_string();
            let coll = nfc(v["collection"].as_str().unwrap_or("").trim());
            let base = nfc(v["base"].as_str().unwrap_or("").trim());
            if tipo.is_empty() || coll.is_empty() || base.is_empty() {
                return (400, json!({"error":"faltam 'tipo', 'collection' e 'base' (a amostra)"}).to_string());
            }
            if instrucao.is_empty() {
                return (400, json!({"error":"molde dirigido exige 'instrucao' (o que extrair)"}).to_string());
            }
            let (api, store, dir, llm_url, ch_url) = { let s = st.lock().unwrap(); (s.ragd_api.clone(), s.store.clone(), s.dir.clone(), s.llm_url.clone(), s.ch_url.clone()) };
            if store != "clickhouse" { return (400, json!({"error":"registry requer clickhouse"}).to_string()); }
            if let Err(e) = chdb::ensure_template_schema(&ch_url) { return (500, json!({"error": format!("schema template: {e}")}).to_string()); }
            let amostra = match fetch_base_text(&api, &coll, &base) { Some(t) => t, None => return (404, json!({"error":"amostra sem texto (base não encontrada no ragd)"}).to_string()) };
            let lib = read_prompts(&dir);
            let (sys, _from) = template_system(&lib);
            let (schema, regras) = match llm_make_template(&llm_url, &sys, &tipo, &amostra, &instrucao) {
                Ok(x) => x,
                Err(e) => return (502, json!({"error": format!("L1 não criou o molde: {e}")}).to_string()),
            };
            let regras_v: Value = serde_json::from_str(&regras).unwrap_or_else(|_| json!([]));
            let compiled = compile_template(&regras_v);
            let rec = apply_template(&amostra, &compiled);
            let n_campos = regras_v.as_array().map(|a| a.len()).unwrap_or(0);
            let n_ok = rec.as_object().map(|o| o.values().filter(|x| !x.as_str().unwrap_or("").is_empty()).count()).unwrap_or(0);
            let cobertura = if n_campos > 0 { n_ok as f64 / n_campos as f64 } else { 0.0 };
            let row = chdb::TemplateRow {
                tipo: tipo.clone(), schema, regras, cobertura, origem: "humano".into(),
                created_at: now_stamp(), version: chdb::now_version(),
            };
            match chdb::upsert_template(&ch_url, &row) {
                Ok(_) => { nlog(&format!("molde DIRIGIDO (humano): tipo={tipo} campos={n_campos} cobertura={:.0}% — extrai no próximo ciclo", cobertura * 100.0));
                           (200, json!({"ok":true,"tipo":tipo,"campos":n_campos,"cobertura":cobertura,"amostra":rec}).to_string()) }
                Err(e) => (500, json!({"error": format!("upsert: {e}")}).to_string()),
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
            let coll = match v["collection"].as_str() { Some(c) if !c.is_empty() => nfc(c), _ => return (400, json!({"error":"falta 'collection'"}).to_string()) };
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

// ── editor de prompts (biblioteca nomeada): tetos de segurança do system + max_tokens por template ──
const PROMPT_MAX_CHARS: usize = 6000;        // teto do system prompt de um template (cabe no ctx com folga)
const PROMPT_MAX_TOKENS_CEIL: u64 = 4000;   // teto MÁXIMO configurável por template (trava runaway mesmo custom)

// ───────────────────────────── nível 1 (consciente) — Classificação {natureza,tipo} (Fase 1) ─────────────────────────────
// A ENTRADA do nível 1: descobre a CLASSE de cada base por LLM leve com constrained decoding,
// persistindo no banco auxiliar (ClickHouse). É a fundação do motor auto-adaptativo — registry
// determinístico + NQI sobem em cima disto. Endpoint/modelo do LLM vem do `nidhogg.cfg` (llm_url);
// a lista de tipos e o prompt são editáveis no ValHalla.
const CLASSIFY_MAX_CHARS: usize = 1000;   // primeiros N chars do chunk 0 (bate com a calibração 89,5%)
const CLASSIFY_TIMEOUT_S: u32 = 40;       // curto e FIXO (é ~40 tokens)
const CLASSIFY_MAX_FAILS: usize = 2;      // 2 falhas de LLM consecutivas abortam o lote do ciclo
const CLASSIFY_PER_CYCLE: usize = 40;     // batch por ciclo — classificação é leve (~3s/base)
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
/// Lê a biblioteca. Ausente/corrompida → seed com os templates BUILTIN (nunca sem prompt).
/// Formato: {templates:{name:{description,system,updated}}}. Os prompts são lidos POR NOME
/// (classificador/extrator/modelador) direto de `templates` — sem roteamento por nível/coleção.
fn read_prompts(dir: &str) -> Value {
    let seed = || json!({
        "templates": {
            "classificador": {
                "description": "Classifica {natureza, tipo} na entrada (Fase 1). Os tipos vêm da lista editável de doctypes.",
                "system": BUILTIN_CLASSIFY_PROMPT, "updated": "" },
            "extrator": {
                "description": "Extrai os registros das bases tabulares como array JSON (Fase 2). {tipo} = a classe.",
                "system": BUILTIN_EXTRACT_PROMPT, "updated": "" }
        }
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

/// Extrai um objeto JSON de um texto (tolera fences/prosa em volta). Análogo ao parse_str_array
/// do ragd, mas pra objeto — usado pelo molde da Fase 3 (`{schema, regras}`) e afins.
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
fn llm_make_template(llm_url: &str, sys: &str, tipo: &str, amostra: &str, instrucao: &str) -> Result<(String, String), String> {
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
    // molde DIRIGIDO (cockpit dos Rejeitados): a instrução do operador entra ANTES do exemplo, dizendo
    // ao L1 exatamente quais campos ancorar. Vazia = mineração automática (o L1 decide sozinho).
    let user = if instrucao.trim().is_empty() {
        format!("TIPO: {tipo}\nDOCUMENTO EXEMPLO:\n{amostra}")
    } else {
        format!("TIPO: {tipo}\nINSTRUÇÃO DO OPERADOR (o que extrair): {}\nDOCUMENTO EXEMPLO:\n{amostra}", instrucao.trim())
    };
    let body = json!({
        "messages": [
            {"role": "system", "content": sys},
            {"role": "user", "content": user}
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
    // [FORMA] clusters (tipo, forma) NÃO-CSV sem molde → escolhe o mais populoso (name = amostra).
    // O molde nasce POR FORMA: 100 mil docs da mesma forma = 1 molde; formas distintas do mesmo
    // tipo = moldes separados. Registry usa chave composta "tipo@forma" ('' de forma = tipo puro).
    let mut por_cluster: std::collections::HashMap<(String, String), (usize, String)> = std::collections::HashMap::new();
    for b in &bases {
        let is_csv = b["csv"].as_i64() == Some(1) || b["csv"].as_u64() == Some(1);
        let natureza = b["natureza"].as_str().unwrap_or("");
        let tipo = b["tipo"].as_str().unwrap_or("");
        let forma = b["forma"].as_str().unwrap_or("");
        let name = b["name"].as_str().unwrap_or("");
        // SÓ natureza=documento (átomos da ponta ESTRUTURADOS: comprovante/nota/holerite/OC). Narrativo
        // (contrato, memorial) e código NÃO têm campos rotulados → molde regex não serve (viraria lixo).
        if is_csv || natureza != "documento" || tipo.is_empty() || tipo == "sem-texto" || name.is_empty() { continue; }
        let key = if forma.is_empty() { tipo.to_string() } else { format!("{tipo}@{forma}") };
        if templates.get(&key).is_some() { continue; }               // já tem molde desta forma
        if forma.is_empty() && templates.get(tipo).is_some() { continue; }
        let e = por_cluster.entry((tipo.to_string(), forma.to_string())).or_insert((0, name.to_string()));
        e.0 += 1;
    }
    let ((tipo, forma), (count, aname)) = match por_cluster.into_iter().max_by_key(|(_, (c, _))| *c) {
        Some((k, v)) => (k, v),
        None => return json!({"ok": true, "collection": coll, "criados": 0, "note": "clusters (tipo,forma) não-CSV já têm molde (ou não há)"}),
    };
    let amostra = match fetch_base_text(api, coll, &aname) { Some(t) => t, None => return json!({"ok": false, "collection": coll, "error": "sem amostra"}) };
    let reg_key = if forma.is_empty() { tipo.clone() } else { format!("{tipo}@{forma}") };
    // HERANÇA: se o molde do TIPO puro já cobre esta forma (cobertura ≥ gate na amostra do
    // cluster), materializa um alias — a decisão persiste e o cluster nunca mais entra na fila.
    if !forma.is_empty() {
        if let Some(t) = templates.get(&tipo) {
            let compiled = compile_template(&t["regras"]);
            let rec = apply_template(&amostra, &compiled);
            let nc = compiled.len();
            let n_ok = rec.as_object().map(|o| o.values().filter(|v| !v.as_str().unwrap_or("").is_empty()).count()).unwrap_or(0);
            let cob = if nc > 0 { n_ok as f64 / nc as f64 } else { 0.0 };
            if cob >= TEMPLATE_MIN_COVERAGE {
                let row = chdb::TemplateRow {
                    tipo: reg_key.clone(), schema: t["schema"].to_string(), regras: t["regras"].to_string(),
                    cobertura: cob, origem: "herdado".into(), created_at: now_stamp(), version: chdb::now_version(),
                };
                if let Err(e) = chdb::upsert_template(ch_url, &row) {
                    return json!({"ok": false, "collection": coll, "error": format!("alias herdado: {e}")});
                }
                nlog(&format!("template {coll}/{reg_key}: molde do tipo puro cobre a forma ({:.0}%) — herdado, sem LLM", cob * 100.0));
                return json!({"ok": true, "collection": coll, "criados": 1, "tipo": reg_key, "origem": "herdado", "cobertura": cob});
            }
        }
    }
    let (sys, _from) = template_system(lib);
    let (schema, regras) = match llm_make_template(llm_url, &sys, &tipo, &amostra, "") {
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
        tipo: reg_key.clone(), schema, regras, cobertura, origem: "llm".into(),
        created_at: now_stamp(), version: chdb::now_version(),
    };
    if let Err(e) = chdb::upsert_template(ch_url, &row) {
        return json!({"ok": false, "collection": coll, "tipo": reg_key, "error": format!("upsert: {e}")});
    }
    nlog(&format!("molde criado: {reg_key} campos={n_campos} cobertura={:.0}% ({count} exemplares aguardando)", cobertura * 100.0));
    json!({"ok": true, "collection": coll, "criados": 1, "tipo": reg_key, "campos": n_campos, "cobertura": cobertura})
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
    if !lib["templates"]["fichas"].is_object() {
        lib["templates"]["fichas"] = json!({
            "description": "Fichas narrativas (L2): personagens/entidades e características por janela de texto.",
            "system": BUILTIN_FICHA_PROMPT, "updated": now_stamp(), "max_tokens": 1200 });
        changed = true;
    }
    if changed { write_prompts(dir, &lib); }
}

const BUILTIN_FICHA_PROMPT: &str = "Você lê um TRECHO de uma obra narrativa em português. Extraia as ENTIDADES NOMEADAS \
(personagens, pessoas, organizações, lugares importantes) que aparecem NESTE trecho. Responda APENAS com um array JSON; \
cada elemento: {\"nome\": \"...\", \"atributos\": [\"característica citada no trecho\", ...], \"relacoes\": [\"nome de outra entidade ligada a esta no trecho\", ...]}. \
Seja FIEL ao trecho: só atributos e relações que o texto afirma. Sem entidades → [].";

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
        let name = nfc(b["name"].as_str().unwrap_or(""));
        !name.is_empty() && store_needs_class(store, dir, ch_url, coll, &name, &base_state_hash(b), &cfg_hash)
    }).collect();
    queue.sort_by(|a, b| a["name"].as_str().unwrap_or("").cmp(b["name"].as_str().unwrap_or("")));

    let pending_before = queue.len();
    let (mut classified, mut no_text, mut fails_total) = (0usize, 0usize, 0usize);
    let mut consecutive_fails = 0usize;
    let at = now_stamp();
    let mut rows: Vec<chdb::ClassRow> = vec![];   // acumula o lote (1 INSERT no fim)
    for b in queue.iter().take(CLASSIFY_PER_CYCLE) {
        let name = nfc(b["name"].as_str().unwrap_or(""));
        let name = name.as_str();
        let sh = base_state_hash(b);
        let has_text = b["has_text"].as_bool().unwrap_or(true);
        // [FORMA] assinatura estrutural carimbada junto com a classe (zero-IA, 1 fetch extra
        // só pra base NOVA/mudada — é o agrupador do "1 molde por forma")
        let forma = if has_text {
            fetch_base_text(api, coll, name).map(|t| form_signature(&t)).unwrap_or_default()
        } else { String::new() };
        let mkrow = |nat: &str, tip: &str, csv: bool, conf: f64| chdb::ClassRow {
            collection: coll.to_string(), name: name.to_string(), state_hash: sh.clone(),
            cfg_hash: cfg_hash.clone(), natureza: nat.to_string(), tipo: tip.to_string(),
            forma: forma.clone(), csv, origem: "llm".to_string(),
            confianca: conf, classified_at: at.clone(), version: chdb::now_version(),
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
    // valor: (tipo, csv, ext_cfg, molde_key) — molde_key = "tipo@forma" quando há molde da
    // forma, senão o tipo puro (fallback). O ecfg carrega a chave: molde novo → re-extrai.
    let mut extraiveis: std::collections::HashMap<String, (String, bool, String, String)> = std::collections::HashMap::new();
    for b in &bases_class {
        let name = match b["name"].as_str() { Some(n) if !n.is_empty() => n.to_string(), _ => continue };
        let tipo = b["tipo"].as_str().unwrap_or("registro").to_string();
        let forma = b["forma"].as_str().unwrap_or("");
        if is_csv(b) {
            extraiveis.insert(name, (tipo, true, ext_cfg_csv.clone(), String::new()));
        } else {
            let composta = if forma.is_empty() { String::new() } else { format!("{tipo}@{forma}") };
            let mkey = if !composta.is_empty() && templates.get(&composta).is_some() { composta }
                       else if templates.get(tipo.as_str()).is_some() { tipo.clone() }
                       else { continue };
            let t = &templates[mkey.as_str()];
            let ecfg = hash_hex(&format!("template|v2nqi|{mkey}|{}", hash_hex(&t["regras"].to_string())));
            extraiveis.insert(name, (tipo, false, ecfg, mkey));
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
        let name = nfc(b["name"].as_str().unwrap_or(""));
        match extraiveis.get(name.as_str()) {
            Some((_, _, ecfg, _)) => chdb::needs_extract(ch_url, coll, &name, &base_state_hash(b), ecfg).unwrap_or(true),
            None => false,
        }
    }).collect();
    queue.sort_by(|a, b| a["name"].as_str().unwrap_or("").cmp(b["name"].as_str().unwrap_or("")));

    let pending_before = queue.len();
    let (mut extracted, mut incompletas, mut fails, mut det_bases, mut tpl_bases) = (0usize, 0usize, 0usize, 0usize, 0usize);
    let mut compiled_cache: std::collections::HashMap<String, Vec<(String, regex::Regex, Vec<String>)>> = std::collections::HashMap::new();
    let at = now_stamp();
    for b in queue.iter().take(EXTRACT_PER_CYCLE) {
        let name = nfc(b["name"].as_str().unwrap_or(""));
        let name = name.as_str();
        let sh = base_state_hash(b);
        let (tipo, _bcsv, ecfg, mkey) = match extraiveis.get(name) { Some(x) => x.clone(), None => continue };
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
            // Fase 3: aplica o MOLDE da forma (ou do tipo puro, fallback). 1 doc = 1 registro. Zero LLM.
            modo = "template";
            let molde_ver = templates[mkey.as_str()]["version"].as_u64().unwrap_or(0);
            let compiled = compiled_cache.entry(mkey.clone())
                .or_insert_with(|| compile_template(&templates[mkey.as_str()]["regras"]));
            // path-tree (Fase 5): cada campo veio de um REGEX ancorado no rótulo
            let origem: std::collections::HashMap<String, (String, String)> = compiled.iter()
                .map(|(c, re, _)| (c.clone(), ("regex".to_string(), re.as_str().to_string()))).collect();
            let n_esperado = compiled.len();
            let rec = apply_template(&text, &compiled[..]);
            // prov aponta o molde REAL aplicado (tipo@forma quando específico)
            let (nqi, prov) = qualidade_prov(coll, name, "template", Some((&mkey, molde_ver)), &origem, &rec, n_esperado);
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
                // saturação = fração do corpus com a digestão da camada ATIVA em dia.
                // L0 digere tudo num passe (1.0 ao minerar); L1 mede pelo `pending` da
                // classificação (a porta da consciência — extração vem atrás dela).
                let mut sat: Option<f64> = if level >= 1 { None } else { Some(1.0) };
                if level >= 1 {
                    // Fase 1: classifica {natureza,tipo} das bases novas/mudadas (doc_class no ClickHouse).
                    let cl = mine_classes(&api, &llm_url, &store, &dir, &ch_url, &lib, coll, force);
                    if cl["classified"].as_u64().unwrap_or(0) > 0 || cl["no_text"].as_u64().unwrap_or(0) > 0 {
                        classified.push(coll.clone());
                    }
                    if cl["ok"].as_bool() == Some(true) && n_bases > 0 {
                        let pending = cl["pending"].as_u64().unwrap_or(0) as f64;
                        sat = Some(((n_bases as f64 - pending) / n_bases as f64).clamp(0.0, 1.0));
                    }
                    // Fase 3: o L1 cria/mantém os MOLDES dos tipos não-CSV (1 tipo por ciclo). Roda
                    // ANTES da extração pra o molde já estar no registry quando o L0 for aplicar.
                    let _tpl = mine_templates(&api, &llm_url, &ch_url, &lib, coll);
                    // Fase 2/4/3: extrai DETERMINISTICAMENTE — csv=1 via parse_tabular, csv=0-com-molde
                    // via apply_template (regex). Zero LLM aqui (o LLM só criou o molde acima).
                    let ex = mine_entities(&api, &store, &ch_url, &lib, coll);
                    if ex["extracted"].as_u64().unwrap_or(0) > 0 { extracted_ents.push(coll.clone()); }
                    det_total += ex["deterministicas"].as_u64().unwrap_or(0) + ex["templates"].as_u64().unwrap_or(0);
                    // [L2] KnowledgeTree: fichas narrativas (LLM local, 1 base/ciclo) e depois
                    // a ligação por valores-chave (zero IA, incremental por fingerprint).
                    if level >= 2 && store == "clickhouse" {
                        let _f = mine_fichas(&api, &llm_url, &ch_url, &lib, coll);
                        let _lk = mine_links(&ch_url, &dir, coll);
                    }
                    // Summary d00a009 REMOVIDO (10/ago): o normalizador aberto (1 LLM pesado/base,
                    // extração às cegas + agregação) foi substituído pela Fase 1 (classifica) + Fase 2/4
                    // (extrai determinístico) + Fase 3 (moldes). Dead-code varrido — não há mais mine_summary.
                }
                // grava se o nível 0 mudou/forçado (a extração não gera escrita no knowledge.json).
                if force || !l0_same {
                    // o ciclo pode demorar minutos: reler o `enabled` do disco antes de gravar —
                    // um POST /collection {enabled:false} no meio do ciclo não pode ser perdido
                    let cur = read_knowledge(&dir, coll);
                    k["enabled"] = cur["enabled"].clone();
                    k["level"] = json!(if level >= 1 { 1 } else { 0 });
                    k["source_hash"] = json!(src);
                    k["updated"] = json!(now_stamp());
                    k["knowledge"] = json!(pillars);
                    if let Some(s) = sat { k["saturation"] = json!(s); }
                    k["provenance"] = json!({
                        "digestion_id": format!("l0-{}", &src[..src.len().min(8)]),
                        "at": now_stamp(), "via": "level0/no-ai",
                        "inputs": {"bases": n_bases, "total_chunks": total_chunks, "source_hash": src},
                    });
                    write_knowledge(&dir, coll, &k);   // nível 0 (léxico) numa escrita atômica
                    if !l0_same || force { mined.push(coll.clone()); }
                } else {
                    // a saturação avança MESMO sem o L0 mudar (a fila da Fase 1 anda por ciclo):
                    // escrita leve só quando o valor de fato mudou (>0.1%)
                    if let Some(s) = sat {
                        let mut cur = read_knowledge(&dir, coll);
                        if (cur["saturation"].as_f64().unwrap_or(0.0) - s).abs() > 0.001 {
                            cur["saturation"] = json!(s);
                            write_knowledge(&dir, coll, &cur);
                        }
                    }
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
  POST /api/nidhogg/reclass         re-tipa base à mão {{\"collection\",\"base\",\"tipo\"}} (origem=humano)
  POST /api/nidhogg/molde           molde dirigido {{\"tipo\",\"instrucao\",\"collection\",\"base\"}}
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
