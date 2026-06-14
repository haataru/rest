use super::*;
impl<'ctx> Codegen<'ctx> {
    pub(crate) fn dup_string_expr(
        &mut self,
        expr: &HirExpr,
        field_type: &Type,
        struct_field_types: &StructFieldTypes,
    ) -> Result<Option<BasicValueEnum<'ctx>>> {
        if !matches!(field_type, Type::String) {
            return Ok(None);
        }
        let ptr_val = match expr {
            HirExpr::String(s, _) => self.compile_string_literal(s),
            _ => self.compile_expr(expr, struct_field_types)?,
        };
        let result = self
            .builder
            .build_call(
                if self.type_of_expr(expr, struct_field_types) == Type::String {
                    self.rest_retain_string_fn
                } else {
                    self.retain_fn
                },
                &[ptr_val.into()],
                "retain",
            )?
            .try_as_basic_value();
        let bv = result
            .basic()
            .expect("__rest_retain should return a basic value");
        Ok(Some(bv))
    }
    pub(crate) fn deep_copy_loaded(
        &mut self,
        val: BasicValueEnum<'ctx>,
        elem_type: &Type,
        struct_field_types: &StructFieldTypes,
    ) -> Result<BasicValueEnum<'ctx>> {
        match elem_type {
            Type::String => {
                let result = self
                    .builder
                    .build_call(
                        if *elem_type == Type::String {
                            self.rest_retain_string_fn
                        } else {
                            self.retain_fn
                        },
                        &[val.into()],
                        "arr_retain",
                    )?
                    .try_as_basic_value()
                    .basic()
                    .expect("__rest_retain should return a basic value");
                Ok(result)
            }
            Type::Struct(struct_name) => {
                let struct_ty = *self
                    .struct_types
                    .get(struct_name)
                    .ok_or_else(|| anyhow::anyhow!("undefined struct `{}`", struct_name))?;
                let size = struct_ty
                    .size_of()
                    .unwrap_or_else(|| self.context.i64_type().const_int(8, false));
                let malloc_args = &[size.into()];
                let heap_ptr = self
                    .builder
                    .build_call(self.rest_alloc_fn, malloc_args, "struct_copy_malloc")?
                    .try_as_basic_value()
                    .basic()
                    .expect("malloc returns basic value");
                let ptr = heap_ptr.into_pointer_value();
                let src_ptr = val.into_pointer_value();
                self.builder.build_call(
                    self.memcpy_fn,
                    &[
                        ptr.into(),
                        src_ptr.into(),
                        size.into(),
                        self.context.bool_type().const_zero().into(),
                    ],
                    "struct_copy_memcpy",
                )?;
                if let Some(fields) = struct_field_types.get(struct_name) {
                    for (i, (_, ty)) in fields.iter().enumerate() {
                        if let Ok(gep) =
                            self.builder
                                .build_struct_gep(struct_ty, ptr, i as u32, "copy_field")
                            && let Ok(field_val) =
                                self.builder
                                    .build_load(self.ptr_ty(), gep, "copy_field_val")
                        {
                            if matches!(ty, Type::String) {
                                let dup = self
                                    .builder
                                    .build_call(
                                        self.rest_retain_string_fn,
                                        &[field_val.into()],
                                        "copy_field_retain",
                                    )?
                                    .try_as_basic_value()
                                    .basic()
                                    .expect("__rest_retain should return a basic value");
                                self.builder.build_store(gep, dup)?;
                            } else if let Type::Struct(..) = ty {
                                let inner_copy =
                                    self.deep_copy_loaded(field_val, &ty, struct_field_types)?;
                                self.builder.build_store(gep, inner_copy)?;
                            }
                        }
                    }
                }
                Ok(heap_ptr)
            }
            _ => Ok(val),
        }
    }
    pub(crate) fn compile_alloc_struct(
        &mut self,
        struct_name: &str,
        fields: &[(String, HirExpr)],
        struct_field_types: &StructFieldTypes,
    ) -> Result<BasicValueEnum<'ctx>> {
        let struct_ty = *self
            .struct_types
            .get(struct_name)
            .ok_or_else(|| anyhow::anyhow!("undefined struct `{}`", struct_name))?;
        let size_val = struct_ty
            .size_of()
            .unwrap_or_else(|| self.context.i64_type().const_int(8, false));
        let malloc_args = &[size_val.into()];
        let heap_ptr = self
            .builder
            .build_call(self.rest_alloc_fn, malloc_args, "malloc")?
            .try_as_basic_value()
            .basic()
            .expect("malloc should return a basic pointer value");
        let struct_field_list = struct_field_types.get(struct_name);
        for (i, (field_name, field_val)) in fields.iter().enumerate() {
            let gep = self.builder.build_struct_gep(
                struct_ty,
                heap_ptr.into_pointer_value(),
                i as u32,
                field_name,
            )?;
            let field_type = match struct_field_list.and_then(|f| f.get(i)).map(|(_, ty)| ty) {
                Some(ty) => ty.clone(),
                None => {
                    debug_assert!(
                        false,
                        "struct `{struct_name}` has no field at index {i} — typeck bug"
                    );
                    continue;
                }
            };
            let dup = self.dup_string_expr(field_val, &field_type, struct_field_types)?;
            let val = if let Some(d) = dup {
                d
            } else {
                let v = self.compile_expr(field_val, struct_field_types)?;
                if let Type::Struct(_) = field_type
                    && let HirExpr::Ident { name: src_name, .. } = field_val
                    && self.is_owner(src_name)
                {
                    self.remove_struct_owner(src_name);
                }
                v
            };
            self.builder.build_store(gep, val)?;
        }
        Ok(heap_ptr)
    }
    pub(crate) fn compile_array_literal(
        &mut self,
        ty: &Type,
        elements: &[HirExpr],
        struct_field_types: &StructFieldTypes,
    ) -> Result<BasicValueEnum<'ctx>> {
        let elem_ty = self.hir_type_to_basic(ty, struct_field_types);
        let elem_size = elem_ty
            .size_of()
            .unwrap_or_else(|| self.context.i64_type().const_int(8, false));
        let count_val = self
            .context
            .i64_type()
            .const_int(elements.len() as u64, false);
        let total_size = self
            .builder
            .build_int_mul(elem_size, count_val, "array_total_size")?;
        let malloc_args = &[total_size.into()];
        let heap_ptr = self
            .builder
            .build_call(self.rest_alloc_fn, malloc_args, "array_malloc")?
            .try_as_basic_value()
            .basic()
            .expect("malloc should return a basic pointer value")
            .into_pointer_value();
        let typed_ptr = self
            .builder
            .build_pointer_cast(heap_ptr, self.ptr_ty(), "array_typed")?;
        for (i, elem) in elements.iter().enumerate() {
            let gep = unsafe {
                self.builder.build_gep(
                    elem_ty,
                    typed_ptr,
                    &[self.context.i32_type().const_int(i as u64, false)],
                    "array_elt",
                )?
            };
            let dup = self.dup_string_expr(elem, ty, struct_field_types)?;
            let val = if let Some(d) = dup {
                d
            } else {
                let v = self.compile_expr(elem, struct_field_types)?;
                if let Type::Struct(_) = ty
                    && let HirExpr::Ident { name: src_name, .. } = elem
                    && self.is_owner(src_name)
                {
                    self.remove_struct_owner(src_name);
                }
                v
            };
            self.builder.build_store(gep, val)?;
        }
        Ok(heap_ptr.into())
    }
}
