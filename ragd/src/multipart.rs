//! multipart — parser MINIMO de multipart/form-data (RFC 7578).
//!
//! Implementacao simples: o body INTEIRO ja' deve estar em memoria (sem stream).
//! Suficiente pra upload de arquivo unico + alguns campos textuais — que e' o uso
//! do POST /ingest_upload. Nao usa dep externa.
//!
//! Limitacoes conhecidas:
//!   - sem suporte a transfer-encoding chunked dentro de uma parte
//!   - sem decodificacao de filename* (RFC 5987 / UTF-8) — usa o filename simples
//!   - sem suporte a base64 dentro de Content-Transfer-Encoding (raro hoje em http)

/// Um campo de form: name="x" + opcionalmente filename="y" e content-type.
pub struct Part {
    pub name: String,
    pub filename: Option<String>,
    pub content_type: Option<String>,
    pub bytes: Vec<u8>,
}

/// Extrai o boundary de um header Content-Type. Ex:
///   "multipart/form-data; boundary=----XYZ" -> Some("----XYZ")
pub fn extract_boundary(content_type: &str) -> Option<String> {
    for token in content_type.split(';') {
        let t = token.trim();
        if let Some(b) = t.strip_prefix("boundary=") {
            let b = b.trim_matches('"');
            if !b.is_empty() { return Some(b.to_string()); }
        }
    }
    None
}

/// Parsea o body inteiro -> Vec<Part>. boundary e' o do Content-Type (sem o "--").
pub fn parse(body: &[u8], boundary: &str) -> Result<Vec<Part>, String> {
    let bnd = format!("--{boundary}");
    let bnd_bytes = bnd.as_bytes();
    let mut parts = Vec::new();
    // posicoes de todas as boundaries no body
    let positions = find_all(body, bnd_bytes);
    if positions.is_empty() {
        return Err(format!("boundary '{bnd}' nao encontrado no body"));
    }
    // intervalos entre boundaries = corpo de cada parte (depois do CRLF que segue a boundary)
    for win in positions.windows(2) {
        let start_b = win[0];
        let end_b = win[1];
        // pula a linha da boundary: ate o CRLF
        let after_b = match find_at(body, b"\r\n", start_b + bnd_bytes.len()) {
            Some(p) => p + 2,
            None => continue,
        };
        if after_b >= end_b { continue; }
        // a parte fecha em CRLF antes da boundary seguinte
        let part_end = if end_b >= 2 && &body[end_b - 2..end_b] == b"\r\n" { end_b - 2 } else { end_b };
        let raw = &body[after_b..part_end];
        parts.push(parse_one(raw)?);
    }
    Ok(parts)
}

fn parse_one(raw: &[u8]) -> Result<Part, String> {
    // headers terminam em CRLF CRLF
    let split = find_at(raw, b"\r\n\r\n", 0)
        .ok_or_else(|| "parte sem separador de headers (CRLF CRLF)".to_string())?;
    let headers = std::str::from_utf8(&raw[..split])
        .map_err(|e| format!("headers nao-utf8: {e}"))?;
    let body = &raw[split + 4..];

    let mut name: Option<String> = None;
    let mut filename: Option<String> = None;
    let mut content_type: Option<String> = None;
    for line in headers.split("\r\n") {
        let lower = line.to_lowercase();
        if let Some(v) = lower.strip_prefix("content-disposition:") {
            // ex: ' form-data; name="file"; filename="foo.py"'
            for token in v.split(';') {
                let t = token.trim();
                if let Some(n) = t.strip_prefix("name=") {
                    name = Some(unquote(n).to_string());
                } else if let Some(f) = t.strip_prefix("filename=") {
                    filename = Some(unquote(f).to_string());
                }
            }
        } else if let Some(v) = lower.strip_prefix("content-type:") {
            content_type = Some(v.trim().to_string());
        }
    }
    let name = name.ok_or_else(|| "parte sem Content-Disposition name=".to_string())?;
    Ok(Part { name, filename, content_type, bytes: body.to_vec() })
}

fn unquote(s: &str) -> &str {
    let s = s.trim();
    s.strip_prefix('"').and_then(|s| s.strip_suffix('"')).unwrap_or(s)
}

/// Posicoes de todas as ocorrencias non-sobrepostas de `needle` em `hay`.
fn find_all(hay: &[u8], needle: &[u8]) -> Vec<usize> {
    let mut out = vec![];
    let mut i = 0;
    while let Some(p) = find_at(hay, needle, i) {
        out.push(p);
        i = p + needle.len();
    }
    out
}

fn find_at(hay: &[u8], needle: &[u8], from: usize) -> Option<usize> {
    if needle.is_empty() || from >= hay.len() || hay.len() - from < needle.len() { return None; }
    let last = hay.len() - needle.len();
    let mut i = from;
    while i <= last {
        if &hay[i..i + needle.len()] == needle { return Some(i); }
        i += 1;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_two_fields_and_file() {
        let bnd = "----TestBoundary";
        let body = format!(
            "--{b}\r\nContent-Disposition: form-data; name=\"name\"\r\n\r\nhist\r\n\
             --{b}\r\nContent-Disposition: form-data; name=\"chunk\"\r\n\r\n2048\r\n\
             --{b}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"foo.py\"\r\n\
             Content-Type: text/plain\r\n\r\ndef foo(): pass\r\n\
             --{b}--\r\n",
            b = bnd
        );
        let parts = parse(body.as_bytes(), bnd).unwrap();
        assert_eq!(parts.len(), 3);
        assert_eq!(parts[0].name, "name");
        assert_eq!(parts[0].bytes, b"hist");
        assert_eq!(parts[2].filename.as_deref(), Some("foo.py"));
        assert_eq!(parts[2].bytes, b"def foo(): pass");
    }
}
