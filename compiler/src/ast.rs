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
    /// A call to a stdlib builtin, e.g. `fs_read_file("a.txt")`. There is no
    /// user-defined-function or module (`use std::x`) syntax yet -- see
    /// docs/grammar-and-types.md Phase 4 notes -- so callees are always one
    /// of the flat builtin names registered in `stdlib.rs`.
    Call {
        name: String,
        args: Vec<Expr>,
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
}

pub type Program = Vec<Stmt>;
