//! AnnealScript Language Server. Talks LSP over stdio, so the same binary
//! works from VS Code and Cursor (a VS Code fork using the same extension
//! API) -- one server, one extension package, no separate Cursor build.
//!
//! Provides:
//! - diagnostics from the real lexer/parser/typechecker in
//!   annealscript-compiler (parse errors, and the Phase 1 intent-boundary
//!   and bound-scope rules from typecheck.rs)
//! - a CodeLens ("Run & Profile") above every `intent`/`bound` statement
//!   that actually executes the file through the compiler's `--profile`
//!   flag and reports real measured latency/confidence/verdict as hints.
//!
//! Syntax highlighting is NOT done here -- that's a TextMate grammar in
//! the VS Code extension (vscode-extension/syntaxes/annealscript.tmLanguage.json).
//! Reimplementing highlighting via LSP semantic tokens when the editor's
//! native grammar mechanism already does it would be duplicate work for a
//! worse result.

use lsp_server::{Connection, Message, Notification as LspNotification, Request as LspRequest, RequestId, Response};
use lsp_types::notification::{
    DidChangeTextDocument, DidOpenTextDocument, Notification as _, PublishDiagnostics,
};
use lsp_types::request::{CodeLensRequest, ExecuteCommand, Request as _};
use lsp_types::{
    CodeLens, CodeLensOptions, Command, Diagnostic, DiagnosticSeverity,
    DidChangeTextDocumentParams, DidOpenTextDocumentParams, ExecuteCommandOptions,
    ExecuteCommandParams, InitializeParams, Position, PublishDiagnosticsParams, Range,
    ServerCapabilities, TextDocumentSyncCapability, TextDocumentSyncKind, Uri,
};
use std::collections::HashMap;
use std::path::PathBuf;

use annealscript_compiler::{lexer, parser, typecheck};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let (connection, io_threads) = Connection::stdio();

    let capabilities = ServerCapabilities {
        text_document_sync: Some(TextDocumentSyncCapability::Kind(TextDocumentSyncKind::FULL)),
        code_lens_provider: Some(CodeLensOptions { resolve_provider: Some(false) }),
        execute_command_provider: Some(ExecuteCommandOptions {
            commands: vec!["annealscript.runProfile".to_string()],
            work_done_progress_options: Default::default(),
        }),
        ..Default::default()
    };
    let server_capabilities = serde_json::to_value(capabilities)?;
    let initialization_params = connection.initialize(server_capabilities)?;
    let _params: InitializeParams = serde_json::from_value(initialization_params)?;

    main_loop(&connection)?;
    io_threads.join()?;
    Ok(())
}

fn main_loop(connection: &Connection) -> Result<(), Box<dyn std::error::Error>> {
    let mut docs: HashMap<Uri, String> = HashMap::new();

    for msg in &connection.receiver {
        match msg {
            Message::Notification(note) => {
                if let Some((uri, text)) = on_notification(&note) {
                    docs.insert(uri.clone(), text.clone());
                    publish_diagnostics(connection, &uri, &text)?;
                }
            }
            Message::Request(req) => {
                if connection.handle_shutdown(&req)? {
                    break;
                }
                handle_request(connection, &docs, req)?;
            }
            Message::Response(_) => {}
        }
    }
    Ok(())
}

fn on_notification(note: &LspNotification) -> Option<(Uri, String)> {
    match note.method.as_str() {
        DidOpenTextDocument::METHOD => {
            let params: DidOpenTextDocumentParams = serde_json::from_value(note.params.clone()).ok()?;
            Some((params.text_document.uri, params.text_document.text))
        }
        DidChangeTextDocument::METHOD => {
            let params: DidChangeTextDocumentParams = serde_json::from_value(note.params.clone()).ok()?;
            let text = params.content_changes.into_iter().next_back()?.text;
            Some((params.text_document.uri, text))
        }
        _ => None,
    }
}

/// Real diagnostics from the real lexer/parser/typechecker -- lex/parse
/// errors carry a real line number (see lexer::Spanned); typecheck
/// diagnostics only carry a textual anchor (no source spans in the AST
/// yet), so we re-find the line by scanning source text for it. Good
/// enough for an alpha; a duplicate anchor string picks the first match.
fn diagnostics_for(source: &str) -> Vec<Diagnostic> {
    let tokens = match lexer::lex(source) {
        Ok(t) => t,
        Err(e) => return vec![diag_at_line(e.line.saturating_sub(1) as u32, e.message)],
    };
    let program = match parser::parse(tokens) {
        Ok(p) => p,
        Err(e) => return vec![diag_at_line(e.line.saturating_sub(1) as u32, e.message)],
    };
    typecheck::check(&program)
        .into_iter()
        .map(|d| diag_at_line(locate_line(source, &[&d.anchor]), d.message))
        .collect()
}

fn diag_at_line(line: u32, message: String) -> Diagnostic {
    Diagnostic {
        range: Range::new(Position::new(line, 0), Position::new(line, 200)),
        severity: Some(DiagnosticSeverity::ERROR),
        source: Some("annealscript".to_string()),
        message,
        ..Default::default()
    }
}

fn locate_line(source: &str, needles: &[&str]) -> u32 {
    for (i, line) in source.lines().enumerate() {
        if needles.iter().all(|n| line.contains(n)) {
            return i as u32;
        }
    }
    0
}

fn publish_diagnostics(connection: &Connection, uri: &Uri, text: &str) -> Result<(), Box<dyn std::error::Error>> {
    let diagnostics = diagnostics_for(text);
    let params = PublishDiagnosticsParams { uri: uri.clone(), diagnostics, version: None };
    connection.sender.send(Message::Notification(LspNotification {
        method: PublishDiagnostics::METHOD.to_string(),
        params: serde_json::to_value(params)?,
    }))?;
    Ok(())
}

fn handle_request(connection: &Connection, docs: &HashMap<Uri, String>, req: LspRequest) -> Result<(), Box<dyn std::error::Error>> {
    match req.method.as_str() {
        CodeLensRequest::METHOD => {
            let params: lsp_types::CodeLensParams = serde_json::from_value(req.params)?;
            let uri = params.text_document.uri;
            let lenses = docs.get(&uri).map(|text| code_lenses(&uri, text)).unwrap_or_default();
            respond(connection, req.id, lenses)?;
        }
        ExecuteCommand::METHOD => {
            let params: ExecuteCommandParams = serde_json::from_value(req.params)?;
            if params.command == "annealscript.runProfile" {
                if let Some(uri) = params.arguments.first().and_then(|v| v.as_str()).and_then(|s| s.parse::<Uri>().ok()) {
                    if let Some(text) = docs.get(&uri) {
                        run_and_publish_profile(connection, &uri, text)?;
                    }
                }
            }
            respond(connection, req.id, serde_json::Value::Null)?;
        }
        _ => {
            respond(connection, req.id, serde_json::Value::Null)?;
        }
    }
    Ok(())
}

fn respond(connection: &Connection, id: RequestId, result: impl serde::Serialize) -> Result<(), Box<dyn std::error::Error>> {
    connection.sender.send(Message::Response(Response::new_ok(id, result)))?;
    Ok(())
}

/// One "Run & Profile" lens above each `intent`/`bound` line.
fn code_lenses(uri: &Uri, text: &str) -> Vec<CodeLens> {
    text.lines()
        .enumerate()
        .filter(|(_, line)| {
            let t = line.trim_start();
            t.starts_with("intent ") || t.starts_with("bound ")
        })
        .map(|(i, _)| CodeLens {
            range: Range::new(Position::new(i as u32, 0), Position::new(i as u32, 0)),
            command: Some(Command {
                title: "▶ Run & Profile".to_string(),
                command: "annealscript.runProfile".to_string(),
                arguments: Some(vec![serde_json::Value::String(uri.as_str().to_string())]),
            }),
            data: None,
        })
        .collect()
}

fn compiler_binary() -> PathBuf {
    let exe = if cfg!(windows) { "annealscript-compiler.exe" } else { "annealscript-compiler" };
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../compiler/target/debug").join(exe)
}

/// Actually runs the buffer through the real interpreter with `--profile`
/// and turns the measured events into Hint diagnostics. Not fabricated:
/// every number here came from an actual execution that just happened.
///
/// Runs the built exe directly. It used to go through `cargo run` because
/// the standalone exe failed with STATUS_DLL_NOT_FOUND -- root cause was
/// z3's `gh-release` download (libz3.dll) never getting staged next to the
/// binary; fixed for good in compiler/build.rs, so the direct path works
/// now and skips the cargo-invocation overhead.
fn run_and_publish_profile(connection: &Connection, uri: &Uri, text: &str) -> Result<(), Box<dyn std::error::Error>> {
    let tmp = std::env::temp_dir().join(format!("annealscript_lsp_{}.anl", std::process::id()));
    std::fs::write(&tmp, text)?;

    let output = std::process::Command::new(compiler_binary()).arg("--profile").arg(&tmp).output();
    std::fs::remove_file(&tmp).ok();

    let output = match output {
        Ok(o) => o,
        Err(e) => {
            let diag = diag_at_line(0, format!("could not run annealscript-compiler: {e} (build it with `cargo build` in compiler/)"));
            publish(connection, uri, vec![diag])?;
            return Ok(());
        }
    };
    let stdout = String::from_utf8_lossy(&output.stdout);
    let Some(json_line) = stdout.lines().find_map(|l| l.strip_prefix("##INTENTSCRIPT_PROFILE## ")) else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        publish(connection, uri, vec![diag_at_line(0, format!("profiling run failed: {stderr}"))])?;
        return Ok(());
    };
    let events: Vec<annealscript_compiler::runtime::ProfileEvent> = serde_json::from_str(json_line)?;

    let mut diags = Vec::new();
    for ev in events {
        let needles: Vec<&str> = match ev.kind.as_str() {
            "intent" => {
                // label: "intent -> {struct} as {var}"
                let var = ev.label.rsplit(' ').next().unwrap_or("");
                vec![var]
            }
            "bound" => {
                // label: "bound {var} {op} {limit}"
                let var = ev.label.split(' ').nth(1).unwrap_or("");
                vec![var]
            }
            _ => vec![],
        };
        let line = locate_line(text, &needles);
        diags.push(Diagnostic {
            range: Range::new(Position::new(line, 0), Position::new(line, 200)),
            severity: Some(DiagnosticSeverity::HINT),
            source: Some("annealscript-profile".to_string()),
            message: format!("{}us -- {}", ev.micros, ev.detail),
            ..Default::default()
        });
    }
    publish(connection, uri, diags)?;
    Ok(())
}

fn publish(connection: &Connection, uri: &Uri, diagnostics: Vec<Diagnostic>) -> Result<(), Box<dyn std::error::Error>> {
    let params = PublishDiagnosticsParams { uri: uri.clone(), diagnostics, version: None };
    connection.sender.send(Message::Notification(LspNotification {
        method: PublishDiagnostics::METHOD.to_string(),
        params: serde_json::to_value(params)?,
    }))?;
    Ok(())
}
