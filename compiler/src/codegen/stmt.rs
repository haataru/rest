use super::*;
impl<'ctx> Codegen<'ctx> {
    pub(crate) fn compile_hir_stmt(
        &mut self,
        stmt: &HirStmt,
        struct_field_types: &StructFieldTypes,
    ) -> Result<()> {
        match stmt {
            HirStmt::Let {
                name,
                ty,
                init,
                owner,
                ..
            } => self.compile_let(name, ty, init, *owner, struct_field_types),
            HirStmt::Expr(expr, _) => {
                let result = self.compile_expr(expr, struct_field_types)?;
                if matches!(
                    expr,
                    HirExpr::AllocStruct(..)
                        | HirExpr::ArrayLiteral(..)
                        | HirExpr::Binary { .. }
                        | HirExpr::Call(..)
                ) && result.is_pointer_value()
                {
                    if self.type_of_expr(expr, struct_field_types) == Type::String {
                        self.builder
                            .build_call(
                                self.rest_release_string_fn,
                                &[result.into()],
                                "release_expr_tmp",
                            )
                            .unwrap();
                    } else {
                        let do_free = self
                            .builder
                            .build_call(self.release_fn, &[result.into()], "release_free_expr_tmp")
                            .unwrap()
                            .try_as_basic_value()
                            .basic()
                            .unwrap()
                            .into_int_value();
                        let do_free_bool = self
                            .builder
                            .build_int_compare(
                                inkwell::IntPredicate::NE,
                                do_free,
                                self.context.i32_type().const_zero(),
                                "do_free_bool",
                            )
                            .unwrap();
                        let then_bb = self.context.append_basic_block(
                            self.builder
                                .get_insert_block()
                                .unwrap()
                                .get_parent()
                                .unwrap(),
                            "free_free_expr_tmp_block",
                        );
                        let merge_bb = self.context.append_basic_block(
                            self.builder
                                .get_insert_block()
                                .unwrap()
                                .get_parent()
                                .unwrap(),
                            "merge_free_free_expr_tmp",
                        );
                        self.builder
                            .build_conditional_branch(do_free_bool, then_bb, merge_bb)
                            .unwrap();
                        self.builder.position_at_end(then_bb);
                        let _ = self.builder.build_call(
                            self.rest_free_fn,
                            &[result.into()],
                            "free_expr_tmp",
                        );
                        self.builder.build_unconditional_branch(merge_bb).unwrap();
                        self.builder.position_at_end(merge_bb);
                    }
                }
                Ok(())
            }
            HirStmt::Fn { .. } | HirStmt::ExternFn { .. } | HirStmt::Struct { .. } => Ok(()),
            HirStmt::If {
                cond, then, else_, ..
            } => self.compile_if(cond, then, else_.as_deref(), struct_field_types),
            HirStmt::While { cond, body, .. } => self.compile_while(cond, body, struct_field_types),
            HirStmt::For {
                var,
                var_ty,
                lo,
                hi,
                body,
                ..
            } => self.compile_for(var, var_ty, lo, hi, body, struct_field_types),
            HirStmt::Break(_) => {
                let &(_, break_bb, owner_depth) =
                    self.loop_stack.last().context("break outside loop")?;
                self.free_owners_since(owner_depth, struct_field_types);
                self.builder.build_unconditional_branch(break_bb)?;
                let fn_val = self.current_fn_val();
                let dead = self.context.append_basic_block(fn_val, "dead");
                self.builder.position_at_end(dead);
                Ok(())
            }
            HirStmt::Continue(_) => {
                let &(continue_bb, _, owner_depth) =
                    self.loop_stack.last().context("continue outside loop")?;
                self.free_owners_since(owner_depth, struct_field_types);
                self.builder.build_unconditional_branch(continue_bb)?;
                let fn_val = self.current_fn_val();
                let dead = self.context.append_basic_block(fn_val, "dead");
                self.builder.position_at_end(dead);
                Ok(())
            }
            HirStmt::Return(Some(v), _) => {
                if let HirExpr::Ident { name, .. } = v {
                    for scope in &mut self.owner_tracking {
                        scope.retain(|owner| match owner {
                            Owner::Struct(n, _) | Owner::Array(n, _, _) | Owner::String(n) => {
                                n != name
                            }
                        });
                    }
                }
                let val = self.compile_expr(v, struct_field_types)?;
                let ret_val = match v {
                    HirExpr::FieldLoad {
                        struct_name, index, ..
                    } => {
                        if let Some(fields) = struct_field_types.get(struct_name)
                            && let Some((_, field_type)) = fields.get(*index)
                        {
                            if *field_type == Type::String {
                                self.builder
                                    .build_call(
                                        self.rest_retain_string_fn,
                                        &[val.into()],
                                        "ret_field_retain",
                                    )?
                                    .try_as_basic_value()
                                    .basic()
                                    .expect("__rest_retain should return a basic value")
                            } else if matches!(field_type, Type::Struct(_)) {
                                self.deep_copy_loaded(val, &field_type, struct_field_types)?
                            } else {
                                val
                            }
                        } else {
                            val
                        }
                    }
                    _ => val,
                };
                self.free_owners_since(0, struct_field_types);
                self.builder.build_return(Some(&ret_val))?;
                Ok(())
            }
            HirStmt::Return(None, _) => {
                self.free_owners_since(0, struct_field_types);
                if self.current_fn_name == "main" {
                    let zero = self.context.i32_type().const_int(0, false);
                    self.builder.build_return(Some(&zero))?;
                } else {
                    self.builder.build_return(None)?;
                }
                Ok(())
            }
        }
    }
    pub(crate) fn compile_let(
        &mut self,
        name: &str,
        ty: &Type,
        init: &HirExpr,
        owner: bool,
        struct_field_types: &StructFieldTypes,
    ) -> Result<()> {
        if name.is_empty() {
            return Ok(());
        }
        let i8_ptr = self.ptr_ty();
        let llvm_decl_ty = self.hir_type_to_basic(ty, struct_field_types);
        let is_struct = matches!(ty, Type::Struct(_));
        let is_array_lit = matches!(init, HirExpr::ArrayLiteral(..));
        if owner && !matches!(init, HirExpr::Ident { .. }) {
            if let Type::Struct(struct_name) = ty {
                self.owner_scope_mut()
                    .push(Owner::Struct(name.to_string(), struct_name.clone()));
            } else {
                debug_assert!(
                    false,
                    "owner=true for non-Struct type {ty:?} — lowering bug"
                );
            }
        }
        let is_heap_alloc = is_struct || is_array_lit;
        let load_ty: BasicTypeEnum = if is_heap_alloc {
            i8_ptr.into()
        } else {
            llvm_decl_ty
        };
        let alloca = self.builder.build_alloca(load_ty, name)?;
        let is_string = matches!(ty, Type::String);
        match init {
            HirExpr::AllocStruct(struct_name, fields, _) => {
                let heap_ptr =
                    self.compile_alloc_struct(struct_name, fields, struct_field_types)?;
                self.builder.build_store(alloca, heap_ptr)?;
            }
            HirExpr::ArrayLiteral(ty, elements, _) => {
                let heap_ptr = self.compile_array_literal(ty, elements, struct_field_types)?;
                self.builder.build_store(alloca, heap_ptr)?;
                self.insert_array_info(name.to_string(), (*ty.clone(), elements.len()));
                self.owner_scope_mut().push(Owner::Array(
                    name.to_string(),
                    *ty.clone(),
                    elements.len(),
                ));
            }
            HirExpr::Ident { name: src_name, .. } => {
                if let Some(&(src_alloca, _)) = self.lookup_value(src_name) {
                    let src_is_owner = self.is_owner(src_name);
                    let loaded = self.builder.build_load(load_ty, src_alloca, src_name)?;
                    if is_string && src_is_owner {
                        let dup = self.builder.build_call(
                            self.rest_retain_string_fn,
                            &[loaded.into()],
                            "let_retain",
                        )?;
                        let dup_val = dup
                            .try_as_basic_value()
                            .basic()
                            .expect("__rest_retain should return a basic value");
                        self.builder.build_store(alloca, dup_val)?;
                    } else if is_struct && src_is_owner {
                        self.builder.build_store(alloca, loaded)?;
                        if let Type::Struct(struct_name) = ty {
                            self.transfer_ownership(src_name, name, struct_name);
                        }
                    } else {
                        self.builder.build_store(alloca, loaded)?;
                    }
                }
            }
            HirExpr::String(s, _) => {
                let val = self.compile_string_literal(s);
                self.builder.build_store(alloca, val)?;
            }
            _ => {
                let val = self.compile_expr(init, struct_field_types)?;
                let val = if is_string {
                    let dup = self.builder.build_call(
                        self.rest_retain_string_fn,
                        &[val.into()],
                        "let_retain",
                    )?;
                    dup.try_as_basic_value()
                        .basic()
                        .expect("__rest_retain should return a basic value")
                } else if let Type::Struct(_) = ty {
                    self.deep_copy_loaded(val, ty, struct_field_types)?
                } else {
                    val
                };
                self.builder.build_store(alloca, val)?;
            }
        }
        if is_string {
            self.owner_scope_mut().push(Owner::String(name.to_string()));
        }
        self.insert_var_type(name.to_string(), ty.clone());
        self.insert_value(name.to_string(), (alloca, load_ty));
        Ok(())
    }
    pub(crate) fn compile_if(
        &mut self,
        cond: &HirExpr,
        then_stmts: &[HirStmt],
        else_stmts: Option<&[HirStmt]>,
        struct_field_types: &StructFieldTypes,
    ) -> Result<()> {
        let cond_val = self.compile_expr(cond, struct_field_types)?;
        let i1 = self.bool_from_value(cond_val)?;
        let fn_val = self.current_fn_val();
        let then_bb = self.context.append_basic_block(fn_val, "then");
        let merge_bb = self.context.append_basic_block(fn_val, "ifcont");
        if let Some(else_stmts) = else_stmts {
            let else_bb = self.context.append_basic_block(fn_val, "else");
            self.builder
                .build_conditional_branch(i1, then_bb, else_bb)?;
            self.builder.position_at_end(then_bb);
            self.enter_scope();
            for stmt in then_stmts {
                self.compile_hir_stmt(stmt, struct_field_types)?;
            }
            self.exit_scope(struct_field_types)?;
            if self
                .builder
                .get_insert_block()
                .map(|bb| bb.get_terminator().is_none())
                .unwrap_or(false)
            {
                self.builder.build_unconditional_branch(merge_bb)?;
            }
            self.builder.position_at_end(else_bb);
            self.enter_scope();
            for stmt in else_stmts {
                self.compile_hir_stmt(stmt, struct_field_types)?;
            }
            self.exit_scope(struct_field_types)?;
            if self
                .builder
                .get_insert_block()
                .map(|bb| bb.get_terminator().is_none())
                .unwrap_or(false)
            {
                self.builder.build_unconditional_branch(merge_bb)?;
            }
        } else {
            self.builder
                .build_conditional_branch(i1, then_bb, merge_bb)?;
            self.builder.position_at_end(then_bb);
            self.enter_scope();
            for stmt in then_stmts {
                self.compile_hir_stmt(stmt, struct_field_types)?;
            }
            self.exit_scope(struct_field_types)?;
            if self
                .builder
                .get_insert_block()
                .map(|bb| bb.get_terminator().is_none())
                .unwrap_or(false)
            {
                self.builder.build_unconditional_branch(merge_bb)?;
            }
        }
        self.builder.position_at_end(merge_bb);
        Ok(())
    }
    pub(crate) fn compile_while(
        &mut self,
        cond: &HirExpr,
        body: &[HirStmt],
        struct_field_types: &StructFieldTypes,
    ) -> Result<()> {
        let fn_val = self.current_fn_val();
        let cond_bb = self.context.append_basic_block(fn_val, "while_cond");
        let body_bb = self.context.append_basic_block(fn_val, "while_body");
        let end_bb = self.context.append_basic_block(fn_val, "while_end");
        let owner_depth = self.owner_tracking.len();
        self.loop_stack.push((cond_bb, end_bb, owner_depth));
        self.builder.build_unconditional_branch(cond_bb)?;
        self.builder.position_at_end(cond_bb);
        let cond_val = self.compile_expr(cond, struct_field_types)?;
        let i1 = self.bool_from_value(cond_val)?;
        self.builder.build_conditional_branch(i1, body_bb, end_bb)?;
        self.builder.position_at_end(body_bb);
        self.enter_scope();
        for stmt in body {
            self.compile_hir_stmt(stmt, struct_field_types)?;
        }
        self.exit_scope(struct_field_types)?;
        let needs_back_edge = self
            .builder
            .get_insert_block()
            .map(|bb| bb.get_terminator().is_none())
            .unwrap_or(false);
        if needs_back_edge {
            self.builder.build_unconditional_branch(cond_bb)?;
        }
        self.loop_stack.pop();
        self.builder.position_at_end(end_bb);
        Ok(())
    }
    pub(crate) fn compile_for(
        &mut self,
        var: &str,
        var_ty: &Type,
        lo: &HirExpr,
        hi: &HirExpr,
        body: &[HirStmt],
        struct_field_types: &StructFieldTypes,
    ) -> Result<()> {
        let fn_val = self.current_fn_val();
        let lo_val = self.compile_expr(lo, struct_field_types)?;
        let hi_val = self.compile_expr(hi, struct_field_types)?;
        let llvm_ty = self.hir_type_to_basic(var_ty, struct_field_types);
        let int_ty = llvm_ty.into_int_type();
        let alloca = self.builder.build_alloca(int_ty, var)?;
        self.builder.build_store(alloca, lo_val)?;
        let cond_bb = self.context.append_basic_block(fn_val, "for_cond");
        let body_bb = self.context.append_basic_block(fn_val, "for_body");
        let inc_bb = self.context.append_basic_block(fn_val, "for_inc");
        let end_bb = self.context.append_basic_block(fn_val, "for_end");
        let owner_depth = self.owner_tracking.len();
        self.loop_stack.push((inc_bb, end_bb, owner_depth));
        self.builder.build_unconditional_branch(cond_bb)?;
        self.builder.position_at_end(cond_bb);
        let cur = self.builder.build_load(int_ty, alloca, var)?;
        let has_work = self.builder.build_int_compare(
            IntPredicate::SLT,
            cur.into_int_value(),
            hi_val.into_int_value(),
            "forcmp",
        )?;
        self.builder
            .build_conditional_branch(has_work, body_bb, end_bb)?;
        self.insert_value(var.to_string(), (alloca, llvm_ty));
        self.builder.position_at_end(body_bb);
        self.enter_scope();
        for stmt in body {
            self.compile_hir_stmt(stmt, struct_field_types)?;
        }
        self.exit_scope(struct_field_types)?;
        let needs_inc = self
            .builder
            .get_insert_block()
            .map(|bb| bb.get_terminator().is_none())
            .unwrap_or(false);
        if needs_inc {
            self.builder.build_unconditional_branch(inc_bb)?;
        }
        self.builder.position_at_end(inc_bb);
        let next = self
            .builder
            .build_load(int_ty, alloca, var)?
            .into_int_value();
        let inc = self
            .builder
            .build_int_add(next, int_ty.const_int(1, false), "inc")?;
        self.builder.build_store(alloca, inc)?;
        self.builder.build_unconditional_branch(cond_bb)?;
        self.loop_stack.pop();
        self.builder.position_at_end(end_bb);
        Ok(())
    }
    pub(crate) fn compile_assign(
        &mut self,
        lhs: &HirExpr,
        rhs: &HirExpr,
        struct_field_types: &StructFieldTypes,
    ) -> Result<BasicValueEnum<'ctx>> {
        match lhs {
            HirExpr::Ident { name, .. } => {
                if let Some(&(alloca, _)) = self.lookup_value(name) {
                    let val = self.compile_expr(rhs, struct_field_types)?;
                    let is_self_assign =
                        matches ! (rhs , HirExpr :: Ident { name : n , .. } if n == name);
                    if !is_self_assign {
                        let old_owners: Vec<Owner> = self
                            .owner_tracking
                            .iter()
                            .flat_map(|scope| scope.iter())
                            .filter(|owner| match owner {
                                Owner::Struct(n, _) | Owner::Array(n, _, _) | Owner::String(n) => {
                                    n == name
                                }
                            })
                            .cloned()
                            .collect();
                        for owner in &old_owners {
                            self.free_owner(owner, struct_field_types);
                        }
                        for scope in &mut self.owner_tracking {
                            scope.retain(|owner| match owner {
                                Owner::Struct(n, _) | Owner::Array(n, _, _) | Owner::String(n) => {
                                    n != name
                                }
                            });
                        }
                    }
                    self.builder.build_store(alloca, val)?;
                    if !is_self_assign {
                        if let HirExpr::Ident { name: src_name, .. } = rhs {
                            if self.is_owner(src_name) {
                                let src_owners: Vec<Owner> = self
                                    .owner_tracking
                                    .iter()
                                    .flat_map(|scope| scope.iter())
                                    .filter(|owner| match owner {
                                        Owner::Struct(n, _)
                                        | Owner::Array(n, _, _)
                                        | Owner::String(n) => n == src_name,
                                    })
                                    .cloned()
                                    .collect();
                                for src_owner in &src_owners {
                                    match src_owner {
                                        Owner::Struct(_, struct_name) => {
                                            self.owner_scope_mut().push(Owner::Struct(
                                                name.to_string(),
                                                struct_name.clone(),
                                            ));
                                        }
                                        Owner::Array(_, elem_ty, count) => {
                                            self.owner_scope_mut().push(Owner::Array(
                                                name.to_string(),
                                                elem_ty.clone(),
                                                *count,
                                            ));
                                        }
                                        Owner::String(_) => {
                                            let dup = self.builder.build_call(
                                                self.rest_retain_string_fn,
                                                &[val.into()],
                                                "assign_retain",
                                            )?;
                                            let dup_val = dup.try_as_basic_value().basic().expect(
                                                "__rest_retain should return a basic value",
                                            );
                                            self.builder.build_store(alloca, dup_val)?;
                                            self.owner_scope_mut()
                                                .push(Owner::String(name.to_string()));
                                        }
                                    }
                                }
                                if !src_owners.is_empty() {
                                    for scope in &mut self.owner_tracking {
                                        scope.retain(|owner| match owner {
                                            Owner::String(_) => true,
                                            Owner::Struct(n, _) | Owner::Array(n, _, _) => {
                                                n != src_name
                                            }
                                        });
                                    }
                                }
                            }
                        } else if let Some(ty) = self.lookup_var_type(name).cloned() {
                            match ty {
                                Type::Struct(n) => self
                                    .owner_scope_mut()
                                    .push(Owner::Struct(name.to_string(), n)),
                                Type::Array(e, c) => self.owner_scope_mut().push(Owner::Array(
                                    name.to_string(),
                                    *e,
                                    c,
                                )),
                                Type::String => {
                                    self.owner_scope_mut().push(Owner::String(name.to_string()))
                                }
                                _ => {}
                            }
                        }
                    }
                }
            }
            HirExpr::FieldLoad {
                object,
                index,
                struct_name,
                ..
            } => {
                let struct_ptr =
                    self.compile_field_load_ptr(object, struct_name, struct_field_types)?;
                let struct_ty = *self
                    .struct_types
                    .get(struct_name)
                    .ok_or_else(|| anyhow::anyhow!("undefined struct `{}`", struct_name))?;
                let gep = self.builder.build_struct_gep(
                    struct_ty,
                    struct_ptr,
                    *index as u32,
                    "field_set",
                )?;
                if let Some(fields) = struct_field_types.get(struct_name)
                    && let Some((_, field_type)) = fields.get(*index)
                {
                    if matches!(field_type, Type::String) {
                        if let Ok(old) = self.builder.build_load(self.ptr_ty(), gep, "old_field") {
                            if *field_type == Type::String {
                                self.builder
                                    .build_call(
                                        self.rest_release_string_fn,
                                        &[old.into()],
                                        "release_string_field",
                                    )
                                    .unwrap();
                            } else {
                                let do_free = self
                                    .builder
                                    .build_call(
                                        self.release_fn,
                                        &[old.into()],
                                        "release_free_old_field",
                                    )
                                    .unwrap()
                                    .try_as_basic_value()
                                    .basic()
                                    .unwrap()
                                    .into_int_value();
                                let do_free_bool = self
                                    .builder
                                    .build_int_compare(
                                        inkwell::IntPredicate::NE,
                                        do_free,
                                        self.context.i32_type().const_zero(),
                                        "do_free_bool",
                                    )
                                    .unwrap();
                                let then_bb = self.context.append_basic_block(
                                    self.builder
                                        .get_insert_block()
                                        .unwrap()
                                        .get_parent()
                                        .unwrap(),
                                    "free_free_old_field_block",
                                );
                                let merge_bb = self.context.append_basic_block(
                                    self.builder
                                        .get_insert_block()
                                        .unwrap()
                                        .get_parent()
                                        .unwrap(),
                                    "merge_free_free_old_field",
                                );
                                self.builder
                                    .build_conditional_branch(do_free_bool, then_bb, merge_bb)
                                    .unwrap();
                                self.builder.position_at_end(then_bb);
                                let _ = self.builder.build_call(
                                    self.rest_free_fn,
                                    &[old.into()],
                                    "free_old_field",
                                );
                                self.builder.build_unconditional_branch(merge_bb).unwrap();
                                self.builder.position_at_end(merge_bb);
                            }
                        }
                    } else if let Type::Struct(inner_name) = field_type
                        && let Ok(old) = self.builder.build_load(self.ptr_ty(), gep, "old_field")
                    {
                        self.free_struct_ptr(
                            old.into_pointer_value(),
                            &inner_name,
                            struct_field_types,
                        );
                    }
                }
                let val = self.compile_expr(rhs, struct_field_types)?;
                if let Some((_, field_type)) = struct_field_types
                    .get(struct_name)
                    .and_then(|f| f.get(*index))
                {
                    if matches!(field_type, Type::String) {
                        if matches!(
                            rhs,
                            HirExpr::Call(..) | HirExpr::Binary { op: BinOp::Add, .. }
                        ) {
                            self.builder.build_store(gep, val)?;
                        } else {
                            let dup = self.builder.build_call(
                                if *field_type == Type::String {
                                    self.rest_retain_string_fn
                                } else {
                                    self.retain_fn
                                },
                                &[val.into()],
                                "field_retain",
                            )?;
                            let dup_val = dup
                                .try_as_basic_value()
                                .basic()
                                .expect("__rest_retain should return a basic value");
                            self.builder.build_store(gep, dup_val)?;
                        }
                    } else if let Type::Struct(_) = field_type {
                        let copied = self.deep_copy_loaded(val, &field_type, struct_field_types)?;
                        if let HirExpr::Ident { name: src_name, .. } = rhs
                            && self.is_owner(src_name)
                        {
                            self.remove_struct_owner(src_name);
                        }
                        self.builder.build_store(gep, copied)?;
                    } else {
                        if let HirExpr::Ident { name: src_name, .. } = rhs
                            && self.is_owner(src_name)
                        {
                            self.remove_struct_owner(src_name);
                        }
                        self.builder.build_store(gep, val)?;
                    }
                } else {
                    self.builder.build_store(gep, val)?;
                }
            }
            HirExpr::ArrayIndex { object, index, .. } => {
                let idx_val = self.compile_expr(index, struct_field_types)?;
                if let HirExpr::Ident { name, .. } = object.as_ref()
                    && let Some(&(ptr_val, _)) = self.lookup_value(name)
                    && let Ok(arr_ptr) = self.builder.build_load(self.ptr_ty(), ptr_val, name)
                {
                    let arr_ptr = arr_ptr.into_pointer_value();
                    let arr_info: Option<(Type, usize)> = self
                        .lookup_array_info(name)
                        .map(|(t, l)| (t.clone(), *l))
                        .or_else(|| match self.lookup_var_type(name) {
                            Some(Type::Array(elem, len)) => Some((*elem.clone(), *len)),
                            _ => None,
                        });
                    let (elem_type, arr_len) = match arr_info {
                        Some((t, l)) => (Some(t), Some(l)),
                        None => (None, None),
                    };
                    if let Some(len) = arr_len {
                        self.bounds_check_array(idx_val.into_int_value(), len, "array_assign")?;
                    }
                    let gep = if let Some(ref elem_type) = elem_type {
                        let elem_llvm_ty = self.hir_type_to_basic(elem_type, struct_field_types);
                        self.builder
                            .build_pointer_cast(arr_ptr, self.ptr_ty(), "array_assign_typed")
                            .ok()
                            .and_then(|typed_ptr| unsafe {
                                self.builder
                                    .build_gep(
                                        elem_llvm_ty,
                                        typed_ptr,
                                        &[idx_val.into_int_value()],
                                        "array_assign_idx",
                                    )
                                    .ok()
                            })
                    } else {
                        unsafe {
                            self.builder
                                .build_gep(
                                    self.context.i8_type(),
                                    arr_ptr,
                                    &[idx_val.into_int_value()],
                                    "array_assign_idx",
                                )
                                .ok()
                        }
                    };
                    if let Some(gep) = gep {
                        if let Some(elem_type) = &elem_type
                            && (matches!(elem_type, Type::String)
                                || matches!(elem_type, Type::Struct(_)))
                            && let Ok(old) = self.builder.build_load(self.ptr_ty(), gep, "old_elem")
                        {
                            if let Type::Struct(sname) = elem_type {
                                self.free_struct_ptr(
                                    old.into_pointer_value(),
                                    sname,
                                    struct_field_types,
                                );
                            } else {
                                if *elem_type == Type::String {
                                    self.builder
                                        .build_call(
                                            self.rest_release_string_fn,
                                            &[old.into()],
                                            "release_string_elem",
                                        )
                                        .unwrap();
                                } else {
                                    let do_free = self
                                        .builder
                                        .build_call(
                                            self.release_fn,
                                            &[old.into()],
                                            "release_free_old_elem",
                                        )
                                        .unwrap()
                                        .try_as_basic_value()
                                        .basic()
                                        .unwrap()
                                        .into_int_value();
                                    let do_free_bool = self
                                        .builder
                                        .build_int_compare(
                                            inkwell::IntPredicate::NE,
                                            do_free,
                                            self.context.i32_type().const_zero(),
                                            "do_free_bool",
                                        )
                                        .unwrap();
                                    let then_bb = self.context.append_basic_block(
                                        self.builder
                                            .get_insert_block()
                                            .unwrap()
                                            .get_parent()
                                            .unwrap(),
                                        "free_free_old_elem_block",
                                    );
                                    let merge_bb = self.context.append_basic_block(
                                        self.builder
                                            .get_insert_block()
                                            .unwrap()
                                            .get_parent()
                                            .unwrap(),
                                        "merge_free_free_old_elem",
                                    );
                                    self.builder
                                        .build_conditional_branch(do_free_bool, then_bb, merge_bb)
                                        .unwrap();
                                    self.builder.position_at_end(then_bb);
                                    let _ = self.builder.build_call(
                                        self.rest_free_fn,
                                        &[old.into()],
                                        "free_old_elem",
                                    );
                                    self.builder.build_unconditional_branch(merge_bb).unwrap();
                                    self.builder.position_at_end(merge_bb);
                                };
                            }
                        }
                        let val = self.compile_expr(rhs, struct_field_types)?;
                        if let Some(elem_type) = &elem_type
                            && matches!(elem_type, Type::String)
                        {
                            if matches!(
                                rhs,
                                HirExpr::Call(..) | HirExpr::Binary { op: BinOp::Add, .. }
                            ) {
                                self.builder.build_store(gep, val)?;
                            } else {
                                let dup = self.builder.build_call(
                                    if *elem_type == Type::String {
                                        self.rest_retain_string_fn
                                    } else {
                                        self.retain_fn
                                    },
                                    &[val.into()],
                                    "array_elem_retain",
                                )?;
                                let dup_val = dup
                                    .try_as_basic_value()
                                    .basic()
                                    .expect("__rest_retain should return a basic value");
                                self.builder.build_store(gep, dup_val)?;
                            }
                        } else {
                            self.builder.build_store(gep, val)?;
                        }
                    }
                }
            }
            _ => {}
        }
        Ok(self.context.i32_type().const_zero().into())
    }
}
