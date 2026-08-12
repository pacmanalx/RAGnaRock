//! auth — JWT (HS256) + usuários/perfis do ValHalla. [#33]
//!
//! Modelagem (fechada 12/ago/2026): perfil = bundle nomeado de CAPACIDADES (verbos)
//! + ESCOPO de coleções; o token carrega as caps RESOLVIDAS (o guard não consulta
//! disco por request — perfil editado vale no próximo login/refresh).
//!
//! Capacidades: buscar · ingerir · apagar · nidhogg.ver · nidhogg.operar ·
//!              admin.config · admin.usuarios · admin.servicos · "*" (todas)
//!
//! Persistência: JSON único (`auth_file`, default ragnarock-auth.json) com secret,
//! perfis e usuários (senha = PBKDF2-HMAC-SHA256, 60k iterações, salt/uso).
//! Bootstrap: arquivo ausente → perfis semente + admin/admin (TROCAR — fica no log).

use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

use hmac::{Hmac, Mac};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

type HmacSha256 = Hmac<Sha256>;

pub const CAPS: &[&str] = &[
    "buscar", "ingerir", "apagar", "nidhogg.ver", "nidhogg.operar",
    "admin.config", "admin.usuarios", "admin.servicos",
];
const PBKDF2_ITERS: u32 = 60_000;
pub const ACCESS_TTL: u64 = 900; // 15 min; refresh usa o session_ttl do cfg (12h default)

// ───────────────────────────── base64url (sem padding) ─────────────────────────────
const B64: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
pub fn b64url(data: &[u8]) -> String {
    let mut out = String::with_capacity((data.len() + 2) / 3 * 4);
    for c in data.chunks(3) {
        let b = [c[0], *c.get(1).unwrap_or(&0), *c.get(2).unwrap_or(&0)];
        let n = ((b[0] as u32) << 16) | ((b[1] as u32) << 8) | b[2] as u32;
        out.push(B64[(n >> 18) as usize & 63] as char);
        out.push(B64[(n >> 12) as usize & 63] as char);
        if c.len() > 1 { out.push(B64[(n >> 6) as usize & 63] as char); }
        if c.len() > 2 { out.push(B64[n as usize & 63] as char); }
    }
    out
}
fn b64url_dec(s: &str) -> Option<Vec<u8>> {
    let mut idx = [255u8; 256];
    for (i, &c) in B64.iter().enumerate() { idx[c as usize] = i as u8; }
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len() * 3 / 4);
    let mut acc = 0u32;
    let mut nbits = 0u32;
    for &c in bytes {
        let v = idx[c as usize];
        if v == 255 { return None; }
        acc = (acc << 6) | v as u32;
        nbits += 6;
        if nbits >= 8 { nbits -= 8; out.push((acc >> nbits) as u8); }
    }
    Some(out)
}

fn hex(data: &[u8]) -> String { data.iter().map(|b| format!("{b:02x}")).collect() }
fn unhex(s: &str) -> Option<Vec<u8>> {
    if s.len() % 2 != 0 { return None; }
    (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).ok()).collect()
}

fn urandom(n: usize) -> Vec<u8> {
    use std::io::Read;
    let mut buf = vec![0u8; n];
    if let Ok(mut f) = std::fs::File::open("/dev/urandom") {
        if f.read_exact(&mut buf).is_ok() { return buf; }
    }
    // fallback temporal (nunca deve acontecer em unix)
    let t = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().subsec_nanos();
    for (i, b) in buf.iter_mut().enumerate() { *b = (t.wrapping_shr((i % 4) as u32 * 8) & 0xff) as u8; }
    buf
}

pub fn now() -> u64 { SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0) }

// ───────────────────────────── senha (PBKDF2-HMAC-SHA256) ─────────────────────────────
fn pbkdf2(password: &str, salt: &[u8], iters: u32) -> Vec<u8> {
    // 1 bloco (32 bytes) basta — dklen = tamanho do SHA-256
    let mut mac = HmacSha256::new_from_slice(password.as_bytes()).expect("hmac key");
    mac.update(salt);
    mac.update(&1u32.to_be_bytes());
    let mut u = mac.finalize().into_bytes();
    let mut out = u.to_vec();
    for _ in 1..iters {
        let mut m = HmacSha256::new_from_slice(password.as_bytes()).expect("hmac key");
        m.update(&u);
        u = m.finalize().into_bytes();
        for (o, b) in out.iter_mut().zip(u.iter()) { *o ^= b; }
    }
    out
}
pub fn hash_password(password: &str) -> (String, String) {
    let salt = urandom(16);
    (hex(&salt), hex(&pbkdf2(password, &salt, PBKDF2_ITERS)))
}
pub fn check_password(password: &str, salt_hex: &str, hash_hex: &str) -> bool {
    let salt = match unhex(salt_hex) { Some(s) => s, None => return false };
    let got = hex(&pbkdf2(password, &salt, PBKDF2_ITERS));
    // comparação em tempo constante
    got.len() == hash_hex.len()
        && got.bytes().zip(hash_hex.bytes()).fold(0u8, |a, (x, y)| a | (x ^ y)) == 0
}

// ───────────────────────────── JWT HS256 ─────────────────────────────
fn sign(secret: &[u8], msg: &str) -> String {
    let mut mac = HmacSha256::new_from_slice(secret).expect("hmac key");
    mac.update(msg.as_bytes());
    b64url(&mac.finalize().into_bytes())
}

pub fn jwt_make(secret: &[u8], claims: &Value) -> String {
    let h = b64url(br#"{"alg":"HS256","typ":"JWT"}"#);
    let p = b64url(claims.to_string().as_bytes());
    let msg = format!("{h}.{p}");
    let s = sign(secret, &msg);
    format!("{msg}.{s}")
}

/// Valida assinatura + exp. Devolve os claims ou None.
pub fn jwt_verify(secret: &[u8], token: &str) -> Option<Value> {
    let mut it = token.split('.');
    let (h, p, s) = (it.next()?, it.next()?, it.next()?);
    if it.next().is_some() { return None; }
    let msg = format!("{h}.{p}");
    let want = sign(secret, &msg);
    if want.len() != s.len()
        || want.bytes().zip(s.bytes()).fold(0u8, |a, (x, y)| a | (x ^ y)) != 0 { return None; }
    let claims: Value = serde_json::from_slice(&b64url_dec(p)?).ok()?;
    if claims["exp"].as_u64().unwrap_or(0) < now() { return None; }
    Some(claims)
}

/// Extrai e valida o Bearer do Authorization. None = sem token ou inválido/expirado.
pub fn bearer_claims(headers: &[(String, String)], secret: &[u8]) -> Option<Value> {
    let h = headers.iter().find(|(k, _)| k.eq_ignore_ascii_case("authorization"))?;
    let tok = h.1.strip_prefix("Bearer ").or_else(|| h.1.strip_prefix("bearer "))?;
    let c = jwt_verify(secret, tok.trim())?;
    (c["typ"].as_str() == Some("access")).then_some(c)
}

/// O token tem a capacidade? ("*" nas caps = todas)
pub fn has_cap(claims: &Value, cap: &str) -> bool {
    claims["caps"].as_array().map(|a| {
        a.iter().any(|c| c.as_str() == Some("*") || c.as_str() == Some(cap))
    }).unwrap_or(false)
}

// ───────────────────────────── store (auth.json) ─────────────────────────────
pub struct Auth {
    pub path: String,
    pub secret: Vec<u8>,
    pub data: Value, // {"perfis":[...], "usuarios":[...]}
}

fn seed() -> Value {
    json!({
        "perfis": [
            {"nome": "admin",    "desc": "Controle total",                        "caps": ["*"],                                                    "colls": ["*"]},
            {"nome": "operador", "desc": "Opera o RAG: busca, ingestão, Nidhogg", "caps": ["buscar", "ingerir", "nidhogg.ver", "nidhogg.operar"],  "colls": ["*"]},
            {"nome": "leitor",   "desc": "Somente consulta",                      "caps": ["buscar", "nidhogg.ver"],                                "colls": ["*"]},
            {"nome": "auditor",  "desc": "Read-only total (compliance)",          "caps": ["buscar", "nidhogg.ver", "admin.servicos"],              "colls": ["*"]},
        ],
        "usuarios": []
    })
}

impl Auth {
    pub fn load(path: &str) -> Auth {
        let (secret, data) = match std::fs::read_to_string(path)
            .ok()
            .and_then(|t| serde_json::from_str::<Value>(&t).ok())
        {
            Some(v) => {
                let sec = v["secret"].as_str().and_then(unhex).unwrap_or_else(|| urandom(32));
                (sec, v)
            }
            None => {
                // bootstrap: perfis semente + admin/admin (trocar!)
                let sec = urandom(32);
                let mut v = seed();
                let (salt, hash) = hash_password("admin");
                v["usuarios"] = json!([{"login": "admin", "nome": "Administrador", "perfil": "admin",
                                        "ativo": true, "salt": salt, "hash": hash}]);
                v["secret"] = json!(hex(&sec));
                eprintln!("⚠️  auth: {path} criado com admin/admin — TROQUE a senha no primeiro login");
                (sec, v)
            }
        };
        let mut a = Auth { path: path.to_string(), secret, data };
        if a.data["secret"].as_str().is_none() { a.data["secret"] = json!(hex(&a.secret)); }
        a.save();
        a
    }

    pub fn save(&self) {
        if let Err(e) = std::fs::write(&self.path, serde_json::to_string_pretty(&self.data).unwrap_or_default()) {
            eprintln!("auth: falha ao gravar {}: {e}", self.path);
        }
    }

    pub fn perfil(&self, nome: &str) -> Option<&Value> {
        self.data["perfis"].as_array()?.iter().find(|p| p["nome"].as_str() == Some(nome))
    }
    pub fn usuario(&self, login: &str) -> Option<&Value> {
        self.data["usuarios"].as_array()?.iter().find(|u| u["login"].as_str() == Some(login))
    }

    fn claims_for(&self, u: &Value, typ: &str, ttl: u64) -> Value {
        let perfil = u["perfil"].as_str().unwrap_or("");
        let p = self.perfil(perfil).cloned().unwrap_or(json!({}));
        json!({
            "sub": u["login"], "name": u["nome"], "perfil": perfil,
            "caps": p["caps"].clone(), "colls": p["colls"].clone(),
            "iss": "ragd", "typ": typ, "iat": now(), "exp": now() + ttl,
            "jti": hex(&urandom(8)),
        })
    }

    /// POST /login {login, password} → tokens + identidade (ou 401).
    pub fn login(&self, body: &str, refresh_ttl: u64) -> (u16, String) {
        let v: Value = match serde_json::from_str(body) {
            Ok(v) => v, Err(e) => return (400, json!({"error": format!("JSON inválido: {e}")}).to_string()),
        };
        let (login, pass) = (v["login"].as_str().unwrap_or(""), v["password"].as_str().unwrap_or(""));
        let u = match self.usuario(login) {
            Some(u) if u["ativo"].as_bool().unwrap_or(false) => u.clone(),
            _ => return (401, json!({"error": "credenciais inválidas"}).to_string()),
        };
        if !check_password(pass, u["salt"].as_str().unwrap_or(""), u["hash"].as_str().unwrap_or("")) {
            return (401, json!({"error": "credenciais inválidas"}).to_string());
        }
        let ac = self.claims_for(&u, "access", ACCESS_TTL);
        let rc = self.claims_for(&u, "refresh", refresh_ttl);
        (200, json!({
            "access": jwt_make(&self.secret, &ac),
            "refresh": jwt_make(&self.secret, &rc),
            "expires_in": ACCESS_TTL,
            "usuario": {"login": u["login"], "nome": u["nome"], "perfil": u["perfil"],
                         "caps": ac["caps"], "colls": ac["colls"]},
        }).to_string())
    }

    /// POST /refresh {refresh} → novo access (re-resolve caps do perfil ATUAL).
    pub fn refresh(&self, body: &str) -> (u16, String) {
        let v: Value = match serde_json::from_str(body) {
            Ok(v) => v, Err(e) => return (400, json!({"error": format!("JSON inválido: {e}")}).to_string()),
        };
        let c = match jwt_verify(&self.secret, v["refresh"].as_str().unwrap_or("")) {
            Some(c) if c["typ"].as_str() == Some("refresh") => c,
            _ => return (401, json!({"error": "refresh inválido ou expirado"}).to_string()),
        };
        let u = match self.usuario(c["sub"].as_str().unwrap_or("")) {
            Some(u) if u["ativo"].as_bool().unwrap_or(false) => u.clone(),
            _ => return (401, json!({"error": "usuário inexistente ou inativo"}).to_string()),
        };
        let ac = self.claims_for(&u, "access", ACCESS_TTL);
        (200, json!({"access": jwt_make(&self.secret, &ac), "expires_in": ACCESS_TTL}).to_string())
    }

    // ───────── CRUD (guard: admin.usuarios — feito pelo chamador) ─────────

    pub fn perfis_json(&self) -> (u16, String) {
        (200, json!({"perfis": self.data["perfis"]}).to_string())
    }

    /// Upsert de perfil {nome, desc, caps[], colls[]}. Caps validadas contra o catálogo.
    pub fn perfil_upsert(&mut self, body: &str) -> (u16, String) {
        let v: Value = match serde_json::from_str(body) {
            Ok(v) => v, Err(e) => return (400, json!({"error": format!("JSON inválido: {e}")}).to_string()),
        };
        let nome = v["nome"].as_str().unwrap_or("").trim().to_string();
        if nome.is_empty() { return (400, json!({"error": "falta 'nome'"}).to_string()); }
        let caps: Vec<String> = v["caps"].as_array().map(|a| a.iter()
            .filter_map(|c| c.as_str().map(String::from)).collect()).unwrap_or_default();
        if caps.is_empty() { return (400, json!({"error": "perfil precisa de ao menos 1 capacidade"}).to_string()); }
        for c in &caps {
            if c != "*" && !CAPS.contains(&c.as_str()) {
                return (400, json!({"error": format!("capacidade desconhecida: {c}"), "caps": CAPS}).to_string());
            }
        }
        let colls: Vec<String> = v["colls"].as_array().map(|a| a.iter()
            .filter_map(|c| c.as_str().map(String::from)).collect()).unwrap_or_else(|| vec!["*".into()]);
        let novo = json!({"nome": nome, "desc": v["desc"].as_str().unwrap_or(""), "caps": caps, "colls": colls});
        let arr = self.data["perfis"].as_array_mut().unwrap();
        match arr.iter_mut().find(|p| p["nome"].as_str() == Some(nome.as_str())) {
            Some(p) => *p = novo.clone(),
            None => arr.push(novo.clone()),
        }
        self.save();
        (200, json!({"ok": true, "perfil": novo}).to_string())
    }

    pub fn perfil_delete(&mut self, nome: &str) -> (u16, String) {
        let em_uso = self.data["usuarios"].as_array().map(|a| a.iter()
            .any(|u| u["perfil"].as_str() == Some(nome))).unwrap_or(false);
        if em_uso { return (409, json!({"error": format!("perfil '{nome}' em uso por usuário(s)")}).to_string()); }
        let arr = self.data["perfis"].as_array_mut().unwrap();
        let before = arr.len();
        arr.retain(|p| p["nome"].as_str() != Some(nome));
        if arr.len() == before { return (404, json!({"error": format!("perfil '{nome}' não existe")}).to_string()); }
        self.save();
        (200, json!({"ok": true, "removed": nome}).to_string())
    }

    /// Lista sem os campos sensíveis (salt/hash).
    pub fn usuarios_json(&self) -> (u16, String) {
        let us: Vec<Value> = self.data["usuarios"].as_array().map(|a| a.iter().map(|u| json!({
            "login": u["login"], "nome": u["nome"], "perfil": u["perfil"], "ativo": u["ativo"],
        })).collect()).unwrap_or_default();
        (200, json!({"usuarios": us}).to_string())
    }

    /// Upsert de usuário {login, nome, perfil, ativo, password?}.
    /// password obrigatória na CRIAÇÃO; opcional no update (mantém a atual).
    pub fn usuario_upsert(&mut self, body: &str) -> (u16, String) {
        let v: Value = match serde_json::from_str(body) {
            Ok(v) => v, Err(e) => return (400, json!({"error": format!("JSON inválido: {e}")}).to_string()),
        };
        let login = v["login"].as_str().unwrap_or("").trim().to_string();
        if login.is_empty() { return (400, json!({"error": "falta 'login'"}).to_string()); }
        let perfil = v["perfil"].as_str().unwrap_or("").to_string();
        if self.perfil(&perfil).is_none() {
            return (400, json!({"error": format!("perfil desconhecido: {perfil}")}).to_string());
        }
        let ativo = v["ativo"].as_bool().unwrap_or(true);
        // trava: não deixar o último admin ativo ser desativado/rebaixado
        if !ativo || !self.perfil_tem_admin(&perfil) {
            if self.eh_ultimo_admin(&login) {
                return (409, json!({"error": "é o último usuário ativo com admin.usuarios — não pode ser desativado/rebaixado"}).to_string());
            }
        }
        let exists = self.usuario(&login).cloned();
        let (salt, hash) = match v["password"].as_str().filter(|p| !p.is_empty()) {
            Some(p) => hash_password(p),
            None => match &exists {
                Some(u) => (u["salt"].as_str().unwrap_or("").into(), u["hash"].as_str().unwrap_or("").into()),
                None => return (400, json!({"error": "criação exige 'password'"}).to_string()),
            },
        };
        let novo = json!({"login": login, "nome": v["nome"].as_str().unwrap_or(""), "perfil": perfil,
                          "ativo": ativo, "salt": salt, "hash": hash});
        let arr = self.data["usuarios"].as_array_mut().unwrap();
        match arr.iter_mut().find(|u| u["login"].as_str() == Some(login.as_str())) {
            Some(u) => *u = novo.clone(),
            None => arr.push(novo.clone()),
        }
        self.save();
        (200, json!({"ok": true, "usuario": {"login": novo["login"], "nome": novo["nome"],
                     "perfil": novo["perfil"], "ativo": novo["ativo"]}}).to_string())
    }

    pub fn usuario_delete(&mut self, login: &str) -> (u16, String) {
        if self.eh_ultimo_admin(login) {
            return (409, json!({"error": "é o último usuário ativo com admin.usuarios — não pode ser removido"}).to_string());
        }
        let arr = self.data["usuarios"].as_array_mut().unwrap();
        let before = arr.len();
        arr.retain(|u| u["login"].as_str() != Some(login));
        if arr.len() == before { return (404, json!({"error": format!("usuário '{login}' não existe")}).to_string()); }
        self.save();
        (200, json!({"ok": true, "removed": login}).to_string())
    }

    fn perfil_tem_admin(&self, nome: &str) -> bool {
        self.perfil(nome).and_then(|p| p["caps"].as_array()).map(|a| a.iter()
            .any(|c| c.as_str() == Some("*") || c.as_str() == Some("admin.usuarios"))).unwrap_or(false)
    }
    /// `login` é o único usuário ATIVO cujo perfil tem admin.usuarios?
    fn eh_ultimo_admin(&self, login: &str) -> bool {
        let admins: Vec<&str> = self.data["usuarios"].as_array().map(|a| a.iter()
            .filter(|u| u["ativo"].as_bool().unwrap_or(false)
                && self.perfil_tem_admin(u["perfil"].as_str().unwrap_or("")))
            .filter_map(|u| u["login"].as_str()).collect()).unwrap_or_default();
        admins.len() == 1 && admins[0] == login
    }
}

/// Guard dos endpoints de CRUD: Bearer válido + capacidade. Err = resposta pronta.
pub fn require_cap(headers: &[(String, String)], auth: &Auth, cap: &str) -> Result<Value, (u16, String)> {
    match bearer_claims(headers, &auth.secret) {
        Some(c) if has_cap(&c, cap) => Ok(c),
        Some(_) => Err((403, json!({"error": format!("sem a capacidade '{cap}'")}).to_string())),
        None => Err((401, json!({"error": "token ausente, inválido ou expirado"}).to_string())),
    }
}
