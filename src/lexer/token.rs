use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Span {
    pub start: usize,
    pub end: usize,
    pub line: usize,
    pub col: usize,
}

impl Span {
    pub fn new(start: usize, end: usize, line: usize, col: usize) -> Self {
        Self {
            start,
            end,
            line,
            col,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Token {
    pub kind: TokenKind,
    pub span: Span,
}

impl Token {
    pub fn new(kind: TokenKind, span: Span) -> Self {
        Self { kind, span }
    }
}

#[derive(Debug, Clone)]
pub struct LexError {
    pub message: String,
    pub span: Span,
}

impl fmt::Display for LexError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "lex error at {}:{}: {}",
            self.span.line, self.span.col, self.message
        )
    }
}

impl std::error::Error for LexError {}

#[derive(Debug, Clone, PartialEq)]
pub enum TokenKind {
    Int(i64, IntegerSuffix),
    Float(f64, FloatSuffix),
    String(String),
    Ident(String),

    // Keywords
    Let,
    Fn,
    Struct,
    Return,
    If,
    Else,
    While,
    For,
    In,
    Break,
    Continue,
    True,
    False,

    // Type keywords
    I8,
    I16,
    I32,
    I64,
    U8,
    U16,
    U32,
    U64,
    F32,
    F64,
    StringTy,
    Bool,
    Void,

    // Punctuation
    Semicolon,
    Colon,
    Comma,
    Dot,
    Arrow, // ->
    LParen,
    RParen,
    LBrace,
    RBrace,
    LBracket,
    RBracket,

    // Operators
    Plus,
    Minus,
    PlusPlus,
    MinusMinus,
    Star,
    Slash,
    Percent,
    Eq,   // =
    EqEq, // ==
    Bang,
    BangEq, // !  !=
    Lt,
    LtEq,
    Gt,
    GtEq, // < <= > >=
    Amp,
    AmpAmp, // & &&
    Pipe,
    PipePipe, // | ||
    Caret,    // ^
    LtLt,
    GtGt,   // << >>
    DotDot, // ..

    // Compound assignment
    PlusEq,
    MinusEq,
    StarEq,
    SlashEq,
    PercentEq,
    AmpEq,
    PipeEq,
    CaretEq,
    LtLtEq,
    GtGtEq,

    Eof,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum FloatSuffix {
    None,
    F32,
    F64,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum IntegerSuffix {
    None,
    I8,
    I16,
    I32,
    I64,
    U8,
    U16,
    U32,
    U64,
}

impl fmt::Display for TokenKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TokenKind::Int(_, _) => write!(f, "integer"),
            TokenKind::Float(..) => write!(f, "float"),
            TokenKind::String(_) => write!(f, "string"),
            TokenKind::Ident(n) => write!(f, "identifier `{}`", n),
            TokenKind::Let => write!(f, "`let`"),
            TokenKind::Fn => write!(f, "`def`"),
            TokenKind::Struct => write!(f, "`model`"),
            TokenKind::Return => write!(f, "`return`"),
            TokenKind::If => write!(f, "`if`"),
            TokenKind::Else => write!(f, "`else`"),
            TokenKind::While => write!(f, "`while`"),
            TokenKind::For => write!(f, "`for`"),
            TokenKind::In => write!(f, "`in`"),
            TokenKind::Break => write!(f, "`break`"),
            TokenKind::Continue => write!(f, "`continue`"),
            TokenKind::True => write!(f, "`true`"),
            TokenKind::False => write!(f, "`false`"),
            TokenKind::I8 => write!(f, "`i8`"),
            TokenKind::I16 => write!(f, "`i16`"),
            TokenKind::I32 => write!(f, "`i32`"),
            TokenKind::I64 => write!(f, "`i64`"),
            TokenKind::U8 => write!(f, "`u8`"),
            TokenKind::U16 => write!(f, "`u16`"),
            TokenKind::U32 => write!(f, "`u32`"),
            TokenKind::U64 => write!(f, "`u64`"),
            TokenKind::F32 => write!(f, "`f32`"),
            TokenKind::F64 => write!(f, "`f64`"),
            TokenKind::StringTy => write!(f, "`string`"),
            TokenKind::Bool => write!(f, "`bool`"),
            TokenKind::Void => write!(f, "`void`"),
            TokenKind::Semicolon => write!(f, "`;`"),
            TokenKind::Colon => write!(f, "`:`"),
            TokenKind::Comma => write!(f, "`,`"),
            TokenKind::Dot => write!(f, "`.`"),
            TokenKind::Arrow => write!(f, "`->`"),
            TokenKind::LParen => write!(f, "`(`"),
            TokenKind::RParen => write!(f, "`)`"),
            TokenKind::LBrace => write!(f, "`{{`"),
            TokenKind::RBrace => write!(f, "`}}`"),
            TokenKind::LBracket => write!(f, "`[`"),
            TokenKind::RBracket => write!(f, "`]`"),
            TokenKind::Plus => write!(f, "`+`"),
            TokenKind::Minus => write!(f, "`-`"),
            TokenKind::PlusPlus => write!(f, "`++`"),
            TokenKind::MinusMinus => write!(f, "`--`"),
            TokenKind::Star => write!(f, "`*`"),
            TokenKind::Slash => write!(f, "`/`"),
            TokenKind::Percent => write!(f, "`%`"),
            TokenKind::Eq => write!(f, "`=`"),
            TokenKind::EqEq => write!(f, "`==`"),
            TokenKind::Bang => write!(f, "`!`"),
            TokenKind::BangEq => write!(f, "`!=`"),
            TokenKind::Lt => write!(f, "`<`"),
            TokenKind::LtEq => write!(f, "`<=`"),
            TokenKind::Gt => write!(f, "`>`"),
            TokenKind::GtEq => write!(f, "`>=`"),
            TokenKind::Amp => write!(f, "`&`"),
            TokenKind::AmpAmp => write!(f, "`&&`"),
            TokenKind::Pipe => write!(f, "`|`"),
            TokenKind::PipePipe => write!(f, "`||`"),
            TokenKind::Caret => write!(f, "`^`"),
            TokenKind::LtLt => write!(f, "`<<`"),
            TokenKind::GtGt => write!(f, "`>>`"),
            TokenKind::DotDot => write!(f, "`..`"),
            TokenKind::PlusEq => write!(f, "`+=`"),
            TokenKind::MinusEq => write!(f, "`-=`"),
            TokenKind::StarEq => write!(f, "`*=`"),
            TokenKind::SlashEq => write!(f, "`/=`"),
            TokenKind::PercentEq => write!(f, "`%=`"),
            TokenKind::AmpEq => write!(f, "`&=`"),
            TokenKind::PipeEq => write!(f, "`|=`"),
            TokenKind::CaretEq => write!(f, "`^=`"),
            TokenKind::LtLtEq => write!(f, "`<<=`"),
            TokenKind::GtGtEq => write!(f, "`>>=`"),
            TokenKind::Eof => write!(f, "end of file"),
        }
    }
}
