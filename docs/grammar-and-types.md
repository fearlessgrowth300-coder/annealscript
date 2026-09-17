# IntentScript — Phase 1: Grammar & Type Specification

> **Superseded by [`/SPEC.md`](../SPEC.md).** Kept for history: this was
> the original design spec, written before any of it was implemented. Most
> of what it lists as "deferred" in §8 is now real — see SPEC.md's
> implementation-status sections instead of trusting this file's dates.

Status: design spec. The Phase 0 prototype (`interpreter.py`) implements a
subset of this grammar with no compile-time checking (it clamps/defaults at
runtime instead of rejecting at compile time). Gaps are called out inline.

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
                | identifier
                | literal ;

field_access    = identifier , "." , identifier ;

resolve_expr    = "resolve" , expr , "{" ,
                      "above" , number , "=>" , expr , "," ,
                      "else" , "=>" , expr ,
                  "}" ;                        (* collapses Probability<T> -> T *)

dict_literal    = "{" , [ pair , { "," , pair } ] , "}" ;
pair            = string , ":" , value ;
value           = string | number | boolean ;

literal         = string | number | boolean ;
```

`resolve_expr` is new in this phase (not yet implemented) — it is the only
way a `Probability<T>` becomes a plain `T`. Everything else above matches
what `interpreter.py` already parses, except `type` annotations on `let` and
`default_clause` on struct fields, both new here.

## 3. Type universe

- **Deterministic types**: `int`, `float`, `bool`, `string`, and any
  user-declared `struct` name.
- **Probabilistic type**: `Probability<T>` for any type `T`. A value of this
  type is a pair `(value: T, confidence: float in [0.0, 1.0])`.
- **No implicit conversion** exists between `T` and `Probability<T>` in
  either direction. A `Probability<T>` can only become `T` through
  `resolve_expr` (§5). Assigning a `Probability<T>` where `T` is expected,
  or vice versa, is a compile-time type error.

Sources of `Probability<T>` values:
- A field of an `intent`-mapped struct declared as `Probability<T>` receives
  the fuzzy-match confidence score from the source-key resolution (see §4).
- (Future phase) local model inference expressions.

## 4. Intent boundary rule (compile-time)

For `intent source -> StructName as var`, every field `f: T` of
`StructName` where `T` is a **deterministic** type (not `Probability<T>`)
must carry a `default` clause:

```
struct User {
  username: string default ""
  age: int default 0
}
```

Rationale: the source of an `intent` mapping is untyped external data
(scraped HTML, a third-party JSON payload) whose shape is not known at
compile time. A deterministic field can never be allowed to end up in an
undefined state at runtime (that's the crash `intent` exists to prevent),
so the compiler forces the fallback to be declared up front instead of
silently defaulting to `None`/null the way the Phase 0 prototype does.

Compile error, if no default is present and the field isn't wrapped:
```
field 'age' is deterministic but reachable via intent without a default;
add 'default <value>' or declare 'age: Probability<int>'
```

A field declared `Probability<T>` needs no default — an unresolved or
low-confidence match is still a valid `Probability<T>` value (confidence
near 0), just not yet collapsed to a usable `T`.

## 5. Collapsing `Probability<T>` to `T`

```
let trust: Probability<float> = ...
let verified: float = resolve trust {
  above 0.8 => trust,
  else => 0.0
}
```

`resolve` is the only expression of type `T` that may read a
`Probability<T>`. This is what a deterministic consumer (a `bound` block,
a function expecting `int`, a struct field typed `T`) must go through.

## 6. Bound boundary rule (compile-time)

- `bound` accepts only a variable of deterministic numeric type (`int` or
  `float`). Binding a `Probability<int>` directly is a type error — collapse
  it with `resolve` first.
- `limit` must be a compile-time numeric constant of the same type as the
  bound variable.
- **Scope enforcement**: while a variable is under an active `bound`
  constraint, every `set` to it must occur lexically inside that `bound`
  block (or a block nested within it). A `set` reaching that variable from
  outside the block is a compile-time error, not a runtime clamp:

```
let speed = 0

bound speed <= 15 {
  set speed = 10        # ok — inside the bound scope
}

set speed = 999          # ERROR: 'speed' is safety-bounded in the scope
                          # above; mutation outside it is rejected
```

This upgrades the Phase 0 runtime behavior (clamp-after-the-fact, see
`exec_bound` in `interpreter.py`) into a static guarantee: an unbounded
write to a safety-critical variable is caught before the program runs, not
patched after it misbehaves once.

## 7. Worked rejection example

```
struct User {
  username: string default ""
  age: int                        # ERROR: no default, not Probability-wrapped
  trust: Probability<float>       # OK
}
```

## 8. What's deferred past this phase

- Parsing/type-checking implementation for `type` annotations, `default`,
  `Probability<T>`, and `resolve` — Phase 1 is the spec only.
- Static scope analysis for the bound rule (§6) — the prototype only clamps
  at runtime inside the block.
- Confidence-score propagation from the fuzzy matcher into an actual
  `Probability<T>` runtime value (today it's discarded; only the coerced
  value or `None` survives).
