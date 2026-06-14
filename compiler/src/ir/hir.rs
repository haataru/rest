use crate::lexer::Span;
use crate::ops::{BinOp, UnOp};
use crate::sema::Type;

#[derive(Debug, Clone)]
pub enum HirStmt {
    Let {
        name: String,
        ty: Type,
        init: HirExpr,
        owner: bool,
        span: Span,
    },
    Expr(HirExpr, Span),
    Fn {
        name: String,
        params: Vec<(String, Type)>,
        ret: Type,
        body: Vec<HirStmt>,
        decorators: Vec<crate::parser::Decorator>,
        span: Span,
    },
    Struct {
        name: String,
        span: Span,
    },
    If {
        cond: HirExpr,
        then: Vec<HirStmt>,
        else_: Option<Vec<HirStmt>>,
        span: Span,
    },
    While {
        cond: HirExpr,
        body: Vec<HirStmt>,
        span: Span,
    },
    For {
        var: String,
        var_ty: Type,
        lo: HirExpr,
        hi: HirExpr,
        body: Vec<HirStmt>,
        span: Span,
    },
    Break(Span),
    Continue(Span),
    Return(Option<HirExpr>, Span),
}

#[derive(Debug, Clone)]
pub enum HirExpr {
    Int(i64, Type, Span),
    Float(f64, Type, Span),
    String(String, Span),
    Bool(bool, Span),
    /// Identifier reference. `ty` is the inferred type of the
    /// referenced variable (set by the Lowerer from the typeck's
    /// `expr_types` cache). It is what allows the borrow checker to
    /// distinguish Copy types (i32, bool, f32, ...) from owned types
    /// (string, struct, array) when deciding whether a `let y = x;`
    /// actually moves `x`.
    Ident {
        name: String,
        ty: Type,
        span: Span,
    },

    AllocStruct(String, Vec<(String, HirExpr)>, Span),
    Call(String, Vec<HirExpr>, Span),
    FieldLoad {
        object: Box<HirExpr>,
        index: usize,
        struct_name: String,
        span: Span,
    },
    ArrayIndex {
        object: Box<HirExpr>,
        index: Box<HirExpr>,
        span: Span,
    },
    ArrayLiteral(Box<Type>, Vec<HirExpr>, Span),
    Unary(UnOp, Box<HirExpr>, Span),
    Binary {
        lhs: Box<HirExpr>,
        op: BinOp,
        rhs: Box<HirExpr>,
        ty: Type,
        span: Span,
    },
    Assign {
        lhs: Box<HirExpr>,
        rhs: Box<HirExpr>,
        span: Span,
    },
    Print(Box<HirExpr>, Span),
}

impl HirExpr {
    pub fn span(&self) -> Span {
        match self {
            HirExpr::Int(_, _, s) => *s,
            HirExpr::Float(_, _, s) => *s,
            HirExpr::String(_, s) => *s,
            HirExpr::Bool(_, s) => *s,
            HirExpr::Ident { span, .. } => *span,

            HirExpr::AllocStruct(_, _, s) => *s,
            HirExpr::Call(_, _, s) => *s,
            HirExpr::FieldLoad { span, .. } => *span,
            HirExpr::ArrayIndex { span, .. } => *span,
            HirExpr::ArrayLiteral(_, _, s) => *s,
            HirExpr::Unary(_, _, s) => *s,
            HirExpr::Binary { span, .. } => *span,
            HirExpr::Assign { span, .. } => *span,
            HirExpr::Print(_, s) => *s,
        }
    }
}
