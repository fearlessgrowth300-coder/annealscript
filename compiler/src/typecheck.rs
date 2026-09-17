//! Static type checker enforcing the two compile-time rules from
//! docs/grammar-and-types.md Phase 1 (§4 intent boundary rule, §6 bound
//! scope rule). Neither rule was actually enforced until now -- Phase 1
//! only specified them, Phase 3's solver only reasons about numeric
//! *values* inside a bound block, not about who's allowed to write to a
//! bounded variable from outside it. This module is what makes both rules
//! real instead of aspirational.
//!
//! Deliberately NOT attempted here: full Hindley-Milner-style inference,
//! generics beyond `Probability<T>`, or checking expression types beyond
//! what's needed for these two rules -- e.g. arithmetic/boolean expression
//! types and function-call argument counts are only checked at runtime,
//! not here.

use crate::ast::{Program, Stmt, Type};

#[derive(Debug, Clone, PartialEq)]
pub struct Diagnostic {
    pub message: String,
    /// Byte-ish line hint for editor integration; the parser doesn't thread
    /// real source spans through the AST yet, so this is best-effort: the
    /// struct/bound's own declaration order, not a line number. The LSP
    /// layer re-derives line numbers by re-scanning source text for the
    /// offending name (see annealscript-lsp/src/main.rs).
    pub anchor: String,
}

pub fn check(program: &Program) -> Vec<Diagnostic> {
    let mut diags = Vec::new();
    check_intent_boundary(program, &mut diags);
    check_bound_scope(program, &mut diags);
    diags
}

/// §4: every deterministic (non-`Probability<T>`) struct field must declare
/// a `default`, because an `intent` block's source data is untyped and its
/// shape isn't known until runtime.
fn check_intent_boundary(program: &Program, diags: &mut Vec<Diagnostic>) {
    for stmt in program {
        if let Stmt::StructDef { name, fields } = stmt {
            for field in fields {
                let is_probability = matches!(field.ty, Type::Probability(_));
                if !is_probability && field.default.is_none() {
                    diags.push(Diagnostic {
                        message: format!(
                            "field '{}' of struct '{name}' is deterministic but has no default; \
                             add 'default <value>' or declare it 'Probability<{:?}>'",
                            field.name, field.ty
                        ),
                        anchor: format!("struct {name}"),
                    });
                }
            }
        }
    }
}

/// §6: while a variable is under an active `bound` constraint, every `set`
/// to it must occur lexically inside that block (or one nested within it).
/// A `set` to that variable anywhere else in the same statement list is
/// rejected here, at compile time, rather than only being caught (or not)
/// by the Phase 3 runtime guard.
fn check_bound_scope(program: &Program, diags: &mut Vec<Diagnostic>) {
    walk_scope(program, &[], diags);
}

fn walk_scope(stmts: &[Stmt], active_bounds: &[String], diags: &mut Vec<Diagnostic>) {
    // First pass: which variables does THIS scope put under bound.
    let mut scoped_here: Vec<String> = Vec::new();
    for stmt in stmts {
        if let Stmt::Bound { var, .. } = stmt {
            scoped_here.push(var.clone());
        }
    }

    for stmt in stmts {
        match stmt {
            Stmt::Set { name, .. } => {
                if active_bounds.contains(name) {
                    diags.push(Diagnostic {
                        message: format!(
                            "variable '{name}' is safety-bounded in an enclosing 'bound' block; \
                             mutating it outside that block is rejected"
                        ),
                        anchor: format!("set {name}"),
                    });
                } else if scoped_here.contains(name) {
                    // A `set` at this same scope level, but not textually
                    // inside the `bound { }` body -- e.g. after it, still
                    // a violation of the same rule.
                    diags.push(Diagnostic {
                        message: format!(
                            "variable '{name}' is safety-bounded by a 'bound' block in this scope; \
                             mutating it outside that block is rejected"
                        ),
                        anchor: format!("set {name}"),
                    });
                }
            }
            Stmt::Bound { var, body, .. } => {
                // Inside its own body, `var`'s writes are fine (that's the
                // point of the block) -- descend without adding `var` to
                // the forbidden set for its own body, but nested bound
                // blocks inside still inherit outer constraints.
                let mut nested_active = active_bounds.to_vec();
                nested_active.retain(|v| v != var);
                walk_scope(body, &nested_active, diags);
            }
            Stmt::If { then_body, else_body, .. } => {
                // `if`/`while` aren't their own scope for this rule -- a
                // `set` inside one is exactly as "outside the bound block"
                // as one written next to it, so both this scope's own
                // bounds (`scoped_here`) and the enclosing ones carry in.
                let combined = combine(active_bounds, &scoped_here);
                walk_scope(then_body, &combined, diags);
                walk_scope(else_body, &combined, diags);
            }
            Stmt::While { body, .. } => {
                let combined = combine(active_bounds, &scoped_here);
                walk_scope(body, &combined, diags);
            }
            Stmt::FnDef { body, .. } => {
                // No closures (see ast.rs on `Stmt::FnDef`): a function
                // body is a fresh scope that can't see the caller's bound
                // variables at all, so it starts with no active bounds.
                walk_scope(body, &[], diags);
            }
            _ => {}
        }
    }
}

fn combine(active_bounds: &[String], scoped_here: &[String]) -> Vec<String> {
    let mut combined = active_bounds.to_vec();
    combined.extend(scoped_here.iter().cloned());
    combined
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lexer::lex;
    use crate::parser::parse;

    fn check_src(src: &str) -> Vec<Diagnostic> {
        check(&parse(lex(src).unwrap()).unwrap())
    }

    #[test]
    fn flags_deterministic_field_without_default() {
        let diags = check_src("struct User { age: int }");
        assert_eq!(diags.len(), 1);
        assert!(diags[0].message.contains("age"));
    }

    #[test]
    fn allows_deterministic_field_with_default() {
        let diags = check_src(r#"struct User { age: int default 0 }"#);
        assert!(diags.is_empty());
    }

    #[test]
    fn allows_probability_field_without_default() {
        let diags = check_src("struct User { trust: Probability<float> }");
        assert!(diags.is_empty());
    }

    #[test]
    fn flags_set_outside_the_bound_block() {
        let diags = check_src(
            r#"
let speed = 0
bound speed <= 15 {
  set speed = 10
}
set speed = 999
"#,
        );
        assert_eq!(diags.len(), 1);
        assert!(diags[0].message.contains("speed"));
    }

    #[test]
    fn flags_set_inside_an_if_at_the_same_scope_as_the_bound() {
        let diags = check_src(
            r#"
let speed = 0
bound speed <= 15 {
  set speed = 10
}
if true {
  set speed = 999
}
"#,
        );
        assert_eq!(diags.len(), 1);
        assert!(diags[0].message.contains("speed"));
    }

    #[test]
    fn allows_set_inside_an_if_nested_inside_the_bound_block() {
        let diags = check_src(
            r#"
let speed = 0
bound speed <= 15 {
  if true {
    set speed = 10
  }
}
"#,
        );
        assert!(diags.is_empty());
    }

    #[test]
    fn function_body_is_not_constrained_by_an_outer_bound() {
        let diags = check_src(
            r#"
let speed = 0
bound speed <= 15 {
  set speed = 10
}
fn reset() {
  set speed = 999
}
"#,
        );
        assert!(diags.is_empty(), "a same-named var inside a closure-free function body isn't the outer variable");
    }

    #[test]
    fn allows_set_inside_the_bound_block() {
        let diags = check_src(
            r#"
let speed = 0
bound speed <= 15 {
  set speed = 10
}
"#,
        );
        assert!(diags.is_empty());
    }
}
