use std::env;
use std::fs;
use std::process::ExitCode;

use annealscript_compiler::{lexer, parser, runtime, split, typecheck};

fn main() -> ExitCode {
    let args: Vec<String> = env::args().skip(1).collect();
    let profile = args.iter().any(|a| a == "--profile");
    let path = match args.iter().find(|a| *a != "--profile") {
        Some(p) => p.clone(),
        None => {
            eprintln!("usage: annealscript-compiler [--profile] <file.anl>");
            return ExitCode::FAILURE;
        }
    };

    let src = match fs::read_to_string(&path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("error reading {path}: {e}");
            return ExitCode::FAILURE;
        }
    };

    let tokens = match lexer::lex(&src) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("lex error at line {}: {}", e.line, e.message);
            return ExitCode::FAILURE;
        }
    };

    let program = match parser::parse(tokens) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("parse error at line {}: {}", e.line, e.message);
            return ExitCode::FAILURE;
        }
    };

    let diagnostics = typecheck::check(&program);
    if !diagnostics.is_empty() {
        for d in &diagnostics {
            eprintln!("type error ({}): {}", d.anchor, d.message);
        }
        return ExitCode::FAILURE;
    }

    if !profile {
        let unit = split::split(program.clone());
        println!("-> LLVM IR target ({} statements, codegen unimplemented):", unit.deterministic.len());
        for stmt in &unit.deterministic {
            println!("     {stmt:?}");
        }
        println!("-> tensor execution graph target ({} statements, codegen unimplemented):", unit.intent_ops.len());
        for stmt in &unit.intent_ops {
            println!("     {stmt:?}");
        }
        println!("\n-- dual execution runtime --");
    }

    let mut rt = runtime::Runtime::new();
    if let Err(e) = rt.run(&program) {
        eprintln!("runtime error: {e}");
        return ExitCode::FAILURE;
    }

    if profile {
        // A single marked line so a caller (the LSP) can find it without
        // parsing the program's own print() output.
        println!("##INTENTSCRIPT_PROFILE## {}", serde_json::to_string(&rt.profile).unwrap());
    }

    ExitCode::SUCCESS
}
