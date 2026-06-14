use crate::lexer::{IntegerSuffix, Span, Token, TokenKind};
use crate::ops::{BinOp, UnOp};
use crate::sema::Type;
use crate::parser::ast::*;

pub(crate) struct Parser<'a> {
    tokens: &'a [Token],
    pos: usize,
}

impl<'a> Parser<'a> {
    pub fn new(tokens: &'a [Token]) -> Self {
        Self { tokens, pos: 0 }
    }

    pub fn parse_file(&mut self) -> Result<Vec<Stmt>, ParseError> {
        let mut stmts = Vec::new();
        while !self.is_eof() {
            stmts.push(self.parse_decl_or_stmt()?);
        }
        Ok(stmts)
    }

    fn parse_decl_or_stmt(&mut self) -> Result<Stmt, ParseError> {
        match self.peek_kind() {
            TokenKind::Fn => self.parse_fn_decl(),
            TokenKind::Struct => self.parse_struct_decl(),
            _ => self.parse_stmt(),
        }
    }

    fn parse_stmt(&mut self) -> Result<Stmt, ParseError> {
        match self.peek_kind() {
            TokenKind::Let => self.parse_let(),
            TokenKind::If => self.parse_if(),
            TokenKind::While => self.parse_while(),
            TokenKind::For => self.parse_for(),
            TokenKind::Break => self.parse_break(),
            TokenKind::Continue => self.parse_continue(),
            TokenKind::Return => self.parse_return(),
            TokenKind::LBrace => {
                let span = self.peek_token().span;
                self.advance();
                let _stmts = self.parse_stmts_until(TokenKind::RBrace)?;
                Err(self.make_error("unexpected block".into(), span))
            }
            _ => {
                let expr = self.parse_expression()?;
                self.expect_semicolon()?;
                Ok(Stmt::Expr(expr))
            }
        }
    }

    fn expect_semicolon(&mut self) -> Result<(), ParseError> {
        if self.peek_is(&TokenKind::Semicolon) {
            self.advance();
            Ok(())
        } else {
            Err(self.make_error("expected `;`".into(), self.peek_token().span))
        }
    }

    fn parse_fn_decl(&mut self) -> Result<Stmt, ParseError> {
        let start = self.peek_token().span;
        self.advance(); // fn
        let name = self.expect_ident()?;
        self.expect(&TokenKind::LParen)?;
        let mut params = Vec::new();
        while !self.peek_is(&TokenKind::RParen) {
            let pname = self.expect_ident()?;
            self.expect(&TokenKind::Colon)?;
            let pty = self.parse_type()?;
            params.push((pname, pty));
            if self.peek_is(&TokenKind::Comma) {
                self.advance();
            }
        }
        self.expect(&TokenKind::RParen)?;
        let ret = if self.peek_is(&TokenKind::Arrow) {
            self.advance();
            Some(self.parse_type()?)
        } else {
            None
        };
        self.expect(&TokenKind::LBrace)?;
        let body = self.parse_stmts_until(TokenKind::RBrace)?;
        Ok(Stmt::Fn(name, params, ret, body, self.span_since(start)))
    }

    fn parse_struct_decl(&mut self) -> Result<Stmt, ParseError> {
        let start = self.peek_token().span;
        self.advance(); // struct
        let name = self.expect_ident()?;
        self.expect(&TokenKind::LBrace)?;
        let mut fields = Vec::new();
        while !self.peek_is(&TokenKind::RBrace) {
            let fname = self.expect_ident()?;
            self.expect(&TokenKind::Colon)?;
            let fty = self.parse_type()?;
            fields.push((fname, fty));
            if self.peek_is(&TokenKind::Comma) {
                self.advance();
            }
        }
        self.expect(&TokenKind::RBrace)?;
        Ok(Stmt::Struct(name, fields, self.span_since(start)))
    }

    fn parse_let(&mut self) -> Result<Stmt, ParseError> {
        let start = self.peek_token().span;
        self.advance(); // let
        let name = self.expect_ident()?;
        let ty_annot = if self.peek_is(&TokenKind::Colon) {
            self.advance();
            Some(self.parse_type()?)
        } else {
            None
        };
        let init = if self.peek_is(&TokenKind::Eq) {
            self.advance();
            Some(self.parse_expression()?)
        } else {
            None
        };
        self.expect_semicolon()?;
        Ok(Stmt::Let(name, ty_annot, init, self.span_since(start)))
    }

    fn parse_if(&mut self) -> Result<Stmt, ParseError> {
        let start = self.peek_token().span;
        self.advance(); // if
        let cond = self.parse_expression()?;
        Self::reject_assignment_in_condition(&cond)?;
        let then_stmts = self.parse_block()?;
        let else_stmts = if self.peek_is(&TokenKind::Else) {
            self.advance();
            if self.peek_is(&TokenKind::If) {
                let inner = self.parse_if()?;
                Some(vec![inner])
            } else {
                Some(self.parse_block()?)
            }
        } else {
            None
        };
        Ok(Stmt::If(cond, then_stmts, else_stmts, self.span_since(start)))
    }

    fn parse_while(&mut self) -> Result<Stmt, ParseError> {
        let start = self.peek_token().span;
        self.advance(); // while
        let cond = self.parse_expression()?;
        Self::reject_assignment_in_condition(&cond)?;
        let body = self.parse_block()?;
        Ok(Stmt::While(cond, body, self.span_since(start)))
    }

    fn reject_assignment_in_condition(expr: &Expr) -> Result<(), ParseError> {
        // Look through parens to find a direct assignment, e.g. `if ((x = 1))`.
        // We deliberately do NOT recurse into Unary/Binary — `if (!!x)` or
        // `if (a && b)` should be left alone, only top-level `=` is suspicious.
        let inner = match expr {
            Expr::Paren(e, _) => e.as_ref(),
            other => other,
        };
        if matches!(inner, Expr::Assign(..)) {
            return Err(ParseError {
                message: "assignment is not allowed in if/while condition; did you mean `==`?".into(),
                span: expr.span(),
            });
        }
        Ok(())
    }

    fn parse_for(&mut self) -> Result<Stmt, ParseError> {
        let start = self.peek_token().span;
        self.advance(); // for
        let var = self.expect_ident()?;
        self.expect(&TokenKind::In)?;
        let lo = self.parse_expression()?;
        self.expect(&TokenKind::DotDot)?;
        let hi = self.parse_expression()?;
        let body = self.parse_block()?;
        Ok(Stmt::For(var, lo, hi, body, self.span_since(start)))
    }

    fn parse_break(&mut self) -> Result<Stmt, ParseError> {
        let span = self.peek_token().span;
        self.advance(); // break
        self.expect_semicolon()?;
        Ok(Stmt::Break(span))
    }

    fn parse_continue(&mut self) -> Result<Stmt, ParseError> {
        let span = self.peek_token().span;
        self.advance(); // continue
        self.expect_semicolon()?;
        Ok(Stmt::Continue(span))
    }

    fn parse_return(&mut self) -> Result<Stmt, ParseError> {
        let span = self.peek_token().span;
        self.advance(); // return
        let value = if self.peek_is(&TokenKind::Semicolon) {
            None
        } else {
            Some(self.parse_expression()?)
        };
        self.expect_semicolon()?;
        Ok(Stmt::Return(value, span))
    }

    fn parse_block(&mut self) -> Result<Vec<Stmt>, ParseError> {
        self.expect(&TokenKind::LBrace)?;
        self.parse_stmts_until(TokenKind::RBrace)
    }

    fn parse_stmts_until(&mut self, end: TokenKind) -> Result<Vec<Stmt>, ParseError> {
        let mut stmts = Vec::new();
        while !self.peek_is(&end) && !self.is_eof() {
            stmts.push(self.parse_decl_or_stmt()?);
        }
        self.expect(&end)?;
        Ok(stmts)
    }

    fn parse_expression(&mut self) -> Result<Expr, ParseError> {
        let mut expr = self.parse_unary()?;
        expr = self.parse_binary_op(0, expr)?;
        if self.peek_is(&TokenKind::Eq) {
            let span = expr.span();
            self.advance();
            let rhs = self.parse_expression()?;
            expr = Expr::Assign(Box::new(expr), Box::new(rhs), self.span_since(span));
        }
        Ok(expr)
    }

    fn parse_binary_op(&mut self, min_prec: u32, mut lhs: Expr) -> Result<Expr, ParseError> {
        loop {
            let op = match self.peek_kind() {
                TokenKind::Plus => BinOp::Add,
                TokenKind::Minus => BinOp::Sub,
                TokenKind::Star => BinOp::Mul,
                TokenKind::Slash => BinOp::Div,
                TokenKind::Percent => BinOp::Rem,
                TokenKind::EqEq => BinOp::Eq,
                TokenKind::BangEq => BinOp::Ne,
                TokenKind::Lt => BinOp::Lt,
                TokenKind::LtEq => BinOp::Le,
                TokenKind::Gt => BinOp::Gt,
                TokenKind::GtEq => BinOp::Ge,
                TokenKind::AmpAmp => BinOp::And,
                TokenKind::PipePipe => BinOp::Or,
                TokenKind::Pipe => BinOp::BitOr,
                TokenKind::Caret => BinOp::BitXor,
                TokenKind::Amp => BinOp::BitAnd,
                TokenKind::LtLt => BinOp::Shl,
                TokenKind::GtGt => BinOp::Shr,
                _ => break,
            };
            if precedence(op) < min_prec {
                break;
            }
            self.advance();
            let rhs_prec = precedence(op) + 1;
            let lhs_span = lhs.span();
            let rhs_unary = self.parse_unary()?;
            let rhs = self.parse_binary_op(rhs_prec, rhs_unary)?;
            lhs = Expr::Binary(Box::new(lhs), op, Box::new(rhs), self.span_since(lhs_span));
        }
        Ok(lhs)
    }

    fn parse_unary(&mut self) -> Result<Expr, ParseError> {
        match self.peek_kind() {
            TokenKind::Minus => {
                let span = self.peek_token().span;
                self.advance();
                let expr = self.parse_unary()?;
                Ok(Expr::Unary(UnOp::Neg, Box::new(expr), span))
            }
            TokenKind::Bang => {
                let span = self.peek_token().span;
                self.advance();
                let expr = self.parse_unary()?;
                Ok(Expr::Unary(UnOp::Not, Box::new(expr), span))
            }
            TokenKind::MinusMinus => {
                let span = self.peek_token().span;
                self.advance();
                let expr = self.parse_unary()?;
                let one = Expr::Int(1, IntegerSuffix::None, span);
                let sub = Expr::Binary(Box::new(expr.clone()), BinOp::Sub, Box::new(one), span);
                Ok(Expr::Assign(Box::new(expr), Box::new(sub), span))
            }
            _ => self.parse_postfix(),
        }
    }

    fn parse_postfix(&mut self) -> Result<Expr, ParseError> {
        let mut expr = self.parse_primary()?;
        loop {
            match self.peek_kind() {
                TokenKind::LParen => {
                    self.advance();
                    let mut args = Vec::new();
                    while !self.peek_is(&TokenKind::RParen) {
                        args.push(self.parse_expression()?);
                        if self.peek_is(&TokenKind::Comma) {
                            self.advance();
                        }
                    }
                    self.expect(&TokenKind::RParen)?;
                    if let Expr::Ident(name, ident_span) = expr {
                        expr = Expr::Call(name, args, self.span_since(ident_span));
                    } else {
                        return Err(self.make_error("cannot call non-identifier".into(), self.peek_token().span));
                    }
                }
                TokenKind::Dot => {
                    self.advance();
                    let field = self.expect_ident()?;
                    let span = self.span_since(expr.span());
                    expr = Expr::FieldAccess(Box::new(expr), field, span);
                }
                TokenKind::LBracket => {
                    self.advance();
                    let idx = self.parse_expression()?;
                    self.expect(&TokenKind::RBracket)?;
                    let span = self.span_since(expr.span());
                    expr = Expr::ArrayIndex(Box::new(expr), Box::new(idx), span);
                }
                TokenKind::PlusEq | TokenKind::MinusEq | TokenKind::StarEq |
                TokenKind::SlashEq | TokenKind::PercentEq | TokenKind::AmpEq |
                TokenKind::PipeEq | TokenKind::CaretEq | TokenKind::LtLtEq |
                TokenKind::GtGtEq => {
                    let lhs_span = expr.span();
                    let op = compound_to_binop(&self.peek_kind());
                    self.advance();
                    let rhs = self.parse_expression()?;
                    let bin = Expr::Binary(Box::new(expr.clone()), op, Box::new(rhs), self.span_since(lhs_span));
                    expr = Expr::Assign(Box::new(expr), Box::new(bin), self.span_since(lhs_span));
                }
                TokenKind::PlusPlus => {
                    let span = self.peek_token().span;
                    self.advance();
                    let one = Expr::Int(1, IntegerSuffix::None, span);
                    let add = Expr::Binary(Box::new(expr.clone()), BinOp::Add, Box::new(one), span);
                    expr = Expr::Assign(Box::new(expr), Box::new(add), span);
                }
                TokenKind::MinusMinus => {
                    let span = self.peek_token().span;
                    self.advance();
                    let one = Expr::Int(1, IntegerSuffix::None, span);
                    let sub = Expr::Binary(Box::new(expr.clone()), BinOp::Sub, Box::new(one), span);
                    expr = Expr::Assign(Box::new(expr), Box::new(sub), span);
                }
                TokenKind::LBrace => {
                    // Struct literal initializer: Ident { field: expr, ... }
                    let name_span = expr.span();
                    let name = match &expr {
                        Expr::Ident(n, _) => n.clone(),
                        Expr::FieldAccess(_, field, _) => field.clone(),
                        _ => break,
                    };
                    self.advance();
                    let mut fields = Vec::new();
                    while !self.peek_is(&TokenKind::RBrace) {
                        let fname = self.expect_ident()?;
                        self.expect(&TokenKind::Colon)?;
                        let fval = self.parse_expression()?;
                        fields.push((fname, fval));
                        if self.peek_is(&TokenKind::Comma) {
                            self.advance();
                        }
                    }
                    self.expect(&TokenKind::RBrace)?;
                    expr = Expr::Struct(name, fields, self.span_since(name_span));
                }
                _ => break,
            }
        }
        Ok(expr)
    }

    fn parse_primary(&mut self) -> Result<Expr, ParseError> {
        match self.peek_kind() {
            TokenKind::Int(n, suffix) => {
                let span = self.peek_token().span;
                self.advance();
                Ok(Expr::Int(n, suffix, span))
            }
            TokenKind::Float(n, suffix) => {
                let span = self.peek_token().span;
                self.advance();
                Ok(Expr::Float(n, suffix, span))
            }
            TokenKind::String(s) => {
                let span = self.peek_token().span;
                self.advance();
                Ok(Expr::String(s, span))
            }
            TokenKind::True => {
                let span = self.peek_token().span;
                self.advance();
                Ok(Expr::Bool(true, span))
            }
            TokenKind::False => {
                let span = self.peek_token().span;
                self.advance();
                Ok(Expr::Bool(false, span))
            }

            TokenKind::Ident(name) => {
                let span = self.peek_token().span;
                self.advance();
                Ok(Expr::Ident(name, span))
            }
            TokenKind::Struct => {
                let start = self.peek_token().span;
                self.advance();
                let name = self.expect_ident()?;
                self.expect(&TokenKind::LBrace)?;
                let mut fields = Vec::new();
                while !self.peek_is(&TokenKind::RBrace) {
                    let fname = self.expect_ident()?;
                    self.expect(&TokenKind::Colon)?;
                    let fval = self.parse_expression()?;
                    fields.push((fname, fval));
                    if self.peek_is(&TokenKind::Comma) {
                        self.advance();
                    }
                }
                self.expect(&TokenKind::RBrace)?;
                Ok(Expr::Struct(name, fields, self.span_since(start)))
            }
            TokenKind::I8 | TokenKind::I16 | TokenKind::I32 | TokenKind::I64
            | TokenKind::U8 | TokenKind::U16 | TokenKind::U32 | TokenKind::U64
            | TokenKind::F32 | TokenKind::F64 | TokenKind::Bool | TokenKind::StringTy => {
                let start = self.peek_token().span;
                let ty = self.parse_base_type()?;
                // Array literal: i32{1, 2, 3}
                self.expect(&TokenKind::LBrace)?;
                let mut elems = Vec::new();
                while !self.peek_is(&TokenKind::RBrace) {
                    elems.push(self.parse_expression()?);
                    if self.peek_is(&TokenKind::Comma) {
                        self.advance();
                    }
                }
                self.expect(&TokenKind::RBrace)?;
                Ok(Expr::ArrayLiteral(Box::new(ty), elems, self.span_since(start)))
            }
            TokenKind::LBracket => {
                let start = self.peek_token().span;
                self.advance();
                let mut elems = Vec::new();
                while !self.peek_is(&TokenKind::RBracket) {
                    elems.push(self.parse_expression()?);
                    if self.peek_is(&TokenKind::Comma) {
                        self.advance();
                    }
                }
                self.expect(&TokenKind::RBracket)?;
                let ty = Type::Void;
                Ok(Expr::ArrayLiteral(Box::new(ty), elems, self.span_since(start)))
            }
            TokenKind::LParen => {
                let start = self.peek_token().span;
                self.advance();
                let expr = self.parse_expression()?;
                self.expect(&TokenKind::RParen)?;
                Ok(Expr::Paren(Box::new(expr), start))
            }
            _ => Err(self.make_error("expected expression".into(), self.peek_token().span)),
        }
    }

    fn parse_base_type(&mut self) -> Result<Type, ParseError> {
        match self.peek_kind() {
            TokenKind::I8 => { self.advance(); Ok(Type::I8) }
            TokenKind::I16 => { self.advance(); Ok(Type::I16) }
            TokenKind::I32 => { self.advance(); Ok(Type::I32) }
            TokenKind::I64 => { self.advance(); Ok(Type::I64) }
            TokenKind::U8 => { self.advance(); Ok(Type::U8) }
            TokenKind::U16 => { self.advance(); Ok(Type::U16) }
            TokenKind::U32 => { self.advance(); Ok(Type::U32) }
            TokenKind::U64 => { self.advance(); Ok(Type::U64) }
            TokenKind::F32 => { self.advance(); Ok(Type::F32) }
            TokenKind::F64 => { self.advance(); Ok(Type::F64) }
            TokenKind::Bool => { self.advance(); Ok(Type::Bool) }
            TokenKind::StringTy => { self.advance(); Ok(Type::String) }
            TokenKind::Void => { self.advance(); Ok(Type::Void) }
            TokenKind::Ident(name) => {
                self.advance();
                Ok(Type::Struct(name))
            }
            _ => Err(self.make_error("expected type".into(), self.peek_token().span)),
        }
    }

    fn parse_type(&mut self) -> Result<Type, ParseError> {
        let base = self.parse_base_type()?;
        if self.peek_is(&TokenKind::LBracket) {
            self.advance();
            let size = match self.peek_kind() {
                TokenKind::Int(n, _) => {
                    self.advance();
                    usize::try_from(n).map_err(|_| {
                        self.make_error("array size overflow".into(), self.peek_token().span)
                    })?
                }
                _ => {
                    return Err(self.make_error("expected array size".into(), self.peek_token().span));
                }
            };
            self.expect(&TokenKind::RBracket)?;
            Ok(Type::Array(Box::new(base), size))
        } else {
            Ok(base)
        }
    }

    fn expect_ident(&mut self) -> Result<String, ParseError> {
        match self.peek_kind() {
            TokenKind::Ident(name) => {
                self.advance();
                Ok(name)
            }
            _ => Err(self.make_error("expected identifier".into(), self.peek_token().span)),
        }
    }

    fn expect(&mut self, kind: &TokenKind) -> Result<(), ParseError> {
        if self.peek_is(kind) {
            self.advance();
            Ok(())
        } else {
            Err(self.make_error(format!("expected {}, found {}", kind, self.peek_kind()), self.peek_token().span))
        }
    }

    fn peek_is(&self, kind: &TokenKind) -> bool {
        self.tokens.get(self.pos).map(|t| &t.kind == kind).unwrap_or(false)
    }

    fn peek_kind(&self) -> TokenKind {
        self.tokens.get(self.pos).map(|t| t.kind.clone()).unwrap_or(TokenKind::Eof)
    }

    fn peek_token(&self) -> &Token {
        self.tokens.get(self.pos).unwrap_or_else(|| {
            // SAFETY: tokens always contains at least the Eof token at the end
            &self.tokens[self.tokens.len() - 1]
        })
    }

    fn advance(&mut self) {
        self.pos += 1;
    }

    fn is_eof(&self) -> bool {
        self.peek_is(&TokenKind::Eof)
    }

    fn span_since(&self, start: Span) -> Span {
        let end = self.peek_token().span;
        Span {
            start: start.start,
            end: end.start,
            line: start.line,
            col: start.col,
        }
    }

    fn make_error(&self, message: String, span: Span) -> ParseError {
        ParseError { message, span }
    }
}

fn precedence(op: BinOp) -> u32 {
    match op {
        BinOp::Or => 1,
        BinOp::And => 2,
        BinOp::BitOr => 3,
        BinOp::BitXor => 4,
        BinOp::BitAnd => 5,
        BinOp::Eq | BinOp::Ne | BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge => 6,
        BinOp::Shl | BinOp::Shr => 7,
        BinOp::Add | BinOp::Sub => 8,
        BinOp::Mul | BinOp::Div | BinOp::Rem => 9,
    }
}

fn compound_to_binop(kind: &TokenKind) -> BinOp {
    match kind {
        TokenKind::PlusEq => BinOp::Add,
        TokenKind::MinusEq => BinOp::Sub,
        TokenKind::StarEq => BinOp::Mul,
        TokenKind::SlashEq => BinOp::Div,
        TokenKind::PercentEq => BinOp::Rem,
        TokenKind::AmpEq => BinOp::BitAnd,
        TokenKind::PipeEq => BinOp::BitOr,
        TokenKind::CaretEq => BinOp::BitXor,
        TokenKind::LtLtEq => BinOp::Shl,
        TokenKind::GtGtEq => BinOp::Shr,
        _ => panic!("internal: unexpected token in compound_assign_op (compiler bug)"),
    }
}
