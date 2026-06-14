use crate::lexer::FloatSuffix;
use crate::lexer::IntegerSuffix;
use crate::lexer::Span;
use crate::ops::{BinOp, UnOp};
use crate::sema::Type;

#[derive(Debug, Clone)]
pub struct Decorator {
    pub name: String,
    pub arg: Option<String>,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub enum Stmt {
    Let(String, Option<Type>, Option<Expr>, Span),
    Expr(Expr),
    If(Expr, Vec<Stmt>, Option<Vec<Stmt>>, Span),
    While(Expr, Vec<Stmt>, Span),
    For(String, Expr, Expr, Vec<Stmt>, Span),
    Break(Span),
    Continue(Span),
    Return(Option<Expr>, Span),
    Fn(
        String,
        Vec<(String, Type)>,
        Option<Type>,
        Vec<Stmt>,
        Vec<Decorator>,
        Span,
    ),
    ExternFn(String, Vec<(String, Type)>, Option<Type>, Span),
    Struct(String, Vec<(String, Type)>, Span),
}

#[derive(Debug, Clone)]
pub enum Expr {
    Int(i64, IntegerSuffix, Span),
    String(String, Span),
    Float(f64, FloatSuffix, Span),
    Bool(bool, Span),
    Ident(String, Span),
    Struct(String, Vec<(String, Expr)>, Span),
    Call(String, Vec<Expr>, Span),
    FieldAccess(Box<Expr>, String, Span),
    ArrayIndex(Box<Expr>, Box<Expr>, Span),
    ArrayLiteral(Box<Type>, Vec<Expr>, Span),

    Unary(UnOp, Box<Expr>, Span),
    Binary(Box<Expr>, BinOp, Box<Expr>, Span),
    Assign(Box<Expr>, Box<Expr>, Span),
    Paren(Box<Expr>, Span),
    
    AddressOf(Box<Expr>, Span),
    Dereference(Box<Expr>, Span),
    SizeOf(Type, Span),
    Cast(Box<Expr>, Type, Span),
}

impl Expr {
    pub fn span(&self) -> Span {
        match self {
            Expr::Int(_, _, span) => *span,
            Expr::String(_, span) => *span,
            Expr::Float(_, _, span) => *span,
            Expr::Bool(_, span) => *span,
            Expr::Ident(_, span) => *span,
            Expr::Struct(_, _, span) => *span,
            Expr::Call(_, _, span) => *span,
            Expr::FieldAccess(_, _, span) => *span,
            Expr::ArrayIndex(_, _, span) => *span,
            Expr::ArrayLiteral(_, _, span) => *span,

            Expr::Unary(_, _, span) => *span,
            Expr::Binary(_, _, _, span) => *span,
            Expr::Assign(_, _, span) => *span,
            Expr::Paren(_, span) => *span,
            
            Expr::AddressOf(_, span) => *span,
            Expr::Dereference(_, span) => *span,
            Expr::SizeOf(_, span) => *span,
            Expr::Cast(_, _, span) => *span,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ParseError {
    pub message: String,
    pub span: Span,
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "parse error at {}:{}: {}",
            self.span.line, self.span.col, self.message
        )
    }
}

impl std::error::Error for ParseError {}
