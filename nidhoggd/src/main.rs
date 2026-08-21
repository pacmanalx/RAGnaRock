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
// Régua RECRAVADA 13/ago/2026 (Pacman, textual): "L3 será o mesmo que a L2 só que 100% LLM
// bound e teremos a L4 que é a camada propositiva". L2 grafa DETERMINÍSTICO; L3 grafa o que o
// determinístico não alcança (100% LLM); L4 (propositiva: parâmetros do usuário + recursão +
// pesquisa externa) NÃO se implementa sem discussão prévia — aqui é só o nome reservado.
fn level_name(l: u8) -> &'static str {
    match l { 0 => "minerador", 1 => "consciente", 2 => "estrutural", 3 => "estrutural-llm", 4 => "propositivo", _ => "minerador" }
}
fn level_num(s: &str) -> u8 {
    match s.trim().to_lowercase().as_str() {
        "consciente" | "1" => 1, "estrutural" | "2" => 2, "estrutural-llm" | "3" => 3,
        "propositivo" | "4" => 4,
        // "burro" aceito como sinônimo retrocompatível de "minerador" (nome antigo do nível 0).
        "minerador" | "burro" | "0" | _ => 0,
    }
}
fn levels_json() -> Value {
    json!([
        {"n":0,"name":"minerador","ia":false,"desc":"Zero IA. Minera a estrutura do corpus — assinatura léxica (as raízes que só a coleção tem), dicionário e digest do cache. O material bruto sobre o qual todos os níveis de IA trabalham."},
        {"n":1,"name":"consciente","ia":true,"desc":"1º nível com IA. Classifica cada documento em {natureza, tipo} por IA leve (vocabulário editável) e normaliza o dado — a camada de significado que vive no ClickHouse, aponta pro corpus e sobrevive à deleção da coleção."},
        {"n":2,"name":"estrutural","ia":false,"desc":"Grafa as relações DETERMINISTICAMENTE sobre o dado já normalizado — nós de valor, censo de menções, dimensões e co-ocorrência de cena. O grafo navegável do conhecimento, zero IA."},
        {"n":3,"name":"estrutural-llm","ia":true,"desc":"O MESMO que o L2, só que 100% LLM-bound: destila as relações que o determinístico não alcança — quem é o quê de quem, temas de cena — e grava no MESMO grafo, com selo de origem."},
        {"n":4,"name":"propositivo","ia":true,"desc":"A camada PROPOSITIVA: você cadastra QUESTÕES DIRETAS (\"quanto faturamos este mês?\", \"qual o ROI do contrato X?\") e o worm responde todo ciclo sobre o conhecimento acumulado. Determinística no ponto de inferência: o contexto é montado por regra, a resposta é do LLM. Cada mudança de perspectiva vira etapa na timeline."}
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
    // Rótulo do MODELO por trás do llm_url. Entra no ext_cfg_hash de TODA camada LLM, então
    // trocar de modelo re-mastiga o corpus — sem isso o checkpoint acha que já processou tudo
    // e o modelo novo fica ocioso (medido em 15/ago ao plugar o Kimi: zero chamadas). Também
    // é carimbado na procedência de cada registro, pra saber qual modelo produziu o quê.
    llm_tag: String,
    // Bearer do endpoint, quando o provedor exige autenticação (Kimi/Moonshot, OpenRouter…).
    // Vazio = sem header, que é o caso do llama-server local. Existe pra que provedor
    // autenticado NÃO precise de um processo shim só pra carimbar o Authorization —
    // o shim se justifica quando o DIALETO é outro (Bedrock Converse), não a credencial.
    // ⚠️ É segredo: mora no cfg (0600 no disco), nunca no git, e as rotas de leitura de
    // config mascaram o valor.
    llm_key: String,
    llm_temp: f64,       // temperatura enviada a TODAS as camadas. 0 = determinístico (llama
                         // local); 1 = obrigatório no Kimi K-series; -1 = omite o campo.
    llm_extra: String,   // objeto JSON mesclado no corpo (campo específico do provedor)
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
                 llm_tag: "local".to_string(),
                 llm_key: String::new(),   // vazio = sem Authorization (llama local)
                 llm_temp: 0.0, llm_extra: String::new(),
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
            "level"    => cfg.level = level_num(v).min(4),
            "dir"      => cfg.dir = v.to_string(),
            "cadence"  => if let Ok(n) = v.parse() { cfg.cadence = n },
            "cors_origin" => cfg.cors_origin = v.to_string(),
            "llm_url"  => cfg.llm_url = v.to_string(),
            "llm_tag"  => cfg.llm_tag = v.to_string(),
            "llm_key"  => cfg.llm_key = v.to_string(),
            "llm_temp" => if let Ok(n) = v.parse() { cfg.llm_temp = n },
            "llm_extra" => cfg.llm_extra = v.to_string(),
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
    llm_tag: String,       // rótulo do modelo — entra no checkpoint e na procedência
    llm_url: String,       // IA da frota p/ nível >=1 (ver comentário na Config)
    store: String,         // backend do acumulado: "clickhouse" | "sqlite"
    ch_url: String,        // endpoint HTTP do ClickHouse
    cfg_path: String,
    started: Instant,
    last_cycle: String,
    ragd_online: bool,
    // saúde do MODELO (llm_url). O worm depende dele do nível 1 pra cima; sem isso a
    // falha era SILENCIOSA — worm parava e a tela continuava verde.
    llm_online: bool,
    llm_erro: String,
    llm_checked: String,     // cache do keepalive (atualizado por thread leve) — status NUNCA faz curl ao vivo
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
        // Bearer só quando há chave: provedor autenticado (Kimi/Moonshot) exige, llama local
        // recusaria nada mas não precisa. Vai como argumento do processo filho, não em env —
        // ⚠️ isso é visível num `ps` da máquina; aceitável porque a Aron é servidor de uso
        // único, e o alternativo (env herdado) apareceria em /proc/<pid>/environ do mesmo jeito.
        let auth = format!("Authorization: Bearer {}", llm_key());
        if tool == "curl" {
            cmd.args(["-s", "-m", &secs.to_string(), "-H", "Content-Type: application/json"]);
            if !llm_key().is_empty() { cmd.args(["-H", &auth]); }
            cmd.args(["-d", body, url]);
        } else {
            cmd.args(["-q", "-O", "-", "--tries=1", &format!("--timeout={secs}"),
                      "--header=Content-Type: application/json"]);
            if !llm_key().is_empty() { cmd.arg(format!("--header={auth}")); }
            cmd.args([&format!("--post-data={body}"), url]);
        }
        if let Ok(out) = cmd.output() {
            if out.status.success() && !out.stdout.is_empty() { return Some(String::from_utf8_lossy(&out.stdout).to_string()); }
        }
    }
    None
}
// ── Diário de mastigação do LLM ("o esquilinho") ──
// TODA chamada de IA (classificador, modelador, extrator…) passa por llm_post e deixa registro
// COMPLETO em JSONL: prompt, resposta, contexto e latência. É o que permite ver a evolução do
// entendimento ciclo a ciclo. Caminho: <dir>/llm-ledger.jsonl (setado no boot a partir do cfg).
static LLM_LEDGER: std::sync::OnceLock<String> = std::sync::OnceLock::new();

/// Ajusta o corpo ao DIALETO do provedor, num ponto só — todas as camadas (classificador,
/// modelador, extrator, relacoes, analista, comparador) passam por aqui, então nenhuma delas
/// precisa saber com quem está falando. Medido contra a API do Kimi em 15/ago:
///   • `temperature` — o Kimi K-series recusa qualquer valor ≠1 ("only 1 is allowed for this
///     model"); o llama local quer 0 pra ser determinístico. `llm_temp` no cfg escolhe, e
///     `llm_temp = -1` OMITE o campo (provedor que não aceita o parâmetro de jeito nenhum).
///   • `json_schema.name` — as camadas montam `{"schema": …}`; o llama.cpp aceita sem `name`,
///     o Kimi devolve 400. Batizar aqui é inócuo pro llama (testado) e obrigatório pro Kimi.
///   • `llm_extra` — objeto JSON do cfg mesclado no corpo, pra campo que só um provedor
///     entende (ex.: `{"thinking":{"type":"disabled"}}`, que no Kimi zera os reasoning_tokens
///     — 241 tokens e 7,3s viram 0 e 2,1s, e esses tokens saíam do MESMO `max_tokens`).
fn ajusta_dialeto(body: &str) -> String {
    let mut v: Value = match serde_json::from_str(body) { Ok(v) => v, Err(_) => return body.to_string() };
    let t = llm_temp();
    if t < 0.0 { v.as_object_mut().map(|o| o.remove("temperature")); }
    else { v["temperature"] = json!(t); }
    if v["response_format"]["type"] == "json_schema" && v["response_format"]["json_schema"]["name"].is_null() {
        v["response_format"]["json_schema"]["name"] = json!("resposta");
    }
    if let Ok(extra) = serde_json::from_str::<Value>(llm_extra()) {
        if let (Some(o), Some(e)) = (v.as_object_mut(), extra.as_object()) {
            for (k, val) in e { o.insert(k.clone(), val.clone()); }
        }
    }
    v.to_string()
}

fn llm_post(tag: &str, ctx: &str, url: &str, body: &str, secs: u32) -> Option<String> {
    let t0 = std::time::Instant::now();
    let body = &ajusta_dialeto(body);
    let resp = http_post_t(url, body, secs);
    let ms = t0.elapsed().as_millis() as u64;
    let (conteudo, finish) = match resp.as_deref() {
        Some(r) => {
            let rv: Value = serde_json::from_str(r).unwrap_or_else(|_| json!({}));
            (rv["choices"][0]["message"]["content"].as_str().unwrap_or("").to_string(),
             rv["choices"][0]["finish_reason"].as_str().unwrap_or("").to_string())
        }
        None => (String::new(), String::new()),
    };
    if let Some(path) = LLM_LEDGER.get() {
        let req: Value = serde_json::from_str(body).unwrap_or_else(|_| json!({}));
        let entry = json!({
            "ts": now_stamp(), "tag": tag, "ctx": ctx, "ms": ms, "ok": resp.is_some(),
            "url": url, "messages": req["messages"], "finish": finish, "resposta": conteudo,
        });
        use std::io::Write;
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
            let _ = writeln!(f, "{entry}");
        }
    }
    // rastro curto no log principal (o diário guarda o inteiro teor)
    nlog(&format!("🐿️ llm {tag} [{ctx}] {ms}ms → {}", if resp.is_some() {
        format!("{}ch{}", conteudo.len(), if finish == "length" { " (CORTADO)" } else { "" })
    } else { "SEM RESPOSTA".to_string() }));
    resp
}

/// Busca o /health do ragd (usado SÓ pela thread de keepalive, nunca no caminho do request).
fn fetch_ragd_health(api: &str) -> Option<Value> {
    http_get(&format!("{api}/health")).and_then(|s| serde_json::from_str(&s).ok())
}
/// Thread leve de keepalive: pinga o ragd periodicamente e cacheia no State.
fn keepalive(state: Arc<Mutex<State>>) {
    loop {
        let (api, llm) = { let s = state.lock().unwrap(); (s.ragd_api.clone(), s.llm_url.clone()) };
        let health = fetch_ragd_health(&api);
        let (llm_ok, llm_err) = check_llm(&llm);
        if let Ok(mut s) = state.lock() {
            s.ragd_online = health.is_some();
            s.ragd_health = health.unwrap_or(Value::Null);
            s.llm_online = llm_ok;
            s.llm_erro = llm_err;
            s.llm_checked = now_stamp();
        }
        std::thread::sleep(Duration::from_secs(15));
    }
}

/// Sonda o endpoint do modelo. Usa `GET /v1/models` do próprio host do `llm_url` — é a rota
/// que llama.cpp, o shim do Bedrock e qualquer OpenAI-compatible expõem, e não gasta token.
/// Devolve (online, motivo) — o motivo vai pra tela, porque "fora do ar" sem porquê não ajuda.
fn check_llm(llm_url: &str) -> (bool, String) {
    let base = match llm_url.find("/v1/") {
        Some(i) => format!("{}/v1/models", &llm_url[..i]),
        None => match llm_url.rfind("/chat/completions") {
            Some(i) => format!("{}/models", &llm_url[..i]),
            None => llm_url.to_string(),
        },
    };
    // GET COM Bearer, e só aqui: `http_get_t` também fala com o ragd, e mandar a credencial
    // do provedor de LLM pro nosso próprio daemon seria espalhar segredo à toa.
    let mut cmd = std::process::Command::new("curl");
    cmd.args(["-s", "-m", "8"]);
    if !llm_key().is_empty() { cmd.args(["-H", &format!("Authorization: Bearer {}", llm_key())]); }
    cmd.arg(&base);
    let sonda = cmd.output().ok().filter(|o| o.status.success() && !o.stdout.is_empty())
                   .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
                   .or_else(|| if llm_key().is_empty() { http_get_t(&base, 8) } else { None });
    match sonda {
        Some(body) if !body.trim().is_empty() => {
            // 200 com corpo de erro também acontece (ex.: credencial expirada no shim)
            if body.contains("\"error\"") && !body.contains("\"data\"") && !body.contains("\"models\"") {
                let m: Value = serde_json::from_str(&body).unwrap_or(Value::Null);
                let msg = m["error"]["message"].as_str().unwrap_or("erro reportado pelo endpoint");
                (false, msg.chars().take(200).collect())
            } else { (true, String::new()) }
        }
        Some(_) => (false, "endpoint respondeu vazio".to_string()),
        None => (false, format!("sem resposta de {base} (timeout ou conexão recusada)")),
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
    // texto: precisa de corpo e ser NOME, não descrição — frase longa não é identidade.
    // Campos de nome próprio (mencao/personagem/nome) aceitam 3 chars: Sam, Eva, Rui existem.
    let min = if c.contains("mencao") || c.contains("personagem") || c.contains("nome") { 3 } else { 5 };
    if t.chars().count() < min || t.chars().count() > 60 || !t.chars().any(|ch| ch.is_alphabetic()) { return None; }
    use unicode_normalization::UnicodeNormalization;
    let folded: String = t.nfd()
        .filter(|ch| !unicode_normalization::char::is_combining_mark(*ch))
        .collect::<String>().to_lowercase().split_whitespace().collect::<Vec<_>>().join(" ");
    Some(folded)
}

// [L2] Extrator narrativo = CENSO DETERMINÍSTICO de menções (zero IA, texto INTEIRO).
// A v1/v2 usava LLM em 4 janelas de 2000 chars (0,3% de um livro) a 1 base/ciclo — Gandalf
// não cruzava e Ellen White nem aparecia. Doutrina aplicada a si mesma: o L0 varre o volume
// (nomes próprios por capitalização, CPU barata, cobertura 100%); o LLM vira enriquecedor
// dirigido no futuro. Os registros {mencao, freq} entram no dump e o mine_links cruza.
const MENCAO_BASES_PER_CYCLE: usize = 60;  // varredura é CPU — o corpus inteiro em poucos ciclos
const MENCAO_TOP_MAX: usize = 2500;        // proteção patológica, NÃO ranking por vaga
const MENCAO_MIN_FREQ: u32 = 3;            // aparece ≥3× no livro = personagem/entidade, não acaso

/// v7: SEM escassez artificial — entra TUDO que passa no piso de frequência. Raridade não é
/// irrelevância: o Pônei Saltitante (freq 6 na trilogia) importa PORQUE é raro e específico;
/// ranking por vaga só deixava a corte principal. O teto de 800 é rede contra texto patológico.
fn mencao_top(_n_chars: usize) -> usize { MENCAO_TOP_MAX }

/// Nomes próprios por heurística zero-IA: sequências de palavras Capitalizadas (com conectores
/// "de/da/do/G." — José de Arimateia, Ellen G. White), contadas no texto INTEIRO. Palavra única
/// capitalizada só vale se a forma minúscula NÃO domina (mata o "The/O/A" de início de frase).
fn extract_mencoes(text: &str, top: usize) -> Vec<(String, u32)> {
    extract_mencoes_lf(text, top, MENCAO_MIN_FREQ, None)
}
/// [v9] Conta as palavras de inicial MINÚSCULA de um texto — a estatística que desmascara
/// "palavra comum capitalizada por posição". No censo por chunk ela é computada no LIVRO
/// INTEIRO (por chunk era fraca demais: "Então" 1× no chunk passava e inflava o teto).
fn lower_freq_de(text: &str) -> std::collections::HashMap<String, u32> {
    let mut m = std::collections::HashMap::new();
    for w in text.split(|c: char| !(c.is_alphanumeric() || c == '.' || c == '\'' || c == '-')) {
        if w.chars().next().map(|c| c.is_lowercase()).unwrap_or(false) {
            *m.entry(w.to_lowercase()).or_insert(0) += 1;
        }
    }
    m
}
/// Variante com piso parametrizado e estatística de minúsculas EXTERNA (o censo por chunk
/// passa a do livro inteiro; None = computa local, comportamento clássico).
fn extract_mencoes_lf(text: &str, top: usize, min_freq: u32,
                      lf_ext: Option<&std::collections::HashMap<String, u32>>) -> Vec<(String, u32)> {
    const CONECTORES: &[&str] = &["de", "da", "do", "dos", "das", "van", "von", "del", "la", "e"];
    let stop = |w: &str| palavra_vazia(&w.to_lowercase()) || matches!(w.to_lowercase().as_str(),
        "the" | "a" | "o" | "os" | "as" | "um" | "uma" | "and" | "but" | "he" | "she" | "it"
        | "in" | "on" | "at" | "of" | "to" | "is" | "was" | "for" | "with" | "that" | "this"
        | "chapter" | "capitulo" | "capítulo" | "livro" | "book" | "parte" | "part"
        | "i" | "ii" | "iii" | "iv" | "v" | "vi" | "não" | "nao" | "sim" | "mas" | "por"
        | "quando" | "então" | "entao" | "depois" | "antes" | "agora" | "assim" | "como");
    let cap = |w: &str| {
        let mut ch = w.chars();
        matches!(ch.next(), Some(c) if c.is_uppercase())
            && w.chars().skip(1).all(|c| c.is_lowercase() || c == '.')
            && w.chars().filter(|c| c.is_alphabetic()).count() >= 2
    };
    let inicial = |w: &str| w.len() <= 2 && w.ends_with('.') && w.chars().next().map(|c| c.is_uppercase()).unwrap_or(false);
    // contagem de minúsculas (pra rejeitar palavra comum capitalizada por posição) —
    // externa quando o chamador tem estatística melhor (o livro inteiro)
    let local_lf;
    let lower_freq: &std::collections::HashMap<String, u32> = match lf_ext {
        Some(m) => m,
        None => { local_lf = lower_freq_de(text); &local_lf }
    };
    let words: Vec<&str> = text.split(|c: char| !(c.is_alphanumeric() || c == '.' || c == '\'' || c == '-'))
        .filter(|w| !w.is_empty()).collect();
    let mut counts: std::collections::HashMap<String, u32> = std::collections::HashMap::new();
    let mut i = 0;
    while i < words.len() {
        let w = words[i].trim_matches(|c: char| c == '\'' || c == '-');
        if (cap(w) || inicial(w)) && !stop(w) {
            let mut seq: Vec<&str> = vec![w];
            let mut j = i + 1;
            while j < words.len() {
                let nx = words[j].trim_matches(|c: char| c == '\'' || c == '-');
                if cap(nx) || inicial(nx) { if !stop(nx) { seq.push(nx); j += 1; continue; } else { break; } }
                // conector minúsculo NO MEIO (José de Arimateia) — só se o próximo for Cap
                if seq.len() >= 1 && CONECTORES.contains(&nx.to_lowercase().as_str())
                    && j + 1 < words.len() && cap(words[j + 1].trim_matches(|c: char| c == '\'' || c == '-')) {
                    seq.push(nx); j += 1; continue;
                }
                break;
            }
            i = j;
            // palavra única: rejeita se a forma minúscula domina (é palavra comum, não nome)
            if seq.len() == 1 {
                let low = seq[0].to_lowercase();
                let lf = lower_freq.get(&low).copied().unwrap_or(0);
                if lf >= 2 { continue; }
                // [v10] VERBO capitalizado por POSIÇÃO (abre item de lista/frase). Caso real:
                // "**Amplificar §3.7** — alavanca que o Arthur (CFO!) precisa" virou entidade,
                // o L3 destilou "Arthur é CFO da Amplificar" e o L4 respondeu isso como fato.
                // Infinitivo português (-ar/-er/-ir) que TAMBÉM aparece em minúscula no texto
                // não é nome próprio. Piso de 1 só aqui: nome real (Xavier, Éder) nunca tem
                // forma minúscula no corpo do texto; palavra comum inglesa (never, after) já
                // cai no lf>=2 acima.
                if lf >= 1 && low.chars().count() >= 5
                    && (low.ends_with("ar") || low.ends_with("er") || low.ends_with("ir")) { continue; }
            }
            // poda ponto final de palavra NÃO-inicial ("Gandalf." → "Gandalf"; "G." fica)
            let nome = seq.iter().map(|w| {
                if w.ends_with('.') && w.chars().filter(|c| c.is_alphabetic()).count() >= 3 {
                    w.trim_end_matches('.')
                } else { w }
            }).collect::<Vec<_>>().join(" ");
            if nome.chars().filter(|c| c.is_alphabetic()).count() >= 3 && nome.chars().count() <= 50 {
                *counts.entry(nome).or_insert(0) += 1;
            }
        } else { i += 1; }
    }
    let mut v: Vec<(String, u32)> = counts.into_iter().filter(|(_, c)| *c >= min_freq).collect();
    v.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    v.truncate(top);
    v
}

// ── [v11] TERMOS TÉCNICOS — a metade do censo que a capitalização não alcança ────────────
// O censo de nome próprio é ótimo pra Tolkien e cego pra documento corporativo: achou 6
// entidades no BRIEFING_INTEGRACAO_VTEX inteiro (22 KB). Com 3 nomes por chunk, o L3 recebia
// matéria-prima insuficiente, inventava o `b` e a âncora vetava 47/49 — corretamente. O que
// falta nesses textos não é nome, é TERMO COMPOSTO recorrente: "trade policy", "regra de
// comissão", "custo atômico", "chão de fábrica", "torre de controle".
// Determinístico como manda a doutrina do L2: n-gramas de 2 a 4 palavras, bordas que não são
// stopword, miolo só com conector, piso de frequência. Zero IA.
const TERMO_MIN_FREQ: u32 = 3;
const TERMO_N_MIN: usize = 2;
const TERMO_N_MAX: usize = 4;

/// Stopwords de BORDA — n-grama não pode começar nem terminar com uma delas.
/// [v12] Palavra que NUNCA carrega identidade: quantificador, indefinido, numeral por extenso.
/// Nasceu da inspeção das relações do corpus corporativo (15/ago): o censo v11b pescava
/// sintagma de prosa analítica como se fosse termo técnico — `metade de baixo`, `seis frentes`,
/// `quarto das respostas`, `mesma instância` — e o L3 grafava aresta em cima disso. Vale para
/// os DOIS lados do censo: borda de n-grama (aqui) e nome próprio de palavra única (`Nenhuma`,
/// que em célula de tabela vem capitalizada e sem par minúsculo, escapando do teste de lf).
/// Não confundir com termo técnico legítimo: `chão de fábrica`, `inferência em cpu` e
/// `trade policy` seguem passando — nenhuma dessas começa ou termina em palavra funcional.
fn palavra_vazia(w: &str) -> bool {
    matches!(w,
        // indefinidos e quantificadores
        "nenhum"|"nenhuma"|"nenhuns"|"nenhumas"|"algum"|"alguma"|"alguns"|"algumas"
        |"outro"|"outra"|"outros"|"outras"|"mesmo"|"mesma"|"mesmos"|"mesmas"
        |"vário"|"vária"|"vários"|"várias"|"varios"|"varias"|"muitos"|"muitas"|"poucos"|"poucas"
        |"ambos"|"ambas"|"qualquer"|"quaisquer"|"próprio"|"própria"|"próprios"|"próprias"
        |"proprio"|"propria"|"tal"|"tais"|"demais"|"diversos"|"diversas"
        // frações e porções (o caso "metade de baixo" / "quarto das respostas")
        |"metade"|"metades"|"terço"|"terços"|"terco"|"tercos"|"quarto"|"quartos"
        |"maioria"|"minoria"|"parcela"|"porção"|"porcao"|"trecho"|"pedaço"|"pedaco"
        // numerais por extenso (o caso "seis frentes")
        |"dois"|"duas"|"três"|"tres"|"quatro"|"cinco"|"seis"|"sete"|"oito"|"nove"|"dez"
        |"onze"|"doze"|"vinte"|"trinta"|"cem"|"mil"|"primeiro"|"primeira"|"segundo"|"segunda"
        |"terceiro"|"terceira"|"último"|"última"|"ultimo"|"ultima"
        // dêiticos de posição que viram sintagma solto
        |"cima"|"baixo"|"lado"|"frente"|"trás"|"tras"|"dentro"|"fora"|"meio")
}

fn termo_stop(w: &str) -> bool {
    if palavra_vazia(w) { return true; }
    matches!(w,
        "a"|"o"|"os"|"as"|"um"|"uma"|"uns"|"umas"|"de"|"da"|"do"|"dos"|"das"|"e"|"ou"|"mas"
        |"que"|"se"|"por"|"para"|"com"|"sem"|"sob"|"sobre"|"no"|"na"|"nos"|"nas"|"ao"|"aos"
        |"à"|"às"|"em"|"pelo"|"pela"|"pelos"|"pelas"|"este"|"esta"|"estes"|"estas"|"esse"
        |"essa"|"esses"|"essas"|"isso"|"isto"|"aquilo"|"seu"|"sua"|"seus"|"suas"|"meu"|"minha"
        |"nosso"|"nossa"|"dele"|"dela"|"deles"|"delas"|"lhe"|"lhes"|"é"|"são"|"foi"|"eram"
        |"ser"|"sendo"|"ter"|"tem"|"têm"|"tinha"|"há"|"havia"|"está"|"estão"|"estava"|"estavam"
        |"não"|"nao"|"sim"|"já"|"ainda"|"também"|"só"|"apenas"|"mais"|"menos"|"muito"|"pouco"
        |"todo"|"toda"|"todos"|"todas"|"cada"|"qual"|"quais"|"como"|"quando"|"onde"|"porque"
        |"pois"|"então"|"entao"|"assim"|"depois"|"antes"|"agora"|"aqui"|"ali"|"lá"|"the"|"of"
        |"to"|"in"|"on"|"at"|"and"|"or"|"for"|"with"|"is"|"was"|"are"|"be"|"by"|"as"|"it"
        |"that"|"this"|"from"|"você"|"eu"|"ele"|"ela"|"nós"|"eles"|"elas"|"quem"|"cujo"|"cuja"
        |"fazer"|"pode"|"podem"|"deve"|"devem"|"vai"|"vão"|"ir")
}
/// Conectores tolerados NO MIOLO ("regra de comissão", "chão de fábrica").
fn termo_conector(w: &str) -> bool {
    matches!(w, "de"|"da"|"do"|"dos"|"das"|"por"|"com"|"em"|"no"|"na"|"a"|"ao"|"à")
}

/// N-gramas recorrentes de um texto. `nomes_norm` = o que o censo de nome próprio já pegou
/// (dedup case-insensitive: "Master Data" e "master data" são a MESMA entidade, não duas).
fn extract_termos(text: &str, min_freq: u32, nomes_norm: &std::collections::HashSet<String>,
                  podar: bool) -> Vec<(String, u32)> {
    let ws: Vec<String> = text
        .split(|c: char| !(c.is_alphanumeric() || c == '-' || c == '\''))
        .filter(|w| !w.is_empty())
        .map(|w| w.to_lowercase())
        .collect();
    // token válido = ao menos 2 LETRAS (mata "---", numeração solta, marcador de markdown)
    let ok = |w: &String| w.chars().filter(|c| c.is_alphabetic()).count() >= 2;
    let mut c: std::collections::HashMap<String, u32> = std::collections::HashMap::new();
    for n in TERMO_N_MIN..=TERMO_N_MAX {
        if ws.len() < n { break; }
        for i in 0..=(ws.len() - n) {
            let g = &ws[i..i + n];
            if !g.iter().all(ok) { continue; }
            if termo_stop(&g[0]) || termo_stop(&g[n - 1]) { continue; }
            // miolo: só palavra de conteúdo ou conector
            if g[1..n - 1].iter().any(|w| termo_stop(w) && !termo_conector(w)) { continue; }
            *c.entry(g.join(" ")).or_insert(0) += 1;
        }
    }
    let cand: std::collections::HashMap<String, u32> =
        c.into_iter().filter(|(_, f)| *f >= min_freq).collect();
    // plural: "trade policy" + "trade policies" são o mesmo termo — funde no singular
    let mut fundido: std::collections::HashMap<String, u32> = std::collections::HashMap::new();
    for (g, f) in &cand {
        let sing = plural_para_singular(g);
        *fundido.entry(if cand.contains_key(&sing) || sing == *g { sing } else { g.clone() }).or_insert(0) += f;
    }
    // poda de subsumido: n-grama contido em outro MAIS LONGO e igualmente frequente não
    // acrescenta nada ("regra de" morre pra "regra de comissão")
    let mut v: Vec<(String, u32)> = fundido.iter()
        // a poda só faz sentido sobre o TOTAL: dentro de um chunk toda frequência tende a 1,
        // aí todo n-grama curto é "igualmente frequente" a um longo que o contém e morreria
        // à toa — foi o que sumiu com "regra de comissão" na primeira rodada da v11.
        .filter(|(g, f)| !podar || !fundido.iter().any(|(h, hf)| h != *g && h.contains(g.as_str()) && hf >= f))
        .filter(|(g, _)| !nomes_norm.contains(*g))          // dedup contra o censo de nomes
        .map(|(g, f)| (g.clone(), *f))
        .collect();
    v.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    v
}
/// Singular ingênuo pro casamento de plural — só o suficiente pra fundir "policies"/"policy"
/// e "ordens"/"ordem" na ÚLTIMA palavra. Não é lematizador e não precisa ser.
fn plural_para_singular(g: &str) -> String {
    let mut p: Vec<&str> = g.split(' ').collect();
    let last = match p.pop() { Some(l) => l, None => return g.to_string() };
    let s = if let Some(r) = last.strip_suffix("ies") { format!("{r}y") }
            else if let Some(r) = last.strip_suffix("ões") { format!("{r}ão") }
            else if last.ends_with("s") && !last.ends_with("ss") && last.chars().count() > 3 {
                last[..last.len() - 1].to_string()
            } else { last.to_string() };
    p.push(&s);
    p.join(" ")
}

fn mine_fichas(api: &str, _llm_url: &str, ch_url: &str, _lib: &Value, coll: &str) -> Value {
    // v9: por chunk COM anti-ruído global (lower_freq do livro inteiro) + teto 2500 —
    // a v8 inflava de "Então/Depois" por-chunk e cortava o Pônei no teto de 800.
    let ecfg = hash_hex("mencao|v12|verbo-infinitivo|lf-global|posicoes|miolo|termos-ngrama|poda-agregada|palavra-vazia");
    let bases: Vec<Value> = chdb::classes_summary(ch_url, Some(coll)).ok()
        .and_then(|v| v["bases"].as_array().cloned()).unwrap_or_default();
    let mut feitas = 0usize;
    let mut mencoes_total = 0usize;
    let mut termos_total = 0usize;
    for b in &bases {
        if feitas >= MENCAO_BASES_PER_CYCLE { break; }
        if b["natureza"].as_str() != Some("narrativo") { continue; }
        let name = match b["name"].as_str() { Some(n) if !n.is_empty() => n, _ => continue };
        let (sh, _) = chdb::get_class_hashes(ch_url, coll, name).unwrap_or_default();
        if !chdb::needs_extract(ch_url, coll, name, &sh, &ecfg, true).unwrap_or(true) { continue; }
        let chunks = match fetch_base_chunks(api, coll, name) { Some(c) if !c.is_empty() => c, _ => continue };
        // miolo POR ÍNDICE de chunk (5%–95% em docs com >10 chunks): capa/licença Gutenberg fora
        let nch = chunks.len();
        let (clo, chi) = if nch > 10 { (nch * 5 / 100, nch * 95 / 100) } else { (0, nch) };
        // anti-ruído GLOBAL: a estatística de minúsculas vem do miolo INTEIRO do livro
        let miolo_lf = {
            let todo: String = chunks[clo..chi.max(clo + 1).min(nch)].iter()
                .map(|(_, t)| t.as_str()).collect::<Vec<_>>().join(" ");
            lower_freq_de(&todo)
        };
        // varre POR CHUNK e acumula (freq total, posições) por menção — a chave da cena
        let mut acc: std::collections::BTreeMap<String, (String, u32, Vec<usize>)> = std::collections::BTreeMap::new();
        for (cid, ctext) in &chunks[clo..chi.max(clo + 1).min(nch)] {
            for (nome, f) in extract_mencoes_lf(ctext, 300, 1, Some(&miolo_lf)) {
                // dentro do chunk o piso é 1 (o piso global ≥3 aplica no total)
                let key = norm_valor("mencao", &nome).unwrap_or_else(|| nome.to_lowercase());
                let e = acc.entry(key).or_insert((nome, 0, vec![]));
                e.1 += f;
                if e.2.last() != Some(cid) && e.2.len() < 400 { e.2.push(*cid); }
            }
        }
        let mut mencoes: Vec<(String, u32, Vec<usize>)> = acc.into_values()
            .filter(|(_, f, _)| *f >= MENCAO_MIN_FREQ).collect();
        mencoes.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        mencoes.truncate(mencao_top(0));
        // [v11] segunda passada: TERMOS técnicos, com as MESMAS posições por chunk (é o campo
        // `chunks` que o L3 usa pra montar a cena — sem ele o termo existe no dump e nunca
        // chega ao modelo). Marcados com kind="termo" pra que quem lê a âncora saiba o que é.
        let nomes_norm: std::collections::HashSet<String> =
            mencoes.iter().map(|(n, _, _)| n.to_lowercase()).collect();
        let mut tacc: std::collections::BTreeMap<String, (String, u32, Vec<usize>)> = std::collections::BTreeMap::new();
        for (cid, ctext) in &chunks[clo..chi.max(clo + 1).min(nch)] {
            for (termo, f) in extract_termos(ctext, 1, &nomes_norm, false) {
                let e = tacc.entry(termo.clone()).or_insert((termo, 0, vec![]));
                e.1 += f;
                if e.2.last() != Some(cid) && e.2.len() < 400 { e.2.push(*cid); }
            }
        }
        let mut termos: Vec<(String, u32, Vec<usize>)> = tacc.into_values()
            .filter(|(_, f, _)| *f >= TERMO_MIN_FREQ).collect();
        // AGORA sim a poda de subsumido, com as frequências totais na mão
        let freq_de: std::collections::HashMap<String, u32> =
            termos.iter().map(|(t, f, _)| (t.clone(), *f)).collect();
        termos.retain(|(t, f, _)| !freq_de.iter().any(|(h, hf)| h != t && h.contains(t.as_str()) && hf >= f));
        termos.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        termos.truncate(mencao_top(0));
        let n_termos = termos.len();
        let version = chdb::now_version();
        let at = now_stamp();
        let rows: Vec<chdb::EntidadeRow> = mencoes.iter().map(|(n, f, p)| (n, f, p, "nome"))
            .chain(termos.iter().map(|(n, f, p)| (n, f, p, "termo")))
            .enumerate().map(|(idx, (nome, freq, pos, kind))| {
            let pos_s: Vec<String> = pos.iter().map(|p| p.to_string()).collect();
            chdb::EntidadeRow {
                collection: coll.to_string(), base: name.to_string(), tipo: "mencao".to_string(),
                idx: idx as u32,
                dado: json!({"mencao": nome, "freq": freq.to_string(), "chunks": pos_s.join(","), "kind": kind}).to_string(),
                modo: "mencao".to_string(), nqi: 1.0,
                prov: json!({"via": if kind == "termo" { "termo-ngrama" } else { "mencao-det" },
                             "freq": freq, "n_chunks": pos.len(), "scan": "por-chunk", "kind": kind}).to_string(),
                state_hash: sh.clone(), ext_cfg_hash: ecfg.clone(), version, extracted_at: at.clone(),
            }
        }).collect();
        // base SEM menção grava sentinela (senão nunca checkpointa e fica eterna na fila)
        let rows = if rows.is_empty() {
            vec![chdb::EntidadeRow {
                collection: coll.to_string(), base: name.to_string(), tipo: "mencao".to_string(),
                idx: 0, dado: json!({"mencao": ""}).to_string(), modo: "mencao".to_string(),
                nqi: 1.0, prov: json!({"via": "mencao-det", "vazio": true, "n_chunks": 0, "scan": "por-chunk"}).to_string(),
                state_hash: sh.clone(), ext_cfg_hash: ecfg.clone(), version, extracted_at: at.clone(),
            }]
        } else { rows };
        // base SEM menções também grava (rows vazio ⇒ marca via linha sentinela? não: precisa
        // de ao menos 1 linha pro checkpoint — bases sem nome próprio ganham 1 registro vazio)
        let rows = if rows.is_empty() {
            vec![chdb::EntidadeRow {
                collection: coll.to_string(), base: name.to_string(), tipo: "mencao".to_string(),
                idx: 0, dado: json!({"mencao": ""}).to_string(), modo: "mencao".to_string(),
                nqi: 1.0, prov: json!({"via": "mencao-det", "vazio": true}).to_string(),
                state_hash: sh.clone(), ext_cfg_hash: ecfg.clone(), version, extracted_at: at.clone(),
            }]
        } else { rows };
        match chdb::insert_entities(ch_url, &rows) {
            Ok(_) => { feitas += 1; mencoes_total += mencoes.len(); termos_total += n_termos; }
            Err(e) => nlog(&format!("mencoes {coll}/{name}: insert falhou ({e})")),
        }
    }
    if feitas > 0 {
        nlog(&format!("mencões {coll}: {feitas} base(s) varridas, {mencoes_total} nome(s) + {termos_total} termo(s)"));
    }
    json!({"ok": true, "collection": coll, "bases": feitas, "mencoes": mencoes_total, "termos": termos_total})
}

// [L3] Estrutural-LLM — a MESMA grafação do L2, mas 100% LLM-bound (régua 13/ago). Pega as
// CENAS mais densas do censo (chunks onde mais entidades co-ocorrem), manda o trecho + a lista
// de presentes pro LLM local e destila relações {a, rel, b, tema}. Os registros entram no dump
// (tipo="relacao", idx=chunk) e o mine_links os cola no MESMO grafo — o nó do LLM funde com o
// nó do censo pela mesma normalização. LLM-bound = caro: 1 base/ciclo, poucas janelas.
const RELACAO_BASES_PER_CYCLE: usize = 1;
const RELACAO_JANELAS: usize = 4;          // cenas por base por passada
const RELACAO_JANELA_MAX_CHARS: usize = 6_000;
const RELACAO_TOP_MENCOES: usize = 40;     // entidades mais frequentes que definem "cena densa"
const RELACAO_TOP_NOMES: usize = 25;       // [v11] cota mínima de NOME próprio dentro do teto

/// Normaliza um nome para o casamento contra a lista de entidades (NFC + minúsculas +
/// espaços colapsados). É a chave do veto determinístico do L3.
fn norm_ent(s: &str) -> String {
    nfc(s).to_lowercase().split_whitespace().collect::<Vec<_>>().join(" ")
}

/// O candidato do LLM casa com alguma entidade do censo?
///
/// Match exato normalizado derrubava 95% das propostas (medido 14/ago: 47/49 e 39/43) — o
/// modelo escreve "Hinode" onde o censo tem "Grupo Hinode" e a relação legítima morria junto.
/// Então aceitamos também o candidato que é SUBCONJUNTO de tokens de uma entidade, e **só
/// nessa direção**: "Hinode" ⊂ "Grupo Hinode" passa; o inverso, não. É o que mantém a porta
/// fechada pro lixo que motivou a âncora — "melhorar busca com IA" NÃO é subconjunto de "IA",
/// é superconjunto, e continua caindo.
fn casa_entidade(cand: &str, entidades: &[String]) -> bool {
    let c = norm_ent(cand);
    if c.is_empty() { return false; }
    if entidades.iter().any(|e| *e == c) { return true; }
    let ct: Vec<&str> = c.split(' ').collect();
    entidades.iter().any(|e| {
        let et: Vec<&str> = e.split(' ').collect();
        // subconjunto próprio: todo token do candidato está na entidade, e a entidade tem mais
        ct.len() < et.len() && ct.iter().all(|t| et.contains(t))
    })
}

/// Verbos de ligação PUROS. "X é Y" quase nunca é relação — é atributo disfarçado de aresta
/// ("OMS API com paginação é armadilha clássica"). O caso legítimo carrega complemento e não
/// cai aqui: "é CFO de", "é fornecedor de", "é mentor de".
const REL_LIGACAO: &[&str] = &[
    "é", "e", "são", "sao", "era", "eram", "foi", "foram", "ser", "sendo",
    "tem", "têm", "ter", "possui", "possuem", "está", "esta", "estão", "estao",
    "não é", "nao e", "não são", "nao sao", "não tem", "nao tem", "existe", "existem",
];

/// Rótulo do modelo em uso (do cfg `llm_tag`). Global porque TODA camada LLM precisa dele
/// no checkpoint e na procedência — propagar por parâmetro tocaria uma dúzia de assinaturas
/// sem ganho. Setado uma vez na subida.
static LLM_TAG: std::sync::OnceLock<String> = std::sync::OnceLock::new();
fn llm_tag() -> &'static str { LLM_TAG.get().map(|s| s.as_str()).unwrap_or("local") }

// Credencial e dialeto do provedor — publicados no boot, lidos por http_post_t/ajusta_dialeto.
// Globais pelo mesmo motivo do LLM_TAG: `llm_url` já viaja como parâmetro por meia dúzia de
// assinaturas e enfiar mais três em cada uma só espalharia ruído.
static LLM_KEY: std::sync::OnceLock<String> = std::sync::OnceLock::new();
fn llm_key() -> &'static str { LLM_KEY.get().map(|s| s.as_str()).unwrap_or("") }
static LLM_TEMP: std::sync::OnceLock<f64> = std::sync::OnceLock::new();
fn llm_temp() -> f64 { *LLM_TEMP.get().unwrap_or(&0.0) }
static LLM_EXTRA: std::sync::OnceLock<String> = std::sync::OnceLock::new();
fn llm_extra() -> &'static str { LLM_EXTRA.get().map(|s| s.as_str()).unwrap_or("") }

fn mine_relacoes(api: &str, llm_url: &str, ch_url: &str, lib: &Value, coll: &str) -> Value {
    let (sys, max_tokens) = relacao_system(lib);
    // checkpoint ACOPLADO ao prompt E ao filtro: mudar qualquer um dos dois re-mastiga tudo
    let ecfg = hash_hex(&format!("relacao|v4-censo-v12|{}|{}", llm_tag(), hash_hex(&sys)));
    let bases: Vec<Value> = chdb::classes_summary(ch_url, Some(coll)).ok()
        .and_then(|v| v["bases"].as_array().cloned()).unwrap_or_default();
    let (mut feitas, mut rel_total) = (0usize, 0usize);
    for b in &bases {
        if feitas >= RELACAO_BASES_PER_CYCLE { break; }
        if b["natureza"].as_str() != Some("narrativo") { continue; }
        let name = match b["name"].as_str() { Some(n) if !n.is_empty() => n, _ => continue };
        let (sh, _) = chdb::get_class_hashes(ch_url, coll, name).unwrap_or_default();
        if !chdb::needs_extract_tipo(ch_url, coll, name, &sh, &ecfg, "relacao").unwrap_or(true) { continue; }
        // pré-requisito: o censo do L2 (o L3 trabalha SOBRE o determinístico, nunca às cegas).
        // Sem menções ainda → NÃO checkpointa (volta quando o censo passar).
        let mencoes = chdb::mencoes_da_base(ch_url, coll, name).unwrap_or_default();
        let lidos: Vec<(String, u32, Vec<u32>, bool)> = mencoes.iter().filter_map(|m| {
            let d: Value = serde_json::from_str(m.as_str()?).ok()?;
            let nome = d["mencao"].as_str()?.trim().to_string();
            if nome.is_empty() { return None; }
            let freq: u32 = d["freq"].as_str().and_then(|s| s.parse().ok()).unwrap_or(0);
            let poss: Vec<u32> = d["chunks"].as_str().unwrap_or("")
                .split(',').filter_map(|s| s.trim().parse().ok()).collect();
            // [v11] kind ausente = registro do censo antigo, que só tinha nome próprio
            let termo = d["kind"].as_str() == Some("termo");
            Some((nome, freq, poss, termo))
        }).collect();
        if lidos.is_empty() { continue; }
        // [v11] COTA por natureza, não uma ordenação só. Termo composto é frequente por
        // construção ("trade policy" 12×) e, num ranking único, empurraria nome raro pra fora
        // do teto — é o problema do Pônei Saltitante que a v7 do censo existiu pra resolver.
        // Cada natureza disputa a própria cota; sobra de uma é cedida à outra.
        let (mut nomes, mut termos): (Vec<_>, Vec<_>) = lidos.into_iter().partition(|(_, _, _, t)| !*t);
        nomes.sort_by(|a, b| b.1.cmp(&a.1));
        termos.sort_by(|a, b| b.1.cmp(&a.1));
        let cota_n = RELACAO_TOP_NOMES.min(nomes.len()).max(RELACAO_TOP_MENCOES.saturating_sub(termos.len()));
        let cota_t = RELACAO_TOP_MENCOES.saturating_sub(cota_n.min(nomes.len()));
        nomes.truncate(cota_n);
        termos.truncate(cota_t);
        let mut top: Vec<(String, u32, Vec<u32>)> = nomes.into_iter().chain(termos)
            .map(|(n, f, p, _)| (n, f, p)).collect();
        top.sort_by(|a, b| b.1.cmp(&a.1));
        // cena densa = chunk com MAIS entidades do topo presentes
        let mut por_chunk: std::collections::HashMap<u32, u32> = std::collections::HashMap::new();
        for (_, _, poss) in &top { for p in poss { *por_chunk.entry(*p).or_insert(0) += 1; } }
        let mut cenas: Vec<(u32, u32)> = por_chunk.into_iter().filter(|(_, n)| *n >= 2).collect();
        cenas.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        cenas.truncate(RELACAO_JANELAS);
        if cenas.is_empty() { continue; }
        let chunks = match fetch_base_chunks(api, coll, name) { Some(c) if !c.is_empty() => c, _ => continue };
        let (version, at) = (chdb::now_version(), now_stamp());
        let mut rows: Vec<chdb::EntidadeRow> = vec![];
        // uma janela que falha NÃO derruba mais a base inteira. Antes, resposta cortada por
        // max_tokens (determinística: a mesma janela corta sempre) impedia o checkpoint e a base
        // voltava a cada ciclo queimando 39s de Qwen — ANALISE_PROPOSTA_SKYONE fez isso 3× em
        // 15 minutos. Agora a janela ruim é pulada e as outras rendem; só não checkpointa se
        // NENHUMA janela responder.
        let (mut jan_ok, mut jan_falha) = (0usize, 0usize);
        // contadores do veto — vão pro log: sem eles o filtro é uma caixa-preta que "some" com relação
        let (mut vet_ent, mut vet_lig, mut propostas) = (0usize, 0usize, 0usize);
        for (cid, _) in &cenas {
            let texto = match chunks.iter().find(|(i, _)| *i == *cid as usize) {
                Some((_, t)) => t.chars().take(RELACAO_JANELA_MAX_CHARS).collect::<String>(),
                None => continue,
            };
            let presentes: Vec<&str> = top.iter().filter(|(_, _, p)| p.contains(cid))
                .map(|(n, _, _)| n.as_str()).collect();
            if presentes.len() < 2 { continue; }
            let schema = json!({"type": "object", "properties": {"relacoes": {"type": "array", "items": {
                "type": "object", "properties": {"a": {"type": "string"}, "rel": {"type": "string"},
                "b": {"type": "string"}, "tema": {"type": "string"}}, "required": ["a", "rel", "b"]}}},
                "required": ["relacoes"]});
            let body = json!({
                "messages": [{"role": "system", "content": sys},
                             {"role": "user", "content": format!("ENTIDADES PRESENTES: {}\n\nTRECHO (chunk {cid}):\n{texto}", presentes.join(", "))}],
                "temperature": 0, "max_tokens": max_tokens,
                "response_format": {"type": "json_schema", "json_schema": {"schema": schema}}
            }).to_string();
            let obj = llm_post("relacoes", &format!("L3 {coll}/{name} chunk {cid}"), llm_url, &body, 150)
                .and_then(|resp| serde_json::from_str::<Value>(&resp).ok())
                .and_then(|rv| rv["choices"][0]["message"]["content"].as_str().map(String::from))
                .and_then(|c| extract_json_object(&c));
            let arr = match obj.as_ref().and_then(|o| o["relacoes"].as_array()) {
                Some(a) => a.clone(),
                // janela sem resposta/JSON → pula ESTA janela (a base segue com as outras)
                None => { jan_falha += 1; continue; }
            };
            jan_ok += 1;
            // ÂNCORA DETERMINÍSTICA DO L3 (14/ago) — a mesma doutrina do censo no caso
            // "Amplificar": o LLM PROPÕE, o determinístico VETA. Só passa relação cujas DUAS
            // pontas estão na lista de entidades que o próprio modelo recebeu. É o que mata o
            // "b" fabricado a partir de pedaço de frase ("armadilha clássica", "melhorar busca
            // com IA", "nomeado desde o dia 1") — instrução em prompt não segura isso num 7B.
            let presentes_norm: Vec<String> = presentes.iter().map(|p| norm_ent(p)).collect();
            for r in &arr {
                let (a, rel, bb) = (r["a"].as_str().unwrap_or("").trim(),
                                    r["rel"].as_str().unwrap_or("").trim(),
                                    r["b"].as_str().unwrap_or("").trim());
                if a.is_empty() || rel.is_empty() || bb.is_empty() || a == bb { continue; }
                propostas += 1;
                if !casa_entidade(a, &presentes_norm) || !casa_entidade(bb, &presentes_norm) {
                    vet_ent += 1; continue;
                }
                // verbo de ligação puro = atributo, não laço
                if REL_LIGACAO.contains(&norm_ent(rel).as_str()) { vet_lig += 1; continue; }
                let tema = r["tema"].as_str().unwrap_or("").trim();
                let mut dado = json!({"a": a, "rel": rel, "b": bb});
                if !tema.is_empty() { dado["tema"] = json!(tema); }
                rows.push(chdb::EntidadeRow {
                    collection: coll.to_string(), base: name.to_string(), tipo: "relacao".to_string(),
                    idx: *cid, dado: dado.to_string(), modo: "llm".to_string(), nqi: 0.8,
                    prov: json!({"via": "relacao-llm", "chunk": cid, "presentes": presentes.len(),
                                 "llm": llm_tag()}).to_string(),
                    state_hash: sh.clone(), ext_cfg_hash: ecfg.clone(), version, extracted_at: at.clone(),
                });
            }
        }
        if jan_ok == 0 {
            nlog(&format!("L3 {coll}/{name}: {jan_falha} janela(s) sem resposta e nenhuma OK — sem checkpoint, re-tenta"));
            continue;
        }
        if jan_falha > 0 {
            nlog(&format!("L3 {coll}/{name}: {jan_falha} janela(s) sem resposta puladas, {jan_ok} OK — checkpointa mesmo assim"));
        }
        if vet_ent + vet_lig > 0 {
            nlog(&format!("L3 {coll}/{name}: âncora vetou {}/{} propostas ({} ponta fora do censo, {} verbo de ligação)",
                          vet_ent + vet_lig, propostas, vet_ent, vet_lig));
        }
        // sentinela: cenas mastigadas e nada destilado TAMBÉM checkpointa (senão fila eterna)
        let n_novas = rows.len();
        let rows = if rows.is_empty() {
            vec![chdb::EntidadeRow {
                collection: coll.to_string(), base: name.to_string(), tipo: "relacao".to_string(),
                idx: 0, dado: json!({"a": ""}).to_string(), modo: "llm".to_string(), nqi: 1.0,
                prov: json!({"via": "relacao-llm", "vazio": true}).to_string(),
                state_hash: sh.clone(), ext_cfg_hash: ecfg.clone(), version, extracted_at: at.clone(),
            }]
        } else { rows };
        match chdb::insert_entities(ch_url, &rows) {
            Ok(_) => { feitas += 1; rel_total += n_novas; }
            Err(e) => nlog(&format!("L3 {coll}/{name}: insert falhou ({e})")),
        }
    }
    if feitas > 0 { nlog(&format!("L3 {coll}: {feitas} base(s) mastigadas, {rel_total} relação(ões) destiladas")); }
    json!({"ok": true, "collection": coll, "bases": feitas, "relacoes": rel_total})
}

// ───────────────────── [L4] Perguntas & Respostas — a camada propositiva ─────────────────────
// Doutrina do Pacman (13/ago): "L4 é determinística DO PONTO DE INFERÊNCIA". O humano cadastra
// a QUESTÃO DIRETA; o sistema monta o contexto por REGRA (agregados do dump + registros +
// trechos do corpus pelo RAG) e o LLM responde. Três tipos de resposta:
//   • tabular  — a resposta é uma TABELA cumulativa sobre o dump (colunas + linhas)
//   • oneshot  — responde 1× e congela (fato que não muda)
//   • vivo     — re-responde TODO ciclo; um comparador decide se a perspectiva MUDOU e só
//                então materializa nova ETAPA. A timeline é o histórico de mudanças de
//                entendimento — a mesma pergunta sob a perspectiva de cada ciclo.
const L4_CTX_MAX_CHARS: usize = 14_000;   // teto do contexto (a mesma disciplina do modelador)
const L4_REGISTROS: usize = 150;          // registros do dump na amostra determinística
const L4_TRECHOS: usize = 6;              // trechos do corpus trazidos pelo RAG
// Fatia da PROVA no orçamento. Um chunk tem ~2000 chars, então isto compra ~4 passagens
// INTEIRAS — contra os ~150 chars picados do snippet que a versão anterior mandava. É a
// maior seção de propósito: o corpus é a única fonte que o analista pode citar como prova.
const L4_PROVA_CHARS: usize = 8_000;
const L4_PISTAS_CHARS: usize = 2_500;     // relações do L3 (pista, não prova)

/// Monta o CONTEXTO — a metade determinística da L4. Nada aqui é decidido por IA: agregados
/// do dump, amostra de registros, as relações do L3 e os trechos que o RAGnaRock casa com a
/// pergunta.
///
/// ORDEM DE CONSTRUÇÃO ≠ ORDEM DE APRESENTAÇÃO, e isso é deliberado (15/ago). A PROVA é
/// montada PRIMEIRO, pra reservar o seu naco do orçamento, mas entra POR ÚLTIMO no texto —
/// o rótulo dela diz "vence qualquer pista acima" e o prompt do analista se apoia nessa
/// hierarquia de confiança, herdada do caso "Amplificar". Antes, o dump era escrito primeiro
/// e comia 2/3 do teto; a PROVA ficava com as migalhas do fim. Foi o que fez o L4 responder
/// "não sei quem é Sandro Rodrigues" com o PREP_CEO_SANDRO ingerido (diagnosticado 14/ago no
/// ledger: 9,4k de fichas alfabéticas do dump contra ~1,1k de trechos picados).
fn l4_contexto(api: &str, ch_url: &str, coll: &str, pergunta: &str) -> (String, String) {
    // o ClickHouse devolve contagem ora como número, ora como string (FORMAT JSON) — normaliza
    let num = |v: &Value| -> String {
        v.as_u64().map(|n| n.to_string())
            .or_else(|| v.as_str().map(String::from))
            .unwrap_or_else(|| "0".into())
    };
    // orçamento em CARACTERES (não bytes): o corte final é `chars().take()`, e em português
    // acentuado len() em bytes diverge ~5-10% — misturar as duas unidades erraria a conta.
    let nchars = |s: &String| s.chars().count();

    // ── 1) PROVA: o texto original. Construída primeiro pra garantir a fatia. ──────────────
    let mut s_prova = String::new();
    let req = json!({"query": pergunta, "base": "*",
                     "collection": if coll == "*" { Value::Null } else { json!(coll) },
                     "k": L4_TRECHOS}).to_string();
    if let Some(resp) = http_post_t(&format!("{api}/search"), &req, 30) {
        if let Ok(v) = serde_json::from_str::<Value>(&resp) {
            let hits = v["hits"].as_array().cloned().unwrap_or_default();
            let mut vistos: Vec<(String, u64)> = Vec::new();
            let (mut usados, mut fora) = (0usize, 0usize);
            for h in &hits {
                let base = h["base"].as_str().unwrap_or("").to_string();
                let hcoll = h["collection"].as_str().unwrap_or("default").to_string();
                let id = h["chunk"].as_u64().unwrap_or(0);
                // /search devolve mais de um hit do mesmo chunk (casamentos distintos)
                if vistos.contains(&(base.clone(), id)) { continue; }
                if nchars(&s_prova) >= L4_PROVA_CHARS { fora += 1; continue; }
                vistos.push((base.clone(), id));
                // CHUNK INTEIRO em vez do `snippet`: o snippet é uma janela de ~150 chars
                // centrada no casamento — o `.take(900)` de antes era inócuo porque o texto
                // já chegava cortado. Aqui vem a passagem que dá pra LER.
                let creq = json!({"base": base, "collection": hcoll, "id": id}).to_string();
                let inteiro = http_post_t(&format!("{api}/chunk"), &creq, 20)
                    .and_then(|r| serde_json::from_str::<Value>(&r).ok())
                    .and_then(|cv| cv["chunks"][0]["text"].as_str().map(String::from));
                // falha de fetch DEGRADA pro snippet, nunca descarta o trecho: um soluço do
                // ragd esvaziaria a PROVA em silêncio e o "não sei" viraria caça ao prompt.
                let texto = match inteiro {
                    Some(t) if !t.trim().is_empty() => t,
                    _ => h["snippet"].as_str().unwrap_or("").to_string(),
                };
                // os marcadores «» cercam CADA sílaba casada ("d«o»cum«e»nt«o»") — realce é
                // pra tela, nunca pro modelo, que recebe a frase picada e não a lê.
                let limpo: String = texto.chars().filter(|c| *c != '«' && *c != '»').collect();
                let sobra = L4_PROVA_CHARS.saturating_sub(nchars(&s_prova));
                s_prova.push_str(&format!("[{}/{} chunk {}] {}\n",
                    base, hcoll, id, limpo.chars().take(sobra).collect::<String>()));
                usados += 1;
            }
            if fora > 0 {
                // sem isto, PROVA curta se confunde com "o corpus não tinha nada"
                nlog(&format!("L4 contexto: {usados} trecho(s) inteiros na PROVA, {fora} hit(s) \
                               fora do orçamento de {L4_PROVA_CHARS} chars"));
            }
        }
    }

    // ── 2) PISTAS: as relações destiladas pelo L3 ─────────────────────────────────────────
    let mut s_pistas = String::new();
    if let Ok(rels) = chdb::relacoes_json(ch_url, Some(coll), 200) {
        // ÂNCORA também aqui: o L4 lê relação como CONHECIMENTO, então só entra a que tem as
        // duas pontas confirmadas pelo censo NA BASE. Foi o furo do caso "Amplificar" — o
        // grafo já filtrava, mas o contexto do analista lia o dump cru e afirmava a pista.
        // Regra: a ponta que SE APRESENTA COMO NOME PRÓPRIO (inicial maiúscula) precisa estar
        // ancorada — é ela que afirma identidade ("é CFO de Amplificar"). Ponta em frase
        // descritiva ("falta de Capex/payback/ROI") não afirma entidade e passa: derrubá-la
        // custaria relações verdadeiras.
        let ancora = chdb::mencoes_ancora(ch_url, coll).unwrap_or_default();
        let arr: Vec<Value> = rels["relacoes"].as_array().cloned().unwrap_or_default()
            .into_iter().filter(|r| {
                let base = r["base"].as_str().unwrap_or("").to_string();
                ["a", "b"].iter().all(|lado| {
                    let txt = r["dado"][*lado].as_str().unwrap_or("").trim();
                    let parece_nome = txt.chars().next().map(|c| c.is_uppercase()).unwrap_or(false);
                    if !parece_nome { return true; }
                    match norm_valor("mencao", txt) {
                        Some(n) => ancora.contains(&(base.clone(), n)),
                        None => true,   // não normaliza como identidade (número, frase longa)
                    }
                })
            }).collect();
        for r in &arr {
            if nchars(&s_pistas) >= L4_PISTAS_CHARS { break; }
            let d = &r["dado"];
            s_pistas.push_str(&format!("- {} —[{}]→ {}{}  ({})\n",
                d["a"].as_str().unwrap_or(""), d["rel"].as_str().unwrap_or(""),
                d["b"].as_str().unwrap_or(""),
                d["tema"].as_str().map(|t| format!(" [tema: {t}]")).unwrap_or_default(),
                r["base"].as_str().unwrap_or("")));
        }
    }

    // ── 3) o mapa do acumulado + os registros crus, COM O QUE SOBRAR ──────────────────────
    let mut s_topo = String::new();
    if let Ok(sum) = chdb::entities_summary(ch_url, Some(coll), None) {
        s_topo.push_str("== O QUE O SISTEMA ACUMULOU (por tipo de registro) ==\n");
        for t in sum["por_tipo"].as_array().map(|a| a.as_slice()).unwrap_or(&[]) {
            s_topo.push_str(&format!("- tipo={} registros={} bases={} modo={}\n",
                t["tipo"].as_str().unwrap_or("?"), num(&t["c"]),
                num(&t["bases"]), t["modo"].as_str().unwrap_or("?")));
        }
    }
    let gasto = nchars(&s_prova) + nchars(&s_pistas) + nchars(&s_topo);
    let teto_dump = L4_CTX_MAX_CHARS.saturating_sub(gasto + 400);   // 400 = rótulos das seções
    let mut s_dump = String::new();
    if let Ok(regs) = chdb::registros_escopo(ch_url, coll, L4_REGISTROS) {
        for r in &regs {
            if nchars(&s_dump) >= teto_dump { break; }
            s_dump.push_str(&format!("[{} #{}] {} :: {}\n",
                r["tipo"].as_str().unwrap_or("?"), r["idx"].as_u64().unwrap_or(0),
                r["base"].as_str().unwrap_or(""), r["dado"].as_str().unwrap_or("{}")));
        }
    }

    // ── montagem: a PROVA fecha o contexto (o rótulo dela fala das pistas "acima") ─────────
    let mut ctx = s_topo;
    if !s_dump.is_empty() {
        ctx.push_str("\n== REGISTROS DO DUMP (dado extraído, o material da conta) ==\n");
        ctx.push_str(&s_dump);
    }
    if !s_pistas.is_empty() {
        // o rótulo carrega a hierarquia de confiança no PRÓPRIO contexto (não só no
        // prompt): pista automática pode estar errada; o texto abaixo é que é prova.
        ctx.push_str("\n== PISTAS: RELAÇÕES DESTILADAS AUTOMATICAMENTE (podem conter erro — direção A→B) ==\n");
        ctx.push_str(&s_pistas);
    }
    if !s_prova.is_empty() {
        ctx.push_str("\n== PROVA: TRECHOS DO CORPUS (texto original — vence qualquer pista acima) ==\n");
        ctx.push_str(&s_prova);
    }
    let ctx: String = ctx.chars().take(L4_CTX_MAX_CHARS).collect();
    let fp = chdb::fingerprint_escopo(ch_url, coll).unwrap_or_default();
    (ctx, fp)
}

/// Último fingerprint JÁ PROCESSADO por pergunta. Vive em memória de propósito: a alternativa
/// seria reinserir a etapa na tabela só pra atualizar o hash, e a timeline lista todas as
/// linhas — duplicaria a etapa na tela do operador.
static L4_VISTO: std::sync::OnceLock<Mutex<std::collections::HashMap<String, String>>>
    = std::sync::OnceLock::new();
fn l4_visto() -> &'static Mutex<std::collections::HashMap<String, String>> {
    L4_VISTO.get_or_init(|| Mutex::new(std::collections::HashMap::new()))
}

/// Fingerprint das ENTRADAS de uma resposta do L4 — o que decide se vale gastar IA de novo.
/// Guardado como `ctx_hash` na etapa; igual ao anterior ⇒ a resposta seria a mesma e o ciclo
/// pula sem chamar o modelo.
///
/// Cobre as cinco coisas que mudam a resposta:
///   1. o DUMP no escopo (contagem + version) — extração/censo/relação nova;
///   2. o CORPUS no ragd — base ingerida que o `/search` já enxerga mas que o censo ainda não
///      mastigou; sem isto o gate atrasaria a resposta em um ciclo. Se o ragd não responder,
///      degrada pra string vazia em vez de travar: perder frescor é melhor que perder resposta;
///   3. o TEXTO e o TIPO da pergunta — editar a pergunta tem que re-responder;
///   4. o PROMPT do analista (editável na biblioteca do ValHalla);
///   5. o MODELO (`llm_tag`) — trocar de modelo re-abre tudo, como no L3.
fn l4_fingerprint(api: &str, ch_url: &str, coll: &str, texto: &str, tipo: &str, lib: &Value) -> String {
    let dump = chdb::fingerprint_escopo(ch_url, coll).unwrap_or_default();
    // pede TODAS as bases e filtra aqui: nome de coleção aceita espaço e acento, e montar
    // querystring exigiria um percent-encode que este daemon não tem (o /bases é barato).
    let corpus = http_get_t(&format!("{api}/bases"), 10)
        .and_then(|r| serde_json::from_str::<Value>(&r).ok())
        .map(|v| {
            let vazio = vec![];
            let bases = v["bases"].as_array().unwrap_or(&vazio).iter()
                .filter(|b| coll == "*" || b["collection"].as_str() == Some(coll));
            let (mut n, mut ch) = (0usize, 0u64);
            for b in bases { n += 1; ch += b["n_chunks"].as_u64().unwrap_or(0); }
            format!("{n}-{ch}")
        })
        .unwrap_or_default();
    let sys = lib["templates"]["analista"]["system"].as_str().unwrap_or("");
    hash_hex(&format!("l4|{dump}|{corpus}|{texto}|{tipo}|{}|{}", hash_hex(sys), llm_tag()))
}

/// Chama o analista (LLM) com a pergunta + contexto determinístico. `tabular` muda a forma da
/// resposta (tabela em vez de texto). Devolve o JSON estruturado validado.
fn l4_responder(llm_url: &str, lib: &Value, ctxlabel: &str, pergunta: &str, tipo: &str, ctx: &str)
    -> Result<Value, String> {
    let sys = match lib["templates"]["analista"]["system"].as_str() {
        Some(s) if !s.trim().is_empty() => s.to_string(),
        _ => BUILTIN_ANALISTA_PROMPT.to_string(),
    };
    let max_tokens = lib["templates"]["analista"]["max_tokens"].as_u64().unwrap_or(1500) as u32;
    // structured output: tabular pede colunas+linhas; os demais, texto corrido
    let resposta_schema = if tipo == "tabular" {
        json!({"type": "object", "properties": {
            "colunas": {"type": "array", "items": {"type": "string"}},
            "linhas": {"type": "array", "items": {"type": "array", "items": {"type": "string"}}},
            "nota": {"type": "string"}}, "required": ["colunas", "linhas"]})
    } else {
        json!({"type": "object", "properties": {"texto": {"type": "string"}}, "required": ["texto"]})
    };
    let schema = json!({"type": "object", "properties": {
        "resposta": resposta_schema,
        "fontes": {"type": "array", "items": {"type": "object", "properties": {
            "base": {"type": "string"}, "trecho": {"type": "string"}}, "required": ["base"]}},
        "proximas": {"type": "array", "items": {"type": "string"}}
    // `fontes` OBRIGATÓRIA: o prompt já mandava citar, e o modelo devolvia `[]` assim mesmo —
    // resposta certa e não-auditável. Medido em 15/ago com o MESMO contexto: schema pedindo só
    // `resposta` → `fontes: []`; exigindo as duas → cita `PREP_CEO_SANDRO/real chunk 0` com o
    // trecho verbatim. Instrução em prompt não segura o que o schema não exige.
    }, "required": ["resposta", "fontes"]});
    let forma = if tipo == "tabular" {
        "A resposta DEVE ser uma TABELA (colunas + linhas). Some/conte a partir dos REGISTROS DO DUMP."
    } else {
        "A resposta DEVE ser texto corrido, direto e curto (até 6 linhas)."
    };
    let body = json!({
        "messages": [
            {"role": "system", "content": sys},
            {"role": "user", "content": format!(
                "PERGUNTA: {pergunta}\n\nFORMA DA RESPOSTA: {forma}\n\n{ctx}")}
        ],
        "temperature": 0, "max_tokens": max_tokens,
        "response_format": {"type": "json_schema", "json_schema": {"schema": schema}}
    }).to_string();
    let resp = llm_post("analista", ctxlabel, llm_url, &body, 240).ok_or("sem resposta (analista)")?;
    let rv: Value = serde_json::from_str(&resp).map_err(|_| "resposta não-JSON".to_string())?;
    let content = rv["choices"][0]["message"]["content"].as_str().ok_or("sem content")?;
    extract_json_object(content).ok_or_else(|| "resposta não é JSON válido".to_string())
}

/// O COMPARADOR — o coração da timeline (decisão do Pacman): responde-se todo ciclo, mas só
/// vira ETAPA se a perspectiva mudou. Devolve Some(o que mudou) ou None (nada novo).
fn l4_mudou(llm_url: &str, lib: &Value, ctxlabel: &str, pergunta: &str, antes: &str, agora: &str)
    -> Option<String> {
    if antes.trim() == agora.trim() { return None; }        // idêntico: nem gasta LLM
    let sys = match lib["templates"]["comparador"]["system"].as_str() {
        Some(s) if !s.trim().is_empty() => s.to_string(),
        _ => BUILTIN_COMPARADOR_PROMPT.to_string(),
    };
    let schema = json!({"type": "object", "properties": {
        "mudou": {"type": "boolean"}, "o_que_mudou": {"type": "string"}}, "required": ["mudou"]});
    let body = json!({
        "messages": [
            {"role": "system", "content": sys},
            {"role": "user", "content": format!(
                "PERGUNTA: {pergunta}\n\nRESPOSTA ANTERIOR:\n{antes}\n\nRESPOSTA DE AGORA:\n{agora}")}
        ],
        "temperature": 0, "max_tokens": 300,
        "response_format": {"type": "json_schema", "json_schema": {"schema": schema}}
    }).to_string();
    let obj = llm_post("comparador", ctxlabel, llm_url, &body, 120)
        .and_then(|r| serde_json::from_str::<Value>(&r).ok())
        .and_then(|rv| rv["choices"][0]["message"]["content"].as_str().map(String::from))
        .and_then(|c| extract_json_object(&c))?;
    if obj["mudou"].as_bool() == Some(true) {
        let o = obj["o_que_mudou"].as_str().unwrap_or("").trim();
        Some(if o.is_empty() { "perspectiva mudou".to_string() } else { o.to_string() })
    } else { None }
}

/// Responde UMA pergunta e materializa etapa se mudou. Usado pelo ciclo e pelo /perguntar.
/// `forcar` grava a etapa mesmo sem mudança (o "responder agora" do operador).
fn l4_processar(api: &str, llm_url: &str, ch_url: &str, lib: &Value, p: &Value, forcar: bool) -> Value {
    let nome = p["nome"].as_str().unwrap_or("").to_string();
    let texto = p["texto"].as_str().unwrap_or("").to_string();
    let tipo = p["tipo"].as_str().unwrap_or("vivo").to_string();
    let coll = p["escopo"].as_str().unwrap_or("*").to_string();
    if nome.is_empty() || texto.is_empty() { return json!({"ok": false, "error": "pergunta sem nome/texto"}); }
    let anterior = chdb::ultima_resposta(ch_url, &nome).ok().flatten();
    // ONESHOT: fato que não muda — responde 1× e congela (só o forçar re-abre)
    if tipo == "oneshot" && anterior.is_some() && !forcar {
        return json!({"ok": true, "pergunta": nome, "nova_etapa": false, "note": "one-shot já respondida"});
    }
    // ── SATURAÇÃO DO L4 (15/ago) — o freio que faltava ────────────────────────────────────
    // As outras camadas já não repetem trabalho: a Fase 1 tem `needs_class`, o L1 e o L3 têm
    // `needs_extract_tipo`. O L4 rodava TODAS as perguntas ativas a cada ciclo, e cada uma
    // custa DUAS chamadas (analista + comparador). Medido com o corpus parado: 14 chamadas em
    // 10 min, ~2.000/dia, pra reproduzir resposta idêntica.
    // O contexto é DETERMINÍSTICO: mesmas entradas ⇒ mesma resposta. Então basta reconhecer as
    // entradas. O fingerprint cobre TUDO que muda a resposta — não só o dump:
    //   dump (contagem+version) · corpus no ragd · texto e tipo da pergunta · prompt do
    //   analista · modelo. Editar a pergunta, editar o prompt na biblioteca ou trocar de
    //   modelo re-dispara, que é a mesma disciplina do `ext_cfg_hash` do L3.
    let fp = l4_fingerprint(api, ch_url, &coll, &texto, &tipo, lib);
    if !forcar && !fp.is_empty() {
        // duas fontes pro "já vi este escopo", e a segunda não é luxo: quando o comparador diz
        // "mesma perspectiva" NADA é gravado, então o ctx_hash persistido continua velho e a
        // pergunta recalcularia pra sempre — o gate só pegaria quem gerasse etapa nova (medido
        // ao vivo: 1 saturada de 3). O cache em memória fecha esse buraco sem inventar linha na
        // timeline (a tela lista TODAS as linhas da tabela: reinserir duplicaria a etapa).
        // Custo de um restart do daemon = uma rodada a mais. Barato perto de mexer no schema.
        let visto = l4_visto().lock().ok().and_then(|m| m.get(&nome).cloned()).unwrap_or_default();
        let persistido = anterior.as_ref().and_then(|a| a["ctx_hash"].as_str()).unwrap_or("");
        if visto == fp || persistido == fp {
            return json!({"ok": true, "pergunta": nome, "nova_etapa": false, "saturado": true,
                          "note": "nada mudou no escopo desde a última resposta — sem chamada de IA"});
        }
    }
    let t0 = std::time::Instant::now();
    let (ctx, _fp_ctx) = l4_contexto(api, ch_url, &coll, &texto);
    if ctx.trim().is_empty() { return json!({"ok": false, "pergunta": nome, "error": "contexto vazio (nada acumulado no escopo)"}); }
    let ctxlabel = format!("L4 {nome} [{coll}]");
    let obj = match l4_responder(llm_url, lib, &ctxlabel, &texto, &tipo, &ctx) {
        Ok(o) => o,
        Err(e) => { nlog(&format!("L4 {nome}: {e}")); return json!({"ok": false, "pergunta": nome, "error": e}); }
    };
    let ms = t0.elapsed().as_millis() as u64;
    // normaliza a resposta pra texto comparável (tabela vira JSON estável)
    let resposta_v = obj["resposta"].clone();
    let resposta_s = if tipo == "tabular" { resposta_v.to_string() }
                     else { resposta_v["texto"].as_str().unwrap_or("").trim().to_string() };
    if resposta_s.is_empty() { return json!({"ok": false, "pergunta": nome, "error": "resposta vazia"}); }
    let (seq_ant, texto_ant) = match &anterior {
        Some(a) => (a["seq"].as_u64().unwrap_or(0) as u32, a["resposta"].as_str().unwrap_or("").to_string()),
        None => (0, String::new()),
    };
    // primeira resposta SEMPRE vira etapa; depois, só quando o comparador diz que mudou
    let mudou = if anterior.is_none() { Some("primeira resposta".to_string()) }
                else if forcar { Some(l4_mudou(llm_url, lib, &ctxlabel, &texto, &texto_ant, &resposta_s)
                                        .unwrap_or_else(|| "resposta forçada pelo operador".to_string())) }
                else { l4_mudou(llm_url, lib, &ctxlabel, &texto, &texto_ant, &resposta_s) };
    // carimba o escopo como JÁ PROCESSADO — vale para os DOIS desfechos abaixo. É justamente o
    // caminho "não mudou" que precisa disso: ele não grava linha nenhuma, e sem o carimbo a
    // pergunta voltaria a gastar analista + comparador em todo ciclo, para sempre.
    if let Ok(mut m) = l4_visto().lock() { m.insert(nome.clone(), fp.clone()); }
    let mudou = match mudou {
        Some(m) => m,
        None => return json!({"ok": true, "pergunta": nome, "nova_etapa": false,
                              "note": "mesma perspectiva — timeline inalterada", "ms": ms}),
    };
    let row = chdb::RespostaRow {
        pergunta: nome.clone(), seq: seq_ant + 1, tipo: tipo.clone(),
        resposta: resposta_s, mudou: mudou.clone(),
        // o modelo pode omitir fontes/proximas (são opcionais no schema) — grava array vazio,
        // nunca "null": a UI itera esses campos
        fontes: if obj["fontes"].is_array() { obj["fontes"].to_string() } else { "[]".into() },
        proximas: if obj["proximas"].is_array() { obj["proximas"].to_string() } else { "[]".into() },
        ctx_hash: fp, ms, at: now_stamp(),
    };
    if let Err(e) = chdb::insert_resposta(ch_url, &row) {
        return json!({"ok": false, "pergunta": nome, "error": format!("insert: {e}")});
    }
    nlog(&format!("L4 {nome}: etapa {} — {mudou}", seq_ant + 1));
    json!({"ok": true, "pergunta": nome, "nova_etapa": true, "seq": seq_ant + 1, "mudou": mudou, "ms": ms})
}

/// O ciclo da L4: passa por TODAS as perguntas ativas (o cadastro é global, não por coleção).
fn mine_respostas(api: &str, llm_url: &str, ch_url: &str, lib: &Value) -> Value {
    let ps = match chdb::perguntas(ch_url) { Ok(p) => p, Err(e) => return json!({"ok": false, "error": e}) };
    let lista = ps.as_array().cloned().unwrap_or_default();
    let (mut respondidas, mut etapas, mut saturadas) = (0usize, 0usize, 0usize);
    for p in &lista {
        if p["ativa"].as_bool() == Some(false) { continue; }
        let r = l4_processar(api, llm_url, ch_url, lib, p, false);
        if r["ok"].as_bool() == Some(true) {
            // saturada = escopo intacto, NENHUMA chamada de IA. Contar junto com as respondidas
            // faria o log dizer "3 respondidas" num ciclo que não gastou nada — e é justamente
            // esse número que se olha pra saber se o freio está segurando.
            if r["saturado"].as_bool() == Some(true) { saturadas += 1; continue; }
            respondidas += 1;
            if r["nova_etapa"].as_bool() == Some(true) { etapas += 1; }
        }
    }
    if respondidas > 0 || saturadas > 0 {
        nlog(&format!("L4: {respondidas} pergunta(s) respondida(s), {etapas} etapa(s) nova(s), \
                       {saturadas} saturada(s) (escopo intacto, sem IA)"));
    }
    json!({"ok": true, "perguntas": respondidas, "etapas": etapas, "saturadas": saturadas})
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
    // [ÂNCORA do L3] O que o CENSO viu em cada base — os nomes que a contagem determinística
    // confirma. As pontas das relações do LLM só viram NÓ se estiverem aqui: sem isso, frase
    // solta no slot de entidade ("conduziu a mesa") criava vocabulário novo no grafo.
    // A relação continua inteira no dump (a tela do L3 mostra tudo) — o que a âncora protege
    // é o GRAFO, que é o que o Navigator e o L4 leem como conhecimento.
    let mut confirmados: std::collections::HashSet<(String, String)> = std::collections::HashSet::new();
    for r in &regs {
        if r["tipo"].as_str() != Some("mencao") { continue; }
        let dado: Value = serde_json::from_str(r["dado"].as_str().unwrap_or("{}")).unwrap_or(json!({}));
        let nome = dado.get("mencao").and_then(|v| v.as_str()).unwrap_or("").trim();
        if let Some(norm) = norm_valor("mencao", nome) {
            confirmados.insert((r["base"].as_str().unwrap_or("").to_string(), norm));
        }
    }
    let mut rows: Vec<chdb::NoValorRow> = vec![];
    let mut ancorados_fora = 0usize;
    for r in &regs {
        let dado: Value = serde_json::from_str(r["dado"].as_str().unwrap_or("{}")).unwrap_or(json!({}));
        let obj = match dado.as_object() { Some(o) => o, None => continue };
        // [v8] MENÇÃO: um nó POR POSIÇÃO DE CHUNK (idx = chunk) — a co-ocorrência vira
        // proximidade de CENA com a MESMA regra dos registros (mesmo base+idx).
        if r["tipo"].as_str() == Some("mencao") {
            let nome = obj.get("mencao").and_then(|v| v.as_str()).unwrap_or("").trim();
            if let Some(norm) = norm_valor("mencao", nome) {
                let poss: Vec<u32> = obj.get("chunks").and_then(|v| v.as_str()).unwrap_or("")
                    .split(',').filter_map(|s| s.trim().parse().ok()).collect();
                for p in poss {
                    rows.push(chdb::NoValorRow {
                        collection: coll.to_string(), valor_norm: norm.clone(), valor: nome.to_string(),
                        campo: "mencao".to_string(), tipo: "mencao".to_string(),
                        base: r["base"].as_str().unwrap_or("").to_string(),
                        idx: p, nqi: 1.0, version, linked_at: at.clone(),
                    });
                }
            }
            continue;
        }
        // [L3] RELAÇÃO destilada por LLM: nós pros DOIS lados (a, b) com idx = chunk da cena —
        // a mesma regra (base,idx) da co-ocorrência liga a↔b e ambos às menções do censo na
        // mesma cena. Normalização de "mencao" de propósito: o nó do LLM FUNDE com o do censo.
        // O rótulo do laço (rel) NÃO vira nó (viraria hub de ruído: "serve", "trai"…) — ele
        // vive no registro do dump (drill-down). Tema vira nó próprio (campo="tema").
        if r["tipo"].as_str() == Some("relacao") {
            let base = r["base"].as_str().unwrap_or("").to_string();
            let cid = r["idx"].as_u64().unwrap_or(0) as u32;
            let nqi = r["nqi"].as_f64().unwrap_or(0.0);
            for lado in ["a", "b"] {
                let nome = obj.get(lado).and_then(|v| v.as_str()).unwrap_or("").trim();
                if let Some(norm) = norm_valor("mencao", nome) {
                    // ÂNCORA: só entra no grafo o que o censo determinístico confirmou na base
                    if !confirmados.contains(&(base.clone(), norm.clone())) { ancorados_fora += 1; continue; }
                    rows.push(chdb::NoValorRow {
                        collection: coll.to_string(), valor_norm: norm, valor: nome.to_string(),
                        campo: "relacao".to_string(), tipo: "relacao".to_string(),
                        base: base.clone(), idx: cid, nqi, version, linked_at: at.clone(),
                    });
                }
            }
            let tema = obj.get("tema").and_then(|v| v.as_str()).unwrap_or("").trim();
            if let Some(norm) = norm_valor("tema", tema) {
                rows.push(chdb::NoValorRow {
                    collection: coll.to_string(), valor_norm: norm, valor: tema.to_string(),
                    campo: "tema".to_string(), tipo: "relacao".to_string(),
                    base, idx: cid, nqi, version, linked_at: at.clone(),
                });
            }
            continue;
        }
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
    if let Err(e) = chdb::clear_nos(ch_url, coll) {
        return json!({"ok": false, "collection": coll, "error": format!("clear nós: {e}")});
    }
    if let Err(e) = chdb::insert_nos(ch_url, &rows) {
        return json!({"ok": false, "collection": coll, "error": format!("insert nós: {e}")});
    }
    // persiste o fingerprint (escrita leve — mesmo padrão da saturação)
    let mut cur = read_knowledge(dir, coll);
    cur["link_src"] = json!(fp);
    write_knowledge(dir, coll, &cur);
    nlog(&format!("L2 {coll}: {} nó(s) de valor ligados de {} registro(s){}", rows.len(), regs.len(),
        if ancorados_fora > 0 { format!(" · {ancorados_fora} ponta(s) do L3 fora da âncora do censo") } else { String::new() }));
    json!({"ok": true, "collection": coll, "linked": rows.len(), "registros": regs.len(),
           "ancorados_fora": ancorados_fora})
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
        // saúde do modelo — o front usa pra travar as telas L0-L4 quando cai
        "llm_online": st.llm_online,
        "llm_tag": st.llm_tag,
        "llm_url": st.llm_url,
        // presença da credencial, NUNCA o valor — esta rota é lida pelo browser
        "llm_auth": !llm_key().is_empty(),
        "llm_erro": st.llm_erro,
        "llm_checked": st.llm_checked,
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
            (200, json!({"status":"ok","module":"nidhogg","version":VERSION,"on":s.on,
                         "level":level_name(s.level),
                         "llm_online":s.llm_online,"llm_tag":s.llm_tag}).to_string())
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
        // [L3] relações destiladas pelo LLM (tipo="relacao" no dump) — a leitura do cockpit.
        (Method::Get, "/api/nidhogg/relacoes") => {
            let (store, ch_url) = { let s = st.lock().unwrap(); (s.store.clone(), s.ch_url.clone()) };
            let coll = query_param(query, "collection").map(|c| nfc(&pdec(&c)));
            let n: usize = query_param(query, "n").and_then(|v| v.parse().ok()).unwrap_or(200).min(1000);
            if store != "clickhouse" {
                (200, json!({"count": 0, "relacoes": [], "note": "relações requerem clickhouse"}).to_string())
            } else {
                match chdb::relacoes_json(&ch_url, coll.as_deref(), n) {
                    Ok(v) => (200, v.to_string()),
                    Err(e) => (500, json!({"error": format!("store: {e}")}).to_string()),
                }
            }
        }
        // [Diário de mastigação] cauda do llm-ledger pro ValHalla (?n=30, máx 200). Prompt e
        // resposta vão TRUNCADOS (o inteiro teor fica no arquivo) com os tamanhos reais anotados.
        (Method::Get, "/api/nidhogg/llm_ledger") => {
            let n: usize = query_param(query, "n").and_then(|v| v.parse().ok()).unwrap_or(30).min(200);
            let path = match LLM_LEDGER.get() { Some(p) => p.clone(), None => return (200, json!({"entries": []}).to_string()) };
            const TAIL_BYTES: u64 = 12 * 1024 * 1024;   // entradas podem ter MB (documento no prompt)
            let (texto, cortado) = (|| -> std::io::Result<(String, bool)> {
                use std::io::{Read, Seek, SeekFrom};
                let mut f = std::fs::File::open(&path)?;
                let len = f.metadata()?.len();
                let start = len.saturating_sub(TAIL_BYTES);
                f.seek(SeekFrom::Start(start))?;
                let mut buf = Vec::new();
                f.read_to_end(&mut buf)?;
                Ok((String::from_utf8_lossy(&buf).into_owned(), start > 0))
            })().unwrap_or((String::new(), false));
            let corta = |s: &str, max: usize| -> String {
                if s.chars().count() <= max { s.to_string() }
                else { format!("{}…", s.chars().take(max).collect::<String>()) }
            };
            let mut linhas: Vec<&str> = texto.lines().filter(|l| !l.trim().is_empty()).collect();
            if cortado && !linhas.is_empty() { linhas.remove(0); }   // primeira pode ser parcial
            let entries: Vec<Value> = linhas.iter().rev().take(n)
                .filter_map(|l| serde_json::from_str::<Value>(l).ok())
                .map(|e| {
                    let sys = e["messages"][0]["content"].as_str().unwrap_or("");
                    let user = e["messages"][1]["content"].as_str().unwrap_or("");
                    let resp = e["resposta"].as_str().unwrap_or("");
                    json!({
                        "ts": e["ts"], "tag": e["tag"], "ctx": e["ctx"], "ms": e["ms"],
                        "ok": e["ok"], "finish": e["finish"],
                        "system": corta(sys, 2000), "system_len": sys.chars().count(),
                        "user": corta(user, 4000), "user_len": user.chars().count(),
                        "resposta": corta(resp, 6000), "resposta_len": resp.chars().count(),
                    })
                }).collect();
            (200, json!({"file": path, "entries": entries}).to_string())
        }
        // [Dimensões] cadastro (a ponte L2→L3): eixos declarados de navegação/exigência.
        (Method::Get, "/api/nidhogg/dimensoes") => {
            let (store, ch_url) = { let s = st.lock().unwrap(); (s.store.clone(), s.ch_url.clone()) };
            if store != "clickhouse" { return (200, json!({"dimensoes": []}).to_string()); }
            let mut dims = chdb::dimensoes(&ch_url).unwrap_or_else(|_| json!([]));
            // seed na primeira visita: dois eixos que o corpus atual já alimenta
            if dims.as_array().map(|a| a.is_empty()).unwrap_or(true) {
                dims = json!([
                    {"nome": "CNPJ/CPF", "descricao": "identidade fiscal — liga contratos, comprovantes e cadastros",
                     "campos": ["*_cnpj", "*_cpf", "cnpj", "cpf"], "tipos": []},
                    {"nome": "Pessoas & Entidades", "descricao": "nomes próprios — personagens, pessoas, organizações",
                     "campos": ["mencao", "*_nome", "personagem"], "tipos": []},
                ]);
                let _ = chdb::write_dimensoes(&ch_url, &dims);
            }
            (200, json!({"dimensoes": dims}).to_string())
        }
        (Method::Post, "/api/nidhogg/dimensoes") => {
            let (store, ch_url) = { let s = st.lock().unwrap(); (s.store.clone(), s.ch_url.clone()) };
            if store != "clickhouse" { return (400, json!({"error": "requer clickhouse"}).to_string()); }
            let v: Value = match serde_json::from_str(body) { Ok(v) => v, Err(e) => return (400, json!({"error": format!("JSON inválido: {e}")}).to_string()) };
            let dims = &v["dimensoes"];
            let arr = match dims.as_array() { Some(a) => a, None => return (400, json!({"error": "falta 'dimensoes' (array)"}).to_string()) };
            for d in arr {
                let nome = d["nome"].as_str().unwrap_or("");
                if nome.trim().is_empty() { return (400, json!({"error": "dimensão sem 'nome'"}).to_string()); }
                let campos = d["campos"].as_array().map(|a| a.len()).unwrap_or(0);
                if campos == 0 { return (400, json!({"error": format!("dimensão '{nome}' sem 'campos'")}).to_string()); }
                for c in d["campos"].as_array().unwrap() {
                    let cs = c.as_str().unwrap_or("");
                    if cs.is_empty() || !cs.chars().all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '*' | '.' | '-')) {
                        return (400, json!({"error": format!("padrão de campo inválido em '{nome}': {cs:?} (use letras, dígitos, _ . - e *)")}).to_string());
                    }
                }
            }
            match chdb::write_dimensoes(&ch_url, dims) {
                Ok(_) => { nlog(&format!("dimensões salvas: {} eixo(s)", arr.len())); (200, json!({"ok": true, "dimensoes": dims}).to_string()) }
                Err(e) => (500, json!({"error": format!("store: {e}")}).to_string()),
            }
        }
        // [Dimensões] valores de um eixo (o primeiro clique da corrente)
        (Method::Get, "/api/nidhogg/dimensao/valores") => {
            let (store, ch_url) = { let s = st.lock().unwrap(); (s.store.clone(), s.ch_url.clone()) };
            let nome = query_param(query, "nome").map(|v| pdec(&v)).unwrap_or_default();
            let coll = query_param(query, "collection").map(|c| nfc(&pdec(&c)))
                .filter(|c| !c.is_empty()).unwrap_or_else(|| "*".to_string());
            let q = query_param(query, "q").map(|v| pdec(&v)).unwrap_or_default();
            if store != "clickhouse" || nome.is_empty() {
                return (400, json!({"error": "requer clickhouse + ?nome="}).to_string());
            }
            let dims = chdb::dimensoes(&ch_url).unwrap_or_else(|_| json!([]));
            let dim = dims.as_array().and_then(|a| a.iter().find(|d| d["nome"].as_str() == Some(nome.as_str()))).cloned();
            let dim = match dim { Some(d) => d, None => return (404, json!({"error": format!("dimensão '{nome}' não existe")}).to_string()) };
            let padroes: Vec<String> = dim["campos"].as_array().map(|a| a.iter()
                .filter_map(|c| c.as_str().map(String::from)).collect()).unwrap_or_default();
            match chdb::dimensao_valores(&ch_url, &coll, &padroes, &q, 200) {
                Ok(mut v) => { v["nome"] = json!(nome); (200, v.to_string()) }
                Err(e) => (500, json!({"error": format!("store: {e}")}).to_string()),
            }
        }
        // [Dimensões→L3] GAPS: onde o eixo declarado NÃO alcança — tipos do corpus sem nenhum
        // campo que case os padrões. É a demanda de mastigação que o humano injetou.
        (Method::Get, "/api/nidhogg/dimensoes/gaps") => {
            let (store, ch_url) = { let s = st.lock().unwrap(); (s.store.clone(), s.ch_url.clone()) };
            let coll = query_param(query, "collection").map(|c| nfc(&pdec(&c)))
                .filter(|c| !c.is_empty()).unwrap_or_else(|| "*".to_string());
            if store != "clickhouse" { return (200, json!({"gaps": []}).to_string()); }
            let dims = chdb::dimensoes(&ch_url).unwrap_or_else(|_| json!([]));
            let tc = chdb::tipos_campos(&ch_url, &coll).unwrap_or_default();
            // tipos EXTRAÍVEIS do corpus (documento/tabela — narrativo não gera registro rotulado)
            let classes = chdb::classes_summary(&ch_url, if coll == "*" { None } else { Some(&coll) })
                .unwrap_or_else(|_| json!({}));
            let mut tipos_corpus: std::collections::BTreeMap<String, String> = std::collections::BTreeMap::new();
            for b in classes["bases"].as_array().map(|a| a.as_slice()).unwrap_or(&[]) {
                let (nat, tip) = (b["natureza"].as_str().unwrap_or(""), b["tipo"].as_str().unwrap_or(""));
                if matches!(nat, "documento" | "tabela") && !tip.is_empty() && tip != "sem-texto" {
                    tipos_corpus.insert(tip.to_string(), nat.to_string());
                }
            }
            let casa = |p: &str, campo: &str| -> bool {
                // wildcard simples: '*' casa qualquer trecho
                let partes: Vec<&str> = p.split('*').collect();
                let mut resto = campo;
                for (i, parte) in partes.iter().enumerate() {
                    if parte.is_empty() { continue; }
                    if i == 0 && !p.starts_with('*') {
                        if !resto.starts_with(parte) { return false; }
                        resto = &resto[parte.len()..];
                    } else if i == partes.len() - 1 && !p.ends_with('*') {
                        if !resto.ends_with(parte) { return false; }
                    } else {
                        match resto.find(parte) { Some(pos) => resto = &resto[pos + parte.len()..], None => return false }
                    }
                }
                true
            };
            let gaps: Vec<Value> = dims.as_array().map(|a| a.iter().map(|d| {
                let nome = d["nome"].as_str().unwrap_or("");
                let padroes: Vec<&str> = d["campos"].as_array().map(|a| a.iter()
                    .filter_map(|c| c.as_str()).collect()).unwrap_or_default();
                let alvo: Vec<String> = d["tipos"].as_array()
                    .filter(|a| !a.is_empty())
                    .map(|a| a.iter().filter_map(|t| t.as_str().map(String::from)).collect())
                    .unwrap_or_else(|| tipos_corpus.keys().cloned().collect());
                let cobertos: std::collections::BTreeSet<&String> = alvo.iter().filter(|t| {
                    tc.iter().any(|(tt, cc)| tt == *t && padroes.iter().any(|p| casa(p, cc)))
                }).collect();
                let faltando: Vec<&String> = alvo.iter().filter(|t| !cobertos.contains(t)).collect();
                json!({"nome": nome, "alvo": alvo.len(), "cobertos": cobertos.len(),
                       "gaps": faltando, "nota": if faltando.is_empty() { "eixo plenamente alimentado" }
                               else { "tipos sem campo do eixo — candidatos a molde dirigido (L3)" }})
            }).collect()).unwrap_or_default();
            (200, json!({"collection": coll, "gaps": gaps}).to_string())
        }
        // [Think Navigator] sugestões leves de tema. collection é FILTRO opcional (default: todas).
        (Method::Get, "/api/nidhogg/suggest") => {
            let (store, ch_url) = { let s = st.lock().unwrap(); (s.store.clone(), s.ch_url.clone()) };
            let coll = query_param(query, "collection").map(|c| nfc(&pdec(&c)))
                .filter(|c| !c.is_empty()).unwrap_or_else(|| "*".to_string());
            let q = query_param(query, "q").map(|v| pdec(&v)).unwrap_or_default();
            if store != "clickhouse" || q.is_empty() {
                (400, json!({"error": "requer clickhouse + ?q="}).to_string())
            } else {
                match chdb::suggest_json(&ch_url, &coll, &q, 8) {
                    Ok(v) => (200, v.to_string()),
                    Err(e) => (500, json!({"error": format!("store: {e}")}).to_string()),
                }
            }
        }
        // [Think Navigator] expande UM nó do mindmap. collection é FILTRO opcional (default: todas).
        (Method::Get, "/api/nidhogg/node") => {
            let (store, ch_url) = { let s = st.lock().unwrap(); (s.store.clone(), s.ch_url.clone()) };
            let coll = query_param(query, "collection").map(|c| nfc(&pdec(&c)))
                .filter(|c| !c.is_empty()).unwrap_or_else(|| "*".to_string());
            let norm = query_param(query, "norm").map(|v| pdec(&v)).unwrap_or_default();
            if store != "clickhouse" || norm.is_empty() {
                (400, json!({"error": "requer clickhouse + ?norm="}).to_string())
            } else {
                match chdb::node_json(&ch_url, &coll, &norm, 24) {
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
            let tem_molde = chdb::get_templates(&ch_url).ok().map(|t| {
                let util = |k: &str| t.get(k).map(|m| m["origem"].as_str() != Some("reprovado")
                    && m["regras"].as_array().map(|a| !a.is_empty()).unwrap_or(false)).unwrap_or(false);
                util(tipo.as_str()) || (!forma.is_empty() && util(&format!("{tipo}@{forma}")))
            }).unwrap_or(false);
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
            let amostra = match fetch_base_text(&api, &coll, &base) { Some(t) => cap_amostra(&t), None => return (404, json!({"error":"amostra sem texto (base não encontrada no ragd)"}).to_string()) };
            let lib = read_prompts(&dir);
            let (sys, _from) = template_system(&lib);
            let (schema, regras) = match llm_make_template(&llm_url, &format!("molde-dirigido {coll}/{base} tipo={tipo}"), &sys, &tipo, &amostra, &instrucao) {
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
        // ── [L4] cadastro de PERGUNTAS (blob versionado, como doctypes/dimensões) ──
        (Method::Get, "/api/nidhogg/perguntas") => {
            let (store, ch_url) = { let s = st.lock().unwrap(); (s.store.clone(), s.ch_url.clone()) };
            if store != "clickhouse" { return (200, json!({"perguntas": []}).to_string()); }
            match chdb::perguntas(&ch_url) {
                Ok(p) => (200, json!({"perguntas": p}).to_string()),
                Err(e) => (500, json!({"error": format!("store: {e}")}).to_string()),
            }
        }
        (Method::Post, "/api/nidhogg/perguntas") => {
            let v: Value = match serde_json::from_str(body) { Ok(v) => v, Err(e) => return (400, json!({"error":format!("JSON inválido: {e}")}).to_string()) };
            let ps = match v["perguntas"].as_array() { Some(a) => a.clone(), None => return (400, json!({"error":"falta 'perguntas' (array)"}).to_string()) };
            // saneia: nome e texto obrigatórios; tipo dentro do vocabulário; escopo default '*'
            let mut limpo: Vec<Value> = vec![];
            for p in &ps {
                let nome = p["nome"].as_str().unwrap_or("").trim().to_string();
                let texto = p["texto"].as_str().unwrap_or("").trim().to_string();
                if nome.is_empty() || texto.is_empty() { continue; }
                let tipo = match p["tipo"].as_str().unwrap_or("vivo") {
                    t @ ("tabular" | "oneshot" | "vivo") => t, _ => "vivo",
                };
                limpo.push(json!({
                    "nome": nome, "texto": texto, "tipo": tipo,
                    "escopo": nfc(p["escopo"].as_str().unwrap_or("*").trim()),
                    "ativa": p["ativa"].as_bool().unwrap_or(true),
                    "pai": p["pai"].as_str().unwrap_or(""),   // recursão declarada: filha de qual pergunta
                }));
            }
            let (store, ch_url) = { let s = st.lock().unwrap(); (s.store.clone(), s.ch_url.clone()) };
            if store != "clickhouse" { return (400, json!({"error":"perguntas requerem clickhouse"}).to_string()); }
            match chdb::write_perguntas(&ch_url, &json!(limpo)) {
                Ok(_) => { nlog(&format!("L4: cadastro de perguntas atualizado ({} ativa(s))", limpo.len()));
                           (200, json!({"ok": true, "perguntas": limpo}).to_string()) }
                Err(e) => (500, json!({"error": format!("store: {e}")}).to_string()),
            }
        }
        // [L4] a TIMELINE de uma pergunta — as etapas em ordem cronológica
        (Method::Get, "/api/nidhogg/respostas") => {
            let (store, ch_url) = { let s = st.lock().unwrap(); (s.store.clone(), s.ch_url.clone()) };
            let p = query_param(query, "pergunta").map(|v| nfc(&pdec(&v))).unwrap_or_default();
            if p.is_empty() { return (400, json!({"error":"falta ?pergunta="}).to_string()); }
            if store != "clickhouse" { return (200, json!({"etapas": []}).to_string()); }
            match chdb::timeline(&ch_url, &p, 100) {
                Ok(v) => (200, v.to_string()),
                Err(e) => (500, json!({"error": format!("store: {e}")}).to_string()),
            }
        }
        // [L4] LIMPA o que uma pergunta gerou — apaga a timeline inteira e destrava a pergunta.
        // POST (e não DELETE) porque o preflight CORS aqui só libera GET/POST, e a API do
        // nidhoggd inteira é GET/POST. Duas coisas TÊM que andar juntas (senão a pergunta fica
        // muda pra sempre): apagar as linhas E esquecer o fingerprint em memória — o gate de
        // saturação compara `l4_visto[nome] == fp` e, sem a limpeza, ele diria "já vi este
        // escopo" para uma pergunta que não tem mais resposta nenhuma.
        (Method::Post, "/api/nidhogg/respostas/limpar") => {
            let v: Value = match serde_json::from_str(body) { Ok(v) => v, Err(e) => return (400, json!({"error":format!("JSON inválido: {e}")}).to_string()) };
            // trim e mais nada: o nome tem que ser BYTE-IDÊNTICO ao que /perguntar casa e ao que
            // `insert_resposta` gravou (ambos usam `p["nome"]` cru). Normalizar aqui (nfc) criaria
            // uma segunda convenção — e o desencontro seria SILENCIOSO: a mutation casaria zero
            // linhas e o `remove` do cache seria no-op, devolvendo ok com 0 etapas apagadas.
            let nome = v["pergunta"].as_str().unwrap_or("").trim().to_string();
            if nome.is_empty() { return (400, json!({"error":"falta 'pergunta' (o nome cadastrado)"}).to_string()); }
            let (store, ch_url) = { let s = st.lock().unwrap(); (s.store.clone(), s.ch_url.clone()) };
            if store != "clickhouse" { return (400, json!({"error":"L4 requer clickhouse"}).to_string()); }
            match chdb::delete_respostas(&ch_url, &nome) {
                Ok(n) => {
                    if let Ok(mut m) = l4_visto().lock() { m.remove(&nome); }
                    nlog(&format!("L4 {nome}: timeline limpa ({n} etapa(s) apagada(s)) — responde do zero no próximo ciclo"));
                    (200, json!({"ok":true,"pergunta":nome,"etapas_apagadas":n}).to_string())
                }
                Err(e) => (500, json!({"error": format!("store: {e}")}).to_string()),
            }
        }
        // [L4] responde UMA pergunta AGORA (o operador não espera o ciclo). LENTO: contexto +
        // analista + comparador. Grava etapa mesmo sem mudança (forcar=true) pra dar retorno visível.
        (Method::Post, "/api/nidhogg/perguntar") => {
            let v: Value = match serde_json::from_str(body) { Ok(v) => v, Err(e) => return (400, json!({"error":format!("JSON inválido: {e}")}).to_string()) };
            let nome = v["pergunta"].as_str().unwrap_or("").trim().to_string();
            if nome.is_empty() { return (400, json!({"error":"falta 'pergunta' (o nome cadastrado)"}).to_string()); }
            let (api, store, dir, llm_url, ch_url) = { let s = st.lock().unwrap();
                (s.ragd_api.clone(), s.store.clone(), s.dir.clone(), s.llm_url.clone(), s.ch_url.clone()) };
            if store != "clickhouse" { return (400, json!({"error":"L4 requer clickhouse"}).to_string()); }
            let ps = chdb::perguntas(&ch_url).unwrap_or_else(|_| json!([]));
            let p = match ps.as_array().and_then(|a| a.iter().find(|p| p["nome"].as_str() == Some(&nome))) {
                Some(p) => p.clone(),
                None => return (404, json!({"error": format!("pergunta '{nome}' não está no cadastro")}).to_string()),
            };
            let lib = read_prompts(&dir);
            let r = l4_processar(&api, &llm_url, &ch_url, &lib, &p, true);
            let code = if r["ok"].as_bool() == Some(true) { 200 } else { 502 };
            (code, r.to_string())
        }
        (Method::Post, "/api/nidhogg") => {
            let v: Value = match serde_json::from_str(body) { Ok(v) => v, Err(e) => return (400, json!({"error":format!("JSON inválido: {e}")}).to_string()) };
            let mut s = st.lock().unwrap();
            if let Some(on) = v["on"].as_bool() { s.on = on; let p = s.cfg_path.clone(); set_cfg_key(&p, "nidhogg", if on {"true"} else {"false"}); }
            if let Some(lv) = v["level"].as_str().map(level_num).or_else(|| v["level"].as_u64().map(|n| n as u8)) {
                let lv = lv.min(4); s.level = lv; let p = s.cfg_path.clone(); set_cfg_key(&p, "level", level_name(lv));
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
// Molde se aprende com a CABEÇA do documento (os campos rotulados aparecem cedo). Sem teto, uma
// base classificada errado (livro de 1,8M chars…) derruba o transporte antes de chegar no LLM.
const TEMPLATE_SAMPLE_MAX_CHARS: usize = 12_000;
fn cap_amostra(s: &str) -> String {
    if s.chars().count() <= TEMPLATE_SAMPLE_MAX_CHARS { s.to_string() }
    else { s.chars().take(TEMPLATE_SAMPLE_MAX_CHARS).collect() }
}
/// Marca uma tentativa de molde REPROVADA no registry (schema/regras vazios, origem="reprovado").
/// A presença da chave tira o cluster da fila (fim do re-try a cada ciclo); a extração IGNORA
/// moldes reprovados; o destrave é humano: molde dirigido (sobrescreve) ou re-tipagem.
fn marca_reprovado(ch_url: &str, reg_key: &str, cobertura: f64, motivo: &str) {
    let row = chdb::TemplateRow {
        tipo: reg_key.to_string(), schema: "[]".into(), regras: "[]".into(),
        cobertura, origem: "reprovado".into(), created_at: now_stamp(), version: chdb::now_version(),
    };
    match chdb::upsert_template(ch_url, &row) {
        Ok(_) => nlog(&format!("molde {reg_key}: REPROVADO gravado ({motivo}) — sai da fila; destrave via molde dirigido ou re-tipagem")),
        Err(e) => nlog(&format!("molde {reg_key}: falha ao gravar reprovação: {e}")),
    }
}
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
fn llm_make_template(llm_url: &str, ctx: &str, sys: &str, tipo: &str, amostra: &str, instrucao: &str) -> Result<(String, String), String> {
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
    let resp = llm_post("modelador", ctx, llm_url, &body, 180).ok_or_else(|| "sem resposta (template)".to_string())?;
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
    let amostra_full = match fetch_base_text(api, coll, &aname) { Some(t) => t, None => return json!({"ok": false, "collection": coll, "error": "sem amostra"}) };
    let amostra = cap_amostra(&amostra_full);
    if amostra.len() < amostra_full.len() {
        nlog(&format!("template {coll}/{tipo}: amostra {aname} capada em {TEMPLATE_SAMPLE_MAX_CHARS} chars (original {})", amostra_full.chars().count()));
    }
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
    let (schema, regras) = match llm_make_template(llm_url, &format!("mineração {coll} tipo={tipo}"), &sys, &tipo, &amostra, "") {
        Ok(x) => x,
        Err(e) => {
            nlog(&format!("template {coll}/{tipo}: {e}"));
            // sem memória disso, o cluster mais populoso re-tenta TODO ciclo e tranca a fila
            marca_reprovado(ch_url, &reg_key, 0.0, &format!("LLM falhou: {e}"));
            return json!({"ok": false, "collection": coll, "tipo": tipo, "error": e});
        }
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
        marca_reprovado(ch_url, &reg_key, cobertura, &format!("cobertura {:.0}% < gate", cobertura * 100.0));
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
    if !lib["templates"]["analista"].is_object() {
        lib["templates"]["analista"] = json!({
            "description": "L4: responde a questão direta cadastrada usando SÓ o contexto montado (agregados do dump + registros + trechos do corpus). Cita fontes e propõe dimensões não exploradas.",
            "system": BUILTIN_ANALISTA_PROMPT, "updated": now_stamp(), "max_tokens": 1500 });
        changed = true;
    }
    if !lib["templates"]["comparador"].is_object() {
        lib["templates"]["comparador"] = json!({
            "description": "L4: decide se a resposta deste ciclo MUDA A PERSPECTIVA da anterior — é o que faz a timeline registrar mudanças de entendimento, não repetições.",
            "system": BUILTIN_COMPARADOR_PROMPT, "updated": now_stamp(), "max_tokens": 300 });
        changed = true;
    }
    if !lib["templates"]["relacoes"].is_object() {
        lib["templates"]["relacoes"] = json!({
            "description": "Relações estruturais (L3, 100% LLM): destila quem-é-o-quê-de-quem e o tema da cena nas janelas mais densas do censo. Editar re-mastiga (checkpoint por hash do prompt).",
            "system": BUILTIN_RELACAO_PROMPT, "updated": now_stamp(), "max_tokens": 1200 });
        changed = true;
    }
    if changed { write_prompts(dir, &lib); }
}

const BUILTIN_FICHA_PROMPT: &str = "Você lê um TRECHO de uma obra narrativa em português. Extraia as ENTIDADES NOMEADAS \
(personagens, pessoas, organizações, lugares importantes) que aparecem NESTE trecho. Responda APENAS com um array JSON; \
cada elemento: {\"nome\": \"...\", \"atributos\": [\"característica citada no trecho\", ...], \"relacoes\": [\"nome de outra entidade ligada a esta no trecho\", ...]}. \
Seja FIEL ao trecho: só atributos e relações que o texto afirma. Sem entidades → [].";

// [L4] O analista: responde a questão direta SÓ com o que o contexto determinístico trouxe.
// A regra do "não sei" é o que separa análise de invenção — e é o que torna a timeline
// confiável (uma resposta que muda porque o dado chegou, não porque o modelo alucinou).
const BUILTIN_ANALISTA_PROMPT: &str = "Você é o analista do RAGnaRock. Responde a PERGUNTA usando EXCLUSIVAMENTE o \
CONTEXTO fornecido. \
HIERARQUIA DE CONFIANÇA (a regra mais importante): os TRECHOS DO CORPUS são PROVA — texto original. As RELAÇÕES DESTILADAS são \
apenas PISTAS de uma leitura automática anterior e PODEM ESTAR ERRADAS. Quando uma pista contradisser o texto, o TEXTO VENCE; \
quando uma pista não tiver apoio no texto, NÃO a afirme como fato. \
DIREÇÃO: a relação 'A —[laço]→ B' significa que A exerce o laço sobre B, nessa ordem. NUNCA inverta os lados. \
Regras: (1) NUNCA invente número, nome ou fato fora do contexto — se o material não permite responder, diga exatamente o que falta; \
(2) ao somar ou contar, use os REGISTROS DO DUMP e diga quantos registros entraram na conta; (3) cite as FONTES (base de cada \
afirmação); (4) em \"proximas\", liste até 3 DIMENSÕES NÃO EXPLORADAS que os dados permitem investigar. \
Responda APENAS o JSON pedido, em português.";

// [L4] O comparador: o guardião da timeline. Só uma MUDANÇA DE PERSPECTIVA vira etapa —
// reformulação, sinônimo ou ordem diferente das mesmas frases NÃO conta.
const BUILTIN_COMPARADOR_PROMPT: &str = "Você compara duas respostas para a MESMA pergunta, produzidas em ciclos diferentes. \
Decida se a resposta de AGORA MUDA A PERSPECTIVA em relação à anterior. Mudança de perspectiva = número/fato diferente, conclusão \
diferente, informação nova relevante, ou contradição. NÃO é mudança: reformulação, sinônimo, ordem das frases, detalhe irrelevante. \
Responda APENAS JSON: {\"mudou\": true|false, \"o_que_mudou\": \"em uma frase, o que mudou (vazio se não mudou)\"}. Seja RIGOROSO: \
na dúvida, mudou=false.";

// [L3] O prompt do destilador de relações — a régua manda: MESMO objetivo do L2 (grafar
// relações), mas 100% LLM. O trecho vem das cenas mais densas do censo (chunks onde mais
// entidades co-ocorrem) e a lista de presentes ANCORA os nomes (o nó do LLM COLA no nó do
// censo pela mesma normalização).
const BUILTIN_RELACAO_PROMPT: &str = "Você lê um TRECHO de uma obra em português e a lista das ENTIDADES presentes nele. \
Destile as RELAÇÕES que o texto AFIRMA entre essas entidades — o que uma contagem determinística não alcança: quem é o quê de quem, \
quem fez o quê a quem, e o TEMA da cena. Responda APENAS JSON: {\"relacoes\":[{\"a\":\"entidade\",\"rel\":\"laço curto (1-4 palavras: \
mentor de, viaja com, trai, serve…)\",\"b\":\"entidade\",\"tema\":\"tema da cena em 1-3 palavras (opcional)\"}]}. \
Use os nomes EXATOS da lista de entidades; seja FIEL ao trecho (só o que ele afirma); sem relações → {\"relacoes\":[]}.";

/// System do destilador de relações (L3): template "relacoes" editável ou o BUILTIN.
fn relacao_system(lib: &Value) -> (String, u32) {
    let sys = match lib["templates"]["relacoes"]["system"].as_str() {
        Some(s) if !s.trim().is_empty() => s.to_string(),
        _ => BUILTIN_RELACAO_PROMPT.to_string(),
    };
    let max_tokens = lib["templates"]["relacoes"]["max_tokens"].as_u64().unwrap_or(1200) as u32;
    (sys, max_tokens)
}

/// [v8] Chunks SEPARADOS de uma base (id + texto) — o censo de menções varre por chunk
/// pra gravar as POSIÇÕES (a posição do chunk é o eixo do tempo narrativo).
fn fetch_base_chunks(api: &str, coll: &str, name: &str) -> Option<Vec<(usize, String)>> {
    let req = json!({"collection": coll, "base": name, "id": 0, "after": 999_999}).to_string();
    let v: Value = serde_json::from_str(&http_post_t(&format!("{api}/chunk"), &req, 120)?).ok()?;
    let chunks = v["chunks"].as_array()?;
    Some(chunks.iter().filter_map(|c| {
        let id = c["id"].as_u64()? as usize;
        let t = c["text"].as_str()?.to_string();
        if t.trim().is_empty() { None } else { Some((id, t)) }
    }).collect())
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
fn llm_classify(llm_url: &str, ctx: &str, sys: &str, text: &str, naturezas: &[String], tipos: &[String])
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
    let resp = llm_post("classificador", ctx, llm_url, &body, CLASSIFY_TIMEOUT_S)
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
    let (nat, tip) = llm_classify(llm_url, &format!("classe {coll}/{name}"), sys, &text, naturezas, tipos)?;
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
fn llm_extract_records(llm_url: &str, ctx: &str, sys: &str, tipo: &str, text: &str) -> Result<Vec<Value>, String> {
    let sys_r = sys.replace("{tipo}", tipo);
    let body = json!({
        "messages": [{"role":"system","content":sys_r},{"role":"user","content":format!("DOCUMENTO:\n{text}")}],
        "temperature": 0, "max_tokens": EXTRACT_MAX_TOKENS
    }).to_string();
    let to = ((text.len() / 90) + (EXTRACT_MAX_TOKENS as usize / 10) + 90).min(400) as u32;
    let resp = llm_post("extrator", ctx, llm_url, &body, to).ok_or(format!("sem resposta (timeout {to}s)"))?;
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
            // molde ÚTIL = existe, tem regras e não é uma reprovação gravada (fila liberada ≠ extraível)
            let util = |k: &str| templates.get(k).map(|t|
                t["origem"].as_str() != Some("reprovado")
                && t["regras"].as_array().map(|a| !a.is_empty()).unwrap_or(false)).unwrap_or(false);
            let composta = if forma.is_empty() { String::new() } else { format!("{tipo}@{forma}") };
            let mkey = if !composta.is_empty() && util(&composta) { composta }
                       else if util(tipo.as_str()) { tipo.clone() }
                       else { continue };
            let t = &templates[mkey.as_str()];
            let ecfg = hash_hex(&format!("template|v2nqi|{mkey}|{}", hash_hex(&t["regras"].to_string())));
            extraiveis.insert(name, (tipo, false, ecfg, mkey));
        }
    }
    // ponto cego: natureza=tabela sem csv E sem molde → ninguém extrai (VISÍVEL, não silencioso)
    let blind = bases_class.iter()
        .filter(|b| b["natureza"].as_str() == Some("tabela") && !is_csv(b)
                && !templates.get(b["tipo"].as_str().unwrap_or("")).map(|t|
                    t["origem"].as_str() != Some("reprovado")
                    && t["regras"].as_array().map(|a| !a.is_empty()).unwrap_or(false)).unwrap_or(false)).count();
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
            Some((_, _, ecfg, _)) => chdb::needs_extract(ch_url, coll, &name, &base_state_hash(b), ecfg, false).unwrap_or(true),
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
                    // [L2] KnowledgeTree: censo de menções (determinístico) e a ligação por
                    // valores-chave (zero IA, incremental por fingerprint).
                    if level >= 2 && store == "clickhouse" {
                        let _f = mine_fichas(&api, &llm_url, &ch_url, &lib, coll);
                        // [L3] estrutural-LLM: destila relações das cenas densas (100% LLM,
                        // 1 base/ciclo) ANTES do link — o mesmo mine_links cola tudo no grafo.
                        if level >= 3 {
                            let _r = mine_relacoes(&api, &llm_url, &ch_url, &lib, coll);
                        }
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
    // [L4] As perguntas cadastradas são GLOBAIS (cada uma declara seu próprio escopo), então
    // rodam UMA vez por ciclo, fora do laço de coleções. Responde todas; só materializa etapa
    // onde a perspectiva mudou.
    let mut l4 = json!(null);
    if level >= 4 && store == "clickhouse" {
        l4 = mine_respostas(&api, &llm_url, &ch_url, &lib);
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
           "classified": classified, "extracted": extracted_ents, "l4": l4,
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
          chaves: port, ragd_api, nidhogg(on/off), level(minerador|consciente|estrutural|estrutural-llm), dir, cadence, cors_origin
  nasce DESLIGADO (precisa de IA). Liga pelo ValHalla ou pelo cfg.
rotas:
  GET  /health
  GET  /api/nidhogg                 status (nível, cadência, keepalive do ragd, conhecimento)
  GET  /api/nidhogg/collections     coleções do ragd + estado de digestão (liga/desliga por coleção)
  GET  /api/nidhogg/knowledge       conhecimento destilado (?collection=&type=&level=) — só leitura
  GET  /api/nidhogg/relacoes        relações destiladas pelo L3 (?collection=&n=) — só leitura
  GET  /api/nidhogg/perguntas       cadastro de questões diretas do L4
  POST /api/nidhogg/perguntas       {{\"perguntas\":[{{nome,texto,tipo:tabular|oneshot|vivo,escopo,ativa}}]}}
  GET  /api/nidhogg/respostas       timeline de uma pergunta (?pergunta=nome)
  POST /api/nidhogg/respostas/limpar {{\"pergunta\":nome}} apaga a timeline (pergunta volta a responder do zero)
  POST /api/nidhogg/perguntar       {{\"pergunta\":\"nome\"}} responde AGORA (lento: analista + comparador)
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
    // diário de mastigação do LLM: <dir>/llm-ledger.jsonl (todas as consultas/respostas de IA)
    let _ = std::fs::create_dir_all(&cfg.dir);
    let _ = LLM_LEDGER.set(format!("{}/llm-ledger.jsonl", cfg.dir.trim_end_matches('/')));
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
    let _ = LLM_TAG.set(cfg.llm_tag.clone());   // publica o rótulo do modelo pra todas as camadas
    let _ = LLM_KEY.set(cfg.llm_key.clone());   // idem credencial e dialeto do provedor
    let _ = LLM_TEMP.set(cfg.llm_temp);
    let _ = LLM_EXTRA.set(cfg.llm_extra.clone());
    let state = Arc::new(Mutex::new(State {
        on: cfg.on, level: cfg.level, dir: cfg.dir.clone(), cadence: cfg.cadence,
        ragd_api: cfg.ragd_api.clone(), llm_url: cfg.llm_url.clone(),
        llm_tag: cfg.llm_tag.clone(),
        store: cfg.store.clone(), ch_url: cfg.ch_url.clone(), cfg_path: cfg.cfg_path.clone(),
        started: Instant::now(), last_cycle: String::new(),
        ragd_online: false, ragd_health: Value::Null, cycle_running: false,
        llm_online: false, llm_erro: String::new(), llm_checked: String::new(),
    }));

    println!("🐉 Níðhöggr {VERSION} — camada de inteligência (daemon de módulos)");
    println!("   estado: {} · nível {} · cadência {}s · ragd {} · conhecimento em {:?}",
             if cfg.on {"LIGADO"} else {"desligado"}, level_name(cfg.level), cfg.cadence, cfg.ragd_api, cfg.dir);
    println!("   IA: llm_tag={} · {}", cfg.llm_tag, cfg.llm_url);

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
