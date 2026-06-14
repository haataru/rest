use std::collections::{HashMap, HashSet};

use crate::lexer::{IntegerSuffix, Span};
use crate::ops::BinOp;
use crate::parser::{Expr, Stmt};
use crate::sema::ty::{float_suffix_to_type, suffix_to_type};
use crate::sema::{Type, TypeContext};
use crate::util::ScopeStack;

#[derive(Debug, Clone)]
pub enum TypeckError {
    UndefinedVariable(String, Span),
    UndefinedFunction(String, Span),
    InvalidDecorator {
        message: String,
        span: Span,
    },
    NotAStruct {
        name: String,
        span: Span,
    },
    DuplicateField {
        field: String,
        span: Span,
    },
    DuplicateDefinition {
        name: String,
        kind: String,
        span: Span,
    },
    WrongArgCount {
        expected: usize,
        actual: usize,
        span: Span,
    },
    TypeMismatch {
        expected: Type,
        found: Type,
        span: Span,
    },
    MissingTypeAnnotation {
        name: String,
        span: Span,
    },
    MissingReturn {
        span: Span,
    },
    UnexpectedReturnValue {
        span: Span,
    },
    NoSuchField {
        name: String,
        field: String,
        span: Span,
    },
    LiteralOutOfRange {
        value: i64,
        ty: Type,
        span: Span,
    },
    BreakOutsideLoop {
        span: Span,
    },
    ContinueOutsideLoop {
        span: Span,
    },
    ReturnOutsideFunction {
        span: Span,
    },
    NotAFunction {
        name: String,
        span: Span,
    },

    VoidVariable {
        name: String,
        span: Span,
    },
    Unassignable {
        span: Span,
    },
    NotABool {
        context: String,
        span: Span,
    },
    IndexNotInteger {
        span: Span,
    },
    BinaryTypeMismatch {
        op: String,
        expected: Type,
        found: Type,
        span: Span,
    },
}

impl std::fmt::Display for TypeckError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TypeckError::UndefinedVariable(name, span) => {
                write!(
                    f,
                    "type error at {}:{}: undefined variable `{}`",
                    span.line, span.col, name
                )
            }
            TypeckError::UndefinedFunction(name, span) => {
                write!(
                    f,
                    "type error at {}:{}: undefined function `{}`",
                    span.line, span.col, name
                )
            }
            TypeckError::InvalidDecorator { message, span } => {
                write!(f, "type error at {}:{}: {}", span.line, span.col, message)
            }
            TypeckError::NotAStruct { name, span } => {
                write!(
                    f,
                    "type error at {}:{}: `{}` is not a struct",
                    span.line, span.col, name
                )
            }
            TypeckError::DuplicateField { field, span } => {
                write!(
                    f,
                    "type error at {}:{}: duplicate field `{}`",
                    span.line, span.col, field
                )
            }
            TypeckError::DuplicateDefinition { name, kind, span } => {
                write!(
                    f,
                    "type error at {}:{}: duplicate {} `{}`",
                    span.line, span.col, kind, name
                )
            }
            TypeckError::WrongArgCount {
                expected,
                actual,
                span,
            } => {
                write!(
                    f,
                    "type error at {}:{}: expected {} arguments, found {}",
                    span.line, span.col, expected, actual
                )
            }
            TypeckError::TypeMismatch {
                expected,
                found,
                span,
            } => {
                write!(
                    f,
                    "type error at {}:{}: expected `{}`, found `{}`",
                    span.line, span.col, expected, found
                )
            }
            TypeckError::MissingTypeAnnotation { name, span } => {
                write!(
                    f,
                    "type error at {}:{}: type annotation required for `{}`",
                    span.line, span.col, name
                )
            }
            TypeckError::MissingReturn { span } => {
                write!(
                    f,
                    "type error at {}:{}: missing return statement in function with return type",
                    span.line, span.col
                )
            }
            TypeckError::UnexpectedReturnValue { span } => {
                write!(
                    f,
                    "type error at {}:{}: unexpected return value in void function",
                    span.line, span.col
                )
            }
            TypeckError::NoSuchField { name, field, span } => {
                write!(
                    f,
                    "type error at {}:{}: no field `{}` on struct `{}`",
                    span.line, span.col, field, name
                )
            }
            TypeckError::LiteralOutOfRange { value, ty, span } => {
                write!(
                    f,
                    "type error at {}:{}: literal `{}` does not fit in type `{}`",
                    span.line, span.col, value, ty
                )
            }
            TypeckError::BreakOutsideLoop { span } => {
                write!(
                    f,
                    "type error at {}:{}: break outside loop",
                    span.line, span.col
                )
            }
            TypeckError::ContinueOutsideLoop { span } => {
                write!(
                    f,
                    "type error at {}:{}: continue outside loop",
                    span.line, span.col
                )
            }
            TypeckError::ReturnOutsideFunction { span } => {
                write!(
                    f,
                    "type error at {}:{}: return outside function",
                    span.line, span.col
                )
            }
            TypeckError::NotAFunction { name, span } => {
                write!(
                    f,
                    "type error at {}:{}: `{}` is not a function",
                    span.line, span.col, name
                )
            }

            TypeckError::VoidVariable { name, span } => {
                write!(
                    f,
                    "type error at {}:{}: variable `{}` cannot have type `void`",
                    span.line, span.col, name
                )
            }
            TypeckError::Unassignable { span } => {
                write!(
                    f,
                    "type error at {}:{}: expression is not assignable",
                    span.line, span.col
                )
            }
            TypeckError::NotABool { context, span } => {
                write!(
                    f,
                    "type error at {}:{}: {} requires `bool`, found non-bool expression",
                    span.line, span.col, context
                )
            }
            TypeckError::IndexNotInteger { span } => {
                write!(
                    f,
                    "type error at {}:{}: array index must be an integer",
                    span.line, span.col
                )
            }
            TypeckError::BinaryTypeMismatch {
                op,
                expected,
                found,
                span,
            } => {
                write!(
                    f,
                    "type error at {}:{}: operator `{}` requires operands of type `{}`, found `{}`",
                    span.line, span.col, op, expected, found
                )
            }
        }
    }
}

impl std::error::Error for TypeckError {}

fn has_return_value(stmts: &[Stmt]) -> bool {
    for s in stmts {
        match s {
            Stmt::Return(Some(_), _) => return true,
            Stmt::If(_, then_s, else_s, _) => {
                let then_returns = has_return_value(then_s);
                let else_returns = else_s
                    .as_ref()
                    .map(|e| has_return_value(e))
                    .unwrap_or(false);
                if then_returns && else_returns {
                    return true;
                }
            }
            Stmt::While(_, body, _) | Stmt::For(_, _, _, body, _) if has_return_value(body) => {
                return true;
            }
            _ => {}
        }
    }
    false
}

fn find_return_value_span(stmts: &[Stmt]) -> Option<Span> {
    for s in stmts {
        match s {
            Stmt::Return(Some(_), span) => return Some(*span),
            Stmt::If(_, then_s, else_s, _) => {
                if let Some(span) = find_return_value_span(then_s) {
                    return Some(span);
                }
                if let Some(els) = else_s
                    && let Some(span) = find_return_value_span(els)
                {
                    return Some(span);
                }
            }
            Stmt::While(_, body, _) | Stmt::For(_, _, _, body, _) => {
                if let Some(span) = find_return_value_span(body) {
                    return Some(span);
                }
            }
            _ => {}
        }
    }
    None
}

pub struct TypeChecker {
    scopes: ScopeStack<Type>,
    struct_fields: HashMap<String, Vec<(String, Type)>>,
    loop_depth: usize,
    fn_depth: usize,
    fn_ret_ty: Option<Type>,
    expr_types: HashMap<Span, Type>,
}

impl TypeChecker {
    pub fn new() -> Self {
        Self {
            scopes: ScopeStack::new(),
            struct_fields: HashMap::new(),
            loop_depth: 0,
            fn_depth: 0,
            fn_ret_ty: None,
            expr_types: HashMap::new(),
        }
    }

    /// Consume the checker and produce a TypeContext for use by the Lowerer.
    pub fn into_context(self) -> TypeContext {
        TypeContext {
            struct_types: self.struct_fields,
            expr_types: self.expr_types,
        }
    }

    pub fn check(&mut self, stmts: &[Stmt]) -> Result<(), TypeckError> {
        for stmt in stmts {
            match stmt {
                Stmt::Fn(name, params, ret, _, _, span) => {
                    if self.lookup(name).is_some() {
                        return Err(TypeckError::DuplicateDefinition {
                            name: name.clone(),
                            kind: "function".to_string(),
                            span: *span,
                        });
                    }
                    let ret_ty = ret.clone().unwrap_or(Type::Void);
                    let param_tys: Vec<Type> = params.iter().map(|(_, t)| t.clone()).collect();
                    self.define(name.clone(), Type::Fn(param_tys, Box::new(ret_ty.clone())));
                }
                Stmt::ExternFn(name, params, ret, span) => {
                    if self.lookup(name).is_some() {
                        return Err(TypeckError::DuplicateDefinition {
                            name: name.clone(),
                            kind: "function".to_string(),
                            span: *span,
                        });
                    }
                    let ret_ty = ret.clone().unwrap_or(Type::Void);
                    let param_tys: Vec<Type> = params.iter().map(|(_, t)| t.clone()).collect();
                    self.define(name.clone(), Type::Fn(param_tys, Box::new(ret_ty)));
                }
                Stmt::Struct(name, fields, span) => {
                    if self.struct_fields.contains_key(name) {
                        return Err(TypeckError::DuplicateDefinition {
                            name: name.clone(),
                            kind: "struct".to_string(),
                            span: *span,
                        });
                    }
                    let mut seen = HashSet::new();
                    for (fname, fty) in fields {
                        if !seen.insert(fname.clone()) {
                            return Err(TypeckError::DuplicateField {
                                field: fname.clone(),
                                span: *span,
                            });
                        }
                        if *fty == Type::Void {
                            return Err(TypeckError::VoidVariable {
                                name: fname.clone(),
                                span: *span,
                            });
                        }
                    }
                    self.struct_fields.insert(name.clone(), fields.clone());
                }
                _ => {}
            }
        }
        for stmt in stmts {
            self.check_stmt(stmt)?;
        }
        Ok(())
    }

    fn enter_scope(&mut self) {
        self.scopes.enter();
    }

    fn exit_scope(&mut self) {
        self.scopes.exit();
    }

    fn lookup(&self, name: &str) -> Option<Type> {
        self.scopes.get(name).cloned()
    }

    fn define(&mut self, name: String, ty: Type) {
        self.scopes.define(name, ty);
    }

    fn check_stmt(&mut self, stmt: &Stmt) -> Result<(), TypeckError> {
        match stmt {
            Stmt::Let(name, ty_annot, init, span) => {
                if let Some(annot) = ty_annot
                    && *annot == Type::Void
                {
                    return Err(TypeckError::VoidVariable {
                        name: name.clone(),
                        span: *span,
                    });
                }
                let ty = match init {
                    Some(e) => {
                        // Pass the annotation as a hint so plain `Int`
                        // literals get range-checked against the
                        // declared type (e.g. `let x: u64 = MAX;`)
                        // instead of the default i32.
                        let inferred = self.infer_expr_with_hint(e, ty_annot.as_ref())?;
                        // Reject `let x = void_expression;` — the
                        // inferred type is Void, which has no runtime
                        // representation. typeck would otherwise let
                        // it through to codegen where it would
                        // trigger a debug_assert (and silently emit
                        // wrong IR in release builds).
                        if inferred == Type::Void {
                            return Err(TypeckError::VoidVariable {
                                name: name.clone(),
                                span: *span,
                            });
                        }
                        if let Some(annot) = ty_annot
                            && &inferred != annot
                        {
                            return Err(TypeckError::TypeMismatch {
                                expected: annot.clone(),
                                found: inferred,
                                span: *span,
                            });
                        }
                        inferred
                    }
                    None => {
                        if let Some(annot) = ty_annot {
                            annot.clone()
                        } else {
                            return Err(TypeckError::MissingTypeAnnotation {
                                name: name.clone(),
                                span: *span,
                            });
                        }
                    }
                };
                self.define(name.clone(), ty);
                Ok(())
            }
            Stmt::Expr(expr) => {
                self.infer_expr(expr)?;
                Ok(())
            }
            Stmt::If(cond, then_stmts, else_stmts, _) => {
                let cond_ty = self.infer_expr(cond)?;
                if cond_ty != Type::Bool {
                    return Err(TypeckError::NotABool {
                        context: "`if` condition".into(),
                        span: cond.span(),
                    });
                }
                self.enter_scope();
                for s in then_stmts {
                    self.check_stmt(s)?;
                }
                self.exit_scope();
                if let Some(els) = else_stmts {
                    self.enter_scope();
                    for s in els {
                        self.check_stmt(s)?;
                    }
                    self.exit_scope();
                }
                Ok(())
            }
            Stmt::While(cond, body, _) => {
                let cond_ty = self.infer_expr(cond)?;
                if cond_ty != Type::Bool {
                    return Err(TypeckError::NotABool {
                        context: "`while` condition".into(),
                        span: cond.span(),
                    });
                }
                self.loop_depth += 1;
                self.enter_scope();
                for s in body {
                    self.check_stmt(s)?;
                }
                self.exit_scope();
                self.loop_depth -= 1;
                Ok(())
            }
            Stmt::For(var, lo, hi, body, _) => {
                let lo_ty = self.infer_expr(lo)?;
                let hi_ty = self.infer_expr(hi)?;
                if !lo_ty.is_integer() || !hi_ty.is_integer() {
                    let (bad_ty, bad_span) = if !lo_ty.is_integer() {
                        (lo_ty, lo.span())
                    } else {
                        (hi_ty, hi.span())
                    };
                    return Err(TypeckError::BinaryTypeMismatch {
                        op: "for range".into(),
                        expected: Type::I32,
                        found: bad_ty,
                        span: bad_span,
                    });
                }
                if lo_ty != hi_ty {
                    return Err(TypeckError::TypeMismatch {
                        expected: lo_ty.clone(),
                        found: hi_ty,
                        span: hi.span(),
                    });
                }
                self.loop_depth += 1;
                self.enter_scope();
                self.define(var.clone(), lo_ty.clone());
                for s in body {
                    self.check_stmt(s)?;
                }
                self.exit_scope();
                self.loop_depth -= 1;
                Ok(())
            }
            Stmt::Fn(_name, params, ret, body, decorators, span) => {
                for dec in decorators {
                    if !["Get", "Post", "Put", "Delete"].contains(&dec.name.as_str()) {
                        return Err(TypeckError::InvalidDecorator {
                            message: format!(
                                "unknown decorator `@{}`. Expected @Get, @Post, @Put, or @Delete",
                                dec.name
                            ),
                            span: dec.span,
                        });
                    }
                    if dec.arg.is_none() {
                        return Err(TypeckError::InvalidDecorator {
                            message: format!(
                                "decorator `@{}` requires a path argument (e.g. `(\"/path\")`)",
                                dec.name
                            ),
                            span: dec.span,
                        });
                    }
                }

                let ret_ty = ret.clone().unwrap_or(Type::Void);
                self.fn_depth += 1;
                let prev_ret = self.fn_ret_ty.replace(ret_ty.clone());
                self.enter_scope();
                for (pname, pty) in params {
                    if *pty == Type::Void {
                        return Err(TypeckError::VoidVariable {
                            name: pname.clone(),
                            span: *span,
                        });
                    }
                    self.define(pname.clone(), pty.clone());
                }
                for s in body {
                    self.check_stmt(s)?;
                }
                self.exit_scope();
                self.fn_ret_ty = prev_ret;
                self.fn_depth -= 1;
                if ret_ty != Type::Void && !has_return_value(body) {
                    return Err(TypeckError::MissingReturn { span: *span });
                }
                if ret_ty == Type::Void
                    && let Some(span) = find_return_value_span(body)
                {
                    return Err(TypeckError::UnexpectedReturnValue { span });
                }
                Ok(())
            }
            Stmt::ExternFn(_, _, _, _) => Ok(()),
            Stmt::Struct(_, _, _) => Ok(()),
            Stmt::Break(span) => {
                if self.loop_depth == 0 {
                    return Err(TypeckError::BreakOutsideLoop { span: *span });
                }
                Ok(())
            }
            Stmt::Continue(span) => {
                if self.loop_depth == 0 {
                    return Err(TypeckError::ContinueOutsideLoop { span: *span });
                }
                Ok(())
            }
            Stmt::Return(value, span) => {
                if self.fn_depth == 0 {
                    return Err(TypeckError::ReturnOutsideFunction { span: *span });
                }
                if let Some(v) = value {
                    let val_ty = self.infer_expr(v)?;
                    if let Some(expected_ty) = &self.fn_ret_ty
                        && *expected_ty != Type::Void
                        && val_ty != *expected_ty
                    {
                        return Err(TypeckError::TypeMismatch {
                            expected: expected_ty.clone(),
                            found: val_ty,
                            span: *span,
                        });
                    }
                }
                Ok(())
            }
        }
    }

    fn infer_expr(&mut self, expr: &Expr) -> Result<Type, TypeckError> {
        let result = self.infer_expr_impl(expr, None);
        if let Ok(ref ty) = result {
            self.expr_types.insert(expr.span(), ty.clone());
        }
        result
    }

    /// Like [`Self::infer_expr`] but lets the caller supply a target
    /// type hint. The hint is used only for plain `Int` literals without
    /// a suffix — `let x: u64 = 9223372036854775807;` checks the range
    /// against `u64` rather than the default `i32`. For all other
    /// expression kinds, the hint is ignored.
    fn infer_expr_with_hint(
        &mut self,
        expr: &Expr,
        hint: Option<&Type>,
    ) -> Result<Type, TypeckError> {
        let result = self.infer_expr_impl(expr, hint);
        if let Ok(ref ty) = result {
            self.expr_types.insert(expr.span(), ty.clone());
        }
        result
    }

    fn infer_expr_impl(&mut self, expr: &Expr, hint: Option<&Type>) -> Result<Type, TypeckError> {
        match expr {
            Expr::Int(v, suffix, span) => {
                // If there's a suffix, the suffix wins. Otherwise, an
                // integer-typed hint (from a let annotation) is used so
                // `let x: u64 = 9223372036854775807;` does not get
                // rejected for "not fitting in i32".
                let ty = if *suffix == IntegerSuffix::None {
                    if let Some(
                        t @ (Type::I8
                        | Type::I16
                        | Type::I32
                        | Type::I64
                        | Type::U8
                        | Type::U16
                        | Type::U32
                        | Type::U64),
                    ) = hint
                    {
                        t.clone()
                    } else {
                        suffix_to_type(suffix)
                    }
                } else {
                    suffix_to_type(suffix)
                };
                let fits = match &ty {
                    Type::I8 => *v >= i8::MIN as i64 && *v <= i8::MAX as i64,
                    Type::I16 => *v >= i16::MIN as i64 && *v <= i16::MAX as i64,
                    Type::I32 => *v >= i32::MIN as i64 && *v <= i32::MAX as i64,
                    Type::I64 => true,
                    Type::U8 => *v >= 0 && *v <= u8::MAX as i64,
                    Type::U16 => *v >= 0 && *v <= u16::MAX as i64,
                    Type::U32 => *v >= 0 && *v <= u32::MAX as i64,
                    // u64 literals are stored in i64; values above
                    // i64::MAX cannot reach this code path (they
                    // fail to parse), so checking `*v >= 0` is
                    // sufficient.
                    Type::U64 => *v >= 0,
                    _ => true,
                };
                if !fits {
                    return Err(TypeckError::LiteralOutOfRange {
                        value: *v,
                        ty,
                        span: *span,
                    });
                }
                Ok(ty)
            }
            Expr::String(..) => Ok(Type::String),
            Expr::Float(_, suffix, _) => Ok(float_suffix_to_type(suffix)),
            Expr::Bool(..) => Ok(Type::Bool),
            Expr::Ident(name, span) => self
                .lookup(name)
                .ok_or_else(|| TypeckError::UndefinedVariable(name.clone(), *span)),

            Expr::Struct(name, fields, span) => {
                let mut seen = HashSet::new();
                for (field, _) in fields {
                    if !seen.insert(field.clone()) {
                        return Err(TypeckError::DuplicateField {
                            field: field.clone(),
                            span: *span,
                        });
                    }
                }
                let was_declared = self.struct_fields.contains_key(name);
                if !was_declared {
                    let inferred: Vec<(String, Type)> = fields
                        .iter()
                        .map(|(n, v)| Ok((n.clone(), self.infer_expr(v)?)))
                        .collect::<Result<_, TypeckError>>()?;
                    self.struct_fields.insert(name.clone(), inferred);
                } else {
                    // The struct was declared; look up its field list. If
                    // it has somehow disappeared between the
                    // `contains_key` check and this lookup, that is an
                    // internal compiler bug — surface a structured error
                    // rather than panicking.
                    let def = match self.struct_fields.get(name).cloned() {
                        Some(def) => def,
                        None => {
                            return Err(TypeckError::NotAStruct {
                                name: name.clone(),
                                span: *span,
                            });
                        }
                    };
                    for (field, value) in fields {
                        let val_ty = self.infer_expr(value)?;
                        if let Some((_, expected_ty)) = def.iter().find(|(n, _)| n == field) {
                            if &val_ty != expected_ty {
                                return Err(TypeckError::TypeMismatch {
                                    expected: expected_ty.clone(),
                                    found: val_ty,
                                    span: *span,
                                });
                            }
                        } else {
                            return Err(TypeckError::NoSuchField {
                                name: name.clone(),
                                field: field.clone(),
                                span: *span,
                            });
                        }
                    }
                }
                Ok(Type::Struct(name.clone()))
            }
            Expr::Call(callee, args, span) => {
                if callee == "print" {
                    if args.len() != 1 {
                        return Err(TypeckError::WrongArgCount {
                            expected: 1,
                            actual: args.len(),
                            span: *span,
                        });
                    }
                    self.infer_expr(&args[0])?;
                    return Ok(Type::Void);
                }
                let fn_ty = self
                    .lookup(callee)
                    .ok_or_else(|| TypeckError::UndefinedFunction(callee.clone(), *span))?;
                match fn_ty {
                    Type::Fn(param_tys, ret_ty) => {
                        if args.len() != param_tys.len() {
                            return Err(TypeckError::WrongArgCount {
                                expected: param_tys.len(),
                                actual: args.len(),
                                span: *span,
                            });
                        }
                        for (i, arg) in args.iter().enumerate() {
                            let arg_ty = self.infer_expr(arg)?;
                            if arg_ty != param_tys[i] {
                                return Err(TypeckError::TypeMismatch {
                                    expected: param_tys[i].clone(),
                                    found: arg_ty,
                                    span: *span,
                                });
                            }
                        }
                        Ok(*ret_ty)
                    }
                    _ => Err(TypeckError::NotAFunction {
                        name: callee.clone(),
                        span: *span,
                    }),
                }
            }
            Expr::FieldAccess(inner, field_name, span) => {
                let ty = self.infer_expr(inner)?;
                match ty {
                    Type::Struct(name) => {
                        let fields = self.struct_fields.get(&name).ok_or_else(|| {
                            TypeckError::NotAStruct {
                                name: name.clone(),
                                span: *span,
                            }
                        })?;
                        fields
                            .iter()
                            .find(|(n, _)| n == field_name)
                            .map(|(_, ty)| ty.clone())
                            .ok_or_else(|| TypeckError::NoSuchField {
                                name: name.clone(),
                                field: field_name.clone(),
                                span: *span,
                            })
                    }
                    _ => Err(TypeckError::NotAStruct {
                        name: format!("{}", ty),
                        span: *span,
                    }),
                }
            }
            Expr::ArrayIndex(arr, idx, span) => {
                let arr_ty = self.infer_expr(arr)?;
                let idx_ty = self.infer_expr(idx)?;
                if !idx_ty.is_integer() {
                    return Err(TypeckError::IndexNotInteger { span: *span });
                }
                match arr_ty {
                    Type::Array(elem, _) => Ok(*elem),
                    _ => Err(TypeckError::NotAStruct {
                        name: format!("{}", arr_ty),
                        span: *span,
                    }),
                }
            }
            Expr::ArrayLiteral(ty, elems, span) => {
                if elems.is_empty() {
                    return Err(TypeckError::TypeMismatch {
                        expected: Type::Array(Box::new(Type::Void), 0),
                        found: Type::Void,
                        span: *span,
                    });
                }
                let first_ty = self.infer_expr(&elems[0])?;
                let expected = if **ty != Type::Void {
                    if first_ty != **ty {
                        return Err(TypeckError::TypeMismatch {
                            expected: *ty.clone(),
                            found: first_ty,
                            span: elems[0].span(),
                        });
                    }
                    *ty.clone()
                } else {
                    first_ty.clone()
                };
                for e in &elems[1..] {
                    let elem_ty = self.infer_expr(e)?;
                    if elem_ty != expected {
                        return Err(TypeckError::TypeMismatch {
                            expected: expected.clone(),
                            found: elem_ty,
                            span: e.span(),
                        });
                    }
                }
                Ok(Type::Array(Box::new(expected), elems.len()))
            }
            Expr::Unary(_, expr, _) => self.infer_expr(expr),
            Expr::Binary(lhs, op, rhs, span) => {
                let lty = self.infer_expr(lhs)?;
                let rty = self.infer_expr(rhs)?;
                let is_str_cat = *op == BinOp::Add && lty == Type::String;
                if is_str_cat {
                    if lty != rty {
                        return Err(TypeckError::TypeMismatch {
                            expected: lty.clone(),
                            found: rty,
                            span: *span,
                        });
                    }
                    return Ok(lty);
                }
                if matches!(&lty, Type::Struct(_)) || matches!(&rty, Type::Struct(_)) {
                    return Err(TypeckError::BinaryTypeMismatch {
                        op: op.to_string(),
                        expected: Type::I32,
                        found: lty,
                        span: *span,
                    });
                }
                let is_logical = matches!(op, BinOp::And | BinOp::Or);
                if is_logical {
                    if lty != Type::Bool {
                        return Err(TypeckError::BinaryTypeMismatch {
                            op: op.to_string(),
                            expected: Type::Bool,
                            found: lty,
                            span: *span,
                        });
                    }
                    if rty != Type::Bool {
                        return Err(TypeckError::BinaryTypeMismatch {
                            op: op.to_string(),
                            expected: Type::Bool,
                            found: rty,
                            span: *span,
                        });
                    }
                    return Ok(Type::Bool);
                }
                let is_bitwise = matches!(
                    op,
                    BinOp::BitAnd | BinOp::BitOr | BinOp::BitXor | BinOp::Shl | BinOp::Shr
                );
                if is_bitwise {
                    if !lty.is_integer() {
                        return Err(TypeckError::BinaryTypeMismatch {
                            op: op.to_string(),
                            expected: Type::I32,
                            found: lty,
                            span: *span,
                        });
                    }
                    if !rty.is_integer() {
                        return Err(TypeckError::BinaryTypeMismatch {
                            op: op.to_string(),
                            expected: Type::I32,
                            found: rty,
                            span: *span,
                        });
                    }
                    if !matches!(op, BinOp::Shl | BinOp::Shr) && lty != rty {
                        return Err(TypeckError::TypeMismatch {
                            expected: lty.clone(),
                            found: rty,
                            span: *span,
                        });
                    }
                    return Ok(lty);
                }
                if lty != rty {
                    return Err(TypeckError::TypeMismatch {
                        expected: lty.clone(),
                        found: rty,
                        span: *span,
                    });
                }
                Ok(match op {
                    BinOp::Eq | BinOp::Ne | BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge => {
                        Type::Bool
                    }
                    _ => lty,
                })
            }
            Expr::Assign(lhs, rhs, span) => {
                match lhs.as_ref() {
                    Expr::Ident(..) | Expr::FieldAccess(..) | Expr::ArrayIndex(..) | Expr::Dereference(..) => {}
                    _ => return Err(TypeckError::Unassignable { span: *span }),
                }
                let lhs_ty = self.infer_expr(lhs)?;
                let rhs_ty = self.infer_expr(rhs)?;
                if lhs_ty != rhs_ty {
                    return Err(TypeckError::TypeMismatch {
                        expected: lhs_ty,
                        found: rhs_ty,
                        span: *span,
                    });
                }
                Ok(Type::Void)
            }
            Expr::Paren(inner, _) => self.infer_expr(inner),
            Expr::AddressOf(expr, span) => {
                let inner_ty = self.infer_expr(expr)?;
                match expr.as_ref() {
                    Expr::Ident(..) | Expr::FieldAccess(..) | Expr::ArrayIndex(..) | Expr::Dereference(..) => {}
                    _ => return Err(TypeckError::Unassignable { span: *span }),
                }
                Ok(Type::Pointer(Box::new(inner_ty)))
            }
            Expr::Dereference(expr, span) => {
                let inner_ty = self.infer_expr(expr)?;
                match inner_ty {
                    Type::Pointer(inner) => Ok(*inner),
                    _ => return Err(TypeckError::TypeMismatch {
                        expected: Type::Pointer(Box::new(Type::Void)),
                        found: inner_ty,
                        span: *span,
                    }),
                }
            }
            Expr::SizeOf(_, _) => Ok(Type::I64),
            Expr::Cast(expr, target_ty, span) => {
                let expr_ty = self.infer_expr(expr)?;
                let valid = match (&expr_ty, target_ty) {
                    (t1, t2) if (t1.is_integer() || t1 == &Type::F32 || t1 == &Type::F64) &&
                                (t2.is_integer() || t2 == &Type::F32 || t2 == &Type::F64) => true,
                    (Type::Pointer(_), Type::Pointer(_)) => true,
                    (Type::String, Type::Pointer(_)) => true,
                    (Type::Struct(_), Type::Pointer(_)) => true,
                    (Type::Pointer(_), t2) if t2.is_integer() => true,
                    (t1, Type::Pointer(_)) if t1.is_integer() => true,
                    _ => false,
                };
                if !valid {
                    return Err(TypeckError::TypeMismatch {
                        expected: target_ty.clone(),
                        found: expr_ty,
                        span: *span,
                    });
                }
                Ok(target_ty.clone())
            }
        }
    }
}
