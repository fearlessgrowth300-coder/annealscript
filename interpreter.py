"""IntentScript toy interpreter.

Proves one mechanic: an `intent` block maps messy/mismatched dict data into a
declared struct by fuzzy key matching + type coercion, never raising on
missing or misnamed fields.

    struct User {
      username: string
      age: int
    }
    let raw = {"usr_nm": "alice", "Age": "30"}
    intent raw -> User as user
    print(user.username)

Run: python interpreter.py examples/self_heal.is
"""
import difflib
import re
import sys

KEYWORDS = {"struct", "let", "intent", "as", "print", "true", "false", "bound", "set"}
TYPES = {"string", "int", "float", "bool"}


# ---------- lexer ----------

TOKEN_RE = re.compile(r"""
    (?P<WS>[ \t]+)
  | (?P<NL>\n)
  | (?P<COMMENT>\#[^\n]*)
  | (?P<ARROW>->)
  | (?P<LE><=)
  | (?P<GE>>=)
  | (?P<STRING>"(?:[^"\\]|\\.)*")
  | (?P<NUMBER>\d+\.\d+|\d+)
  | (?P<IDENT>[A-Za-z_][A-Za-z0-9_]*)
  | (?P<PUNCT>[{}():,.=])
""", re.VERBOSE)


class Token:
    def __init__(self, kind, value, line):
        self.kind = kind
        self.value = value
        self.line = line

    def __repr__(self):
        return f"{self.kind}:{self.value!r}"


def lex(src):
    tokens = []
    depth = 0  # brace/paren depth; newlines are insignificant while > 0
    pos = 0
    line = 1
    while pos < len(src):
        m = TOKEN_RE.match(src, pos)
        if not m:
            raise SyntaxError(f"line {line}: unexpected character {src[pos]!r}")
        pos = m.end()
        kind = m.lastgroup
        text = m.group()
        if kind in ("WS", "COMMENT"):
            continue
        if kind == "NL":
            line += 1
            if depth == 0:
                tokens.append(Token("NEWLINE", "\n", line))
            continue
        if kind == "PUNCT":
            if text in "({":
                depth += 1
            elif text in ")}":
                depth = max(0, depth - 1)
            tokens.append(Token(text, text, line))
            continue
        if kind == "ARROW":
            tokens.append(Token("ARROW", text, line))
            continue
        if kind in ("LE", "GE"):
            tokens.append(Token(text, text, line))
            continue
        if kind == "STRING":
            tokens.append(Token("STRING", text[1:-1], line))
            continue
        if kind == "NUMBER":
            tokens.append(Token("NUMBER", text, line))
            continue
        if kind == "IDENT":
            if text in KEYWORDS:
                tokens.append(Token(text, text, line))
            else:
                tokens.append(Token("IDENT", text, line))
            continue
    tokens.append(Token("EOF", None, line))
    return tokens


# ---------- AST ----------

class StructDef:
    def __init__(self, name, fields):
        self.name = name
        self.fields = fields  # list of (name, type)


class LetStmt:
    def __init__(self, name, expr):
        self.name = name
        self.expr = expr


class IntentStmt:
    def __init__(self, source_expr, struct_name, var_name):
        self.source_expr = source_expr
        self.struct_name = struct_name
        self.var_name = var_name


class PrintStmt:
    def __init__(self, expr):
        self.expr = expr


class SetStmt:
    def __init__(self, name, expr):
        self.name = name
        self.expr = expr


class BoundStmt:
    """Wraps statements with a deterministic constraint on a variable:
    after every statement in body, `var` is clamped back within
    `op`/`limit` if dynamic/probabilistic code pushed it out of range."""
    def __init__(self, var_name, op, limit, body):
        self.var_name = var_name
        self.op = op
        self.limit = limit
        self.body = body


class DictLiteral:
    def __init__(self, pairs):
        self.pairs = pairs  # list of (key_str, value)


class Literal:
    def __init__(self, value):
        self.value = value


class FieldAccess:
    def __init__(self, obj_name, field_name):
        self.obj_name = obj_name
        self.field_name = field_name


class Name:
    def __init__(self, name):
        self.name = name


# ---------- parser ----------

class Parser:
    def __init__(self, tokens):
        self.tokens = tokens
        self.i = 0

    def peek(self):
        return self.tokens[self.i]

    def advance(self):
        tok = self.tokens[self.i]
        self.i += 1
        return tok

    def expect(self, kind):
        tok = self.peek()
        if tok.kind != kind:
            raise SyntaxError(f"line {tok.line}: expected {kind}, got {tok.kind}")
        return self.advance()

    def skip_newlines(self):
        while self.peek().kind == "NEWLINE":
            self.advance()

    def parse_program(self):
        stmts = []
        self.skip_newlines()
        while self.peek().kind != "EOF":
            stmts.append(self.parse_statement())
            self.skip_newlines()
        return stmts

    def parse_statement(self):
        tok = self.peek()
        if tok.kind == "struct":
            return self.parse_struct()
        if tok.kind == "let":
            return self.parse_let()
        if tok.kind == "intent":
            return self.parse_intent()
        if tok.kind == "print":
            return self.parse_print()
        if tok.kind == "set":
            return self.parse_set()
        if tok.kind == "bound":
            return self.parse_bound()
        raise SyntaxError(f"line {tok.line}: unexpected token {tok.kind}")

    def parse_struct(self):
        self.expect("struct")
        name = self.expect("IDENT").value
        self.expect("{")
        fields = []
        while self.peek().kind != "}":
            fname = self.expect("IDENT").value
            self.expect(":")
            ftype = self.expect("IDENT").value
            if ftype not in TYPES:
                raise SyntaxError(f"unknown type {ftype!r} for field {fname!r}")
            fields.append((fname, ftype))
            if self.peek().kind == ",":
                self.advance()
        self.expect("}")
        return StructDef(name, fields)

    def parse_let(self):
        self.expect("let")
        name = self.expect("IDENT").value
        self.expect("=")
        expr = self.parse_expr()
        return LetStmt(name, expr)

    def parse_intent(self):
        self.expect("intent")
        source_expr = self.parse_expr()
        self.expect("ARROW")
        struct_name = self.expect("IDENT").value
        self.expect("as")
        var_name = self.expect("IDENT").value
        return IntentStmt(source_expr, struct_name, var_name)

    def parse_print(self):
        self.expect("print")
        self.expect("(")
        expr = self.parse_expr()
        self.expect(")")
        return PrintStmt(expr)

    def parse_set(self):
        self.expect("set")
        name = self.expect("IDENT").value
        self.expect("=")
        expr = self.parse_expr()
        return SetStmt(name, expr)

    def parse_bound(self):
        self.expect("bound")
        var_name = self.expect("IDENT").value
        op_tok = self.peek()
        if op_tok.kind not in ("<=", ">="):
            raise SyntaxError(f"line {op_tok.line}: expected <= or >= in bound")
        self.advance()
        limit_tok = self.expect("NUMBER")
        limit = float(limit_tok.value) if "." in limit_tok.value else int(limit_tok.value)
        self.expect("{")
        self.skip_newlines()
        body = []
        while self.peek().kind != "}":
            body.append(self.parse_statement())
            self.skip_newlines()
        self.expect("}")
        return BoundStmt(var_name, op_tok.kind, limit, body)

    def parse_expr(self):
        tok = self.peek()
        if tok.kind == "{":
            return self.parse_dict()
        if tok.kind == "STRING":
            self.advance()
            return Literal(tok.value)
        if tok.kind == "NUMBER":
            self.advance()
            return Literal(float(tok.value) if "." in tok.value else int(tok.value))
        if tok.kind in ("true", "false"):
            self.advance()
            return Literal(tok.kind == "true")
        if tok.kind == "IDENT":
            self.advance()
            if self.peek().kind == ".":
                self.advance()
                field = self.expect("IDENT").value
                return FieldAccess(tok.value, field)
            return Name(tok.value)
        raise SyntaxError(f"line {tok.line}: unexpected token {tok.kind} in expression")

    def parse_dict(self):
        self.expect("{")
        pairs = []
        while self.peek().kind != "}":
            key = self.expect("STRING").value
            self.expect(":")
            value = self.parse_value()
            pairs.append((key, value))
            if self.peek().kind == ",":
                self.advance()
        self.expect("}")
        return DictLiteral(pairs)

    def parse_value(self):
        tok = self.peek()
        if tok.kind == "STRING":
            self.advance()
            return tok.value
        if tok.kind == "NUMBER":
            self.advance()
            return float(tok.value) if "." in tok.value else int(tok.value)
        if tok.kind in ("true", "false"):
            self.advance()
            return tok.kind == "true"
        raise SyntaxError(f"line {tok.line}: unexpected token {tok.kind} in dict value")


# ---------- intent resolution (the core mechanic) ----------

def _normalize(key):
    return re.sub(r"[^a-z0-9]", "", key.lower())


def coerce(value, target_type):
    """Coerce value to target_type. Returns (ok, coerced_value)."""
    if target_type == "string":
        return True, str(value)
    if target_type == "int":
        if isinstance(value, bool):
            return False, None
        if isinstance(value, int):
            return True, value
        if isinstance(value, float) and value.is_integer():
            return True, int(value)
        if isinstance(value, str) and re.fullmatch(r"-?\d+", value.strip()):
            return True, int(value.strip())
        return False, None
    if target_type == "float":
        if isinstance(value, bool):
            return False, None
        if isinstance(value, (int, float)):
            return True, float(value)
        if isinstance(value, str):
            try:
                return True, float(value.strip())
            except ValueError:
                return False, None
        return False, None
    if target_type == "bool":
        if isinstance(value, bool):
            return True, value
        if isinstance(value, str) and value.strip().lower() in ("true", "false"):
            return True, value.strip().lower() == "true"
        if isinstance(value, (int, float)) and value in (0, 1):
            return True, bool(value)
        return False, None
    return False, None


def resolve_intent(raw, fields, threshold=0.4):
    """Fuzzy-map raw dict keys onto struct fields, coercing types.

    Never raises: an unresolved or uncoercible field becomes None, and a
    warning is printed to stderr — this IS the self-healing behavior.
    """
    available = list(raw.keys())
    normalized_available = {k: _normalize(k) for k in available}
    result = {}
    for fname, ftype in fields:
        target_norm = _normalize(fname)
        best_key, best_score = None, 0.0
        for k in available:
            score = difflib.SequenceMatcher(None, target_norm, normalized_available[k]).ratio()
            if score > best_score:
                best_key, best_score = k, score
        if best_key is not None and best_score >= threshold:
            ok, coerced = coerce(raw[best_key], ftype)
            if ok:
                result[fname] = coerced
                available.remove(best_key)
                continue
            print(f"[intent] warning: field {fname!r} matched {best_key!r} "
                  f"but value {raw[best_key]!r} does not coerce to {ftype}; using None",
                  file=sys.stderr)
        else:
            print(f"[intent] warning: no source field found for {fname!r} "
                  f"(needed for {ftype}); using None", file=sys.stderr)
        result[fname] = None
    return result


# ---------- interpreter ----------

class StructInstance:
    def __init__(self, struct_name, values):
        self.struct_name = struct_name
        self.values = values


class Interpreter:
    def __init__(self):
        self.structs = {}
        self.env = {}

    def run(self, stmts):
        for stmt in stmts:
            self.exec_stmt(stmt)

    def exec_stmt(self, stmt):
        if isinstance(stmt, StructDef):
            self.structs[stmt.name] = stmt.fields
        elif isinstance(stmt, LetStmt):
            self.env[stmt.name] = self.eval_expr(stmt.expr)
        elif isinstance(stmt, IntentStmt):
            raw = self.eval_expr(stmt.source_expr)
            if not isinstance(raw, dict):
                raise TypeError("intent source must be a dict/struct-like value")
            if stmt.struct_name not in self.structs:
                raise NameError(f"unknown struct {stmt.struct_name!r}")
            fields = self.structs[stmt.struct_name]
            values = resolve_intent(raw, fields)
            self.env[stmt.var_name] = StructInstance(stmt.struct_name, values)
        elif isinstance(stmt, PrintStmt):
            print(self.eval_expr(stmt.expr))
        elif isinstance(stmt, SetStmt):
            if stmt.name not in self.env:
                raise NameError(f"cannot set undeclared variable {stmt.name!r}")
            self.env[stmt.name] = self.eval_expr(stmt.expr)
        elif isinstance(stmt, BoundStmt):
            self.exec_bound(stmt)
        else:
            raise TypeError(f"unknown statement {stmt!r}")

    def exec_bound(self, stmt):
        for inner in stmt.body:
            self.exec_stmt(inner)
            if stmt.var_name in self.env:
                value = self.env[stmt.var_name]
                violated = (stmt.op == "<=" and value > stmt.limit) or \
                           (stmt.op == ">=" and value < stmt.limit)
                if violated:
                    print(f"[bound] warning: {stmt.var_name}={value!r} violates "
                          f"{stmt.op} {stmt.limit}; clamped to {stmt.limit}",
                          file=sys.stderr)
                    self.env[stmt.var_name] = stmt.limit

    def eval_expr(self, expr):
        if isinstance(expr, Literal):
            return expr.value
        if isinstance(expr, DictLiteral):
            return dict(expr.pairs)
        if isinstance(expr, Name):
            return self.env[expr.name]
        if isinstance(expr, FieldAccess):
            obj = self.env[expr.obj_name]
            if isinstance(obj, StructInstance):
                return obj.values.get(expr.field_name)
            return obj[expr.field_name]
        raise TypeError(f"unknown expression {expr!r}")


def run_source(src):
    tokens = lex(src)
    stmts = Parser(tokens).parse_program()
    interp = Interpreter()
    interp.run(stmts)
    return interp


if __name__ == "__main__":
    if len(sys.argv) != 2:
        print("usage: python interpreter.py <file.is>", file=sys.stderr)
        sys.exit(1)
    with open(sys.argv[1], "r", encoding="utf-8") as f:
        source = f.read()
    run_source(source)
