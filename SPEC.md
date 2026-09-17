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
                | intent_stmt
                | bound_stmt
                | print_stmt ;

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

expr            = dict_literal
                | field_access
                | resolve_expr
                | call_expr
                | identifier
                | literal ;

field_access    = identifier , "." , identifier ;

resolve_expr    = "resolve" , expr , "{" ,
                      "above" , number , "=>" , expr , "," ,
                      "else" , "=>" , expr ,
                  "}" ;                        (* collapses Probability<T> -> T *)

call_expr       = identifier , "(" , [ expr , { "," , expr } ] , ")" ;
                  (* calls a stdlib builtin -- see §7. No user-defined
                     functions or `use`/module-path syntax yet. *)

dict_literal    = "{" , [ pair , { "," , pair } ] , "}" ;
pair            = string , ":" , value ;
value           = string | number | boolean ;

literal         = string | number | boolean ;
```

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
  solver) falls back to a runtime check before each write commits.

```
let speed = 0
bound speed <= 15 {
  set speed = 10        # OK -- inside the bound scope, and provably safe
}
set speed = 999          # COMPILE ERROR -- outside the bound scope
```

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

- User-defined functions, `use`/module paths, arithmetic expressions,
  control flow (`if`/loops) — the grammar in §2 is everything that exists.
- LLVM IR codegen, a standalone tensor execution graph, GGML bindings.
- A hosted package registry; TLS in `net_get`; semver ranges in `annealpm`
  (exact version or `*`/latest only).
- General SMT reasoning beyond the bound rule — `solver.rs` documents
  exactly when the interval-style encoding it uses would need to become
  a richer one.
