use std::collections::HashSet;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use inkwell::OptimizationLevel;

use crate::codegen;
use crate::ir::Lowerer;
use crate::lexer::Lexer;
use crate::parser::{Parser, Stmt};
use crate::sema::TypeChecker;

pub fn run(inputs: &[PathBuf], output: &Path, opt_level: OptimizationLevel) -> Result<()> {
    let mut all_stmts = Vec::new();
    let mut visited = HashSet::new();

    for input in inputs {
        let abs_path = std::fs::canonicalize(input).unwrap_or_else(|_| input.to_path_buf());
        load_module(&abs_path, &mut all_stmts, &mut visited)?;
    }

    let mut checker = TypeChecker::new();
    checker.check(&all_stmts)?;
    let ctx = checker.into_context();
    let mut lowerer = Lowerer::new(ctx);
    let hir = lowerer.lower(&all_stmts)?;
    let struct_field_types = lowerer.struct_types().clone();
    codegen::generate(output, &hir, struct_field_types, opt_level)
}

fn load_module(
    path: &Path,
    all_stmts: &mut Vec<Stmt>,
    visited: &mut HashSet<PathBuf>,
) -> Result<()> {
    if !visited.insert(path.to_path_buf()) {
        return Ok(()); // Already loaded
    }

    let source = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read module: {}", path.display()))?;

    let mut lexer = Lexer::new(&source);
    let tokens = lexer.tokenize()?;
    let mut parser = Parser::new(&tokens);
    let stmts = parser.parse_file()?;

    let current_dir = path.parent().unwrap_or_else(|| Path::new(""));

    for stmt in stmts {
        if let Stmt::Import(ref import_path, _) = stmt {
            let mut resolved_path = current_dir.join(import_path);
            if resolved_path.extension().is_none() {
                resolved_path.set_extension("rest");
            }
            if !resolved_path.exists() {
                // Try std directory relative to current working dir
                let std_path = Path::new("std").join(import_path).with_extension("rest");
                if std_path.exists() {
                    resolved_path = std_path;
                }
            }
            let abs_path = std::fs::canonicalize(&resolved_path)
                .unwrap_or_else(|_| resolved_path.clone());
            load_module(&abs_path, all_stmts, visited)?;
        } else {
            all_stmts.push(stmt);
        }
    }

    Ok(())
}
