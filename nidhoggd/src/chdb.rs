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
/// unitários ("too many parts") — SEMPRE agregamos o ciclo num único INSERT.
fn ch_insert(url: &str, table: &str, rows_ndjson: &str, secs: u32) -> Result<(), String> {
    let q = format!("INSERT INTO {table} FORMAT JSONEachRow");
    // query vai como parâmetro; o body são as linhas NDJSON
    let full = format!("{url}?query={}", urlencode(&q));
    let out = Command::new("curl")
        .args(["-s", "-m", &secs.to_string(), &full, "--data-binary", rows_ndjson])
        .output()
        .map_err(|e| format!("curl falhou: {e}"))?;
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
         state_hash String, cfg_hash String, natureza String, tipo String, confianca Float64, \
         classified_at String, version UInt64) ENGINE=ReplacingMergeTree(version) \
         ORDER BY (collection, name)", 15)?;
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
        "SELECT state_hash, cfg_hash FROM nidhogg.doc_class FINAL \
         WHERE collection={coll:String} AND name={name:String} LIMIT 1 FORMAT TabSeparated",
        &[("coll", collection), ("name", name)], 10)?;
    let line = body.lines().next().unwrap_or("");
    if line.is_empty() { return Ok(true); }   // sem linha = nunca classificada
    let mut it = line.split('\t');
    let sh = it.next().unwrap_or("");
    let ch = it.next().unwrap_or("");
    Ok(sh != state_hash || ch != cfg_hash)
}

/// Grava um LOTE de classes (um único INSERT). Reclassificar é INSERT com version maior.
pub fn insert_classes(url: &str, rows: &[ClassRow]) -> Result<(), String> {
    if rows.is_empty() { return Ok(()); }
    let mut ndjson = String::new();
    for r in rows {
        let line = json!({
            "collection": r.collection, "name": r.name, "state_hash": r.state_hash,
            "cfg_hash": r.cfg_hash, "natureza": r.natureza, "tipo": r.tipo,
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
        "SELECT collection, name, natureza, tipo, confianca, classified_at FROM nidhogg.doc_class FINAL{where_c} \
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
    pub state_hash: String,
    pub ext_cfg_hash: String,
    pub version: u64,
    pub extracted_at: String,
}

pub fn ensure_entidade_schema(url: &str) -> Result<(), String> {
    ch_exec(url,
        "CREATE TABLE IF NOT EXISTS nidhogg.entidade (collection String, base String, tipo String, \
         idx UInt32, dado String, state_hash String, ext_cfg_hash String, version UInt64, \
         extracted_at String) ENGINE=MergeTree ORDER BY (collection, base, version, idx)", 15)?;
    ch_exec(url,
        "CREATE VIEW IF NOT EXISTS nidhogg.entidade_atual AS SELECT * FROM nidhogg.entidade \
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
            "dado": r.dado, "state_hash": r.state_hash, "ext_cfg_hash": r.ext_cfg_hash,
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

    let total: i64 = ch_query_param(url,
        &format!("SELECT count() FROM nidhogg.entidade_atual{where_c} FORMAT TabSeparated"), &params, 10)?
        .trim().parse().unwrap_or(0);
    let bases_body = ch_query_param(url,
        &format!("SELECT collection, base, tipo, count() c FROM nidhogg.entidade_atual{where_c} \
                  GROUP BY collection, base, tipo ORDER BY c DESC LIMIT 300 FORMAT JSON"), &params, 15)?;
    let bv: Value = serde_json::from_str(&bases_body).map_err(|e| format!("json: {e}"))?;
    let por_base = bv["data"].as_array().cloned().unwrap_or_default();

    let mut amostra: Vec<Value> = vec![];
    if base.is_some() {
        let s = ch_query_param(url,
            &format!("SELECT idx, dado FROM nidhogg.entidade_atual{where_c} ORDER BY idx LIMIT 60 FORMAT JSON"), &params, 15)?;
        let sv: Value = serde_json::from_str(&s).map_err(|e| format!("json: {e}"))?;
        for row in sv["data"].as_array().map(|a| a.as_slice()).unwrap_or(&[]) {
            let dado = row["dado"].as_str().and_then(|x| serde_json::from_str::<Value>(x).ok()).unwrap_or(Value::Null);
            amostra.push(json!({"idx": row["idx"], "dado": dado}));
        }
    }
    Ok(json!({"count": total, "por_base": por_base, "amostra": amostra}))
}
