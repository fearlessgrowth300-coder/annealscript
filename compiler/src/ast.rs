//! AST for the grammar defined in docs/grammar-and-types.md (Phase 1).

#[derive(Debug, Clone, PartialEq)]
pub enum Type {
    Int,
    Float,
    Bool,
    String,
    Probability(Box<Type>),
    Named(String),
}

#[derive(Debug, Clone, PartialEq)]
pub enum Literal {
    Str(String),
    Num(f64),
    Bool(bool),
}

#[derive(Debug, Clone, PartialEq)]
pub enum Comparator {
    Le,
    Ge,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    And,
    Or,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum UnOp {
    Neg,
    Not,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    Literal(Literal),
    Ident(String),
    FieldAccess(String, String),
    Dict(Vec<(String, Literal)>),
    Resolve {
        subject: Box<Expr>,
        threshold: f64,
        above: Box<Expr>,
        else_branch: Box<Expr>,
    },
    /// A call to a stdlib builtin (`stdlib.rs`) OR a user-defined `fn`
    /// (`Stmt::FnDef`) -- resolved by the runtime at the call site, since
    /// there's still no module/`use` syntax to disambiguate namespaces.
    Call {
        name: String,
        args: Vec<Expr>,
    },
    Binary {
        op: BinOp,
        left: Box<Expr>,
        right: Box<Expr>,
    },
    Unary {
        op: UnOp,
        expr: Box<Expr>,
    },
    List(Vec<Expr>),
    Index {
        list: Box<Expr>,
        index: Box<Expr>,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct Field {
    pub name: String,
    pub ty: Type,
    pub default: Option<Literal>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Stmt {
    StructDef {
        name: String,
        fields: Vec<Field>,
    },
    Let {
        name: String,
        ty: Option<Type>,
        expr: Expr,
    },
    Set {
        name: String,
        expr: Expr,
    },
    Intent {
        source: Expr,
        struct_name: String,
        var: String,
    },
    Bound {
        var: String,
        op: Comparator,
        limit: f64,
        body: Vec<Stmt>,
    },
    Print(Expr),
    If {
        cond: Expr,
        then_body: Vec<Stmt>,
        else_body: Vec<Stmt>,
    },
    While {
        cond: Expr,
        body: Vec<Stmt>,
    },
    /// Iterates `iter` (must evaluate to a `List`), binding each element to
    /// `var` in the current scope in turn -- same "no block scoping" model
    /// as everything else here, so `var` keeps its last value after the
    /// loop ends, same as Python's own `for` variable does.
    For {
        var: String,
        iter: Expr,
        body: Vec<Stmt>,
    },
    /// Closures capture their defining scope BY VALUE at the moment the
    /// `fn` statement executes (a snapshot, not a live reference) -- see
    /// `Runtime::exec_flow`'s `Stmt::FnDef` arm. That means a function
    /// sees whatever its enclosing scope's variables held at definition
    /// time, but a later `set` to one of those variables in the outer
    /// scope is invisible to an already-defined closure. Real lexical
    /// capture-by-reference would need shared mutable cells
    /// (`Rc<RefCell<...>>`) threaded through the environment; nothing so
    /// far has needed that extra machinery.
    FnDef {
        name: String,
        params: Vec<(String, Type)>,
        ret: Option<Type>,
        body: Vec<Stmt>,
    },
    Return(Option<Expr>),
}

pub type Program = Vec<Stmt>;
