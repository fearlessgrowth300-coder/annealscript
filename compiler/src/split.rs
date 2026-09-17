//! Bifurcation pass: partitions a parsed program into the two codegen
//! destinations described in Phase 2 -- deterministic statements (destined
//! for LLVM IR) and `intent` statements (destined for a tensor execution
//! graph). Actual LLVM/tensor codegen is NOT implemented here: that's a
//! real backend each, not something worth stubbing out before this split
//! itself is proven correct. See docs/grammar-and-types.md Phase 2 notes.

use crate::ast::{Program, Stmt};

#[derive(Debug, PartialEq)]
pub struct CompilationUnit {
    pub deterministic: Vec<Stmt>,
    pub intent_ops: Vec<Stmt>,
}

pub fn split(program: Program) -> CompilationUnit {
    let mut deterministic = Vec::new();
    let mut intent_ops = Vec::new();
    for stmt in program {
        match stmt {
            Stmt::Intent { .. } => intent_ops.push(stmt),
            other => deterministic.push(other),
        }
    }
    CompilationUnit { deterministic, intent_ops }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lexer::lex;
    use crate::parser::parse;

    #[test]
    fn separates_intent_from_deterministic_code() {
        let src = r#"
struct User { username: string default "" }
let raw = {"usr_nm": "alice"}
intent raw -> User as user
print(user.username)
"#;
        let program = parse(lex(src).unwrap()).unwrap();
        let unit = split(program);
        assert_eq!(unit.intent_ops.len(), 1);
        assert!(matches!(unit.intent_ops[0], Stmt::Intent { .. }));
        // struct def, let, print
        assert_eq!(unit.deterministic.len(), 3);
        assert!(unit.deterministic.iter().all(|s| !matches!(s, Stmt::Intent { .. })));
    }
}
