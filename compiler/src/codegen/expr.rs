use super::*;
impl<'ctx> Codegen<'ctx> {
    pub(crate) fn bool_from_value(
        &self,
        val: BasicValueEnum<'ctx>,
    ) -> Result<inkwell::values::IntValue<'ctx>> {
        if val.is_int_value() {
            let iv = val.into_int_value();
            let zero = iv.get_type().const_zero();
            Ok(self
                .builder
                .build_int_compare(IntPredicate::NE, iv, zero, "boolval")?)
        } else if val.is_float_value() {
            let fv = val.into_float_value();
            let zero = fv.get_type().const_zero();
            Ok(self
                .builder
                .build_float_compare(FloatPredicate::ONE, fv, zero, "boolval")?)
        } else if val.is_pointer_value() {
            let pv = val.into_pointer_value();
            Ok(self.builder.build_is_not_null(pv, "boolval")?)
        } else {
            anyhow::bail!("value cannot be used as boolean condition")
        }
    }
    pub(crate) fn compile_expr(
        &mut self,
        expr: &HirExpr,
        struct_field_types: &StructFieldTypes,
    ) -> Result<BasicValueEnum<'ctx>> {
        match expr {
            HirExpr::Int(v, ty, _) => {
                let basic = self.hir_type_to_basic(ty, struct_field_types);
                let result: BasicValueEnum = match basic {
                    BasicTypeEnum::IntType(it) => it.const_int(*v as u64, false).into(),
                    _ => self.context.i32_type().const_int(*v as u64, false).into(),
                };
                Ok(result)
            }
            HirExpr::Float(v, ty, _) => {
                let basic = self.hir_type_to_basic(ty, struct_field_types);
                let result: BasicValueEnum = match basic {
                    BasicTypeEnum::FloatType(ft) => ft.const_float(*v).into(),
                    _ => self.context.f64_type().const_float(*v).into(),
                };
                Ok(result)
            }
            HirExpr::Bool(v, _) => Ok(self
                .context
                .bool_type()
                .const_int(if *v { 1 } else { 0 }, false)
                .into()),
            HirExpr::String(s, _) => Ok(self.compile_string_literal(s)),
            HirExpr::Ident { name, .. } => self.compile_ident(name),
            HirExpr::AllocStruct(struct_name, fields, _) => {
                self.compile_alloc_struct(struct_name, fields, struct_field_types)
            }
            HirExpr::Call(callee, args, _) => self.compile_call(callee, args, struct_field_types),
            HirExpr::FieldLoad {
                object,
                index,
                struct_name,
                ..
            } => self.compile_field_load(object, *index, struct_name, struct_field_types),
            HirExpr::Unary(op, inner, _) => self.compile_unary(*op, inner, struct_field_types),
            HirExpr::ArrayIndex { object, index, .. } => {
                self.compile_array_index(object, index, struct_field_types)
            }
            HirExpr::ArrayLiteral(ty, elements, _) => {
                self.compile_array_literal(ty, elements, struct_field_types)
            }
            HirExpr::Binary {
                lhs, op, rhs, ty, ..
            } => self.compile_binary(lhs, *op, rhs, ty, struct_field_types),
            HirExpr::Assign { lhs, rhs, .. } => self.compile_assign(lhs, rhs, struct_field_types),
            HirExpr::AddressOf(inner, _) => self.compile_address_of(inner, struct_field_types),
            HirExpr::Dereference(inner, _) => self.compile_dereference(inner, struct_field_types),
            HirExpr::SizeOf(ty, _) => self.compile_sizeof(ty, struct_field_types),
            HirExpr::Cast { expr: inner, target_ty, .. } => self.compile_cast(inner, target_ty, struct_field_types),
            HirExpr::Print(arg, _) => self.compile_print(arg, struct_field_types),
        }
    }
    pub(crate) fn compile_unary(
        &mut self,
        op: UnOp,
        inner: &HirExpr,
        struct_field_types: &StructFieldTypes,
    ) -> Result<BasicValueEnum<'ctx>> {
        let val = self.compile_expr(inner, struct_field_types)?;
        match op {
            UnOp::Neg => {
                if val.is_float_value() {
                    let f = val.into_float_value();
                    Ok(self.builder.build_float_neg(f, "neg")?.into())
                } else {
                    let i = val.into_int_value();
                    Ok(self.builder.build_int_neg(i, "neg")?.into())
                }
            }
            UnOp::Not => {
                let i = val.into_int_value();
                Ok(self.builder.build_not(i, "not")?.into())
            }
        }
    }
    pub(crate) fn compile_string_literal(&self, s: &str) -> BasicValueEnum<'ctx> {
        let global = self
            .builder
            .build_global_string_ptr(s, "str")
            .expect("build_global_string_ptr failed");
        let ptr = global.as_pointer_value();
        let i8_ptr_ty = self.ptr_ty();
        let casted = self
            .builder
            .build_pointer_cast(ptr, i8_ptr_ty, "str_cast")
            .expect("build_pointer_cast to i8* failed");
        let len = self.context.i64_type().const_int(s.len() as u64, false);
        let alloc_ptr = self
            .builder
            .build_call(self.rest_alloc_string_fn, &[len.into()], "alloc_str")
            .unwrap()
            .try_as_basic_value()
            .basic()
            .unwrap()
            .into_pointer_value();
        let payload_ptr_ptr = unsafe {
            self.builder
                .build_in_bounds_gep(
                    self.context.i8_type(),
                    alloc_ptr,
                    &[self.context.i64_type().const_int(16, false)],
                    "payload_ptr_ptr",
                )
                .unwrap()
        };
        let i8_ptr_ptr_ty = i8_ptr_ty.ptr_type(inkwell::AddressSpace::default());
        let casted_ptr_ptr = self
            .builder
            .build_pointer_cast(payload_ptr_ptr, i8_ptr_ptr_ty, "cast_ptr_ptr")
            .unwrap();
        let payload_ptr = self
            .builder
            .build_load(i8_ptr_ty, casted_ptr_ptr, "payload_ptr")
            .unwrap()
            .into_pointer_value();
        self.builder
            .build_call(
                self.memcpy_fn,
                &[
                    payload_ptr.into(),
                    casted.into(),
                    len.into(),
                    self.context.bool_type().const_zero().into(),
                ],
                "memcpy",
            )
            .unwrap();
        alloc_ptr.into()
    }
    pub(crate) fn compile_ident(&mut self, name: &str) -> Result<BasicValueEnum<'ctx>> {
        let &(alloca, load_ty) = self
            .lookup_value(name)
            .ok_or_else(|| anyhow::anyhow!("undefined variable `{}`", name))?;
        let loaded = self.builder.build_load(load_ty, alloca, name)?;
        Ok(loaded)
    }
    pub(crate) fn compile_array_index(
        &mut self,
        object: &HirExpr,
        index: &HirExpr,
        struct_field_types: &StructFieldTypes,
    ) -> Result<BasicValueEnum<'ctx>> {
        let idx_val = self.compile_expr(index, struct_field_types)?;
        match object {
            HirExpr::Ident { name, .. } => {
                let &(ptr_val, _) = self
                    .lookup_value(name)
                    .ok_or_else(|| anyhow::anyhow!("undefined variable `{}`", name))?;
                let arr_ptr = self.builder.build_load(self.ptr_ty(), ptr_val, name)?;
                let arr_info: Option<(Type, usize)> = self
                    .lookup_array_info(name)
                    .map(|(t, l)| (t.clone(), *l))
                    .or_else(|| match self.lookup_var_type(name) {
                        Some(Type::Array(elem, len)) => Some((*elem.clone(), *len)),
                        _ => None,
                    });
                if let Some((elem_type, len)) = arr_info {
                    self.bounds_check_array(idx_val.into_int_value(), len, "array_idx_read")?;
                    let elem_llvm_ty = self.hir_type_to_basic(&elem_type, struct_field_types);
                    let typed_ptr = self.builder.build_pointer_cast(
                        arr_ptr.into_pointer_value(),
                        self.ptr_ty(),
                        "array_typed",
                    )?;
                    let gep = unsafe {
                        self.builder.build_gep(
                            elem_llvm_ty,
                            typed_ptr,
                            &[idx_val.into_int_value()],
                            "array_idx",
                        )?
                    };
                    let loaded = self.builder.build_load(elem_llvm_ty, gep, "array_elt")?;
                    let result = self.deep_copy_loaded(loaded, &elem_type, struct_field_types)?;
                    Ok(result)
                } else {
                    let ptr = arr_ptr.into_pointer_value();
                    let elem_ty = self.type_of_expr(object, struct_field_types);
                    let (elem_llvm_ty, target_ty) = match elem_ty {
                        Type::Pointer(inner) => (self.hir_type_to_basic(&inner, struct_field_types), *inner),
                        _ => (self.context.i8_type().into(), Type::I8),
                    };
                    let gep = unsafe {
                        self.builder.build_gep(
                            elem_llvm_ty,
                            ptr,
                            &[idx_val.into_int_value()],
                            "array_idx",
                        )?
                    };
                    let loaded = self.builder.build_load(elem_llvm_ty, gep, "array_elt")?;
                    Ok(loaded)
                }
            }
            _ => {
                if let HirExpr::ArrayLiteral(_, elements, _) = object {
                    self.bounds_check_array(
                        idx_val.into_int_value(),
                        elements.len(),
                        "array_idx_read",
                    )?;
                }
                let val = self.compile_expr(object, struct_field_types)?;
                let ptr = val.into_pointer_value();
                let (elem_llvm_ty, elem_type, use_deep_copy) = match object {
                    HirExpr::ArrayLiteral(ty, _, _) => {
                        let ty_ref: &Type = ty.as_ref();
                        let llvm = self.hir_type_to_basic(ty_ref, struct_field_types);
                        (
                            llvm,
                            Some(ty_ref.clone()),
                            matches!(ty_ref, Type::String | Type::Struct(_)),
                        )
                    }
                    HirExpr::AllocStruct(..) => (self.ptr_ty().into(), None, false),
                    _ => (self.context.i8_type().into(), None, false),
                };
                let typed_ptr =
                    self.builder
                        .build_pointer_cast(ptr, self.ptr_ty(), "array_idx_typed")?;
                let gep = unsafe {
                    self.builder.build_gep(
                        elem_llvm_ty,
                        typed_ptr,
                        &[idx_val.into_int_value()],
                        "array_idx",
                    )?
                };
                let loaded = self.builder.build_load(elem_llvm_ty, gep, "array_elt")?;
                let result = if let Some(et) = &elem_type
                    && use_deep_copy
                {
                    self.deep_copy_loaded(loaded, et, struct_field_types)?
                } else {
                    loaded
                };
                Ok(result)
            }
        }
    }
    pub(crate) fn compile_call(
        &mut self,
        callee: &str,
        args: &[HirExpr],
        struct_field_types: &StructFieldTypes,
    ) -> Result<BasicValueEnum<'ctx>> {
        let fn_val = self
            .functions
            .get(callee)
            .copied()
            .ok_or_else(|| anyhow::anyhow!("undefined function `{}`", callee))?;
        let mut llvm_args: Vec<BasicMetadataValueEnum> = Vec::new();
        let mut temp_allocs: Vec<BasicValueEnum<'ctx>> = Vec::new();
        for (i, arg) in args.iter().enumerate() {
            if let Some(param) = fn_val.get_nth_param(i as u32)
                && param.get_type().is_pointer_type()
                && let HirExpr::Ident { name, .. } = arg
                && let Some(&(alloca, _)) = self.lookup_value(name)
            {
                let loaded = self.builder.build_load(self.ptr_ty(), alloca, name)?;
                llvm_args.push(loaded.into());
                continue;
            }
            let val = self.compile_expr(arg, struct_field_types)?;
            if matches!(
                arg,
                HirExpr::AllocStruct(..)
                    | HirExpr::ArrayLiteral(..)
                    | HirExpr::Call(..)
                    | HirExpr::Binary { .. }
            ) && val.is_pointer_value()
            {
                temp_allocs.push(val);
            }
            llvm_args.push(val.into());
        }
        let call_site = self.builder.build_call(fn_val, &llvm_args, callee)?;
        for val in temp_allocs {
            {
                let do_free = self
                    .builder
                    .build_call(self.release_fn, &[val.into()], "release_free_temp_arg")
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
                    "free_free_temp_arg_block",
                );
                let merge_bb = self.context.append_basic_block(
                    self.builder
                        .get_insert_block()
                        .unwrap()
                        .get_parent()
                        .unwrap(),
                    "merge_free_free_temp_arg",
                );
                self.builder
                    .build_conditional_branch(do_free_bool, then_bb, merge_bb)
                    .unwrap();
                self.builder.position_at_end(then_bb);
                let _ = self
                    .builder
                    .build_call(self.rest_free_fn, &[val.into()], "free_temp_arg");
                self.builder.build_unconditional_branch(merge_bb).unwrap();
                self.builder.position_at_end(merge_bb);
            }
        }
        let ret = call_site
            .try_as_basic_value()
            .basic()
            .unwrap_or_else(|| self.context.i32_type().const_zero().into());
        Ok(ret)
    }
    pub(crate) fn compile_field_load_ptr(
        &mut self,
        object: &HirExpr,
        _struct_name: &str,
        struct_field_types: &StructFieldTypes,
    ) -> Result<PointerValue<'ctx>> {
        let struct_ptr = match object {
            HirExpr::Ident { name: n, .. } => {
                let &(ptr_val, _) = self
                    .lookup_value(n)
                    .ok_or_else(|| anyhow::anyhow!("undefined variable `{}`", n))?;
                let loaded = self.builder.build_load(self.ptr_ty(), ptr_val, n)?;
                loaded.into_pointer_value()
            }
            HirExpr::Dereference(inner, _) => {
                let val = self.compile_expr(inner, struct_field_types)?;
                val.into_pointer_value()
            }
            _ => {
                let val = self.compile_expr(object, struct_field_types)?;
                val.into_pointer_value()
            }
        };
        Ok(struct_ptr)
    }
    pub(crate) fn compile_field_load(
        &mut self,
        object: &HirExpr,
        index: usize,
        struct_name: &str,
        struct_field_types: &StructFieldTypes,
    ) -> Result<BasicValueEnum<'ctx>> {
        let struct_ptr = self.compile_field_load_ptr(object, struct_name, struct_field_types)?;
        let struct_ty = *self
            .struct_types
            .get(struct_name)
            .ok_or_else(|| anyhow::anyhow!("undefined struct `{}`", struct_name))?;
        let gep = self
            .builder
            .build_struct_gep(struct_ty, struct_ptr, index as u32, "field")?;
        let field_type = match struct_field_types
            .get(struct_name)
            .and_then(|f| f.get(index))
            .map(|(_, ty)| ty)
        {
            Some(ty) => ty,
            None => {
                debug_assert!(
                    false,
                    "struct `{struct_name}` has no field at index {index} — typeck bug"
                );
                return Ok(self.context.i32_type().const_zero().into());
            }
        };
        let load_ty = self.hir_type_to_basic(field_type, struct_field_types);
        let loaded = self.builder.build_load(load_ty, gep, "field_val")?;
        Ok(loaded)
    }
    pub(crate) fn is_unsigned_type(ty: &Type) -> bool {
        matches!(ty, Type::U8 | Type::U16 | Type::U32 | Type::U64)
    }
    #[doc = " LLVM requires shift amount to have the same bit width as the value."]
    #[doc = " Extend or truncate `rhs` to match `lhs`'s width (zero-extend, since shift amounts are unsigned)."]
    pub(crate) fn match_int_width(
        &self,
        lhs: inkwell::values::IntValue<'ctx>,
        rhs: inkwell::values::IntValue<'ctx>,
    ) -> inkwell::values::IntValue<'ctx> {
        let lhs_ty = lhs.get_type();
        let rhs_ty = rhs.get_type();
        let lhs_width = lhs_ty.get_bit_width();
        let rhs_width = rhs_ty.get_bit_width();
        if lhs_width == rhs_width {
            rhs
        } else if lhs_width > rhs_width {
            self.builder
                .build_int_z_extend(rhs, lhs_ty, "shift_ext")
                .expect("zext for shift amount should succeed")
        } else {
            self.builder
                .build_int_truncate(rhs, lhs_ty, "shift_trunc")
                .expect("trunc for shift amount should succeed")
        }
    }
    pub(crate) fn compile_binary(
        &mut self,
        lhs: &HirExpr,
        op: BinOp,
        rhs: &HirExpr,
        ty: &Type,
        struct_field_types: &StructFieldTypes,
    ) -> Result<BasicValueEnum<'ctx>> {
        let i32_ty = self.context.i32_type();
        if op == BinOp::And {
            let current_block = self
                .builder
                .get_insert_block()
                .expect("compile_binary &&: must be inside a function body");
            let current_fn = current_block
                .get_parent()
                .expect("basic block must belong to a function");
            let bool_ty = self.context.bool_type();
            let rhs_block = self.context.append_basic_block(current_fn, "and_rhs");
            let merge_block = self.context.append_basic_block(current_fn, "and_merge");
            let false_block = self.context.append_basic_block(current_fn, "and_false");
            let l = self.compile_expr(lhs, struct_field_types)?;
            let l_is_true = self.bool_from_value(l)?;
            self.builder
                .build_conditional_branch(l_is_true, rhs_block, false_block)?;
            self.builder.position_at_end(rhs_block);
            let r = self.compile_expr(rhs, struct_field_types)?;
            let r_is_true = self.bool_from_value(r)?;
            self.builder.build_unconditional_branch(merge_block)?;
            self.builder.position_at_end(false_block);
            self.builder.build_unconditional_branch(merge_block)?;
            self.builder.position_at_end(merge_block);
            let phi = self.builder.build_phi(bool_ty, "and_result")?;
            phi.add_incoming(&[
                (&r_is_true, rhs_block),
                (&bool_ty.const_zero(), false_block),
            ]);
            return Ok(phi.as_basic_value());
        }
        if op == BinOp::Or {
            let current_block = self
                .builder
                .get_insert_block()
                .expect("compile_binary ||: must be inside a function body");
            let current_fn = current_block
                .get_parent()
                .expect("basic block must belong to a function");
            let bool_ty = self.context.bool_type();
            let rhs_block = self.context.append_basic_block(current_fn, "or_rhs");
            let merge_block = self.context.append_basic_block(current_fn, "or_merge");
            let true_block = self.context.append_basic_block(current_fn, "or_true");
            let l = self.compile_expr(lhs, struct_field_types)?;
            let l_is_true = self.bool_from_value(l)?;
            self.builder
                .build_conditional_branch(l_is_true, true_block, rhs_block)?;
            self.builder.position_at_end(rhs_block);
            let r = self.compile_expr(rhs, struct_field_types)?;
            let r_is_true = self.bool_from_value(r)?;
            self.builder.build_unconditional_branch(merge_block)?;
            self.builder.position_at_end(true_block);
            self.builder.build_unconditional_branch(merge_block)?;
            self.builder.position_at_end(merge_block);
            let phi = self.builder.build_phi(bool_ty, "or_result")?;
            phi.add_incoming(&[
                (&r_is_true, rhs_block),
                (&bool_ty.const_int(1, false), true_block),
            ]);
            return Ok(phi.as_basic_value());
        }
        let l = self.compile_expr(lhs, struct_field_types)?;
        let r = self.compile_expr(rhs, struct_field_types)?;
        if op == BinOp::Add && l.is_pointer_value() && r.is_pointer_value() {
            let result =
                self.builder
                    .build_call(self.strcat_fn, &[l.into(), r.into()], "strcat")?;
            return Ok(result
                .try_as_basic_value()
                .basic()
                .expect("__ref_strcat should return a basic value"));
        }
        if (op == BinOp::Eq || op == BinOp::Ne) && l.is_pointer_value() && r.is_pointer_value() {
            if self.type_of_expr(lhs, struct_field_types) == Type::String {
                let is_eq = self
                    .builder
                    .build_call(self.rest_streq_fn, &[l.into(), r.into()], "streq")?
                    .try_as_basic_value()
                    .basic()
                    .unwrap()
                    .into_int_value();
                let res = if op == BinOp::Ne {
                    self.builder.build_not(is_eq, "strne")?
                } else {
                    is_eq
                };
                return Ok(res.into());
            } else {
                let ptr_cmp = self.builder.build_int_compare(
                    if op == BinOp::Eq {
                        inkwell::IntPredicate::EQ
                    } else {
                        inkwell::IntPredicate::NE
                    },
                    self.builder.build_ptr_to_int(
                        l.into_pointer_value(),
                        self.context.i64_type(),
                        "ptr2int_l",
                    )?,
                    self.builder.build_ptr_to_int(
                        r.into_pointer_value(),
                        self.context.i64_type(),
                        "ptr2int_r",
                    )?,
                    "ptr_cmp",
                )?;
                return Ok(ptr_cmp.into());
            }
        }
        let is_float = l.is_float_value();
        if is_float {
            let lf = l.into_float_value();
            let rf = r.into_float_value();
            let result: BasicValueEnum = match op {
                BinOp::Add => self.builder.build_float_add(lf, rf, "add")?.into(),
                BinOp::Sub => self.builder.build_float_sub(lf, rf, "sub")?.into(),
                BinOp::Mul => self.builder.build_float_mul(lf, rf, "mul")?.into(),
                BinOp::Div => self.builder.build_float_div(lf, rf, "div")?.into(),
                BinOp::Rem => self.builder.build_float_rem(lf, rf, "rem")?.into(),
                BinOp::Eq => {
                    let cmp =
                        self.builder
                            .build_float_compare(FloatPredicate::OEQ, lf, rf, "eq")?;
                    self.builder
                        .build_int_z_extend(cmp, i32_ty, "eq_ext")?
                        .into()
                }
                BinOp::Ne => {
                    let cmp =
                        self.builder
                            .build_float_compare(FloatPredicate::ONE, lf, rf, "ne")?;
                    self.builder
                        .build_int_z_extend(cmp, i32_ty, "ne_ext")?
                        .into()
                }
                BinOp::Lt => {
                    let cmp =
                        self.builder
                            .build_float_compare(FloatPredicate::OLT, lf, rf, "lt")?;
                    self.builder
                        .build_int_z_extend(cmp, i32_ty, "lt_ext")?
                        .into()
                }
                BinOp::Le => {
                    let cmp =
                        self.builder
                            .build_float_compare(FloatPredicate::OLE, lf, rf, "le")?;
                    self.builder
                        .build_int_z_extend(cmp, i32_ty, "le_ext")?
                        .into()
                }
                BinOp::Gt => {
                    let cmp =
                        self.builder
                            .build_float_compare(FloatPredicate::OGT, lf, rf, "gt")?;
                    self.builder
                        .build_int_z_extend(cmp, i32_ty, "gt_ext")?
                        .into()
                }
                BinOp::Ge => {
                    let cmp =
                        self.builder
                            .build_float_compare(FloatPredicate::OGE, lf, rf, "ge")?;
                    self.builder
                        .build_int_z_extend(cmp, i32_ty, "ge_ext")?
                        .into()
                }
                _ => anyhow::bail!("unsupported float operator {:?}", op),
            };
            Ok(result)
        } else {
            let li = l.into_int_value();
            let ri = r.into_int_value();
            let unsigned = Self::is_unsigned_type(ty);
            let result: BasicValueEnum = match op {
                BinOp::Add => self.builder.build_int_add(li, ri, "add")?.into(),
                BinOp::Sub => self.builder.build_int_sub(li, ri, "sub")?.into(),
                BinOp::Mul => self.builder.build_int_mul(li, ri, "mul")?.into(),
                BinOp::Div => {
                    let zero = ri.get_type().const_zero();
                    let is_zero = self.builder.build_int_compare(
                        IntPredicate::EQ,
                        ri,
                        zero,
                        "div_zero_check",
                    )?;
                    let current_fn = self.current_fn_val();
                    let cont_block = self.context.append_basic_block(current_fn, "div_cont");
                    let abort_block = self.context.append_basic_block(current_fn, "div_abort");
                    self.builder
                        .build_conditional_branch(is_zero, abort_block, cont_block)?;
                    self.builder.position_at_end(abort_block);
                    self.builder.build_call(self.abort_fn, &[], "div_abort")?;
                    self.builder.build_unreachable()?;
                    self.builder.position_at_end(cont_block);
                    if unsigned {
                        self.builder.build_int_unsigned_div(li, ri, "div")?.into()
                    } else {
                        self.builder.build_int_signed_div(li, ri, "div")?.into()
                    }
                }
                BinOp::Rem => {
                    let zero = ri.get_type().const_zero();
                    let is_zero = self.builder.build_int_compare(
                        IntPredicate::EQ,
                        ri,
                        zero,
                        "rem_zero_check",
                    )?;
                    let current_fn = self.current_fn_val();
                    let cont_block = self.context.append_basic_block(current_fn, "rem_cont");
                    let abort_block = self.context.append_basic_block(current_fn, "rem_abort");
                    self.builder
                        .build_conditional_branch(is_zero, abort_block, cont_block)?;
                    self.builder.position_at_end(abort_block);
                    self.builder.build_call(self.abort_fn, &[], "rem_abort")?;
                    self.builder.build_unreachable()?;
                    self.builder.position_at_end(cont_block);
                    if unsigned {
                        self.builder.build_int_unsigned_rem(li, ri, "rem")?.into()
                    } else {
                        self.builder.build_int_signed_rem(li, ri, "rem")?.into()
                    }
                }
                BinOp::Eq => {
                    let cmp = self
                        .builder
                        .build_int_compare(IntPredicate::EQ, li, ri, "eq")?;
                    self.builder
                        .build_int_z_extend(cmp, i32_ty, "eq_ext")?
                        .into()
                }
                BinOp::Ne => {
                    let cmp = self
                        .builder
                        .build_int_compare(IntPredicate::NE, li, ri, "ne")?;
                    self.builder
                        .build_int_z_extend(cmp, i32_ty, "ne_ext")?
                        .into()
                }
                BinOp::Lt => {
                    let pred = if unsigned {
                        IntPredicate::ULT
                    } else {
                        IntPredicate::SLT
                    };
                    let cmp = self.builder.build_int_compare(pred, li, ri, "lt")?;
                    self.builder
                        .build_int_z_extend(cmp, i32_ty, "lt_ext")?
                        .into()
                }
                BinOp::Le => {
                    let pred = if unsigned {
                        IntPredicate::ULE
                    } else {
                        IntPredicate::SLE
                    };
                    let cmp = self.builder.build_int_compare(pred, li, ri, "le")?;
                    self.builder
                        .build_int_z_extend(cmp, i32_ty, "le_ext")?
                        .into()
                }
                BinOp::Gt => {
                    let pred = if unsigned {
                        IntPredicate::UGT
                    } else {
                        IntPredicate::SGT
                    };
                    let cmp = self.builder.build_int_compare(pred, li, ri, "gt")?;
                    self.builder
                        .build_int_z_extend(cmp, i32_ty, "gt_ext")?
                        .into()
                }
                BinOp::Ge => {
                    let pred = if unsigned {
                        IntPredicate::UGE
                    } else {
                        IntPredicate::SGE
                    };
                    let cmp = self.builder.build_int_compare(pred, li, ri, "ge")?;
                    self.builder
                        .build_int_z_extend(cmp, i32_ty, "ge_ext")?
                        .into()
                }
                BinOp::BitAnd => self.builder.build_and(li, ri, "bitand")?.into(),
                BinOp::BitOr => self.builder.build_or(li, ri, "bitor")?.into(),
                BinOp::BitXor => self.builder.build_xor(li, ri, "xor")?.into(),
                BinOp::Shl => {
                    let ri = self.match_int_width(li, ri);
                    self.builder.build_left_shift(li, ri, "shl")?.into()
                }
                BinOp::Shr => {
                    let ri = self.match_int_width(li, ri);
                    self.builder
                        .build_right_shift(li, ri, unsigned, "shr")?
                        .into()
                }
                BinOp::And | BinOp::Or => {
                    debug_assert!(
                        false,
                        "And/Or reached compile_binary — short-circuit interception bug"
                    );
                    self.context.i32_type().const_zero().into()
                }
            };
            Ok(result)
        }
    }

    pub(crate) fn compile_lvalue_ptr(
        &mut self,
        expr: &HirExpr,
        struct_field_types: &StructFieldTypes,
    ) -> Result<PointerValue<'ctx>> {
        match expr {
            HirExpr::Ident { name, .. } => {
                let &(alloca, _) = self
                    .lookup_value(name)
                    .ok_or_else(|| anyhow::anyhow!("undefined variable `{}`", name))?;
                Ok(alloca)
            }
            HirExpr::FieldLoad {
                object,
                index,
                struct_name,
                ..
            } => {
                let struct_ptr =
                    self.compile_field_load_ptr(object, struct_name, struct_field_types)?;
                let struct_ty = *self.struct_types.get(struct_name).unwrap();
                let gep = self.builder.build_struct_gep(
                    struct_ty,
                    struct_ptr,
                    *index as u32,
                    "lvalue_field",
                )?;
                Ok(gep)
            }
            HirExpr::ArrayIndex { object, index, .. } => {
                let idx_val = self.compile_expr(index, struct_field_types)?;
                let arr_val = self.compile_expr(object, struct_field_types)?;
                let ptr = arr_val.into_pointer_value();
                let elem_ty = self.type_of_expr(object, struct_field_types);
                let elem_llvm_ty = match elem_ty {
                    Type::Array(inner, _) => self.hir_type_to_basic(&inner, struct_field_types),
                    Type::Pointer(inner) => self.hir_type_to_basic(&inner, struct_field_types),
                    _ => self.context.i8_type().into(),
                };
                let typed_ptr = self.builder.build_pointer_cast(ptr, self.ptr_ty(), "lvalue_array_typed")?;
                let gep = unsafe {
                    self.builder.build_gep(
                        elem_llvm_ty,
                        typed_ptr,
                        &[idx_val.into_int_value()],
                        "lvalue_array_idx",
                    )?
                };
                Ok(gep)
            }
            HirExpr::Dereference(inner, _) => {
                let ptr_val = self.compile_expr(inner, struct_field_types)?;
                Ok(ptr_val.into_pointer_value())
            }
            _ => anyhow::bail!("not an lvalue"),
        }
    }

    pub(crate) fn compile_address_of(
        &mut self,
        inner: &HirExpr,
        struct_field_types: &StructFieldTypes,
    ) -> Result<BasicValueEnum<'ctx>> {
        let ptr = self.compile_lvalue_ptr(inner, struct_field_types)?;
        Ok(ptr.into())
    }

    pub(crate) fn compile_dereference(
        &mut self,
        inner: &HirExpr,
        struct_field_types: &StructFieldTypes,
    ) -> Result<BasicValueEnum<'ctx>> {
        let ptr_val = self.compile_expr(inner, struct_field_types)?;
        let ptr = ptr_val.into_pointer_value();
        let inner_ty = self.type_of_expr(inner, struct_field_types);
        let target_ty = match inner_ty {
            Type::Pointer(t) => *t,
            _ => Type::I32,
        };
        let llvm_ty = self.hir_type_to_basic(&target_ty, struct_field_types);
        let loaded = self.builder.build_load(llvm_ty, ptr, "deref")?;
        Ok(loaded)
    }

    pub(crate) fn compile_sizeof(
        &mut self,
        ty: &Type,
        struct_field_types: &StructFieldTypes,
    ) -> Result<BasicValueEnum<'ctx>> {
        let llvm_ty = self.hir_type_to_basic(ty, struct_field_types);
        let size = llvm_ty.size_of().unwrap();
        Ok(size.into())
    }

    pub(crate) fn compile_cast(
        &mut self,
        inner: &HirExpr,
        target_ty: &Type,
        struct_field_types: &StructFieldTypes,
    ) -> Result<BasicValueEnum<'ctx>> {
        let val = self.compile_expr(inner, struct_field_types)?;
        let src_ty = self.type_of_expr(inner, struct_field_types);
        let llvm_target = self.hir_type_to_basic(target_ty, struct_field_types);

        if src_ty.is_integer() && target_ty.is_integer() {
            let src_int = val.into_int_value();
            let target_int = llvm_target.into_int_type();
            let src_width = src_int.get_type().get_bit_width();
            let target_width = target_int.get_bit_width();
            if src_width == target_width {
                return Ok(val);
            } else if src_width > target_width {
                return Ok(self
                    .builder
                    .build_int_truncate(src_int, target_int, "cast_trunc")?
                    .into());
            } else if Self::is_unsigned_type(&src_ty) {
                return Ok(self
                    .builder
                    .build_int_z_extend(src_int, target_int, "cast_zext")?
                    .into());
            } else {
                return Ok(self
                    .builder
                    .build_int_s_extend(src_int, target_int, "cast_sext")?
                    .into());
            }
        }

        if src_ty == Type::String && matches!(target_ty, Type::Pointer(_)) {
            let payload_ptr_ptr = unsafe {
                self.builder
                    .build_in_bounds_gep(
                        self.context.i8_type(),
                        val.into_pointer_value(),
                        &[self.context.i64_type().const_int(16, false)],
                        "str_payload_ptr_ptr",
                    )?
            };
            let i8_ptr_ptr_ty = self.ptr_ty().ptr_type(inkwell::AddressSpace::default());
            let casted_ptr_ptr = self.builder.build_pointer_cast(payload_ptr_ptr, i8_ptr_ptr_ty, "cast_ptr_ptr")?;
            let payload_ptr = self.builder.build_load(self.ptr_ty(), casted_ptr_ptr, "payload_ptr")?.into_pointer_value();
            return Ok(self
                .builder
                .build_pointer_cast(payload_ptr, llvm_target.into_pointer_type(), "cast_ptr")?
                .into());
        }

        if (matches!(src_ty, Type::Pointer(_)) || matches!(src_ty, Type::Struct(_))) && matches!(target_ty, Type::Pointer(_)) {
            return Ok(self
                .builder
                .build_pointer_cast(val.into_pointer_value(), llvm_target.into_pointer_type(), "cast_ptr")?
                .into());
        }

        if matches!(src_ty, Type::Pointer(_)) && target_ty.is_integer() {
            return Ok(self
                .builder
                .build_ptr_to_int(val.into_pointer_value(), llvm_target.into_int_type(), "cast_ptr2int")?
                .into());
        }

        if src_ty.is_integer() && matches!(target_ty, Type::Pointer(_)) {
            return Ok(self
                .builder
                .build_int_to_ptr(val.into_int_value(), llvm_target.into_pointer_type(), "cast_int2ptr")?
                .into());
        }

        Ok(val)
    }
}
