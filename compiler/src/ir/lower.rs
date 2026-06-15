use std::collections::HashMap;

use crate::ir::{HirExpr, HirStmt};
use crate::lexer::Span;
use crate::parser::{Expr, Stmt};
use crate::sema::ty::{float_suffix_to_type, suffix_to_type};
use crate::sema::{Type, TypeContext};

#[derive(Debug, Clone)]
pub(crate) struct LowerError {
    pub message: String,
    pub span: Span,
}

impl std::fmt::Display for LowerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "lower error at {}:{}: {}",
            self.span.line, self.span.col, self.message
        )
    }
}

impl std::error::Error for LowerError {}

pub(crate) struct Lowerer {
    ctx: TypeContext,
}

impl Lowerer {
    pub fn new(ctx: TypeContext) -> Self {
        Self { ctx }
    }

    pub fn struct_types(&self) -> &HashMap<String, Vec<(String, Type)>> {
        &self.ctx.struct_types
    }

    pub fn lower(&mut self, stmts: &[Stmt]) -> Result<Vec<HirStmt>, LowerError> {
        stmts
            .iter()
            .filter(|s| !matches!(s, Stmt::Import(..)))
            .map(|s| self.lower_stmt(s))
            .collect()
    }

    fn get_expr_type(&self, expr: &Expr) -> Result<Type, LowerError> {
        self.ctx
            .expr_types
            .get(&expr.span())
            .cloned()
            .ok_or_else(|| LowerError {
                span: expr.span(),
                message: "cannot infer expression type".into(),
            })
    }

    fn lower_stmt(&mut self, stmt: &Stmt) -> Result<HirStmt, LowerError> {
        match stmt {
            Stmt::Let(name, ty_annot, init, span) => {
                let ty = match (init, ty_annot) {
                    (Some(e), None) => self.get_expr_type(e)?,
                    (_, Some(annot)) => annot.clone(),
                    (None, None) => {
                        return Err(LowerError {
                            message:
                                "variable declaration requires a type annotation or an initializer"
                                    .into(),
                            span: *span,
                        });
                    }
                };
                let hir_init = match init {
                    Some(e) => self.lower_expr(e)?,
                    None => Self::default_init(&ty, *span)?,
                };
                let owner = matches!(&ty, Type::Struct(_));
                Ok(HirStmt::Let {
                    name: name.clone(),
                    ty,
                    init: hir_init,
                    owner,
                    span: *span,
                })
            }
            Stmt::Expr(expr) => {
                let span = expr.span();
                Ok(HirStmt::Expr(self.lower_expr(expr)?, span))
            }
            Stmt::Fn(name, params, ret, body, decorators, span) => {
                let mut lowered_body = Vec::new();
                for s in body {
                    lowered_body.push(self.lower_stmt(s)?);
                }
                Ok(HirStmt::Fn {
                    name: name.clone(),
                    params: params.clone(),
                    ret: ret.clone().unwrap_or(Type::Void),
                    body: lowered_body,
                    decorators: decorators.clone(),
                    span: *span,
                })
            }
            Stmt::ExternFn(name, params, is_variadic, ret, span) => {
                let ret = ret.clone().unwrap_or(Type::Void);
                Ok(HirStmt::ExternFn {
                    name: name.clone(),
                    params: params.clone(),
                    is_variadic: *is_variadic,
                    ret,
                    span: *span,
                })
            }
            Stmt::Struct(name, _, span) => Ok(HirStmt::Struct {
                name: name.clone(),
                span: *span,
            }),
            Stmt::GlobalAsm(asm, span) => Ok(HirStmt::GlobalAsm(asm.clone(), *span)),
            Stmt::Const(name, _, init, span) => {
                let init_hir = self.lower_expr(init)?;
                let ty = self.get_expr_type(init)?;
                Ok(HirStmt::Const {
                    name: name.clone(),
                    ty,
                    init: init_hir,
                    span: *span,
                })
            }
            Stmt::Import(..) => unreachable!("imports should be filtered out"),
            Stmt::If(cond, then_stmts, else_stmts, span) => {
                let hir_cond = self.lower_expr(cond)?;
                let hir_then: Vec<HirStmt> = then_stmts
                    .iter()
                    .map(|s| self.lower_stmt(s))
                    .collect::<Result<_, _>>()?;
                let hir_else = else_stmts
                    .as_ref()
                    .map(|els| {
                        els.iter()
                            .map(|s| self.lower_stmt(s))
                            .collect::<Result<_, _>>()
                    })
                    .transpose()?;
                Ok(HirStmt::If {
                    cond: hir_cond,
                    then: hir_then,
                    else_: hir_else,
                    span: *span,
                })
            }
            Stmt::While(cond, body, span) => {
                let hir_cond = self.lower_expr(cond)?;
                let hir_body: Vec<HirStmt> = body
                    .iter()
                    .map(|s| self.lower_stmt(s))
                    .collect::<Result<_, _>>()?;
                Ok(HirStmt::While {
                    cond: hir_cond,
                    body: hir_body,
                    span: *span,
                })
            }
            Stmt::For(var, lo, hi, body, span) => {
                let hir_lo = self.lower_expr(lo)?;
                let hir_hi = self.lower_expr(hi)?;
                let var_ty = self.get_expr_type(lo)?;
                let hir_body: Vec<HirStmt> = body
                    .iter()
                    .map(|s| self.lower_stmt(s))
                    .collect::<Result<_, _>>()?;
                Ok(HirStmt::For {
                    var: var.clone(),
                    var_ty,
                    lo: hir_lo,
                    hi: hir_hi,
                    body: hir_body,
                    span: *span,
                })
            }
            Stmt::Break(span) => Ok(HirStmt::Break(*span)),
            Stmt::Continue(span) => Ok(HirStmt::Continue(*span)),
            Stmt::Return(value, span) => {
                let hir_v = value.as_ref().map(|v| self.lower_expr(v)).transpose()?;
                Ok(HirStmt::Return(hir_v, *span))
            }
        }
    }

    fn is_builtin(name: &str) -> bool {
        matches!(name, "print")
    }

    fn lower_expr(&self, expr: &Expr) -> Result<HirExpr, LowerError> {
        match expr {
            Expr::Int(v, suffix, span) => {
                let ty = suffix_to_type(suffix);
                Ok(HirExpr::Int(*v, ty, *span))
            }
            Expr::Float(v, suffix, span) => {
                let ty = float_suffix_to_type(suffix);
                Ok(HirExpr::Float(*v, ty, *span))
            }
            Expr::String(s, span) => Ok(HirExpr::String(s.clone(), *span)),
            Expr::Bool(v, span) => Ok(HirExpr::Bool(*v, *span)),
            Expr::Ident(name, span) => {
                // Look up the inferred type from typeck's cache so
                // the borrow checker can distinguish Copy types
                // (i32, bool, f32, ...) from owned types (string,
                // struct, array) when tracking moves.
                let ty = self.get_expr_type(expr).unwrap_or(Type::Void);
                Ok(HirExpr::Ident {
                    name: name.clone(),
                    ty,
                    span: *span,
                })
            }

            Expr::Struct(name, fields, span) => {
                let ordered_fields: Vec<(String, Expr)> = match self.ctx.struct_types.get(name) {
                    Some(field_order) => {
                        let mut field_map = HashMap::new();
                        for (n, v) in fields {
                            field_map.insert(n.clone(), v.clone());
                        }
                        let mut result = Vec::new();
                        for (field_name, _) in field_order {
                            if let Some(val) = field_map.remove(field_name) {
                                result.push((field_name.clone(), val));
                            }
                        }
                        if result.len() != field_order.len() {
                            return Err(LowerError {
                                message: format!("missing fields in struct `{}`", name),
                                span: *span,
                            });
                        }
                        result
                    }
                    None => fields.clone(),
                };
                let lowered: Result<Vec<(String, HirExpr)>, _> = ordered_fields
                    .iter()
                    .map(|(n, v)| Ok((n.clone(), self.lower_expr(v)?)))
                    .collect();
                Ok(HirExpr::AllocStruct(name.clone(), lowered?, *span))
            }
            Expr::Call(callee, args, span) => {
                if Self::is_builtin(callee) && callee == "print" {
                    if args.len() != 1 {
                        return Err(LowerError {
                            message: format!("print() expected 1 argument, found {}", args.len()),
                            span: *span,
                        });
                    }
                    let arg = self.lower_expr(&args[0])?;
                    return Ok(HirExpr::Print(Box::new(arg), *span));
                }
                let lowered: Result<Vec<HirExpr>, _> =
                    args.iter().map(|a| self.lower_expr(a)).collect();
                Ok(HirExpr::Call(callee.clone(), lowered?, self.get_expr_type(expr)?, *span))
            }
            Expr::FieldAccess(inner, field, span) => {
                let object = Box::new(self.lower_expr(inner)?);
                let ty = self.get_expr_type(inner)?;
                match &ty {
                    Type::Struct(name) => {
                        let index = self
                            .ctx
                            .struct_types
                            .get(name)
                            .and_then(|f| f.iter().position(|(n, _)| n == field))
                            .ok_or_else(|| LowerError {
                                message: format!("no field `{}` on struct `{}`", field, name),
                                span: *span,
                            })?;
                        Ok(HirExpr::FieldLoad {
                            object,
                            index,
                            struct_name: name.clone(),
                            span: *span,
                        })
                    }
                    _ => Err(LowerError {
                        message: format!(
                            "cannot access field `{}` on non-struct type `{}`",
                            field, ty
                        ),
                        span: *span,
                    }),
                }
            }
            Expr::ArrayIndex(arr, idx, span) => {
                let object = Box::new(self.lower_expr(arr)?);
                let index = Box::new(self.lower_expr(idx)?);
                Ok(HirExpr::ArrayIndex {
                    object,
                    index,
                    span: *span,
                })
            }
            Expr::ArrayLiteral(ty, elems, span) => {
                let lowered: Result<Vec<HirExpr>, _> =
                    elems.iter().map(|e| self.lower_expr(e)).collect();
                let elem_ty = match self.get_expr_type(expr)? {
                    Type::Array(elem, _) => *elem,
                    other => other,
                };
                let _ = ty;
                Ok(HirExpr::ArrayLiteral(Box::new(elem_ty), lowered?, *span))
            }
            Expr::Unary(op, e, span) => {
                let inner = Box::new(self.lower_expr(e)?);
                Ok(HirExpr::Unary(*op, inner, *span))
            }
            Expr::Binary(lhs, op, rhs, span) => {
                let l = Box::new(self.lower_expr(lhs)?);
                let r = Box::new(self.lower_expr(rhs)?);
                let ty = self.get_expr_type(expr)?;
                Ok(HirExpr::Binary {
                    lhs: l,
                    op: *op,
                    rhs: r,
                    ty,
                    span: *span,
                })
            }
            Expr::Assign(lhs, rhs, span) => {
                let l = Box::new(self.lower_expr(lhs)?);
                let r = Box::new(self.lower_expr(rhs)?);
                Ok(HirExpr::Assign {
                    lhs: l,
                    rhs: r,
                    span: *span,
                })
            }
            Expr::AddressOf(expr, span) => {
                let inner = Box::new(self.lower_expr(expr)?);
                Ok(HirExpr::AddressOf(inner, *span))
            }
            Expr::Dereference(expr, span) => {
                let inner = Box::new(self.lower_expr(expr)?);
                Ok(HirExpr::Dereference(inner, *span))
            }
            Expr::SizeOf(ty, span) => {
                Ok(HirExpr::SizeOf(ty.clone(), *span))
            }
            Expr::Cast(expr, target_ty, span) => {
                let inner = Box::new(self.lower_expr(expr)?);
                Ok(HirExpr::Cast {
                    expr: inner,
                    target_ty: target_ty.clone(),
                    span: *span,
                })
            }
            Expr::Paren(inner, _) => self.lower_expr(inner),
        }
    }

    fn default_init(ty: &Type, span: Span) -> Result<HirExpr, LowerError> {
        Ok(match ty {
            Type::I8 => HirExpr::Int(0, Type::I8, span),
            Type::U8 => HirExpr::Int(0, Type::U8, span),
            Type::I16 => HirExpr::Int(0, Type::I16, span),
            Type::U16 => HirExpr::Int(0, Type::U16, span),
            Type::I32 => HirExpr::Int(0, Type::I32, span),
            Type::U32 => HirExpr::Int(0, Type::U32, span),
            Type::I64 => HirExpr::Int(0, Type::I64, span),
            Type::U64 => HirExpr::Int(0, Type::U64, span),
            Type::F32 => HirExpr::Float(0.0, Type::F32, span),
            Type::F64 => HirExpr::Float(0.0, Type::F64, span),
            Type::Bool => HirExpr::Bool(false, span),
            Type::String => HirExpr::String(String::new(), span),
            Type::Pointer(_) => HirExpr::Int(0, Type::I64, span), // Null pointer
            Type::Struct(_) | Type::Array(_, _) | Type::Fn(..) | Type::Void => {
                return Err(LowerError {
                    message: format!(
                        "type `{}` cannot be default-initialized; provide an explicit value",
                        ty
                    ),
                    span,
                });
            }
        })
    }
}
