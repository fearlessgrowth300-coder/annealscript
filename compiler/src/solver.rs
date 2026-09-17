//! Static safety solver for `bound` blocks -- backed by a real SMT solver
//! (Z3), not hand-rolled interval math.
//!
//! For each `set` targeting the bound variable with a statically-known
//! value range [lo, hi] (a literal, or a copy of a variable this module has
//! been told a range for), this asks Z3: "does there exist a real x in
//! [lo, hi] that violates the bound?" `Sat` => a violation is reachable =>
//! the whole block is rejected before it runs. `Unsat` => proven safe.
//! A value whose range can't be established at all (e.g. it flows through
//! an intent-resolved struct field) can't be posed to Z3 as a bounded
//! variable, so it falls back to `Verdict::Unknown` and the runtime's
//! per-write guard takes over -- see `runtime.rs`.
//!
//! The encoding is intentionally simple because the grammar is: `set`
//! targets can only be literals or bare variable copies, no arithmetic or
//! branching yet. The point of wiring Z3 now (instead of the interval
//! checker this module used to be) is that the same `Solver`/assert/`check`
//! call sites already generalize to richer constraints -- multi-step
//! arithmetic, multiple variables, disjunctive conditions -- once the
//! expression grammar grows into them, without changing the runtime's
//! call site into this module.

use crate::ast::{Comparator, Expr, Literal, Stmt};
use crate::interval::Interval;
use std::collections::HashMap;
use z3::SatResult;
use z3::ast::Real;

#[derive(Debug, PartialEq)]
pub enum Verdict {
    /// Provably safe: every assignment in the body stays within bound.
    /// The runtime can execute the block with zero guard overhead.
    Safe,
    /// Provably unsafe: some assignment always violates the bound. The
    /// block must not run at all -- nothing reaches heap memory.
    Violation { stmt_index: usize },
    /// Can't be decided statically (the value flows from data the solver
    /// can't see into, e.g. an intent-resolved struct field). Caller must
    /// fall back to a runtime guard that checks before each write commits.
    Unknown { stmt_index: usize },
}

fn known_range(expr: &Expr, env_ranges: &HashMap<String, Interval>) -> Option<(f64, f64)> {
    match expr {
        Expr::Literal(Literal::Num(n)) => Some((*n, *n)),
        Expr::Ident(name) => env_ranges.get(name).and_then(|iv| {
            if iv.is_unknown() {
                None
            } else {
                Some((iv.lo, iv.hi))
            }
        }),
        _ => None,
    }
}

/// Scale a float into a (numerator, denominator) pair for `Real::from_rational`,
/// at microsecond precision -- plenty for this grammar's literals.
fn to_rational(v: f64) -> (i64, i64) {
    const SCALE: f64 = 1_000_000.0;
    ((v * SCALE).round() as i64, SCALE as i64)
}

pub fn solve_bound(
    var: &str,
    op: &Comparator,
    limit: f64,
    body: &[Stmt],
    env_ranges: &HashMap<String, Interval>,
) -> Verdict {
    // Conservative bailout: this encoding reasons about a flat list of
    // assignments, not branches or loops. Rather than silently ignoring a
    // `set` hidden inside an `if`/`while` (which would let an unsafe
    // program get misclassified as `Safe`), any control flow in the block
    // makes the whole thing `Unknown` -- the runtime's per-write guard
    // (`Runtime::exec_guarded`) then walks the real nesting and catches
    // every write for real, at whatever depth it's at.
    if body.iter().any(|s| matches!(s, Stmt::If { .. } | Stmt::While { .. })) {
        return Verdict::Unknown { stmt_index: 0 };
    }

    // z3-rs manages an implicit thread-local Context/Solver plumbing; no
    // explicit Context needs to be created or threaded through here.
    for (i, stmt) in body.iter().enumerate() {
        let Stmt::Set { name, expr } = stmt else { continue };
        if name != var {
            continue;
        }
        let Some((lo, hi)) = known_range(expr, env_ranges) else {
            return Verdict::Unknown { stmt_index: i };
        };

        let x = Real::new_const("x");
        let (lo_n, lo_d) = to_rational(lo);
        let (hi_n, hi_d) = to_rational(hi);
        let (limit_n, limit_d) = to_rational(limit);
        let lo_r = Real::from_rational(lo_n, lo_d);
        let hi_r = Real::from_rational(hi_n, hi_d);
        let limit_r = Real::from_rational(limit_n, limit_d);

        let solver = z3::Solver::new();
        solver.assert(&x.ge(&lo_r));
        solver.assert(&x.le(&hi_r));
        let violation = match op {
            Comparator::Le => x.gt(&limit_r),
            Comparator::Ge => x.lt(&limit_r),
        };
        solver.assert(&violation);

        match solver.check() {
            SatResult::Sat => return Verdict::Violation { stmt_index: i },
            SatResult::Unsat => continue,
            SatResult::Unknown => return Verdict::Unknown { stmt_index: i },
        }
    }
    Verdict::Safe
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn proves_safe_when_literal_assignment_is_within_bound() {
        let body = vec![Stmt::Set { name: "speed".into(), expr: Expr::Literal(Literal::Num(10.0)) }];
        let verdict = solve_bound("speed", &Comparator::Le, 15.0, &body, &HashMap::new());
        assert_eq!(verdict, Verdict::Safe);
    }

    #[test]
    fn proves_violation_when_literal_assignment_exceeds_bound() {
        let body = vec![Stmt::Set { name: "speed".into(), expr: Expr::Literal(Literal::Num(22.0)) }];
        let verdict = solve_bound("speed", &Comparator::Le, 15.0, &body, &HashMap::new());
        assert_eq!(verdict, Verdict::Violation { stmt_index: 0 });
    }

    #[test]
    fn proves_safe_using_a_known_variable_range() {
        let mut ranges = HashMap::new();
        ranges.insert("cap".to_string(), Interval::point(12.0));
        let body = vec![Stmt::Set { name: "speed".into(), expr: Expr::Ident("cap".into()) }];
        let verdict = solve_bound("speed", &Comparator::Le, 15.0, &body, &ranges);
        assert_eq!(verdict, Verdict::Safe);
    }

    #[test]
    fn reports_unknown_for_unresolvable_sources() {
        let body = vec![Stmt::Set {
            name: "speed".into(),
            expr: Expr::FieldAccess("reading".into(), "level".into()),
        }];
        let verdict = solve_bound("speed", &Comparator::Le, 15.0, &body, &HashMap::new());
        assert_eq!(verdict, Verdict::Unknown { stmt_index: 0 });
    }

    #[test]
    fn bails_to_unknown_when_body_contains_control_flow() {
        let body = vec![Stmt::If {
            cond: Expr::Literal(Literal::Bool(true)),
            then_body: vec![Stmt::Set { name: "speed".into(), expr: Expr::Literal(Literal::Num(10.0)) }],
            else_body: vec![],
        }];
        let verdict = solve_bound("speed", &Comparator::Le, 15.0, &body, &HashMap::new());
        assert_eq!(verdict, Verdict::Unknown { stmt_index: 0 });
    }

    #[test]
    fn proves_violation_using_a_known_variable_range_that_exceeds_bound() {
        let mut ranges = HashMap::new();
        ranges.insert("cap".to_string(), Interval::point(99.0));
        let body = vec![Stmt::Set { name: "speed".into(), expr: Expr::Ident("cap".into()) }];
        let verdict = solve_bound("speed", &Comparator::Le, 15.0, &body, &ranges);
        assert_eq!(verdict, Verdict::Violation { stmt_index: 0 });
    }
}
