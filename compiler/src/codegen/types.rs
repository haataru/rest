use super::*;
impl<'ctx> Codegen<'ctx> {
    pub(crate) fn lookup_var_type(&self, name: &str) -> Option<&Type> {
        for scope in self.var_types.iter().rev() {
            if let Some(t) = scope.get(name) {
                return Some(t);
            }
        }
        None
    }
    pub(crate) fn insert_var_type(&mut self, name: String, ty: Type) {
        self.var_types_mut().insert(name, ty);
    }
    pub(crate) fn var_types_mut(&mut self) -> &mut HashMap<String, Type> {
        self.var_types
            .last_mut()
            .expect("var_types: enter_scope() must be called before use")
    }
    pub(crate) fn build_struct_types(&mut self, stmts: &[HirStmt], struct_field_types: &StructFieldTypes) {
        for stmt in stmts {
            if let HirStmt::Struct { name, .. } = stmt {
                let fields = match struct_field_types.get(name) {
                    Some(f) => f.clone(),
                    None => continue,
                };
                let field_tys: Vec<BasicTypeEnum> = fields
                    .iter()
                    .map(|(_, ty)| self.hir_type_to_basic(ty, struct_field_types))
                    .collect();
                let struct_ty = self.context.opaque_struct_type(name);
                struct_ty.set_body(&field_tys, false);
                self.struct_types.insert(name.to_string(), struct_ty);
            }
        }
        for (name, fields) in struct_field_types {
            if !self.struct_types.contains_key(name) {
                let field_tys: Vec<BasicTypeEnum> = fields
                    .iter()
                    .map(|(_, ty)| self.hir_type_to_basic(ty, struct_field_types))
                    .collect();
                let struct_ty = self.context.opaque_struct_type(name);
                struct_ty.set_body(&field_tys, false);
                self.struct_types.insert(name.to_string(), struct_ty);
            }
        }
    }
    pub(crate) fn type_of_expr(&self, expr: &HirExpr, struct_field_types: &StructFieldTypes) -> Type {
        match expr {
            HirExpr::Int(..) => Type::I64,
            HirExpr::Float(..) => Type::F64,
            HirExpr::String(..) => Type::String,
            HirExpr::Bool(..) => Type::Bool,
            HirExpr::Ident { ty, .. } => ty.clone(),
            HirExpr::AllocStruct(name, _, _) => Type::Struct(name.clone()),
            HirExpr::FieldLoad {
                struct_name, index, ..
            } => {
                if let Some(fields) = struct_field_types.get(struct_name) {
                    if let Some((_, ty)) = fields.get(*index) {
                        return ty.clone();
                    }
                }
                Type::I64
            }
            HirExpr::ArrayIndex { object, .. } => {
                if let Type::Array(inner, _) = self.type_of_expr(object, struct_field_types) {
                    *inner
                } else {
                    Type::I64
                }
            }
            HirExpr::ArrayLiteral(ty, _, _) => Type::Array(ty.clone(), 0),
            HirExpr::Unary(..) => Type::I64,
            HirExpr::Binary { ty, .. } => ty.clone(),
            HirExpr::AddressOf(inner, _) => Type::Pointer(Box::new(self.type_of_expr(inner, struct_field_types))),
            HirExpr::Dereference(inner, _) => {
                if let Type::Pointer(t) = self.type_of_expr(inner, struct_field_types) {
                    *t
                } else {
                    Type::I64
                }
            }
            HirExpr::SizeOf(..) => Type::I64,
            HirExpr::Cast { target_ty, .. } => target_ty.clone(),
            _ => Type::I64,
        }
    }
    pub(crate) fn ptr_ty(&self) -> inkwell::types::PointerType<'ctx> {
        self.context.ptr_type(inkwell::AddressSpace::default())
    }
    #[allow(clippy::only_used_in_recursion)]
    pub(crate) fn hir_type_to_basic(
        &self,
        ty: &Type,
        struct_field_types: &StructFieldTypes,
    ) -> BasicTypeEnum<'ctx> {
        match ty {
            Type::I8 | Type::U8 => self.context.i8_type().into(),
            Type::I16 | Type::U16 => self.context.i16_type().into(),
            Type::I32 | Type::U32 => self.context.i32_type().into(),
            Type::I64 | Type::U64 => self.context.i64_type().into(),
            Type::F32 => self.context.f32_type().into(),
            Type::F64 => self.context.f64_type().into(),
            Type::Bool => self.context.bool_type().into(),
            Type::String => self.ptr_ty().into(),
            Type::Array(elem, n) => {
                let elem_ty = self.hir_type_to_basic(elem, struct_field_types);
                match elem_ty {
                    BasicTypeEnum::IntType(it) => it.array_type(*n as u32).into(),
                    BasicTypeEnum::FloatType(ft) => ft.array_type(*n as u32).into(),
                    BasicTypeEnum::PointerType(pt) => pt.array_type(*n as u32).into(),
                    BasicTypeEnum::StructType(st) => st.array_type(*n as u32).into(),
                    _ => self.context.i32_type().into(),
                }
            }
            Type::Struct(_) => self.ptr_ty().into(),
            Type::Pointer(_) => self.ptr_ty().into(),
            Type::Fn(..) => self.ptr_ty().into(),
            Type::Void => {
                debug_assert!(false, "Type::Void reached codegen — typeck bug");
                self.context.i32_type().into()
            }
        }
    }
}
