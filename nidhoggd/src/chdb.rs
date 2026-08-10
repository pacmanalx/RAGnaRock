//! Backend ClickHouse do acumulado do Nidhogg — fala HTTP (:8123) reusando o padrão do curl
//! (sem deps async). É o STORE denso onde a camada de significado mora; as inferências leem daqui.
//!
//! Modelo de dedup: `ReplacingMergeTree(version)` com `version` = epoch em MICROSSEGUNDOS (NÃO
//! `now()`, que tem resolução de 1s e empataria dois inserts no mesmo segundo). Reclassificar =
//! INSERT com version maior; a leitura usa `FINAL` pra ver a versão consolidada.
//!
//! O `doctype` é um BLOB JSON versionado (uma linha = a lista inteira): editar é INSERT de uma
//! versão nova — nunca TRUNCATE, então uma falha de escrita não deixa o vocabulário vazio.

use serde_json::{json, Value};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

/// version monotônico ~único: epoch em microssegundos. Colisão exigiria dois inserts da MESMA
/// base no mesmo microssegundo — impossível na prática (cada classify leva segundos).
pub fn now_version() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_micros() as u64).unwrap_or(0)
}

/// Uma classe pronta pra gravar (uma linha do lote de INSERT).
pub struct ClassRow {
    pub collection: String,
    pub name: String,
    pub state_hash: String,
    pub cfg_hash: String,
    pub natureza: String,
    pub tipo: String,
    pub csv: bool,        // determinístico (tabular_spec): é um CSV regular → gate da Fase 2
    pub origem: String,   // "llm" (classificou) | "humano" (re-tipado no cockpit — o LLM NÃO sobrescreve)
    pub confianca: f64,
    pub classified_at: String,
    pub version: u64,
}

/// Executa SQL no ClickHouse via HTTP (curl POST, body = SQL). Vazio é SUCESSO (INSERT/DDL não
/// retornam corpo). Erro do ClickHouse vem com "DB::Exception" no corpo → vira Err.
fn ch_exec(url: &str, sql: &str, secs: u32) -> Result<String, String> {
    let out = Command::new("curl")
        .args(["-s", "-m", &secs.to_string(), url, "--data-binary", sql])
        .output()
        .map_err(|e| format!("curl falhou: {e}"))?;
    if !out.status.success() {
        return Err(format!("curl status {:?}", out.status.code()));
    }
    let body = String::from_utf8_lossy(&out.stdout).to_string();
    if body.contains("DB::Exception") || body.starts_with("Code:") {
        return Err(body.chars().take(200).collect());
    }
    Ok(body)
}

/// INSERT em LOTE via JSONEachRow (a query na URL, as linhas no body). ClickHouse detesta inserts
/// unitários ("too many parts") — SEMPRE agregamos o ciclo num único INSERT. O NDJSON vai por STDIN
/// (`--data-binary @-`), NÃO como argumento: um batch grande (base com muitas linhas + prov) passa de
/// 128KB e estouraria o MAX_ARG_STRLEN do Linux — o insert falharia silencioso e a base travaria.
fn ch_insert(url: &str, table: &str, rows_ndjson: &str, secs: u32) -> Result<(), String> {
    use std::io::Write;
    let q = format!("INSERT INTO {table} FORMAT JSONEachRow");
    let full = format!("{url}?query={}", urlencode(&q));
    let mut child = Command::new("curl")
        .args(["-s", "-m", &secs.to_string(), &full, "--data-binary", "@-"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| format!("curl spawn: {e}"))?;
    {
        let mut si = child.stdin.take().ok_or_else(|| "sem stdin".to_string())?;
        si.write_all(rows_ndjson.as_bytes()).map_err(|e| format!("write stdin: {e}"))?;
    }   // si sai de escopo aqui → fecha o stdin → o curl processa e sai
    let out = child.wait_with_output().map_err(|e| format!("curl wait: {e}"))?;
    let body = String::from_utf8_lossy(&out.stdout).to_string();
    if !out.status.success() || body.contains("DB::Exception") {
        return Err(format!("insert: {}", body.chars().take(200).collect::<String>()));
    }
    Ok(())
}

/// URL-encode mínimo (só o que aparece em query SQL: espaço, e caracteres de sintaxe).
fn urlencode(s: &str) -> String {
    let mut o = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => o.push(b as char),
            _ => o.push_str(&format!("%{:02X}", b)),
        }
    }
    o
}

/// SELECT que usa PARÂMETROS do ClickHouse (`{p:String}` + `param_p=`) — sem interpolar valores no
/// SQL (nada de injection via nome de base). Devolve o corpo (TabSeparated/JSON conforme a query).
fn ch_query_param(url: &str, sql: &str, params: &[(&str, &str)], secs: u32) -> Result<String, String> {
    let mut full = format!("{url}?query={}", urlencode(sql));
    for (k, v) in params {
        full.push_str(&format!("&param_{}={}", k, urlencode(v)));
    }
    let out = Command::new("curl")
        .args(["-s", "-m", &secs.to_string(), &full])
        .output()
        .map_err(|e| format!("curl falhou: {e}"))?;
    let body = String::from_utf8_lossy(&out.stdout).to_string();
    if body.contains("DB::Exception") {
        return Err(body.chars().take(200).collect());
    }
    Ok(body)
}

const SEED_NATUREZAS: &[&str] = &["tabular", "narrativo", "codigo"];
const SEED_TIPOS: &[&str] = &[
    "cadastro", "contrato", "comprovante", "nota_fiscal", "recibo", "boleto", "balanco",
    "extrato", "dre", "folha_pagamento", "ordem_compra", "cotacao", "relatorio", "livro",
    "artigo", "ata", "carta", "oficio", "memorial", "curriculo", "discurso", "codigo_fonte",
    "config", "log", "outro",
];

/// Cria as tabelas (idempotente) e faz o seed do doctype na primeira vez.
pub fn ensure_schema(url: &str) -> Result<(), String> {
    ch_exec(url,
        "CREATE DATABASE IF NOT EXISTS nidhogg", 15)?;
    ch_exec(url,
        "CREATE TABLE IF NOT EXISTS nidhogg.doc_class (collection String, name String, \
         state_hash String, cfg_hash String, natureza String, tipo String, csv UInt8 DEFAULT 0, \
         origem String DEFAULT 'llm', confianca Float64, classified_at String, version UInt64) \
         ENGINE=ReplacingMergeTree(version) ORDER BY (collection, name)", 15)?;
    // migrações idempotentes: colunas que doc_class antigo não tinha
    ch_exec(url, "ALTER TABLE nidhogg.doc_class ADD COLUMN IF NOT EXISTS csv UInt8 DEFAULT 0", 15)?;
    ch_exec(url, "ALTER TABLE nidhogg.doc_class ADD COLUMN IF NOT EXISTS origem String DEFAULT 'llm'", 15)?;
    ch_exec(url,
        "CREATE TABLE IF NOT EXISTS nidhogg.doctype (version UInt64, naturezas String, tipos String) \
         ENGINE=ReplacingMergeTree(version) ORDER BY tuple()", 15)?;
    // seed se vazio
    let n = ch_exec(url, "SELECT count() FROM nidhogg.doctype", 10)?;
    if n.trim() == "0" {
        let nat: Vec<String> = SEED_NATUREZAS.iter().map(|s| s.to_string()).collect();
        let tip: Vec<String> = SEED_TIPOS.iter().map(|s| s.to_string()).collect();
        write_doctypes(url, &nat, &tip)?;
    }
    Ok(())
}

/// Lê a lista de doctypes (a versão mais recente do blob). Vazio → seed vazio (o chamador trata).
pub fn doctypes(url: &str) -> Result<(Vec<String>, Vec<String>), String> {
    let body = ch_exec(url,
        "SELECT naturezas, tipos FROM nidhogg.doctype FINAL ORDER BY version DESC LIMIT 1 FORMAT JSONEachRow", 10)?;
    let line = body.lines().next().unwrap_or("{}");
    let v: Value = serde_json::from_str(line).unwrap_or_else(|_| json!({}));
    let nat = parse_json_arr(v["naturezas"].as_str().unwrap_or("[]"));
    let tip = parse_json_arr(v["tipos"].as_str().unwrap_or("[]"));
    Ok((nat, tip))
}

fn parse_json_arr(s: &str) -> Vec<String> {
    serde_json::from_str::<Value>(s).ok()
        .and_then(|v| v.as_array().map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect()))
        .unwrap_or_default()
}

/// Grava uma nova versão do vocabulário (blob JSON). Atômico: a versão anterior segue válida até
/// esta linha existir — uma falha de escrita NÃO deixa o vocabulário vazio.
pub fn write_doctypes(url: &str, naturezas: &[String], tipos: &[String]) -> Result<(), String> {
    let row = json!({
        "version": now_version(),
        "naturezas": serde_json::to_string(naturezas).unwrap_or_else(|_| "[]".into()),
        "tipos": serde_json::to_string(tipos).unwrap_or_else(|_| "[]".into()),
    });
    ch_insert(url, "nidhogg.doctype", &row.to_string(), 15)
}

/// Uma base precisa de (re)classificação se não há linha OU state_hash/cfg_hash divergem.
/// Resultado vazio (FINAL sem match) = precisa — mapeado INTENCIONALMENTE, não por acidente.
pub fn needs_class(url: &str, collection: &str, name: &str, state_hash: &str, cfg_hash: &str) -> Result<bool, String> {
    let body = ch_query_param(url,
        "SELECT state_hash, cfg_hash, origem FROM nidhogg.doc_class FINAL \
         WHERE collection={coll:String} AND name={name:String} LIMIT 1 FORMAT TabSeparated",
        &[("coll", collection), ("name", name)], 10)?;
    let line = body.lines().next().unwrap_or("");
    if line.is_empty() { return Ok(true); }   // sem linha = nunca classificada
    let mut it = line.split('\t');
    let sh = it.next().unwrap_or("");
    let ch = it.next().unwrap_or("");
    let origem = it.next().unwrap_or("");
    if origem == "humano" { return Ok(false); }   // re-tipagem manual: o LLM NÃO sobrescreve
    Ok(sh != state_hash || ch != cfg_hash)
}

/// Lê (state_hash, cfg_hash) da classe atual de uma base — pra re-tipagem manual preservar os hashes
/// (não fingimos conteúdo). Sem linha ⇒ ("", "").
pub fn get_class_hashes(url: &str, collection: &str, name: &str) -> Result<(String, String), String> {
    let body = ch_query_param(url,
        "SELECT state_hash, cfg_hash FROM nidhogg.doc_class FINAL \
         WHERE collection={coll:String} AND name={name:String} LIMIT 1 FORMAT TabSeparated",
        &[("coll", collection), ("name", name)], 10)?;
    let line = body.lines().next().unwrap_or("");
    let mut it = line.split('\t');
    Ok((it.next().unwrap_or("").to_string(), it.next().unwrap_or("").to_string()))
}

/// Grava um LOTE de classes (um único INSERT). Reclassificar é INSERT com version maior.
pub fn insert_classes(url: &str, rows: &[ClassRow]) -> Result<(), String> {
    if rows.is_empty() { return Ok(()); }
    let mut ndjson = String::new();
    for r in rows {
        let line = json!({
            "collection": r.collection, "name": r.name, "state_hash": r.state_hash,
            "cfg_hash": r.cfg_hash, "natureza": r.natureza, "tipo": r.tipo,
            "csv": if r.csv { 1 } else { 0 }, "origem": r.origem,
            "confianca": r.confianca, "classified_at": r.classified_at, "version": r.version,
        });
        ndjson.push_str(&line.to_string());
        ndjson.push('\n');
    }
    ch_insert(url, "nidhogg.doc_class", &ndjson, 30)
}

/// Distribuição {natureza,tipo} + linhas, por coleção (ou todas se None/"*"). Usa FINAL pra ver o
/// consolidado (dedup do ReplacingMergeTree é eventual).
pub fn classes_summary(url: &str, collection: Option<&str>) -> Result<Value, String> {
    let all = matches!(collection, None | Some("*"));
    let coll = collection.unwrap_or("*");
    let where_c = if all { String::new() } else { " WHERE collection={coll:String}".into() };
    let params: Vec<(&str, &str)> = if all { vec![] } else { vec![("coll", coll)] };

    let count_by = |field: &str| -> Result<Value, String> {
        let sql = format!("SELECT {field}, count() c FROM nidhogg.doc_class FINAL{where_c} \
                           GROUP BY {field} ORDER BY c DESC FORMAT JSON");
        let body = ch_query_param(url, &sql, &params, 15)?;
        let v: Value = serde_json::from_str(&body).map_err(|e| format!("json: {e}"))?;
        let mut map = serde_json::Map::new();
        for row in v["data"].as_array().map(|a| a.as_slice()).unwrap_or(&[]) {
            let k = row[field].as_str().unwrap_or("?").to_string();
            let c = row["c"].as_str().and_then(|s| s.parse::<i64>().ok())
                .or_else(|| row["c"].as_i64()).unwrap_or(0);
            map.insert(k, json!(c));
        }
        Ok(Value::Object(map))
    };

    let total_body = ch_query_param(url,
        &format!("SELECT count() FROM nidhogg.doc_class FINAL{where_c} FORMAT TabSeparated"),
        &params, 10)?;
    let total: i64 = total_body.trim().parse().unwrap_or(0);

    let bases_sql = format!(
        "SELECT collection, name, natureza, tipo, csv, origem, confianca, classified_at FROM nidhogg.doc_class FINAL{where_c} \
         ORDER BY collection, name FORMAT JSON");
    let bases_body = ch_query_param(url, &bases_sql, &params, 20)?;
    let bv: Value = serde_json::from_str(&bases_body).map_err(|e| format!("json bases: {e}"))?;
    let bases: Vec<Value> = bv["data"].as_array().cloned().unwrap_or_default();

    Ok(json!({
        "collection": coll, "count": total,
        "naturezas": count_by("natureza")?, "tipos": count_by("tipo")?,
        "bases": bases,
    }))
}

// ───────────────────────────── Fase 2: extração de entidades (o dump denso) ─────────────────────────────
// `version` POR EXTRAÇÃO da base (todas as entidades de uma extração compartilham version). A view
// `entidade_atual` mostra só a extração mais RECENTE de cada base — re-extrair com MENOS registros
// não deixa fantasmas (o que ReplacingMergeTree por (base,idx) deixaria) e sem mutation. Toda leitura
// passa pela view: a prova de completude (COUNT) e o /entities veem o mesmo número.

pub struct EntidadeRow {
    pub collection: String,
    pub base: String,
    pub tipo: String,
    pub idx: u32,
    pub dado: String,        // registro como JSON — JÁ VALIDADO pelo chamador (serde_json)
    pub modo: String,        // "det" (parser CSV, 100% confiável) | "template" (molde regex) | "llm"
    pub nqi: f64,            // Fase 5: NQI = cobertura × precisão DESTE registro (0..1), agregável
    pub prov: String,        // Fase 5: path-tree AUTOCONTIDO (origem de cada campo: via/regra/válido)
    pub state_hash: String,
    pub ext_cfg_hash: String,
    pub version: u64,
    pub extracted_at: String,
}

pub fn ensure_entidade_schema(url: &str) -> Result<(), String> {
    ch_exec(url,
        "CREATE TABLE IF NOT EXISTS nidhogg.entidade (collection String, base String, tipo String, \
         idx UInt32, dado String, modo String DEFAULT 'llm', nqi Float64 DEFAULT 0, prov String DEFAULT '', \
         state_hash String, ext_cfg_hash String, version UInt64, extracted_at String) \
         ENGINE=MergeTree ORDER BY (collection, base, version, idx)", 15)?;
    // migrações idempotentes: colunas que tabelas antigas não tinham
    ch_exec(url, "ALTER TABLE nidhogg.entidade ADD COLUMN IF NOT EXISTS modo String DEFAULT 'llm'", 15)?;
    ch_exec(url, "ALTER TABLE nidhogg.entidade ADD COLUMN IF NOT EXISTS nqi Float64 DEFAULT 0", 15)?;
    ch_exec(url, "ALTER TABLE nidhogg.entidade ADD COLUMN IF NOT EXISTS prov String DEFAULT ''", 15)?;
    // OR REPLACE: a view é SELECT * e precisa reincluir as colunas novas
    ch_exec(url,
        "CREATE OR REPLACE VIEW nidhogg.entidade_atual AS SELECT * FROM nidhogg.entidade \
         WHERE (collection, base, version) IN \
         (SELECT collection, base, max(version) FROM nidhogg.entidade GROUP BY collection, base)", 15)?;
    Ok(())
}

/// Precisa extrair se não há entidades atuais OU state_hash/ext_cfg_hash divergem. Vazio = precisa.
pub fn needs_extract(url: &str, collection: &str, base: &str, state_hash: &str, ext_cfg: &str) -> Result<bool, String> {
    let body = ch_query_param(url,
        "SELECT state_hash, ext_cfg_hash FROM nidhogg.entidade_atual \
         WHERE collection={coll:String} AND base={base:String} LIMIT 1 FORMAT TabSeparated",
        &[("coll", collection), ("base", base)], 10)?;
    let line = body.lines().next().unwrap_or("");
    if line.is_empty() { return Ok(true); }
    let mut it = line.split('\t');
    Ok(it.next().unwrap_or("") != state_hash || it.next().unwrap_or("") != ext_cfg)
}

/// INSERT em lote das entidades de UMA extração (mesmo version). All-or-nothing: o chamador só chega
/// aqui se TODAS as janelas da base extraíram — janela falha ⇒ nada é gravado.
pub fn insert_entities(url: &str, rows: &[EntidadeRow]) -> Result<(), String> {
    if rows.is_empty() { return Ok(()); }
    let mut ndjson = String::new();
    for r in rows {
        let line = json!({
            "collection": r.collection, "base": r.base, "tipo": r.tipo, "idx": r.idx,
            "dado": r.dado, "modo": r.modo, "nqi": r.nqi, "prov": r.prov,
            "state_hash": r.state_hash, "ext_cfg_hash": r.ext_cfg_hash,
            "version": r.version, "extracted_at": r.extracted_at,
        });
        ndjson.push_str(&line.to_string());
        ndjson.push('\n');
    }
    ch_insert(url, "nidhogg.entidade", &ndjson, 45)
}

/// Distribuição/amostra das entidades — SEMPRE pela view `entidade_atual` (uma fonte de verdade).
pub fn entities_summary(url: &str, collection: Option<&str>, base: Option<&str>) -> Result<Value, String> {
    let mut clauses: Vec<&str> = vec![];
    let mut params: Vec<(&str, &str)> = vec![];
    if let Some(c) = collection { if c != "*" { clauses.push("collection={coll:String}"); params.push(("coll", c)); } }
    if let Some(b) = base { clauses.push("base={base:String}"); params.push(("base", b)); }
    let where_c = if clauses.is_empty() { String::new() } else { format!(" WHERE {}", clauses.join(" AND ")) };

    // total + NQI GLOBAL (Fase 5): a saúde média da normalização no escopo
    let tot_body = ch_query_param(url,
        &format!("SELECT count() c, round(avg(nqi),3) nqi FROM nidhogg.entidade_atual{where_c} FORMAT JSONEachRow"), &params, 10)?;
    let tv: Value = serde_json::from_str(tot_body.lines().next().unwrap_or("{}")).unwrap_or_else(|_| json!({}));
    let total = tv["c"].as_i64().or_else(|| tv["c"].as_str().and_then(|s| s.parse().ok())).unwrap_or(0);
    let nqi_global = tv["nqi"].as_f64().unwrap_or(0.0);

    let bases_body = ch_query_param(url,
        &format!("SELECT collection, base, tipo, any(modo) modo, round(avg(nqi),3) nqi, count() c FROM nidhogg.entidade_atual{where_c} \
                  GROUP BY collection, base, tipo ORDER BY c DESC LIMIT 300 FORMAT JSON"), &params, 15)?;
    let bv: Value = serde_json::from_str(&bases_body).map_err(|e| format!("json: {e}"))?;
    let por_base = bv["data"].as_array().cloned().unwrap_or_default();

    // NQI por TIPO (o NQI da §4, por-tipo): revela quais moldes já amadureceram
    let tipo_body = ch_query_param(url,
        &format!("SELECT tipo, any(modo) modo, round(avg(nqi),3) nqi, count() c, uniqExact(base) bases FROM nidhogg.entidade_atual{where_c} \
                  GROUP BY tipo ORDER BY c DESC FORMAT JSON"), &params, 15)?;
    let pv: Value = serde_json::from_str(&tipo_body).map_err(|e| format!("json: {e}"))?;
    let por_tipo = pv["data"].as_array().cloned().unwrap_or_default();

    let mut amostra: Vec<Value> = vec![];
    if base.is_some() {
        let s = ch_query_param(url,
            &format!("SELECT idx, dado, nqi, prov FROM nidhogg.entidade_atual{where_c} ORDER BY idx LIMIT 60 FORMAT JSON"), &params, 15)?;
        let sv: Value = serde_json::from_str(&s).map_err(|e| format!("json: {e}"))?;
        for row in sv["data"].as_array().map(|a| a.as_slice()).unwrap_or(&[]) {
            let dado = row["dado"].as_str().and_then(|x| serde_json::from_str::<Value>(x).ok()).unwrap_or(Value::Null);
            let prov = row["prov"].as_str().and_then(|x| serde_json::from_str::<Value>(x).ok()).unwrap_or(Value::Null);
            amostra.push(json!({"idx": row["idx"], "dado": dado, "nqi": row["nqi"], "prov": prov}));
        }
    }
    Ok(json!({"count": total, "nqi_global": nqi_global, "por_base": por_base, "por_tipo": por_tipo, "amostra": amostra}))
}

// ───────── Fase 3: registry de templates de extração (o molde por TIPO) ─────────
// Um template = { schema (campos), regras (regex ancorado no rótulo + limpeza) }. O L1 cria/ajusta
// (LLM, 1× por tipo); o L0 aplica aos N documentos iguais (determinístico, zero-LLM). Versionado por
// ReplacingMergeTree: ajustar um molde é INSERT com version maior; o L0 lê o consolidado (FINAL).

pub struct TemplateRow {
    pub tipo: String,
    pub schema: String,     // JSON array de campos (nomes)
    pub regras: String,     // JSON array de {campo, regex, limpar[]}
    pub cobertura: f64,     // % média de campos preenchidos na amostra de validação
    pub origem: String,     // "llm" (criado) | "ajuste" (reparado sob gatilho)
    pub created_at: String,
    pub version: u64,
}

pub fn ensure_template_schema(url: &str) -> Result<(), String> {
    ch_exec(url,
        "CREATE TABLE IF NOT EXISTS nidhogg.template (tipo String, schema String, regras String, \
         cobertura Float64, origem String, created_at String, version UInt64) \
         ENGINE=ReplacingMergeTree(version) ORDER BY tipo", 15)?;
    Ok(())
}

/// Templates atuais (versão consolidada), por tipo → {schema, regras, cobertura}. É o registry que
/// o L0 lê pra decidir se um tipo já tem molde e aplicá-lo.
pub fn get_templates(url: &str) -> Result<Value, String> {
    let body = ch_exec(url,
        "SELECT tipo, schema, regras, cobertura, version FROM nidhogg.template FINAL FORMAT JSONEachRow", 15)?;
    let mut map = serde_json::Map::new();
    for line in body.lines() {
        if line.trim().is_empty() { continue; }
        let v: Value = match serde_json::from_str(line) { Ok(v) => v, Err(_) => continue };
        let tipo = match v["tipo"].as_str() { Some(t) if !t.is_empty() => t.to_string(), _ => continue };
        let schema = serde_json::from_str::<Value>(v["schema"].as_str().unwrap_or("[]")).unwrap_or_else(|_| json!([]));
        let regras = serde_json::from_str::<Value>(v["regras"].as_str().unwrap_or("[]")).unwrap_or_else(|_| json!([]));
        map.insert(tipo, json!({"schema": schema, "regras": regras, "cobertura": v["cobertura"], "version": v["version"]}));
    }
    Ok(Value::Object(map))
}

/// Grava/atualiza o molde de um tipo (INSERT com version nova; ReplacingMergeTree consolida).
pub fn upsert_template(url: &str, r: &TemplateRow) -> Result<(), String> {
    let row = json!({
        "tipo": r.tipo, "schema": r.schema, "regras": r.regras, "cobertura": r.cobertura,
        "origem": r.origem, "created_at": r.created_at, "version": r.version,
    });
    ch_insert(url, "nidhogg.template", &row.to_string(), 15)
}

/// Cockpit da ingestão (Fase 6) — os documentos que o motor QUERIA processar mas não viraram dado
/// útil. NÃO inclui narrativo/codigo (esses não geram registro por natureza — não são refugo).
///  · natureza=documento, tipo SEM molde no registry → "sem molde"
///  · natureza=documento, tipo COM molde mas NQI < 0.5 → "nqi baixo" (o molde não casou o doc)
///  · natureza=tabela mas csv=0 → "tabela não-CSV" (o ponto cego)
/// Documento com molde ainda-não-extraído é transitório (vai extrair) → NÃO é refugo.
pub fn rejeitados_summary(url: &str) -> Result<Value, String> {
    let templates = get_templates(url)?;
    // nqi médio por (coll, base) — só as extraídas
    let nqi_body = ch_exec(url,
        "SELECT collection, base, round(avg(nqi),3) nqi FROM nidhogg.entidade_atual \
         GROUP BY collection, base FORMAT JSONEachRow", 15)?;
    let mut nqi_map: std::collections::HashMap<(String, String), f64> = std::collections::HashMap::new();
    for l in nqi_body.lines() {
        if l.trim().is_empty() { continue; }
        if let Ok(v) = serde_json::from_str::<Value>(l) {
            if let (Some(c), Some(b)) = (v["collection"].as_str(), v["base"].as_str()) {
                nqi_map.insert((c.to_string(), b.to_string()), v["nqi"].as_f64().unwrap_or(0.0));
            }
        }
    }
    // candidatos: documento OU (tabela com csv=0)
    let body = ch_exec(url,
        "SELECT collection, name, natureza, tipo, csv FROM nidhogg.doc_class FINAL \
         WHERE natureza='documento' OR (natureza='tabela' AND csv=0) FORMAT JSONEachRow", 20)?;
    let mut lista: Vec<Value> = vec![];
    for l in body.lines() {
        if l.trim().is_empty() { continue; }
        let v: Value = match serde_json::from_str(l) { Ok(v) => v, Err(_) => continue };
        let coll = v["collection"].as_str().unwrap_or("");
        let name = v["name"].as_str().unwrap_or("");
        let natureza = v["natureza"].as_str().unwrap_or("");
        let tipo = v["tipo"].as_str().unwrap_or("");
        if name.is_empty() { continue; }
        let nqi = nqi_map.get(&(coll.to_string(), name.to_string())).copied();
        let motivo = if natureza == "tabela" {
            "tabela não-CSV"
        } else if templates.get(tipo).is_none() {
            "sem molde"
        } else {
            match nqi {
                Some(n) if n < 0.5 => "nqi baixo",   // extraiu, mas o molde não serviu
                Some(_) => continue,                  // extraiu bem — não é refugo
                None => continue,                     // com molde, ainda não extraído — transitório
            }
        };
        lista.push(json!({"collection": coll, "base": name, "natureza": natureza,
                          "tipo": tipo, "motivo": motivo, "nqi": nqi}));
    }
    // pior primeiro: nqi baixo (com nqi) antes de sem molde/ponto cego (sem nqi)
    lista.sort_by(|a, b| a["nqi"].as_f64().unwrap_or(9.0).partial_cmp(&b["nqi"].as_f64().unwrap_or(9.0)).unwrap());
    let mut por_motivo = serde_json::Map::new();
    for r in &lista {
        let m = r["motivo"].as_str().unwrap_or("?").to_string();
        let c = por_motivo.get(&m).and_then(|v| v.as_i64()).unwrap_or(0) + 1;
        por_motivo.insert(m, json!(c));
    }
    Ok(json!({"count": lista.len(), "por_motivo": por_motivo, "rejeitados": lista}))
}
