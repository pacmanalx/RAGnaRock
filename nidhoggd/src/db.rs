//! Banco auxiliar do Nidhogg (SQLite via rusqlite bundled).
//!
//! É a **camada de significado** — separada do corpus, que continua no RAGnaRock (ragd).
//! Guarda a classificação `{natureza, tipo}` por base (a Fase 1 do motor auto-adaptativo) e a
//! lista EDITÁVEL de tipos documentais ("doctypes"), que alimenta o `enum` do constrained
//! decoding. O texto dos documentos NÃO é duplicado aqui — só a classe, apontando pro ragd.
//!
//! Conexões são efêmeras (abre/opera/fecha) em WAL: 1 escritor (o worker) + N leitores (os
//! endpoints do ValHalla) sem travar o `State` do daemon.

use rusqlite::{params, Connection, Result};
use serde_json::{json, Value};
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Naturezas e tipos calibrados (89,5% no Qwen2.5-7B) — seed inicial; editável via ValHalla.
const SEED_NATUREZAS: &[&str] = &["tabular", "narrativo", "codigo"];
const SEED_TIPOS: &[&str] = &[
    "cadastro", "contrato", "comprovante", "nota_fiscal", "recibo", "boleto", "balanco",
    "extrato", "dre", "folha_pagamento", "ordem_compra", "cotacao", "relatorio", "livro",
    "artigo", "ata", "carta", "oficio", "memorial", "curriculo", "discurso", "codigo_fonte",
    "config", "log", "outro",
];

pub fn db_path(dir: &str) -> PathBuf {
    Path::new(dir).join("nidhogg.db")
}

/// Abre a conexão, garante o diretório, aplica WAL + busy_timeout, migra o schema e faz o seed
/// dos doctypes na primeira vez. Barato de chamar por operação.
pub fn open(dir: &str) -> Result<Connection> {
    if let Some(p) = db_path(dir).parent() {
        let _ = std::fs::create_dir_all(p);
    }
    let conn = Connection::open(db_path(dir))?;
    conn.busy_timeout(Duration::from_secs(5))?;
    conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL;")?;
    migrate(&conn)?;
    seed_doctypes_if_empty(&conn)?;
    Ok(conn)
}

fn migrate(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS doc_class (
            collection    TEXT NOT NULL,
            name          TEXT NOT NULL,
            state_hash    TEXT NOT NULL,
            dt_hash       TEXT NOT NULL,
            natureza      TEXT,
            tipo          TEXT,
            confianca     REAL,
            classified_at TEXT,
            PRIMARY KEY (collection, name)
         );
         CREATE INDEX IF NOT EXISTS idx_doc_class_coll ON doc_class(collection);
         CREATE TABLE IF NOT EXISTS doctype (
            kind  TEXT NOT NULL,
            value TEXT NOT NULL,
            ord   INTEGER NOT NULL DEFAULT 0,
            PRIMARY KEY (kind, value)
         );
         CREATE TABLE IF NOT EXISTS meta (
            k TEXT PRIMARY KEY,
            v TEXT
         );",
    )
}

fn seed_doctypes_if_empty(conn: &Connection) -> Result<()> {
    let n: i64 = conn.query_row("SELECT COUNT(*) FROM doctype", [], |r| r.get(0))?;
    if n == 0 {
        write_doctypes(conn, SEED_NATUREZAS, SEED_TIPOS)?;
    }
    Ok(())
}

/// Substitui inteiramente a lista de doctypes (kind='natureza' e kind='tipo'), preservando a
/// ordem informada, e recomputa o `doctypes_hash` no meta. É o que o editor do ValHalla chama.
pub fn write_doctypes<S: AsRef<str>>(conn: &Connection, naturezas: &[S], tipos: &[S]) -> Result<()> {
    conn.execute("DELETE FROM doctype", [])?;
    {
        let mut stmt = conn.prepare("INSERT INTO doctype(kind,value,ord) VALUES (?1,?2,?3)")?;
        for (i, v) in naturezas.iter().enumerate() {
            let v = v.as_ref().trim();
            if !v.is_empty() {
                stmt.execute(params!["natureza", v, i as i64])?;
            }
        }
        for (i, v) in tipos.iter().enumerate() {
            let v = v.as_ref().trim();
            if !v.is_empty() {
                stmt.execute(params!["tipo", v, i as i64])?;
            }
        }
    }
    let (nat, tip) = doctypes(conn)?;
    set_meta(conn, "doctypes_hash", &compute_doctypes_hash(&nat, &tip))?;
    Ok(())
}

/// Retorna `(naturezas, tipos)` na ordem persistida (`ord`).
pub fn doctypes(conn: &Connection) -> Result<(Vec<String>, Vec<String>)> {
    let read = |kind: &str| -> Result<Vec<String>> {
        let mut stmt =
            conn.prepare("SELECT value FROM doctype WHERE kind=?1 ORDER BY ord, value")?;
        let rows = stmt.query_map([kind], |r| r.get::<_, String>(0))?;
        rows.collect()
    };
    Ok((read("natureza")?, read("tipo")?))
}

fn compute_doctypes_hash(naturezas: &[String], tipos: &[String]) -> String {
    let mut h = DefaultHasher::new();
    "nat".hash(&mut h);
    for v in naturezas {
        v.hash(&mut h);
    }
    "tip".hash(&mut h);
    for v in tipos {
        v.hash(&mut h);
    }
    format!("{:016x}", h.finish())
}

/// Hash atual da lista de doctypes — entra no checkpoint de cada classificação. Editar a lista
/// muda o hash → todas as bases voltam a precisar de classificação (o `enum` mudou).
pub fn doctypes_hash(conn: &Connection) -> Result<String> {
    if let Some(v) = get_meta(conn, "doctypes_hash")? {
        return Ok(v);
    }
    let (nat, tip) = doctypes(conn)?;
    let h = compute_doctypes_hash(&nat, &tip);
    set_meta(conn, "doctypes_hash", &h)?;
    Ok(h)
}

pub fn get_meta(conn: &Connection, k: &str) -> Result<Option<String>> {
    conn.query_row("SELECT v FROM meta WHERE k=?1", [k], |r| r.get::<_, String>(0))
        .map(Some)
        .or_else(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => Ok(None),
            other => Err(other),
        })
}

pub fn set_meta(conn: &Connection, k: &str, v: &str) -> Result<()> {
    conn.execute(
        "INSERT INTO meta(k,v) VALUES(?1,?2) ON CONFLICT(k) DO UPDATE SET v=excluded.v",
        params![k, v],
    )?;
    Ok(())
}

/// Uma base precisa de (re)classificação se nunca foi vista, se o corpus mudou (`state_hash`) ou
/// se a lista de doctypes mudou (`dt_hash`).
pub fn needs_class(
    conn: &Connection,
    collection: &str,
    name: &str,
    state_hash: &str,
    dt_hash: &str,
) -> Result<bool> {
    let row: Option<(String, String)> = conn
        .query_row(
            "SELECT state_hash, dt_hash FROM doc_class WHERE collection=?1 AND name=?2",
            params![collection, name],
            |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)),
        )
        .map(Some)
        .or_else(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => Ok(None),
            other => Err(other),
        })?;
    Ok(match row {
        None => true,
        Some((sh, dh)) => sh != state_hash || dh != dt_hash,
    })
}

/// Grava/atualiza a classe de uma base (idempotente por (collection,name)).
#[allow(clippy::too_many_arguments)]
pub fn upsert_class(
    conn: &Connection,
    collection: &str,
    name: &str,
    state_hash: &str,
    dt_hash: &str,
    natureza: &str,
    tipo: &str,
    confianca: f64,
    at: &str,
) -> Result<()> {
    conn.execute(
        "INSERT INTO doc_class(collection,name,state_hash,dt_hash,natureza,tipo,confianca,classified_at)
         VALUES(?1,?2,?3,?4,?5,?6,?7,?8)
         ON CONFLICT(collection,name) DO UPDATE SET
            state_hash=excluded.state_hash, dt_hash=excluded.dt_hash,
            natureza=excluded.natureza, tipo=excluded.tipo,
            confianca=excluded.confianca, classified_at=excluded.classified_at",
        params![collection, name, state_hash, dt_hash, natureza, tipo, confianca, at],
    )?;
    Ok(())
}

/// Remove classes de bases que não existem mais no ragd (GC de fantasmas). `existing` = nomes
/// vivos daquela coleção. Retorna quantas linhas apagou.
pub fn prune_missing(conn: &Connection, collection: &str, existing: &[String]) -> Result<usize> {
    let mut stmt =
        conn.prepare("SELECT name FROM doc_class WHERE collection=?1")?;
    let known: Vec<String> = stmt
        .query_map([collection], |r| r.get::<_, String>(0))?
        .collect::<Result<_>>()?;
    let mut removed = 0usize;
    for name in known {
        if !existing.iter().any(|e| e == &name) {
            conn.execute(
                "DELETE FROM doc_class WHERE collection=?1 AND name=?2",
                params![collection, name],
            )?;
            removed += 1;
        }
    }
    Ok(removed)
}

/// Distribuição de classes por coleção (ou global se `collection` = None/"*"). Devolve contagens
/// por natureza e por tipo (o `GROUP BY` que o SQLite dá de graça) + as linhas para inspeção.
pub fn classes_summary(conn: &Connection, collection: Option<&str>) -> Result<Value> {
    let all = matches!(collection, None) || matches!(collection, Some("*"));
    let coll = collection.unwrap_or("*");

    let count_by = |field: &str| -> Result<Value> {
        let sql = if all {
            format!("SELECT {field}, COUNT(*) FROM doc_class GROUP BY {field} ORDER BY COUNT(*) DESC")
        } else {
            format!("SELECT {field}, COUNT(*) FROM doc_class WHERE collection=?1 GROUP BY {field} ORDER BY COUNT(*) DESC")
        };
        let mut stmt = conn.prepare(&sql)?;
        let mut map = serde_json::Map::new();
        let mapper = |r: &rusqlite::Row| -> Result<(Option<String>, i64)> {
            Ok((r.get::<_, Option<String>>(0)?, r.get::<_, i64>(1)?))
        };
        let rows: Vec<(Option<String>, i64)> = if all {
            stmt.query_map([], mapper)?.collect::<Result<_>>()?
        } else {
            stmt.query_map([coll], mapper)?.collect::<Result<_>>()?
        };
        for (k, n) in rows {
            map.insert(k.unwrap_or_else(|| "?".into()), json!(n));
        }
        Ok(Value::Object(map))
    };

    let (bases_sql, total): (String, i64) = if all {
        (
            "SELECT collection,name,natureza,tipo,confianca,classified_at FROM doc_class ORDER BY collection,name".into(),
            conn.query_row("SELECT COUNT(*) FROM doc_class", [], |r| r.get(0))?,
        )
    } else {
        (
            "SELECT collection,name,natureza,tipo,confianca,classified_at FROM doc_class WHERE collection=?1 ORDER BY name".into(),
            conn.query_row("SELECT COUNT(*) FROM doc_class WHERE collection=?1", [coll], |r| r.get(0))?,
        )
    };
    let mut stmt = conn.prepare(&bases_sql)?;
    let row_mapper = |r: &rusqlite::Row| -> Result<Value> {
        Ok(json!({
            "collection": r.get::<_, String>(0)?,
            "name": r.get::<_, String>(1)?,
            "natureza": r.get::<_, Option<String>>(2)?,
            "tipo": r.get::<_, Option<String>>(3)?,
            "confianca": r.get::<_, Option<f64>>(4)?,
            "classified_at": r.get::<_, Option<String>>(5)?,
        }))
    };
    let bases: Vec<Value> = if all {
        stmt.query_map([], row_mapper)?.collect::<Result<_>>()?
    } else {
        stmt.query_map([coll], row_mapper)?.collect::<Result<_>>()?
    };

    Ok(json!({
        "collection": coll,
        "count": total,
        "naturezas": count_by("natureza")?,
        "tipos": count_by("tipo")?,
        "bases": bases,
    }))
}
