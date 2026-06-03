use std::path::Path;

use anyhow::Result;
use inkwell::OptimizationLevel;

use crate::codegen;
use crate::ir::Lowerer;
use crate::lexer::Lexer;
use crate::parser::Parser;
use crate::sema::{BorrowChecker, TypeChecker};

pub fn run(source: &str, output: &Path, opt_level: OptimizationLevel) -> Result<()> {
    let mut lexer = Lexer::new(source);
    let tokens = lexer.tokenize()?;
    let mut parser = Parser::new(&tokens);
    let stmts = parser.parse_file()?;
    let mut checker = TypeChecker::new();
    checker.check(&stmts)?;
    let ctx = checker.into_context();
    let mut lowerer = Lowerer::new(ctx);
    let hir = lowerer.lower(&stmts)?;
    let struct_field_types = lowerer.struct_types().clone();
    let mut borrowck = BorrowChecker::new();
    borrowck.check(&hir)?;
    codegen::generate(output, &hir, struct_field_types, opt_level)
}
