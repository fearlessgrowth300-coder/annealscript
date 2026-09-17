# AnnealScript Language Specification

Canonical grammar, type system, and implementation status. This supersedes
the Phase 1 draft at `docs/grammar-and-types.md` (kept for history) — it
reflects what's actually built and enforced by `compiler/` today, not just
what was originally proposed.

## 1. Lexical grammar

```
letter      = "a".."z" | "A".."Z" | "_" ;
digit       = "0".."9" ;
identifier  = letter , { letter | digit } ;
number      = digit , { digit } , [ "." , digit , { digit } ] ;
string      = '"' , { character - ('"' | "\") | escape } , '"' ;
escape      = "\" , character ;
boolean     = "true" | "false" ;
comment     = "#" , { character - newline } ;
```

## 2. Syntactic grammar (EBNF)

```
program         = { statement } ;

statement       = struct_def
                | let_stmt
                | set_stmt
                | index_set_stmt
                | intent_stmt
                | bound_stmt
                | print_stmt
                | if_stmt
                | while_stmt
                | for_stmt
                | fn_def
                | return_stmt
                | "break"
                | "continue" ;

if_stmt         = "if" , expr , block , [ "else" , ( block | if_stmt ) ] ;
while_stmt      = "while" , expr , block ;
for_stmt        = "for" , identifier , "in" , expr , block ;  (* expr must be a list *)
block           = "{" , { statement } , "}" ;

(* `set xs[i] = v` (list, bounds-checked) or `set d[k] = v` (dict, always
   inserts/overwrites) -- a distinct statement from set_stmt, not a
   general lvalue on it; see the note on `Stmt::IndexSet` in ast.rs. *)
index_set_stmt  = "set" , identifier , "[" , expr , "]" , "=" , expr ;

fn_def          = "fn" , identifier , "(" , [ param , { "," , param } ] , ")" ,
                   [ "->" , type ] , block ;
param           = identifier , ":" , type ;
return_stmt     = "return" , [ expr ] ;

struct_def      = "struct" , identifier , "{" , field_list , "}" ;
field_list      = [ field , { "," , field } ] ;
field           = identifier , ":" , type , [ default_clause ] ;
default_clause  = "default" , literal ;

type            = deterministic_type
                | probabilistic_type
                | identifier ;               (* reference to a struct type *)

deterministic_type  = "int" | "float" | "bool" | "string" ;
probabilistic_type  = "Probability" , "<" , type , ">" ;

let_stmt        = "let" , identifier , [ ":" , type ] , "=" , expr ;
set_stmt        = "set" , identifier , "=" , expr ;

intent_stmt     = "intent" , expr , "->" , identifier , "as" , identifier ;

bound_stmt      = "bound" , identifier , comparator , number , "{" , { statement } , "}" ;
comparator      = "<=" | ">=" ;

print_stmt      = "print" , "(" , expr , ")" ;

(* precedence, low to high: or, and, equality, comparison, additive,
   multiplicative, unary, primary -- standard precedence-climbing *)
expr            = or_expr ;
or_expr         = and_expr , { "||" , and_expr } ;
and_expr        = equality_expr , { "&&" , equality_expr } ;
equality_expr   = comparison_expr , { ( "==" | "!=" ) , comparison_expr } ;
comparison_expr = additive_expr , { ( "<" | ">" | "<=" | ">=" ) , additive_expr } ;
additive_expr   = multiplicative_expr , { ( "+" | "-" ) , multiplicative_expr } ;
multiplicative_expr = unary_expr , { ( "*" | "/" | "%" ) , unary_expr } ;
unary_expr      = [ "-" | "!" ] , unary_expr | postfix_expr ;

(* a primary followed by zero or more `[index]` suffixes, e.g. `xs[0][1]` *)
postfix_expr    = primary_expr , { "[" , expr , "]" } ;

primary_expr    = dict_literal
                | list_literal
                | field_access
                | resolve_expr
                | call_expr
                | "(" , expr , ")"
                | identifier
                | literal ;

list_literal    = "[" , [ expr , { "," , expr } ] , "]" ;

field_access    = identifier , "." , identifier ;

resolve_expr    = "resolve" , expr , "{" ,
                      "above" , number , "=>" , expr , "," ,
                      "else" , "=>" , expr ,
                  "}" ;                        (* collapses Probability<T> -> T *)

call_expr       = identifier , "(" , [ expr , { "," , expr } ] , ")" ;
                  (* calls a stdlib builtin (§7) OR a user-defined `fn` --
                     resolved at the call site. No `use`/module-path syntax
                     to disambiguate namespaces yet. *)

dict_literal    = "{" , [ pair , { "," , pair } ] , "}" ;
pair            = string , ":" , expr ;   (* values are full expressions, see §6c *)

literal         = string | number | boolean ;
```

Arithmetic on `Value::Num`, string `+` is concatenation, `==`/`!=` work on
any two values of the same shape (structural equality), everything else
requires matching types (no implicit coercion at runtime either).

## 3. Type universe

- **Deterministic types**: `int`, `float`, `bool`, `string`, and any
  user-declared `struct` name.
- **Probabilistic type**: `Probability<T>` for any type `T`. A value of this
  type is a pair `(value: T, confidence: float in [0.0, 1.0])`.
- **No implicit conversion** exists between `T` and `Probability<T>` in
  either direction. A `Probability<T>` can only become `T` through
  `resolve_expr` (§5). Assigning a `Probability<T>` where `T` is expected,
  or vice versa, is a compile-time type error.

Sources of `Probability<T>` values: a field of an `intent`-mapped struct
declared as `Probability<T>` receives a real confidence score from the
tensor-lane key matcher (see §6) — this is implemented, not aspirational;
see `runtime.rs`'s `resolve_intent`.

## 4. Intent boundary rule (enforced at compile time)

For `intent source -> StructName as var`, every field `f: T` of
`StructName` where `T` is a **deterministic** type (not `Probability<T>`)
must carry a `default` clause. **Enforced** by `compiler/src/typecheck.rs`
(`check_intent_boundary`) — a struct without one fails to compile:

```
struct User {
  username: string default ""
  age: int default 0
  trust: Probability<float>       # no default needed
}
```

## 5. Collapsing `Probability<T>` to `T`

```
let v = resolve r.level { above 0.8 => r.level, else => 0 }
```

`resolve` reads a `Probability<T>` and produces `T` by choosing a branch on
confidence, then unwrapping if the chosen branch is itself a `Probability`.
Implemented in `runtime.rs`'s `eval`.

## 6. Bound rule (enforced two ways)

- **Static** (`compiler/src/typecheck.rs`, `check_bound_scope`): while a
  variable is under an active `bound` constraint, every `set` to it must
  occur lexically inside that block. A `set` anywhere else in the same
  scope is a compile error.
- **Dynamic** (`compiler/src/solver.rs`, backed by a real Z3 SMT query):
  for each `set` inside the block, `solve_bound` asks Z3 whether a
  violating value is reachable given what's known about the assigned
  expression. `Safe` runs the block with no guard; `Violation` rejects the
  whole block before it executes (nothing reaches the heap); `Unknown`
  (e.g. the value flows through an intent-resolved field, opaque to the
  solver) falls back to a runtime check before each write commits. A
  `bound` body containing `if`/`while` is **always** `Unknown` -- the Z3
  encoding reasons about a flat list of assignments, not branches or
  loops, so it conservatively defers rather than risk missing a write
  hidden in a branch. The runtime guard (`Runtime::exec_guarded`) still
  walks the real nesting and catches every write to the bound variable at
  whatever depth it's at, `if`/`while`/`for` included.

```
let speed = 0
bound speed <= 15 {
  set speed = 10        # OK -- inside the bound scope, and provably safe
}
set speed = 999          # COMPILE ERROR -- outside the bound scope
```

## 6a. Functions and control flow

AnnealScript is general-purpose as of this section: `if`/`else`, `while`,
and user-defined `fn` with parameters, a return type, and `return` make it
Turing-complete, not just a narrow schema/safety DSL.

```
fn factorial(n: int) -> int {
  if n <= 1 {
    return 1
  }
  return n * factorial(n - 1)
}
```

- **Closures capture by value, at definition time.** When a `fn` statement
  executes, `env` at that moment is snapshotted into the function's entry;
  calling the function starts from that snapshot with the arguments
  overlaid on top (params shadow same-named captures). A `set` on a
  captured name inside the function only mutates its own local copy --
  the caller's actual variable is restored untouched when the call
  returns, because calling swaps `env` out entirely and swaps it back
  afterward. This means: a function sees whatever its enclosing scope held
  *when it was defined*, not live updates made after that point (see
  `ast.rs`'s note on `Stmt::FnDef`); real reference-capturing closures
  would need shared mutable cells (`Rc<RefCell<...>>`) threaded through
  the environment, which nothing so far has needed.
- **Hoisting**: top-level `fn` definitions can be called before their
  textual position in the file (`Runtime::run` registers them all before
  executing anything, with an empty captured scope at that point).
  Execution re-captures a fresher snapshot once it naturally reaches the
  `fn` statement. A function defined inside a nested block is only
  registered once execution reaches it -- no hoisting there.
- **No unit type in the surface syntax**: a bare `return` and a function
  that falls off the end of its body both produce `Value::Unit` internally
  (prints as `()`), but there's no way to name that type in a signature.
- The bound-scope rule (§6) also holds across `if`/`while`/`for` at the
  same scope, but treats a function body as a clean slate with no active
  bounds inherited from the caller -- see `typecheck.rs`'s `walk_scope`.
  This holds even with closures: writes inside a function body can never
  reach the caller's actual storage regardless of what it captured, for
  the same "swap env out, swap it back" reason above.
- **`break`/`continue`** work inside `while`/`for` the way they do in any
  imperative language (`break` stops the loop; `continue` skips to the
  next iteration's condition check). There's no static check for using
  either outside a loop -- it's a runtime error ("'break'/'continue' used
  outside of a loop") instead.

## 6b. Lists

```
let xs = [1, 2, 3]
let first = xs[0]
set xs[1] = 99          # in-place index assignment
for x in xs {
  print(x)
}
```

Lists are `Value::List` in `runtime.rs`. `set xs[i] = v` (`index_set_stmt`,
bounds-checked) mutates the list already bound to `xs` in place.
`list_push` (§7) is still functional -- it returns a *new* list with the
item appended, since changing a list's length isn't index assignment's
job; the idiom for growing one is `set xs = list_push(xs, item)`. `for var
in expr` requires `expr` to evaluate to a list; `var` is bound in the
current scope for each element in turn and keeps its last value after the
loop ends, the same way Python's own `for` variable leaks. There's no
reference/aliasing between two `let`-bound names -- `let ys = xs` copies,
so mutating `ys` never affects `xs`.

## 6c. Dicts

```
let x = 5
let d = {"a": x + 1, "b": "hello"}
let first = d["a"]      # 6
set d["c"] = 100          # inserts or overwrites a key
```

`{...}` is now a general-purpose dict literal -- `Value::Dict` in
`runtime.rs`, holding arbitrary `Value`s (previously only literal values,
used solely as an `intent` source). `d[key]` reads by string key (a
missing key is a runtime error, not `None`/`null`); `set d[key] = v`
always succeeds, inserting the key if it wasn't already there. Same
value/no-aliasing semantics as lists (§6b). This is also still exactly
what `intent` expects as a source, unchanged -- `resolve_intent` reads a
`Value::Dict` directly, so any dict expression (not just a literal) can
feed an `intent` block now.

## 7. Standard library (flat builtins, `compiler/src/stdlib.rs`)

No `use`/module-path syntax exists yet, so these are called directly by
name; each is namespaced in name only:

| Call | Module | Does |
|---|---|---|
| `fs_read_file(path)` / `fs_write_file(path, s)` / `fs_exists(path)` | `std::fs` | real file I/O |
| `net_get(url)` | `std::net` | HTTP/1.1 GET over `std::net::TcpStream`; **plain HTTP only, no TLS** |
| `safety_clamp(v, lo, hi)` | `std::safety` | clamps a value into `[lo, hi]` |
| `tensor_similarity(a, b)` | `std::tensor` | runs the real ONNX Runtime quantized-similarity graph (see §8) |
| `html_extract(html, {field: css_selector, ...})` | `std::html` | real CSS-selector extraction (via `scraper`/html5ever) into a dict; first match only, text content only |
| `list_len(xs)` / `list_push(xs, item)` | (list helpers) | length, and a new list with `item` appended (`list_push` is functional -- growing a list is not the same operation as `set xs[i] = v`, see §6b) |

## 8. Execution model

- **Deterministic statements** (`let`, `set`, `print`, `bound`, struct
  defs) execute as native tree-walking evaluation (`runtime.rs`).
- **`intent` blocks** hash candidate keys into int8[32] vectors and score
  them via a real ONNX Runtime session running a hand-authored (not
  trained) cosine-similarity graph (`tensor_onnx.rs`,
  `models/build_similarity_model.py`).
- **Compilation to LLVM IR / a standalone tensor execution graph** is
  *not* implemented. `split.rs` partitions a parsed program into the two
  eventual codegen targets (a real, tested classification pass), but
  nothing lowers either bucket to IR yet — `main.rs` prints the split and
  then runs the program through the tree-walking runtime instead.

## 9. Tooling

- **annealpm** (`annealpm/`): a package manager over a local filesystem
  registry (no hosted server), handling both `lib` (AnnealScript source)
  and `model` (versioned `.onnx` + metadata) packages, with real SHA-256
  content hashes in `anneal.lock`.
- **annealscript-lsp** (`annealscript-lsp/`): an LSP server providing
  diagnostics (parse errors with real line numbers; the two rules in §4/§6),
  a CodeLens over `intent`/`bound` lines that runs the file and reports
  real measured latency/confidence/verdict, driving the VS Code/Cursor
  extension in `vscode-extension/`.
- **Syntax highlighting** is a TextMate grammar
  (`vscode-extension/syntaxes/annealscript.tmLanguage.json`), not an LSP
  feature — the editor's native mechanism already does this.

## 10. Explicitly out of scope so far

- `use`/module paths, classes/methods, exceptions/`try`-`catch` — each is
  a separate, substantially larger undertaking than what's here (a real
  object model, or a new `Flow` variant threaded through every
  propagation site including the guarded-execution path) and deserves its
  own pass rather than being rushed in alongside a smaller batch.
- LLVM IR codegen, a standalone tensor execution graph, GGML bindings —
  this is still an interpreter, not a compiler to machine code, despite
  the name.
- A hosted package registry; TLS in `net_get`; semver ranges in `annealpm`
  (exact version or `*`/latest only).
- General SMT reasoning over branches/loops — `solver.rs` conservatively
  defers to the runtime guard for any `bound` body containing
  `if`/`while`/`for` rather than attempt it (§6).
