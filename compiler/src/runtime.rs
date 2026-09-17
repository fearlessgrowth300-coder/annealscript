//! Dual execution runtime.
//!
//! Deterministic statements (`let`, `set`, `print`, `bound`) execute as
//! plain native tree-walking evaluation over an in-process value store --
//! that's the "native machine instructions" lane. `intent` statements
//! hash their candidate keys into int8[32] vectors here, then hand them to
//! `tensor_onnx::similarity`, which runs a real ONNX Runtime session in the
//! same process -- that's the "lightweight tensor" lane. It is still a
//! hand-authored graph, not a trained model (see `models/`), but the
//! computation genuinely executes through ONNX Runtime now, not hand
//! written Rust arithmetic standing in for it.
//!
//! Safety bounds are enforced via `solver::solve_bound` (see that module),
//! which is itself backed by a real Z3 SMT query: provably-safe bound
//! blocks run with no guard at all, provably-unsafe ones never execute, and
//! undecidable ones get a per-write check *before* the value is committed
//! to the heap-backed `env` map.

use std::collections::HashMap;

use crate::ast::{BinOp, Comparator, Expr, Field, Literal, Program, Stmt, Type, UnOp};
use crate::interval::Interval;
use crate::solver::{self, Verdict};

#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Num(f64),
    Str(String),
    Bool(bool),
    Dict(HashMap<String, Literal>),
    Struct(HashMap<String, Value>),
    Prob(Box<Value>, f32),
    /// The value of a bare `return;` or of calling a function that falls
    /// off the end of its body without an explicit `return`. There's no
    /// unit/void type in the surface language, just this runtime value.
    Unit,
}

pub fn format_value(v: &Value) -> String {
    match v {
        Value::Num(n) if n.fract() == 0.0 => format!("{}", *n as i64),
        Value::Num(n) => format!("{n}"),
        Value::Str(s) => s.clone(),
        Value::Bool(b) => b.to_string(),
        Value::Dict(_) => "<dict>".to_string(),
        Value::Struct(_) => "<struct>".to_string(),
        Value::Prob(inner, conf) => format!("Probability({}, conf={:.2})", format_value(inner), conf),
        Value::Unit => "()".to_string(),
    }
}

fn as_num(v: &Value, who: &str) -> Result<f64, String> {
    match v {
        Value::Num(n) => Ok(*n),
        other => Err(format!("{who}: expected a number, got {other:?}")),
    }
}

fn eval_binop(op: BinOp, l: Value, r: Value) -> Result<Value, String> {
    match op {
        BinOp::Add => match (&l, &r) {
            (Value::Num(a), Value::Num(b)) => Ok(Value::Num(a + b)),
            (Value::Str(a), Value::Str(b)) => Ok(Value::Str(format!("{a}{b}"))),
            _ => Err(format!("'+' needs two numbers or two strings, got {l:?} and {r:?}")),
        },
        BinOp::Sub => Ok(Value::Num(as_num(&l, "'-'")? - as_num(&r, "'-'")?)),
        BinOp::Mul => Ok(Value::Num(as_num(&l, "'*'")? * as_num(&r, "'*'")?)),
        BinOp::Div => {
            let denom = as_num(&r, "'/'")?;
            if denom == 0.0 {
                return Err("division by zero".into());
            }
            Ok(Value::Num(as_num(&l, "'/'")? / denom))
        }
        BinOp::Mod => {
            let denom = as_num(&r, "'%'")?;
            if denom == 0.0 {
                return Err("modulo by zero".into());
            }
            Ok(Value::Num(as_num(&l, "'%'")? % denom))
        }
        BinOp::Eq => Ok(Value::Bool(l == r)),
        BinOp::Ne => Ok(Value::Bool(l != r)),
        BinOp::Lt => Ok(Value::Bool(as_num(&l, "'<'")? < as_num(&r, "'<'")?)),
        BinOp::Le => Ok(Value::Bool(as_num(&l, "'<='")? <= as_num(&r, "'<='")?)),
        BinOp::Gt => Ok(Value::Bool(as_num(&l, "'>'")? > as_num(&r, "'>'")?)),
        BinOp::Ge => Ok(Value::Bool(as_num(&l, "'>='")? >= as_num(&r, "'>='")?)),
        BinOp::And => match (&l, &r) {
            (Value::Bool(a), Value::Bool(b)) => Ok(Value::Bool(*a && *b)),
            _ => Err(format!("'&&' needs two bools, got {l:?} and {r:?}")),
        },
        BinOp::Or => match (&l, &r) {
            (Value::Bool(a), Value::Bool(b)) => Ok(Value::Bool(*a || *b)),
            _ => Err(format!("'||' needs two bools, got {l:?} and {r:?}")),
        },
    }
}

fn eval_unop(op: UnOp, v: Value) -> Result<Value, String> {
    match op {
        UnOp::Neg => Ok(Value::Num(-as_num(&v, "unary '-'")?)),
        UnOp::Not => match v {
            Value::Bool(b) => Ok(Value::Bool(!b)),
            other => Err(format!("unary '!' needs a bool, got {other:?}")),
        },
    }
}

fn literal_to_value(lit: &Literal) -> Value {
    match lit {
        Literal::Str(s) => Value::Str(s.clone()),
        Literal::Num(n) => Value::Num(*n),
        Literal::Bool(b) => Value::Bool(*b),
    }
}

fn zero_value(ty: &Type) -> Value {
    match ty {
        Type::Int | Type::Float => Value::Num(0.0),
        Type::Bool => Value::Bool(false),
        Type::String => Value::Str(String::new()),
        Type::Probability(inner) => zero_value(inner),
        Type::Named(_) => Value::Str(String::new()),
    }
}

fn coerce(lit: &Literal, ty: &Type) -> Option<Value> {
    match ty {
        Type::String => Some(Value::Str(match lit {
            Literal::Str(s) => s.clone(),
            Literal::Num(n) => n.to_string(),
            Literal::Bool(b) => b.to_string(),
        })),
        Type::Int | Type::Float => match lit {
            Literal::Num(n) => Some(Value::Num(*n)),
            Literal::Str(s) => s.trim().parse::<f64>().ok().map(Value::Num),
            Literal::Bool(_) => None,
        },
        Type::Bool => match lit {
            Literal::Bool(b) => Some(Value::Bool(*b)),
            Literal::Str(s) => match s.trim().to_lowercase().as_str() {
                "true" => Some(Value::Bool(true)),
                "false" => Some(Value::Bool(false)),
                _ => None,
            },
            Literal::Num(n) if *n == 0.0 || *n == 1.0 => Some(Value::Bool(*n == 1.0)),
            _ => None,
        },
        Type::Probability(inner) => coerce(lit, inner),
        Type::Named(_) => None,
    }
}

// ---------- quantized (int8) similarity scorer: the "tensor lane" ----------
// Feature hashing happens here in plain Rust; the actual similarity score
// is computed by `tensor_onnx::similarity`, a real ONNX Runtime inference
// call over a hand-authored (not trained) graph -- see that module.

const VEC_DIM: usize = 32;

fn normalize(s: &str) -> Vec<char> {
    s.chars().filter(|c| c.is_alphanumeric()).flat_map(|c| c.to_lowercase()).collect()
}

pub(crate) fn quantize(s: &str) -> [i8; VEC_DIM] {
    let mut v = [0i8; VEC_DIM];
    let chars = normalize(s);
    if chars.len() < 2 {
        for c in &chars {
            let idx = (*c as usize) % VEC_DIM;
            v[idx] = v[idx].saturating_add(1);
        }
        return v;
    }
    for w in chars.windows(2) {
        let h = (w[0] as usize).wrapping_mul(131).wrapping_add(w[1] as usize);
        v[h % VEC_DIM] = v[h % VEC_DIM].saturating_add(1);
    }
    v
}

fn best_match<'a>(target: &str, available: &'a [String]) -> Option<(&'a String, f32)> {
    let tv = quantize(target);
    available
        .iter()
        .map(|k| (k, crate::tensor_onnx::similarity(&tv, &quantize(k))))
        .fold(None, |best, cur| match best {
            Some((_, s)) if s >= cur.1 => best,
            _ => Some(cur),
        })
}

fn resolve_intent(raw: &HashMap<String, Literal>, fields: &[Field], threshold: f32) -> HashMap<String, Value> {
    let mut available: Vec<String> = raw.keys().cloned().collect();
    let mut result = HashMap::new();
    for field in fields {
        let mut matched: Option<Value> = None;
        if let Some((key, score)) = best_match(&field.name, &available) {
            if score >= threshold {
                if let Some(coerced) = coerce(&raw[key], &field.ty) {
                    let key = key.clone();
                    matched = Some(match &field.ty {
                        Type::Probability(_) => Value::Prob(Box::new(coerced), score),
                        _ => coerced,
                    });
                    available.retain(|k| k != &key);
                }
            }
        }
        let value = matched.unwrap_or_else(|| match &field.ty {
            Type::Probability(_) => Value::Prob(Box::new(zero_value(&field.ty)), 0.0),
            _ => field
                .default
                .as_ref()
                .map(literal_to_value)
                .unwrap_or_else(|| zero_value(&field.ty)),
        });
        result.insert(field.name.clone(), value);
    }
    result
}

// ---------- runtime ----------

/// One measured event from an actual run -- Phase 5's "profiling" is always
/// real numbers from executing the program, never estimated or fabricated.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ProfileEvent {
    pub kind: String, // "intent" | "bound"
    pub label: String,
    pub micros: u128,
    pub detail: String,
}

/// What a statement (or a whole body) did, beyond "ran": `Return` unwinds
/// out of `if`/`while` bodies up to the enclosing function call, the way
/// it would in any imperative language.
enum Flow {
    Normal,
    Return(Value),
}

pub struct Runtime {
    pub structs: HashMap<String, Vec<Field>>,
    pub env: HashMap<String, Value>,
    pub profile: Vec<ProfileEvent>,
    /// name -> (params, body). No closures: a call gets a fresh scope
    /// containing only its bound parameters, not the caller's locals --
    /// see the note on `Stmt::FnDef` in ast.rs for why.
    functions: HashMap<String, (Vec<(String, Type)>, Vec<Stmt>)>,
}

impl Runtime {
    pub fn new() -> Self {
        Runtime { structs: HashMap::new(), env: HashMap::new(), profile: Vec::new(), functions: HashMap::new() }
    }

    pub fn run(&mut self, program: &Program) -> Result<(), String> {
        // Hoist top-level function definitions so a function can be called
        // before its textual definition, same as most languages allow at
        // top level. Functions declared inside a nested block are only
        // registered once execution actually reaches them (no hoisting
        // there -- an accepted simplification, not an oversight).
        for stmt in program {
            if let Stmt::FnDef { name, params, body, .. } = stmt {
                self.functions.insert(name.clone(), (params.clone(), body.clone()));
            }
        }
        self.exec_body(program).map(|_| ())
    }

    fn exec_body(&mut self, body: &[Stmt]) -> Result<Flow, String> {
        for stmt in body {
            match self.exec_flow(stmt)? {
                Flow::Normal => continue,
                flow @ Flow::Return(_) => return Ok(flow),
            }
        }
        Ok(Flow::Normal)
    }

    /// Adapter for call sites that don't propagate control flow (bound
    /// blocks' "provably safe" path): a `return` inside one of those just
    /// completes that one statement instead of unwinding further. Real
    /// call sites that need `return` to actually unwind use `exec_body`.
    fn exec(&mut self, stmt: &Stmt) -> Result<(), String> {
        self.exec_flow(stmt).map(|_| ())
    }

    fn eval(&mut self, expr: &Expr) -> Result<Value, String> {
        match expr {
            Expr::Literal(lit) => Ok(literal_to_value(lit)),
            Expr::Ident(name) => self.env.get(name).cloned().ok_or_else(|| format!("undefined variable {name:?}")),
            Expr::Dict(pairs) => Ok(Value::Dict(pairs.iter().cloned().collect())),
            Expr::FieldAccess(obj, field) => match self.env.get(obj) {
                Some(Value::Struct(m)) => {
                    m.get(field).cloned().ok_or_else(|| format!("struct {obj:?} has no field {field:?}"))
                }
                Some(other) => Err(format!("{obj:?} is not a struct (got {other:?})")),
                None => Err(format!("undefined variable {obj:?}")),
            },
            Expr::Resolve { subject, threshold, above, else_branch } => {
                let subject_val = self.eval(subject)?;
                let confidence = match &subject_val {
                    Value::Prob(_, conf) => *conf as f64,
                    _ => return Err("resolve subject must be a Probability<T> value".into()),
                };
                let branch = if confidence >= *threshold { above } else { else_branch };
                let result = self.eval(branch)?;
                Ok(match result {
                    Value::Prob(inner, _) => *inner,
                    other => other,
                })
            }
            Expr::Binary { op, left, right } => {
                let l = self.eval(left)?;
                let r = self.eval(right)?;
                eval_binop(*op, l, r)
            }
            Expr::Unary { op, expr } => {
                let v = self.eval(expr)?;
                eval_unop(*op, v)
            }
            Expr::Call { name, args } => {
                let arg_values: Result<Vec<Value>, String> = args.iter().map(|a| self.eval(a)).collect();
                let arg_values = arg_values?;
                if let Some((params, body)) = self.functions.get(name).cloned() {
                    if params.len() != arg_values.len() {
                        return Err(format!(
                            "function {name:?} expects {} argument(s), got {}",
                            params.len(),
                            arg_values.len()
                        ));
                    }
                    let local_env: HashMap<String, Value> =
                        params.iter().map(|(pname, _)| pname.clone()).zip(arg_values).collect();
                    let saved_env = std::mem::replace(&mut self.env, local_env);
                    let result = self.exec_body(&body);
                    self.env = saved_env;
                    match result? {
                        Flow::Return(v) => Ok(v),
                        Flow::Normal => Ok(Value::Unit),
                    }
                } else {
                    crate::stdlib::call(name, arg_values)
                }
            }
        }
    }

    fn exec_flow(&mut self, stmt: &Stmt) -> Result<Flow, String> {
        match stmt {
            Stmt::StructDef { name, fields } => {
                self.structs.insert(name.clone(), fields.clone());
                Ok(Flow::Normal)
            }
            Stmt::Let { name, expr, .. } => {
                let v = self.eval(expr)?;
                self.env.insert(name.clone(), v);
                Ok(Flow::Normal)
            }
            Stmt::Set { name, expr } => {
                let v = self.eval(expr)?;
                self.env.insert(name.clone(), v);
                Ok(Flow::Normal)
            }
            Stmt::Print(expr) => {
                let v = self.eval(expr)?;
                println!("{}", format_value(&v));
                Ok(Flow::Normal)
            }
            Stmt::Intent { source, struct_name, var } => self.exec_intent(source, struct_name, var).map(|_| Flow::Normal),
            Stmt::Bound { var, op, limit, body } => self.exec_bound(var, op, *limit, body).map(|_| Flow::Normal),
            Stmt::If { cond, then_body, else_body } => {
                let taken = self.eval_bool(cond, "if")?;
                self.exec_body(if taken { then_body } else { else_body })
            }
            Stmt::While { cond, body } => {
                while self.eval_bool(cond, "while")? {
                    match self.exec_body(body)? {
                        Flow::Normal => continue,
                        flow @ Flow::Return(_) => return Ok(flow),
                    }
                }
                Ok(Flow::Normal)
            }
            Stmt::FnDef { name, params, body, .. } => {
                self.functions.insert(name.clone(), (params.clone(), body.clone()));
                Ok(Flow::Normal)
            }
            Stmt::Return(expr) => {
                let v = match expr {
                    Some(e) => self.eval(e)?,
                    None => Value::Unit,
                };
                Ok(Flow::Return(v))
            }
        }
    }

    fn eval_bool(&mut self, expr: &Expr, who: &str) -> Result<bool, String> {
        match self.eval(expr)? {
            Value::Bool(b) => Ok(b),
            other => Err(format!("{who} condition must be a bool, got {other:?}")),
        }
    }

    fn exec_intent(&mut self, source: &Expr, struct_name: &str, var: &str) -> Result<(), String> {
        let start = std::time::Instant::now();
        let raw = match self.eval(source)? {
            Value::Dict(m) => m,
            other => return Err(format!("intent source must be a dict, got {other:?}")),
        };
        let fields = self.structs.get(struct_name).ok_or_else(|| format!("unknown struct {struct_name:?}"))?.clone();
        let mapped = resolve_intent(&raw, &fields, 0.35);

        let confidences: Vec<f32> = mapped.values().filter_map(|v| match v {
            Value::Prob(_, c) => Some(*c),
            _ => None,
        }).collect();
        let detail = if confidences.is_empty() {
            "no Probability<T> fields".to_string()
        } else {
            format!("avg confidence={:.2}", confidences.iter().sum::<f32>() / confidences.len() as f32)
        };
        self.profile.push(ProfileEvent {
            kind: "intent".to_string(),
            label: format!("intent -> {struct_name} as {var}"),
            micros: start.elapsed().as_micros(),
            detail,
        });

        self.env.insert(var.to_string(), Value::Struct(mapped));
        Ok(())
    }

    fn exec_bound(&mut self, var: &str, op: &Comparator, limit: f64, body: &[Stmt]) -> Result<(), String> {
        let start = std::time::Instant::now();
        let mut ranges: HashMap<String, Interval> = HashMap::new();
        for (k, v) in &self.env {
            if let Value::Num(n) = v {
                ranges.insert(k.clone(), Interval::point(*n));
            }
        }

        let verdict = solver::solve_bound(var, op, limit, body, &ranges);
        let verdict_label = match &verdict {
            Verdict::Safe => "Safe (no guard needed)".to_string(),
            Verdict::Violation { stmt_index } => format!("Violation (statement {stmt_index} rejected, block not run)"),
            Verdict::Unknown { .. } => "Unknown (runtime guard checks each write)".to_string(),
        };
        let result = self.exec_bound_body(var, op, limit, body, verdict);
        self.profile.push(ProfileEvent {
            kind: "bound".to_string(),
            label: format!("bound {var} {op:?} {limit}"),
            micros: start.elapsed().as_micros(),
            detail: verdict_label,
        });
        result
    }

    fn exec_bound_body(&mut self, var: &str, op: &Comparator, limit: f64, body: &[Stmt], verdict: Verdict) -> Result<(), String> {
        match verdict {
            Verdict::Safe => {
                for s in body {
                    self.exec(s)?;
                }
                Ok(())
            }
            Verdict::Violation { stmt_index } => {
                eprintln!(
                    "[solver] rejected bound block on {var:?}: statement {stmt_index} provably violates \
                     {op:?} {limit}; block not executed, heap untouched"
                );
                Ok(())
            }
            Verdict::Unknown { .. } => self.exec_guarded_body(body, var, op, limit).map(|_| ()),
        }
    }

    /// Executes a bound block's body when the solver couldn't prove it
    /// safe, intercepting every write to `var` -- at any nesting depth
    /// inside `if`/`while`, not just directly in the block -- and checking
    /// it before it reaches `env`. `solve_bound` bails out to `Unknown`
    /// the moment it sees any `if`/`while` in the block specifically so
    /// this path, not an unguarded one, is what runs whenever control flow
    /// is present.
    fn exec_guarded(&mut self, stmt: &Stmt, var: &str, op: &Comparator, limit: f64) -> Result<Flow, String> {
        match stmt {
            Stmt::Set { name, expr } if name == var => {
                let candidate = self.eval(expr)?;
                let ok = match &candidate {
                    Value::Num(n) => match op {
                        Comparator::Le => *n <= limit,
                        Comparator::Ge => *n >= limit,
                    },
                    _ => false,
                };
                if ok {
                    self.env.insert(name.clone(), candidate);
                } else {
                    eprintln!(
                        "[solver] runtime guard rejected write: {name:?}={candidate:?} \
                         violates {op:?} {limit}; heap not updated"
                    );
                }
                Ok(Flow::Normal)
            }
            Stmt::If { cond, then_body, else_body } => {
                let taken = self.eval_bool(cond, "if")?;
                self.exec_guarded_body(if taken { then_body } else { else_body }, var, op, limit)
            }
            Stmt::While { cond, body } => {
                while self.eval_bool(cond, "while")? {
                    match self.exec_guarded_body(body, var, op, limit)? {
                        Flow::Normal => continue,
                        flow @ Flow::Return(_) => return Ok(flow),
                    }
                }
                Ok(Flow::Normal)
            }
            other => self.exec_flow(other),
        }
    }

    fn exec_guarded_body(&mut self, body: &[Stmt], var: &str, op: &Comparator, limit: f64) -> Result<Flow, String> {
        for s in body {
            match self.exec_guarded(s, var, op, limit)? {
                Flow::Normal => continue,
                flow @ Flow::Return(_) => return Ok(flow),
            }
        }
        Ok(Flow::Normal)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lexer::lex;
    use crate::parser::parse;

    fn run(src: &str) -> Runtime {
        let program = parse(lex(src).unwrap()).unwrap();
        let mut rt = Runtime::new();
        rt.run(&program).unwrap();
        rt
    }

    #[test]
    fn intent_maps_renamed_and_stringy_fields_and_defaults_missing_ones() {
        let rt = run(
            r#"
struct User {
  username: string default "",
  age: int default 0
}
let raw = {"usr_nm": "alice", "Age": "30"}
intent raw -> User as user
"#,
        );
        match &rt.env["user"] {
            Value::Struct(m) => {
                assert_eq!(m["username"], Value::Str("alice".into()));
                assert_eq!(m["age"], Value::Num(30.0));
            }
            other => panic!("expected Struct, got {other:?}"),
        }
    }

    #[test]
    fn probability_field_carries_confidence_and_resolve_collapses_it() {
        let rt = run(
            r#"
struct Reading {
  level: Probability<int>
}
let raw = {"levl": "42"}
intent raw -> Reading as r
let v = resolve r.level { above 0.3 => r.level, else => 0 }
"#,
        );
        match &rt.env["v"] {
            Value::Num(n) => assert_eq!(*n, 42.0),
            other => panic!("expected collapsed Num, got {other:?}"),
        }
    }

    #[test]
    fn safe_bound_block_executes_with_no_guard_needed() {
        let rt = run(
            r#"
let speed = 0
bound speed <= 15 {
  set speed = 10
}
"#,
        );
        assert_eq!(rt.env["speed"], Value::Num(10.0));
    }

    #[test]
    fn provably_unsafe_bound_block_never_executes() {
        let rt = run(
            r#"
let speed = 0
bound speed <= 15 {
  set speed = 22
}
"#,
        );
        assert_eq!(rt.env["speed"], Value::Num(0.0), "unsafe block must not have run at all");
    }

    #[test]
    fn undecidable_bound_gets_a_runtime_guard_before_commit() {
        let rt = run(
            r#"
struct Reading { level: int default 0 }
let raw = {"level": "10"}
intent raw -> Reading as r

let speed = 0
bound speed <= 15 {
  set speed = r.level
}
"#,
        );
        assert_eq!(rt.env["speed"], Value::Num(10.0), "10 <= 15, guard should allow the commit");

        let rt2 = run(
            r#"
struct Reading { level: int default 0 }
let raw = {"level": "999"}
intent raw -> Reading as r

let speed = 0
bound speed <= 15 {
  set speed = r.level
}
"#,
        );
        assert_eq!(rt2.env["speed"], Value::Num(0.0), "999 > 15, guard must reject before heap commit");
    }

    #[test]
    fn arithmetic_respects_precedence_and_parens() {
        let rt = run("let a = 2 + 3 * 4\nlet b = (2 + 3) * 4\n");
        assert_eq!(rt.env["a"], Value::Num(14.0));
        assert_eq!(rt.env["b"], Value::Num(20.0));
    }

    #[test]
    fn boolean_and_comparison_operators_work() {
        let rt = run("let a = 5 > 3 && 2 < 4\nlet b = 5 == 5 || false\nlet c = !false\n");
        assert_eq!(rt.env["a"], Value::Bool(true));
        assert_eq!(rt.env["b"], Value::Bool(true));
        assert_eq!(rt.env["c"], Value::Bool(true));
    }

    #[test]
    fn if_else_branches_correctly() {
        let rt = run(
            r#"
let x = 10
let y = 0
if x > 5 {
  set y = 1
} else {
  set y = 2
}
"#,
        );
        assert_eq!(rt.env["y"], Value::Num(1.0));
    }

    #[test]
    fn while_loop_sums_to_expected_total() {
        let rt = run(
            r#"
let i = 0
let total = 0
while i < 5 {
  set total = total + i
  set i = i + 1
}
"#,
        );
        assert_eq!(rt.env["total"], Value::Num(10.0)); // 0+1+2+3+4
    }

    #[test]
    fn recursive_function_computes_factorial() {
        let rt = run(
            r#"
fn factorial(n: int) -> int {
  if n <= 1 {
    return 1
  }
  return n * factorial(n - 1)
}
let result = factorial(5)
"#,
        );
        assert_eq!(rt.env["result"], Value::Num(120.0));
    }

    #[test]
    fn function_has_no_access_to_caller_locals() {
        let rt = run(
            r#"
let secret = 42
fn peek() -> int {
  return secret
}
"#,
        );
        let program = parse(lex("print(peek())").unwrap()).unwrap();
        let mut rt2 = rt;
        let err = rt2.run(&program).unwrap_err();
        assert!(err.contains("undefined variable"), "expected an undefined-variable error, got {err:?}");
    }

    #[test]
    fn bound_guard_catches_a_violation_nested_inside_if() {
        let rt = run(
            r#"
let speed = 0
let sensor = 999
bound speed <= 15 {
  if true {
    set speed = sensor
  }
}
"#,
        );
        assert_eq!(rt.env["speed"], Value::Num(0.0), "nested unsafe write must still be caught by the runtime guard");
    }

    #[test]
    fn bound_guard_allows_a_safe_write_nested_inside_if() {
        let rt = run(
            r#"
let speed = 0
let sensor = 10
bound speed <= 15 {
  if true {
    set speed = sensor
  }
}
"#,
        );
        assert_eq!(rt.env["speed"], Value::Num(10.0));
    }
}
