use std::collections::{HashMap, HashSet};

use anyhow::{Result, bail};

use crate::ir::{HirExpr, HirStmt};
use crate::lexer::Span;

/// Minimal borrow checker.
///
/// Tracks which variables have outstanding borrows and rejects:
/// - Using a variable while it's borrowed (move, reassign, fn call)
/// - Returning a borrow of a local variable
pub(crate) struct BorrowChecker;

impl BorrowChecker {
    pub fn new() -> Self {
        Self
    }

    pub fn check(&mut self, stmts: &[HirStmt]) -> Result<()> {
        let mut ctx = BorrowCtx::new();
        ctx.check_stmts(stmts);
        if ctx.errors.is_empty() {
            Ok(())
        } else {
            bail!("borrow check failed:\n{}", ctx.errors.iter().map(|e| e.to_string()).collect::<Vec<_>>().join("\n"))
        }
    }
}

#[derive(Debug, Clone)]
struct BorrowError {
    message: String,
    span: Option<Span>,
}

impl std::fmt::Display for BorrowError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.span {
            Some(s) => write!(f, "borrow error at {}:{}: {}", s.line, s.col, self.message),
            None => write!(f, "borrow error: {}", self.message),
        }
    }
}

struct BorrowCtx {
    /// Map: variable name → set of variable names that borrow from it
    borrowed: HashMap<String, HashSet<String>>,
    /// Variables whose value has been moved out (e.g. `let y = x; x`).
    /// Subsequent use of these is a use-after-move error.
    moved: HashSet<String>,
    /// Collected error messages (instead of bailing on first error)
    errors: Vec<BorrowError>,
}

impl BorrowCtx {
    fn new() -> Self {
        Self {
            borrowed: HashMap::new(),
            moved: HashSet::new(),
            errors: Vec::new(),
        }
    }

    fn is_borrower(&self, name: &str) -> bool {
        self.borrowed.values().any(|borrowers| borrowers.contains(name))
    }

    fn error_at(&mut self, span: Option<Span>, message: String) {
        self.errors.push(BorrowError { message, span });
    }

    fn error_if_borrowed(&mut self, name: &str, context: &str, span: Span) {
        if let Some(borrowers) = self.borrowed.get(name)
            && !borrowers.is_empty()
        {
            let borrowers: Vec<&str> = borrowers.iter().map(String::as_str).collect();
            self.error_at(
                Some(span),
                format!(
                    "cannot {} `{}`: it is borrowed by `{}`",
                    context,
                    name,
                    borrowers.join(", "),
                ),
            );
        }
    }

    /// Mark `name` as moved (its value has been transferred elsewhere).
    /// The caller should have already checked that `name` is not currently
    /// borrowed (via `error_if_borrowed`).
    fn mark_moved(&mut self, name: &str) {
        self.moved.insert(name.to_string());
    }

    /// Reassigning a variable revives it (it now holds a fresh value).
    fn unmark_moved(&mut self, name: &str) {
        self.moved.remove(name);
    }

    /// Record that `name` borrows from `src`. Also transitively records
    /// that `name` borrows from any source that `src` itself borrows from,
    /// so an attempt to move that source will be rejected while `name` is alive.
    fn record_borrow_of(&mut self, name: &str, src: &str) {
        self.borrowed.entry(src.to_string()).or_default().insert(name.to_string());

        let mut transitive = Vec::new();
        let mut seen = std::collections::HashSet::new();
        let mut stack = vec![src.to_string()];
        while let Some(current) = stack.pop() {
            if !seen.insert(current.clone()) {
                continue;
            }
            for (source, borrowers) in &self.borrowed {
                if borrowers.contains(&current) {
                    transitive.push(source.clone());
                    stack.push(source.clone());
                }
            }
        }
        for source in transitive {
            self.borrowed.entry(source).or_default().insert(name.to_string());
        }
    }

    fn check_stmts(&mut self, stmts: &[HirStmt]) {
        for stmt in stmts {
            self.check_stmt(stmt);
        }
    }

    fn check_stmt(&mut self, stmt: &HirStmt) {
        match stmt {
            HirStmt::Let { name, init, span, .. } => {
                self.check_expr(init);
                for src in collect_borrow_sources(init) {
                    self.record_borrow_of(name, &src);
                }
                if let HirExpr::Ident { name: src, ty, .. } = init
                    && !ty.is_copy()
                    && src != name
                {
                    // `let y = x` only moves `x` if `x` is owned
                    // (String/Struct/Array). Copy types (i32, bool,
                    // f32, …) are bitwise duplicated and remain
                    // usable after the assignment. `let x = x;`
                    // (self-shadow) is special: the new binding
                    // shadows the old one, so the old name is no
                    // longer accessible anyway — no use-after-move
                    // to report.
                    self.error_if_borrowed(src, "move", *span);
                    self.mark_moved(src);
                }
            }
            HirStmt::Expr(expr, _) => self.check_expr(expr),
            HirStmt::Fn { body, .. } => {
                let prev_borrowed = std::mem::take(&mut self.borrowed);
                let prev_moved = std::mem::take(&mut self.moved);
                self.check_stmts(body);
                self.borrowed = prev_borrowed;
                self.moved = prev_moved;
            }
            HirStmt::Struct { .. } => {}
            HirStmt::If { cond, then, else_, .. } => {
                self.check_expr(cond);
                self.check_stmts(then);
                if let Some(else_stmts) = else_ {
                    self.check_stmts(else_stmts);
                }
            }
            HirStmt::While { cond, body, .. } => {
                self.check_expr(cond);
                self.check_stmts(body);
            }
            HirStmt::For { lo, hi, body, .. } => {
                self.check_expr(lo);
                self.check_expr(hi);
                self.check_stmts(body);
            }
            HirStmt::Break(_) | HirStmt::Continue(_) => {}
            HirStmt::Return(Some(expr), span) => {
                self.check_expr(expr);
                if let HirExpr::Ident { name, ty, .. } = expr
                    && !ty.is_copy()
                    && self.is_borrower(name) {
                        self.error_at(
                            Some(*span),
                            format!("cannot return local borrow `{}`", name),
                        );
                    }
            }
            HirStmt::Return(None, _) => {}
        }
    }

    /// Like `check_expr` but does not report use-after-move on the
    /// outermost `Ident`. Used for the LHS of an assignment, where the
    /// very point is to revive the variable. Sub-LHS chains (e.g.
    /// `o1.inner` or `a[i]` in `o1.inner = ...`, `a[i] = ...`) recurse
    /// to the base variable so that a borrowed base is still rejected
    /// — `o1.inner = ...` mutates `o1` underneath any active borrow.
    fn check_expr_lhs(&mut self, expr: &HirExpr) {
        match expr {
            HirExpr::Ident { name, span, .. } => {
                self.error_if_borrowed(name, "assign to borrowed variable", *span);
            }
            HirExpr::FieldLoad { object, .. } => {
                self.check_expr_lhs(object);
            }
            HirExpr::ArrayIndex { object, index, .. } => {
                self.check_expr_lhs(object);
                self.check_expr(index);
            }
            _ => self.check_expr(expr),
        }
    }

    fn check_expr(&mut self, expr: &HirExpr) {
        match expr {
            HirExpr::Int(..)
            | HirExpr::Float(..)
            | HirExpr::String(..)
            | HirExpr::Bool(..) => {}
            HirExpr::Ident { name, ty, span } => {
                // Use-after-move only applies to owned types; Copy
                // types can be used freely after a "move" (which is
                // really a copy).
                if !ty.is_copy() && self.moved.contains(name) {
                    self.error_at(
                        Some(*span),
                        format!("use after move: `{}`", name),
                    );
                }
            }
            HirExpr::Print(inner, _) => {
                // `print(&s)` borrows `s` for the duration of the call;
                // after print returns the borrow is gone, so we don't
                // record it. Only `let x = &s;` extends the borrow
                // lifetime, and that's handled in `check_stmt` for `Let`.
                self.check_expr(inner);
            }
            HirExpr::Borrow(inner, _) => {
                // Multiple shared borrows of the same variable are legal
                // (they never conflict with each other), so we only need
                // to check that the source has not been moved. That
                // happens automatically via `check_expr(inner)`.
                // Borrow tracking for the parent Let binding is handled
                // in `check_stmt`.
                self.check_expr(inner);
            }
            HirExpr::AllocStruct(_, fields, span) => {
                for (_, expr) in fields {
                    self.check_expr(expr);
                    if let HirExpr::Ident { name: src, ty, span: inner_span } = expr
                        && !ty.is_copy()
                    {
                        self.error_if_borrowed(src, "move into struct field", *inner_span);
                        self.mark_moved(src);
                    }
                }
                let _ = span;
            }
            HirExpr::ArrayLiteral(_, elems, _) => {
                for elem in elems {
                    self.check_expr(elem);
                    if let HirExpr::Ident { name: src, ty, span: inner_span } = elem
                        && !ty.is_copy()
                    {
                        self.error_if_borrowed(src, "move into array", *inner_span);
                        self.mark_moved(src);
                    }
                }
            }
            HirExpr::Call(_, args, span) => {
                for arg in args {
                    self.check_expr(arg);
                    if let HirExpr::Ident { name: src, ty, span: inner_span } = arg
                        && !ty.is_copy()
                    {
                        self.error_if_borrowed(src, "move into function call", *inner_span);
                        self.mark_moved(src);
                    }
                }
                let _ = span;
            }
            HirExpr::FieldLoad { object, .. } => {
                self.check_expr(object);
            }
            HirExpr::ArrayIndex { object, index, .. } => {
                self.check_expr(object);
                self.check_expr(index);
            }
            HirExpr::Unary(_, inner, _) => {
                self.check_expr(inner);
            }
            HirExpr::Binary { lhs, rhs, .. } => {
                self.check_expr(lhs);
                self.check_expr(rhs);
            }
            HirExpr::Assign { lhs, rhs, span } => {
                // Don't check moved state on the LHS — `x = 42` is
                // precisely the operation that revives a moved variable.
                // We do still need to check sub-expressions for borrow
                // conflicts and use-after-move in sub-objects.
                self.check_expr_lhs(lhs);
                self.check_expr(rhs);
                if let HirExpr::Ident { name, .. } = lhs.as_ref() {
                    // `check_expr_lhs` already reported the borrow
                    // conflict if any. Just revive the variable.
                    self.unmark_moved(name);
                }
                // `lhs.field = rhs` (or `lhs[i] = rhs`) deep-copies the
                // struct/array, so the rhs Ident is NOT moved and remains
                // usable. Only mark moved when assigning to a top-level
                // Ident.
                //
                // Self-assignment (`x = x`) is a copy-back: the read
                // happens before the write, so the value is not moved.
                if let (HirExpr::Ident { name: lhs_name, .. }, HirExpr::Ident { name: src, ty, span: inner_span }) =
                    (lhs.as_ref(), rhs.as_ref())
                    && lhs_name != src
                    && !ty.is_copy()
                {
                    self.error_if_borrowed(src, "move in assignment", *inner_span);
                    self.mark_moved(src);
                }
                let _ = span;
            }
        }
    }
}

/// Walk a `&x` / `&&x` / `&&&x` chain and collect all base identifiers
/// at the end, including through `FieldLoad` / `ArrayIndex` chains.
/// For example, `&s.f` collects `["s"]`, and `&&s.f[i]` collects `["s"]`.
/// Returns an empty vec if the expression is not a borrow chain or ends
/// in an untrackable node.
fn borrow_chain_sources(expr: &HirExpr) -> Vec<String> {
    let mut current = expr;
    let mut found_borrow = false;
    while let HirExpr::Borrow(inner, _) = current {
        found_borrow = true;
        current = inner;
    }
    if !found_borrow {
        return vec![];
    }
    let mut out = Vec::new();
    collect_idents_from_path(current, &mut out);
    out
}

/// Recursively collect all `Ident` names from a chain of `FieldLoad` /
/// `ArrayIndex` / `Ident` (the end of a borrow chain).
fn collect_idents_from_path(expr: &HirExpr, out: &mut Vec<String>) {
    match expr {
        HirExpr::Ident { name, ty, .. } => {
            // Borrowing a Copy value doesn't extend its lifetime —
            // a subsequent move/use of the source is still legal.
            if !ty.is_copy() {
                out.push(name.clone());
            }
        }
        HirExpr::FieldLoad { object, .. } => collect_idents_from_path(object, out),
        HirExpr::ArrayIndex { object, .. } => collect_idents_from_path(object, out),
        _ => {}
    }
}

/// Walk an init expression and collect the base identifiers of every
/// borrow chain hidden inside it (struct fields, array elements, function
/// arguments, nested sub-expressions, …). Each returned name is a source
/// whose lifetime is extended by this `let`, so a later move of that
/// source must be rejected.
fn collect_borrow_sources(expr: &HirExpr) -> Vec<String> {
    let mut out = Vec::new();
    collect_borrow_sources_inner(expr, &mut out);
    out
}

fn collect_borrow_sources_inner(expr: &HirExpr, out: &mut Vec<String>) {
    match expr {
        HirExpr::Borrow(_, _) => {
            out.extend(borrow_chain_sources(expr));
        }
        HirExpr::AllocStruct(_, fields, _) => {
            for (_, e) in fields {
                collect_borrow_sources_inner(e, out);
            }
        }
        HirExpr::ArrayLiteral(_, elems, _) => {
            for e in elems {
                collect_borrow_sources_inner(e, out);
            }
        }
        HirExpr::Call(_, args, _) => {
            for a in args {
                collect_borrow_sources_inner(a, out);
            }
        }
        HirExpr::FieldLoad { object, .. } => {
            collect_borrow_sources_inner(object, out);
        }
        HirExpr::ArrayIndex { object, index, .. } => {
            collect_borrow_sources_inner(object, out);
            collect_borrow_sources_inner(index, out);
        }
        HirExpr::Unary(_, inner, _) => {
            collect_borrow_sources_inner(inner, out);
        }
        HirExpr::Binary { lhs, rhs, .. } => {
            collect_borrow_sources_inner(lhs, out);
            collect_borrow_sources_inner(rhs, out);
        }
        HirExpr::Assign { lhs, rhs, .. } => {
            collect_borrow_sources_inner(lhs, out);
            collect_borrow_sources_inner(rhs, out);
        }
        HirExpr::Print(inner, _) => {
            collect_borrow_sources_inner(inner, out);
        }
        HirExpr::Int(..)
        | HirExpr::Float(..)
        | HirExpr::String(..)
        | HirExpr::Bool(..)
        | HirExpr::Ident { .. } => {}
    }
}
