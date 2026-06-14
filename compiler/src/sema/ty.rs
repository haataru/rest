use std::collections::HashMap;
use std::fmt;

use crate::lexer::{FloatSuffix, IntegerSuffix, Span};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Type {
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
    String,
    Bool,
    Array(Box<Type>, usize),
    Struct(String),
    Fn(Vec<Type>, Box<Type>),
    Void,
}

impl Type {
    pub fn is_integer(&self) -> bool {
        matches!(
            self,
            Type::I8
                | Type::I16
                | Type::I32
                | Type::I64
                | Type::U8
                | Type::U16
                | Type::U32
                | Type::U64
        )
    }

    /// Whether values of this type are `Copy`. Copy types are bitwise
    /// duplicated on assignment, so `let y = x;` does not actually
    /// move `x` and a subsequent use of `x` is legal.
    pub fn is_copy(&self) -> bool {
        matches!(
            self,
            Type::I8
                | Type::I16
                | Type::I32
                | Type::I64
                | Type::U8
                | Type::U16
                | Type::U32
                | Type::U64
                | Type::F32
                | Type::F64
                | Type::Bool
        )
    }
}

impl fmt::Display for Type {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Type::I8 => write!(f, "i8"),
            Type::I16 => write!(f, "i16"),
            Type::I32 => write!(f, "i32"),
            Type::I64 => write!(f, "i64"),
            Type::U8 => write!(f, "u8"),
            Type::U16 => write!(f, "u16"),
            Type::U32 => write!(f, "u32"),
            Type::U64 => write!(f, "u64"),
            Type::F32 => write!(f, "f32"),
            Type::F64 => write!(f, "f64"),
            Type::String => write!(f, "string"),
            Type::Bool => write!(f, "bool"),
            Type::Array(elem, n) => write!(f, "{}[{}]", elem, n),
            Type::Struct(name) => write!(f, "{}", name),
            Type::Fn(params, ret) => {
                write!(f, "fn(")?;
                for (i, p) in params.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}", p)?;
                }
                write!(f, ") -> {}", ret)
            }
            Type::Void => write!(f, "void"),
        }
    }
}

pub fn suffix_to_type(suffix: &IntegerSuffix) -> Type {
    match suffix {
        IntegerSuffix::None => Type::I32,
        IntegerSuffix::I8 => Type::I8,
        IntegerSuffix::I16 => Type::I16,
        IntegerSuffix::I32 => Type::I32,
        IntegerSuffix::I64 => Type::I64,
        IntegerSuffix::U8 => Type::U8,
        IntegerSuffix::U16 => Type::U16,
        IntegerSuffix::U32 => Type::U32,
        IntegerSuffix::U64 => Type::U64,
    }
}

pub fn float_suffix_to_type(suffix: &FloatSuffix) -> Type {
    match suffix {
        FloatSuffix::None | FloatSuffix::F64 => Type::F64,
        FloatSuffix::F32 => Type::F32,
    }
}

/// Pre-computed type information produced by TypeChecker and consumed by Lowerer.
/// Eliminates duplicate type inference between the two passes.
#[derive(Debug, Clone)]
pub struct TypeContext {
    /// Field names with their types for each struct, in declaration order.
    /// The declaration order is preserved by the inner `Vec`; field names can
    /// be derived from this map when needed.
    pub(crate) struct_types: HashMap<String, Vec<(String, Type)>>,
    /// Inferred type for each expression, keyed by its source span.
    pub(crate) expr_types: HashMap<Span, Type>,
}
