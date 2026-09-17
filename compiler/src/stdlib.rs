//! Phase 4 standard library: std::fs, std::net, std::safety, std::tensor,
//! std::html.
//!
//! There is no module/import syntax yet (`use std::fs`), so these are
//! exposed to IntentScript source as flat builtin call names --
//! `fs_read_file(...)`, `net_get(...)`, `safety_clamp(...)`,
//! `tensor_similarity(...)`, `html_extract(...)` -- dispatched from
//! `runtime::Expr::Call`. Each flat name documents which std module it
//! belongs to. Real namespacing (`use`, `::` paths) is deferred until more
//! than one module needs it to disambiguate names.
//!
//! std::net is a hand-rolled HTTP/1.1 client over `std::net::TcpStream` --
//! literally the Rust standard library, no dependency. It deliberately
//! does NOT support HTTPS: TLS needs a real dependency (rustls/native-tls)
//! and there's no script yet that needs it. Plain-HTTP `net_get` is enough
//! to prove the "self-healing ETL" story (fetch JSON, feed it to `intent`).
//!
//! std::html uses the `scraper` crate (html5ever + real CSS selectors) --
//! HTML is not a regular language, so hand-rolling a parser for it would be
//! a correctness trap, not a lazy win. `html_extract` turns a CSS-selector
//! map into a dict of extracted text, which is exactly the shape `intent`
//! already expects as a source -- no grammar changes needed, the dict
//! literal and call-expression syntax added in earlier phases already
//! cover it. Only first-match text extraction is implemented; extracting
//! attributes or repeated elements into a list is deferred until a script
//! actually needs it (there's no list/array type in the language yet).

use crate::runtime::{quantize, Value};
use crate::tensor_onnx;
use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::TcpStream;

pub fn call(name: &str, args: Vec<Value>) -> Result<Value, String> {
    match name {
        "fs_read_file" => fs_read_file(args),
        "fs_write_file" => fs_write_file(args),
        "fs_exists" => fs_exists(args),
        "net_get" => net_get(args),
        "safety_clamp" => safety_clamp(args),
        "tensor_similarity" => tensor_similarity(args),
        "html_extract" => html_extract(args),
        other => Err(format!("unknown builtin {other:?}")),
    }
}

fn expect_str(v: &Value, who: &str) -> Result<String, String> {
    match v {
        Value::Str(s) => Ok(s.clone()),
        other => Err(format!("{who} expects a string argument, got {other:?}")),
    }
}

fn expect_num(v: &Value, who: &str) -> Result<f64, String> {
    match v {
        Value::Num(n) => Ok(*n),
        other => Err(format!("{who} expects a numeric argument, got {other:?}")),
    }
}

// ---------- std::fs ----------

fn fs_read_file(args: Vec<Value>) -> Result<Value, String> {
    let [path] = take_args(args, "fs_read_file")?;
    let path = expect_str(&path, "fs_read_file")?;
    std::fs::read_to_string(&path)
        .map(Value::Str)
        .map_err(|e| format!("fs_read_file({path:?}): {e}"))
}

fn fs_write_file(args: Vec<Value>) -> Result<Value, String> {
    let [path, content] = take_args(args, "fs_write_file")?;
    let path = expect_str(&path, "fs_write_file")?;
    let content = expect_str(&content, "fs_write_file")?;
    std::fs::write(&path, content)
        .map(|_| Value::Bool(true))
        .map_err(|e| format!("fs_write_file({path:?}): {e}"))
}

fn fs_exists(args: Vec<Value>) -> Result<Value, String> {
    let [path] = take_args(args, "fs_exists")?;
    let path = expect_str(&path, "fs_exists")?;
    Ok(Value::Bool(std::path::Path::new(&path).exists()))
}

// ---------- std::net ----------

fn net_get(args: Vec<Value>) -> Result<Value, String> {
    let [url] = take_args(args, "net_get")?;
    let url = expect_str(&url, "net_get")?;
    let rest = url.strip_prefix("http://").ok_or_else(|| {
        format!("net_get({url:?}): only plain http:// URLs are supported (no TLS dependency wired yet)")
    })?;
    let (host_port, path) = match rest.find('/') {
        Some(i) => (&rest[..i], &rest[i..]),
        None => (rest, "/"),
    };
    let host_port = if host_port.contains(':') { host_port.to_string() } else { format!("{host_port}:80") };
    let host = host_port.split(':').next().unwrap_or(host_port.as_str());

    let mut stream = TcpStream::connect(&host_port).map_err(|e| format!("net_get: connect {host_port}: {e}"))?;
    let request = format!("GET {path} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n\r\n");
    stream.write_all(request.as_bytes()).map_err(|e| format!("net_get: write: {e}"))?;

    let mut response = Vec::new();
    stream.read_to_end(&mut response).map_err(|e| format!("net_get: read: {e}"))?;
    let text = String::from_utf8_lossy(&response);
    let body = text.split_once("\r\n\r\n").map(|(_, b)| b).unwrap_or(&text);
    Ok(Value::Str(body.to_string()))
}

// ---------- std::safety ----------

fn safety_clamp(args: Vec<Value>) -> Result<Value, String> {
    let [value, lo, hi] = take_args(args, "safety_clamp")?;
    let value = expect_num(&value, "safety_clamp")?;
    let lo = expect_num(&lo, "safety_clamp")?;
    let hi = expect_num(&hi, "safety_clamp")?;
    Ok(Value::Num(value.max(lo).min(hi)))
}

// ---------- std::tensor ----------

fn tensor_similarity(args: Vec<Value>) -> Result<Value, String> {
    let [a, b] = take_args(args, "tensor_similarity")?;
    let a = expect_str(&a, "tensor_similarity")?;
    let b = expect_str(&b, "tensor_similarity")?;
    let score = tensor_onnx::similarity(&quantize(&a), &quantize(&b));
    Ok(Value::Num(score as f64))
}

// ---------- std::html ----------

fn html_extract(args: Vec<Value>) -> Result<Value, String> {
    let [html, selectors] = take_args(args, "html_extract")?;
    let html = expect_str(&html, "html_extract")?;
    let selectors = match selectors {
        Value::Dict(m) => m,
        other => return Err(format!("html_extract expects a dict of {{field: css_selector}}, got {other:?}")),
    };

    let document = scraper::Html::parse_document(&html);
    let mut result = HashMap::new();
    for (field, selector_lit) in selectors {
        let selector_str = match selector_lit {
            crate::ast::Literal::Str(s) => s,
            other => return Err(format!("html_extract: selector for {field:?} must be a string, got {other:?}")),
        };
        let selector = scraper::Selector::parse(&selector_str)
            .map_err(|e| format!("html_extract: bad CSS selector {selector_str:?} for {field:?}: {e}"))?;
        let text = document
            .select(&selector)
            .next()
            .map(|el| el.text().collect::<String>().trim().to_string())
            .unwrap_or_default();
        result.insert(field, crate::ast::Literal::Str(text));
    }
    Ok(Value::Dict(result))
}

fn take_args<const N: usize>(args: Vec<Value>, who: &str) -> Result<[Value; N], String> {
    args.try_into().map_err(|got: Vec<Value>| format!("{who} expects {N} argument(s), got {}", got.len()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fs_write_then_read_round_trips() {
        let path = std::env::temp_dir().join("intentscript_stdlib_test.txt");
        let path_str = path.to_string_lossy().to_string();
        call("fs_write_file", vec![Value::Str(path_str.clone()), Value::Str("hello".into())]).unwrap();
        assert_eq!(call("fs_exists", vec![Value::Str(path_str.clone())]).unwrap(), Value::Bool(true));
        assert_eq!(call("fs_read_file", vec![Value::Str(path_str.clone())]).unwrap(), Value::Str("hello".into()));
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn safety_clamp_bounds_the_value() {
        assert_eq!(call("safety_clamp", vec![Value::Num(99.0), Value::Num(0.0), Value::Num(15.0)]).unwrap(), Value::Num(15.0));
        assert_eq!(call("safety_clamp", vec![Value::Num(-5.0), Value::Num(0.0), Value::Num(15.0)]).unwrap(), Value::Num(0.0));
        assert_eq!(call("safety_clamp", vec![Value::Num(7.0), Value::Num(0.0), Value::Num(15.0)]).unwrap(), Value::Num(7.0));
    }

    #[test]
    fn net_get_parses_a_response_from_a_local_server() {
        use std::net::TcpListener;
        use std::thread;

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            let mut buf = [0u8; 512];
            let _ = socket.read(&mut buf);
            let body = "hello from test server";
            let response = format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n{}", body.len(), body);
            socket.write_all(response.as_bytes()).unwrap();
        });

        let result = call("net_get", vec![Value::Str(format!("http://127.0.0.1:{port}/"))]).unwrap();
        assert_eq!(result, Value::Str("hello from test server".into()));
    }

    #[test]
    fn html_extract_pulls_fields_out_of_messy_markup() {
        let markup = "<div class='card-402'><h2 class='item-heading'>Bluetooth Speaker</h2>\
                       <span class='val-tag'>$89.99</span></div>";
        let mut selectors = HashMap::new();
        selectors.insert("title".to_string(), crate::ast::Literal::Str(".item-heading".into()));
        selectors.insert("price".to_string(), crate::ast::Literal::Str(".val-tag".into()));

        let result = call("html_extract", vec![Value::Str(markup.into()), Value::Dict(selectors)]).unwrap();
        match result {
            Value::Dict(m) => {
                assert_eq!(m["title"], crate::ast::Literal::Str("Bluetooth Speaker".into()));
                assert_eq!(m["price"], crate::ast::Literal::Str("$89.99".into()));
            }
            other => panic!("expected Dict, got {other:?}"),
        }
    }

    #[test]
    fn html_extract_missing_selector_yields_empty_string() {
        let mut selectors = HashMap::new();
        selectors.insert("missing".to_string(), crate::ast::Literal::Str(".nope".into()));
        let result = call("html_extract", vec![Value::Str("<div></div>".into()), Value::Dict(selectors)]).unwrap();
        match result {
            Value::Dict(m) => assert_eq!(m["missing"], crate::ast::Literal::Str("".into())),
            other => panic!("expected Dict, got {other:?}"),
        }
    }

    #[test]
    fn tensor_similarity_of_identical_strings_is_high() {
        let v = call("tensor_similarity", vec![Value::Str("username".into()), Value::Str("username".into())]).unwrap();
        match v {
            Value::Num(n) => assert!(n > 0.99, "expected ~1.0, got {n}"),
            other => panic!("expected Num, got {other:?}"),
        }
    }
}
