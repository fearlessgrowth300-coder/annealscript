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
    /// No closures: a function body only sees its own parameters, not the
    /// caller's locals. That's a real simplification, not an oversight --
    /// it keeps the runtime's scoping to "one flat map per call" instead of
    /// a captured-environment chain, and nothing so far needs closures.
    FnDef {
        name: String,
        params: Vec<(String, Type)>,
        ret: Option<Type>,
        body: Vec<Stmt>,
    },
    Return(Option<Expr>),
}

pub type Program = Vec<Stmt>;
