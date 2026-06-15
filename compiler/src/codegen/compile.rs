use crate::ir::{HirExpr, HirStmt};
use crate::ops::{BinOp, UnOp};
use crate::sema::Type;
use anyhow::{Context as _, Result, bail};
use inkwell::FloatPredicate;
use inkwell::IntPredicate;
use inkwell::OptimizationLevel;
use inkwell::basic_block::BasicBlock;
use inkwell::builder::Builder;
use inkwell::context::Context;
use inkwell::module::Module;
use inkwell::targets::{
    CodeModel, FileType, InitializationConfig, RelocMode, Target, TargetMachine,
};
use inkwell::types::{BasicMetadataTypeEnum, BasicType, BasicTypeEnum, StructType};
use inkwell::values::{
    BasicMetadataValueEnum, BasicValueEnum, FunctionValue, IntValue, PointerValue,
};
use std::collections::HashMap;
use std::path::Path;
use std::sync::Once;
pub(crate) type StructFieldTypes = HashMap<String, Vec<(String, Type)>>;
#[derive(Debug, Clone)]
pub(crate) enum Owner {
    Struct(String, String),
    Array(String, Type, usize),
    String(String),
}
pub(crate) struct Codegen<'ctx> {
    pub(crate) context: &'ctx Context,
    pub(crate) module: Module<'ctx>,
    pub(crate) builder: Builder<'ctx>,
    pub(crate) current_fn: Option<FunctionValue<'ctx>>,
    pub(crate) current_fn_name: String,
    pub(crate) values: Vec<HashMap<String, (PointerValue<'ctx>, BasicTypeEnum<'ctx>)>>,
    pub(crate) globals: HashMap<String, (PointerValue<'ctx>, BasicTypeEnum<'ctx>)>,
    pub(crate) struct_types: HashMap<String, StructType<'ctx>>,
    pub(crate) printf_fn: FunctionValue<'ctx>,
    pub(crate) malloc_fn: FunctionValue<'ctx>,
    pub(crate) free_fn: FunctionValue<'ctx>,
    pub(crate) memcpy_fn: FunctionValue<'ctx>,
    pub(crate) retain_fn: FunctionValue<'ctx>,
    pub(crate) release_fn: FunctionValue<'ctx>,
    pub(crate) rest_alloc_fn: FunctionValue<'ctx>,
    pub(crate) rest_free_fn: FunctionValue<'ctx>,
    pub(crate) strcat_fn: FunctionValue<'ctx>,
    pub(crate) abort_fn: FunctionValue<'ctx>,
    pub(crate) rest_alloc_string_fn: FunctionValue<'ctx>,
    pub(crate) rest_retain_string_fn: FunctionValue<'ctx>,
    pub(crate) rest_release_string_fn: FunctionValue<'ctx>,
    pub(crate) rest_print_string_fn: FunctionValue<'ctx>,
    pub(crate) rest_streq_fn: FunctionValue<'ctx>,
    pub(crate) rest_register_route_fn: Option<FunctionValue<'ctx>>,
    pub(crate) rest_start_server_fn: Option<FunctionValue<'ctx>>,
    pub(crate) functions: HashMap<String, FunctionValue<'ctx>>,
    pub(crate) routes: Vec<(String, String, FunctionValue<'ctx>)>,
    pub(crate) loop_stack: Vec<(BasicBlock<'ctx>, BasicBlock<'ctx>, usize)>,
    pub(crate) owner_tracking: Vec<Vec<Owner>>,
    pub(crate) array_info: Vec<HashMap<String, (Type, usize)>>,
    pub(crate) var_types: Vec<HashMap<String, Type>>,
    pub(crate) runtime_compiled: bool,
    pub(crate) has_routes: bool,
}

impl<'ctx> Codegen<'ctx> {
    pub fn new(context: &'ctx Context, name: &str) -> Self {
        let module = context.create_module(name);
        let builder = context.create_builder();
        let i8_ptr = context.ptr_type(inkwell::AddressSpace::default());
        let printf_type = context.i32_type().fn_type(&[i8_ptr.into()], true);
        let printf_fn = module.add_function("printf", printf_type, None);
        let malloc_type = i8_ptr.fn_type(&[context.i64_type().into()], false);
        let malloc_fn = module.add_function("malloc", malloc_type, None);
        let free_type = context.void_type().fn_type(&[i8_ptr.into()], false);
        let free_fn = module.add_function("free", free_type, None);
        let memcpy_type = context.void_type().fn_type(
            &[
                i8_ptr.into(),
                i8_ptr.into(),
                context.i64_type().into(),
                context.bool_type().into(),
            ],
            false,
        );
        let memcpy_fn = module.add_function("llvm.memcpy.p0.p0.i64", memcpy_type, None);
        let retain_type = i8_ptr.fn_type(&[i8_ptr.into()], false);
        let retain_fn = module.add_function("__rest_retain", retain_type, None);
        let release_type = context.i32_type().fn_type(&[i8_ptr.into()], false);
        let release_fn = module.add_function("__rest_release", release_type, None);
        let rest_alloc_fn = module.add_function("__rest_alloc", malloc_type, None);
        let rest_free_fn = module.add_function("__rest_free", free_type, None);
        let strcat_type = i8_ptr.fn_type(&[i8_ptr.into(), i8_ptr.into()], false);
        let strcat_fn = module.add_function("rest_strcat", strcat_type, None);
        let abort_type = context.void_type().fn_type(&[], false);
        let abort_fn = module.add_function("abort", abort_type, None);
        let noreturn_kind = inkwell::attributes::Attribute::get_named_enum_kind_id("noreturn");
        let noreturn_attr = context.create_enum_attribute(noreturn_kind, 0);
        let noreturn_attr = context.create_enum_attribute(noreturn_kind, 0);
        abort_fn.add_attribute(inkwell::attributes::AttributeLoc::Function, noreturn_attr);
        let rest_alloc_string_type = i8_ptr.fn_type(&[context.i64_type().into()], false);
        let rest_alloc_string_fn =
            module.add_function("rest_alloc_string", rest_alloc_string_type, None);
        let rest_retain_string_type = context.void_type().fn_type(&[i8_ptr.into()], false);
        let rest_retain_string_fn =
            module.add_function("rest_retain_string", rest_retain_string_type, None);
        let rest_release_string_type = context.void_type().fn_type(&[i8_ptr.into()], false);
        let rest_release_string_fn =
            module.add_function("rest_release_string", rest_release_string_type, None);
        let rest_print_string_type = context.void_type().fn_type(&[i8_ptr.into()], false);
        let rest_print_string_fn =
            module.add_function("rest_print_string", rest_print_string_type, None);
        let rest_streq_type = context
            .bool_type()
            .fn_type(&[i8_ptr.into(), i8_ptr.into()], false);
        let rest_streq_fn = module.add_function("rest_streq", rest_streq_type, None);
        Self {
            context,
            module,
            builder,
    current_fn: None,
    current_fn_name: String::new(),
    values: vec![HashMap::new()],
    globals: HashMap::new(),
    struct_types: HashMap::new(),
            printf_fn,
            malloc_fn,
            free_fn,
            memcpy_fn,
            retain_fn,
            release_fn,
            rest_alloc_fn,
            rest_free_fn,
            strcat_fn,
            abort_fn,
            rest_alloc_string_fn,
            rest_retain_string_fn,
            rest_release_string_fn,
            rest_print_string_fn,
            rest_streq_fn,
    rest_register_route_fn: None,
    rest_start_server_fn: None,
    functions: HashMap::new(),
            routes: Vec::new(),
    loop_stack: Vec::new(),
    owner_tracking: vec![Vec::new()],
            array_info: vec![HashMap::new()],
    var_types: vec![HashMap::new()],
    runtime_compiled: false,
    has_routes: false,
        }
    }
    pub fn generate_ir(&self) -> String {
        self.module.print_to_string().to_string()
    }
    pub fn write_bitcode(&self, path: &Path) -> Result<()> {
        if !self.module.write_bitcode_to_path(path) {
            bail!("failed to write bitcode to {}", path.display());
        }
        Ok(())
    }
    pub fn emit_object(&self, path: &Path, opt_level: OptimizationLevel) -> Result<()> {
        static LLVM_INIT: Once = Once::new();
        LLVM_INIT.call_once(|| {
            Target::initialize_native(&InitializationConfig::default())
                .expect("LLVM native target initialization failed");
        });
        let triple = TargetMachine::get_default_triple();
        let target = Target::from_triple(&triple)?;
        let machine = target
            .create_target_machine(
                &triple,
                "generic",
                "",
                opt_level,
                RelocMode::Default,
                CodeModel::Default,
            )
            .context("failed to create target machine")?;
        machine
            .write_to_file(&self.module, FileType::Object, path)
            .map_err(|e| anyhow::anyhow!(e))?;
        Ok(())
    }
    pub(crate) fn compile_runtime_fns(&mut self) -> Result<()> {
        if self.runtime_compiled {
            return Ok(());
        }
        let i64_ty = self.context.i64_type();
        let i32_ptr = self
            .context
            .i32_type()
            .ptr_type(inkwell::AddressSpace::default());
        let bool_ty = self.context.bool_type();
        let entry = self.context.append_basic_block(self.rest_alloc_fn, "entry");
        self.builder.position_at_end(entry);
        let size = self
            .rest_alloc_fn
            .get_nth_param(0)
            .unwrap()
            .into_int_value();
        let total_size =
            self.builder
                .build_int_add(size, i64_ty.const_int(4, false), "total_size")?;
        let ptr = self
            .builder
            .build_call(self.malloc_fn, &[total_size.into()], "malloc")?
            .try_as_basic_value()
            .basic()
            .unwrap()
            .into_pointer_value();
        let rc_ptr = self.builder.build_pointer_cast(ptr, i32_ptr, "rc_ptr")?;
        self.builder
            .build_store(rc_ptr, self.context.i32_type().const_int(1, false))?;
        let data_ptr = unsafe {
            self.builder.build_in_bounds_gep(
                self.context.i8_type(),
                ptr,
                &[i64_ty.const_int(4, false)],
                "data_ptr",
            )?
        };
        self.builder.build_return(Some(&data_ptr))?;
        let entry = self.context.append_basic_block(self.retain_fn, "entry");
        self.builder.position_at_end(entry);
        let s = self
            .retain_fn
            .get_nth_param(0)
            .unwrap()
            .into_pointer_value();
        let null_bb = self.context.append_basic_block(self.retain_fn, "null");
        let not_null_bb = self.context.append_basic_block(self.retain_fn, "not_null");
        let is_null = self.builder.build_is_null(s, "is_null")?;
        self.builder
            .build_conditional_branch(is_null, null_bb, not_null_bb)?;
        self.builder.position_at_end(null_bb);
        self.builder.build_return(Some(&s))?;
        self.builder.position_at_end(not_null_bb);
        let rc_ptr_i8 = unsafe {
            self.builder.build_in_bounds_gep(
                self.context.i8_type(),
                s,
                &[i64_ty.const_int(0xFFFFFFFFFFFFFFFC, true)],
                "rc_ptr_i8",
            )?
        };
        let rc_ptr = self
            .builder
            .build_pointer_cast(rc_ptr_i8, i32_ptr, "rc_ptr")?;
        self.builder.build_atomicrmw(
            inkwell::AtomicRMWBinOp::Add,
            rc_ptr,
            self.context.i32_type().const_int(1, false),
            inkwell::AtomicOrdering::SequentiallyConsistent,
        )?;
        self.builder.build_return(Some(&s))?;
        let entry = self.context.append_basic_block(self.release_fn, "entry");
        self.builder.position_at_end(entry);
        let s = self
            .release_fn
            .get_nth_param(0)
            .unwrap()
            .into_pointer_value();
        let null_bb = self.context.append_basic_block(self.release_fn, "null");
        let not_null_bb = self.context.append_basic_block(self.release_fn, "not_null");
        let is_null = self.builder.build_is_null(s, "is_null")?;
        self.builder
            .build_conditional_branch(is_null, null_bb, not_null_bb)?;
        self.builder.position_at_end(null_bb);
        self.builder
            .build_return(Some(&self.context.i32_type().const_zero()))?;
        self.builder.position_at_end(not_null_bb);
        let rc_ptr_i8 = unsafe {
            self.builder.build_in_bounds_gep(
                self.context.i8_type(),
                s,
                &[i64_ty.const_int(0xFFFFFFFFFFFFFFFC, true)],
                "rc_ptr_i8",
            )?
        };
        let rc_ptr = self
            .builder
            .build_pointer_cast(rc_ptr_i8, i32_ptr, "rc_ptr")?;
        let old_rc = self.builder.build_atomicrmw(
            inkwell::AtomicRMWBinOp::Sub,
            rc_ptr,
            self.context.i32_type().const_int(1, false),
            inkwell::AtomicOrdering::SequentiallyConsistent,
        )?;
        let is_one = self.builder.build_int_compare(
            inkwell::IntPredicate::EQ,
            old_rc,
            self.context.i32_type().const_int(1, false),
            "is_one",
        )?;
        let i32_is_one =
            self.builder
                .build_int_z_extend(is_one, self.context.i32_type(), "i32_is_one")?;
        self.builder.build_return(Some(&i32_is_one))?;
        let entry = self.context.append_basic_block(self.rest_free_fn, "entry");
        self.builder.position_at_end(entry);
        let s = self
            .rest_free_fn
            .get_nth_param(0)
            .unwrap()
            .into_pointer_value();
        let null_bb = self.context.append_basic_block(self.rest_free_fn, "null");
        let not_null_bb = self
            .context
            .append_basic_block(self.rest_free_fn, "not_null");
        let is_null = self.builder.build_is_null(s, "is_null")?;
        self.builder
            .build_conditional_branch(is_null, null_bb, not_null_bb)?;
        self.builder.position_at_end(null_bb);
        self.builder.build_return(None)?;
        self.builder.position_at_end(not_null_bb);
        let real_ptr = unsafe {
            self.builder.build_in_bounds_gep(
                self.context.i8_type(),
                s,
                &[i64_ty.const_int(0xFFFFFFFFFFFFFFFC, true)],
                "real_ptr",
            )?
        };
        self.builder
            .build_call(self.free_fn, &[real_ptr.into()], "free")?;
        self.builder.build_return(None)?;
        let entry = self.context.append_basic_block(self.rest_retain_string_fn, "entry");
        self.builder.position_at_end(entry);
        self.builder.build_return(None)?;

        let entry = self.context.append_basic_block(self.rest_release_string_fn, "entry");
        self.builder.position_at_end(entry);
        self.builder.build_return(None)?;

        let entry = self.context.append_basic_block(self.rest_print_string_fn, "entry");
        self.builder.position_at_end(entry);
        let print_s = self.rest_print_string_fn.get_nth_param(0).unwrap().into_pointer_value();
        let data_field_ptr = unsafe { self.builder.build_in_bounds_gep(self.context.i8_type(), print_s, &[self.context.i64_type().const_int(16, false)], "data_field_ptr")? };
        let i8_ptr_ty = self.context.i8_type().ptr_type(inkwell::AddressSpace::default());
        let data_field_ptr_casted = self.builder.build_pointer_cast(data_field_ptr, i8_ptr_ty.ptr_type(inkwell::AddressSpace::default()), "data_field_ptr_casted")?;
        let data_ptr = self.builder.build_load(i8_ptr_ty, data_field_ptr_casted, "data_ptr")?;
        self.builder.build_call(self.printf_fn, &[data_ptr.into()], "printf")?;
        self.builder.build_return(None)?;

        let entry = self.context.append_basic_block(self.rest_streq_fn, "entry");
        self.builder.position_at_end(entry);
        self.builder.build_return(Some(&self.context.bool_type().const_zero()))?;

        let entry = self.context.append_basic_block(self.strcat_fn, "entry");
        self.builder.position_at_end(entry);
        let s_strcat = self.strcat_fn.get_nth_param(0).unwrap().into_pointer_value();
        self.builder.build_return(Some(&s_strcat))?;

        let entry = self.context.append_basic_block(self.rest_alloc_string_fn, "entry");
        self.builder.position_at_end(entry);
        let s_len = self.rest_alloc_string_fn.get_nth_param(0).unwrap().into_int_value();
        let struct_size = self.context.i64_type().const_int(24, false);
        let s_ptr = self.builder.build_call(self.malloc_fn, &[struct_size.into()], "malloc_struct")?.try_as_basic_value().basic().unwrap().into_pointer_value();
        let rc_ptr = self.builder.build_pointer_cast(s_ptr, self.context.i64_type().ptr_type(inkwell::AddressSpace::default()), "rc_ptr")?;
        self.builder.build_store(rc_ptr, self.context.i64_type().const_int(1, false))?;
        let len_ptr = unsafe { self.builder.build_in_bounds_gep(self.context.i8_type(), s_ptr, &[self.context.i64_type().const_int(8, false)], "len_ptr")? };
        let len_ptr_i64 = self.builder.build_pointer_cast(len_ptr, self.context.i64_type().ptr_type(inkwell::AddressSpace::default()), "len_ptr_i64")?;
        self.builder.build_store(len_ptr_i64, s_len)?;
        let one = self.context.i64_type().const_int(1, false);
        let data_len = self.builder.build_int_add(s_len, one, "data_len")?;
        let data_ptr = self.builder.build_call(self.malloc_fn, &[data_len.into()], "malloc_data")?.try_as_basic_value().basic().unwrap().into_pointer_value();
        let null_ptr = unsafe { self.builder.build_in_bounds_gep(self.context.i8_type(), data_ptr, &[s_len], "null_ptr")? };
        self.builder.build_store(null_ptr, self.context.i8_type().const_zero())?;
        let data_field_ptr = unsafe { self.builder.build_in_bounds_gep(self.context.i8_type(), s_ptr, &[self.context.i64_type().const_int(16, false)], "data_field_ptr")? };
        let data_field_ptr_casted = self.builder.build_pointer_cast(data_field_ptr, i8_ptr_ty.ptr_type(inkwell::AddressSpace::default()), "data_field_ptr_casted")?;
        self.builder.build_store(data_field_ptr_casted, data_ptr)?;
        self.builder.build_return(Some(&s_ptr))?;

        let void_ty = self.context.void_type();
        let i8_ptr_ty = self
            .context
            .i8_type()
            .ptr_type(inkwell::AddressSpace::default());
        let handler_ty = i8_ptr_ty
            .fn_type(&[i8_ptr_ty.into()], false)
            .ptr_type(inkwell::AddressSpace::default());
        let reg_route_ty = void_ty.fn_type(
            &[i8_ptr_ty.into(), i8_ptr_ty.into(), handler_ty.into()],
            false,
        );
        let reg_fn = self.module.add_function(
            "rest_register_route",
            reg_route_ty,
            None,
        );
        let entry = self.context.append_basic_block(reg_fn, "entry");
        self.builder.position_at_end(entry);
        self.builder.build_return(None)?;
        self.rest_register_route_fn = Some(reg_fn);
        let i32_ty = self.context.i32_type();
        let start_server_ty = void_ty.fn_type(&[i32_ty.into()], false);
        let start_fn = self.module.add_function(
            "rest_start_server",
            start_server_ty,
            None,
        );
        let entry = self.context.append_basic_block(start_fn, "entry");
        self.builder.position_at_end(entry);
        self.builder.build_return(None)?;
        self.rest_start_server_fn = Some(start_fn);
        self.runtime_compiled = true;
        Ok(())
    }
    pub(crate) fn enter_scope(&mut self) {
        self.values.push(HashMap::new());
        self.var_types.push(HashMap::new());
        self.array_info.push(HashMap::new());
        self.owner_tracking.push(Vec::new());
    }
    pub(crate) fn free_struct_ptr(
        &mut self,
        ptr: PointerValue<'ctx>,
        struct_name: &str,
        struct_field_types: &StructFieldTypes,
    ) {
        let Some(struct_ty) = self.struct_types.get(struct_name).copied() else {
            return;
        };
        if let Some(fields) = struct_field_types.get(struct_name) {
            for (i, (_, ty)) in fields.iter().enumerate() {
                if let Ok(gep) = self
                    .builder
                    .build_struct_gep(struct_ty, ptr, i as u32, "field")
                    && (matches!(ty, Type::String) || matches!(ty, Type::Struct(_)))
                    && let Ok(field_val) = self.builder.build_load(self.ptr_ty(), gep, "field_val")
                {
                    if let Type::Struct(inner_name) = ty {
                        let field_ptr = field_val.into_pointer_value();
                        self.free_struct_ptr(field_ptr, inner_name, struct_field_types);
                    } else {
                        self.builder
                            .build_call(
                                self.rest_release_string_fn,
                                &[field_val.into()],
                                "release_string_field",
                            )
                            .unwrap();
                    }
                }
            }
        }
        {
            let do_free = self
                .builder
                .build_call(self.release_fn, &[ptr.into()], "release_free_struct")
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
                "free_free_struct_block",
            );
            let merge_bb = self.context.append_basic_block(
                self.builder
                    .get_insert_block()
                    .unwrap()
                    .get_parent()
                    .unwrap(),
                "merge_free_free_struct",
            );
            self.builder
                .build_conditional_branch(do_free_bool, then_bb, merge_bb)
                .unwrap();
            self.builder.position_at_end(then_bb);
            let _ = self
                .builder
                .build_call(self.rest_free_fn, &[ptr.into()], "free_struct");
            self.builder.build_unconditional_branch(merge_bb).unwrap();
            self.builder.position_at_end(merge_bb);
        }
    }
    pub(crate) fn free_owner_struct(
        &mut self,
        owner_name: &str,
        struct_name: &str,
        struct_field_types: &StructFieldTypes,
    ) {
        let Some(&(ptr_val, _)) = self.lookup_value(owner_name) else {
            return;
        };
        let Ok(struct_ptr) = self.builder.build_load(self.ptr_ty(), ptr_val, owner_name) else {
            return;
        };
        self.free_struct_ptr(
            struct_ptr.into_pointer_value(),
            struct_name,
            struct_field_types,
        );
    }
    pub(crate) fn free_owner_array(
        &mut self,
        owner_name: &str,
        elem_type: &Type,
        count: usize,
        struct_field_types: &StructFieldTypes,
    ) {
        let Some(&(ptr_val, _)) = self.lookup_value(owner_name) else {
            return;
        };
        let Ok(arr_ptr_val) = self.builder.build_load(self.ptr_ty(), ptr_val, owner_name) else {
            return;
        };
        let arr_ptr = arr_ptr_val.into_pointer_value();
        let elem_llvm_ty = self.hir_type_to_basic(elem_type, struct_field_types);
        for i in 0..count {
            let gep = unsafe {
                let idx = self.context.i32_type().const_int(i as u64, false);
                self.builder
                    .build_gep(elem_llvm_ty, arr_ptr, &[idx], "array_elem")
                    .ok()
            };
            let Some(gep) = gep else { continue };
            if matches!(elem_type, Type::String) {
                if let Ok(elem) = self
                    .builder
                    .build_load(self.ptr_ty(), gep, "array_elem_val")
                {
                    {
                        let do_free = self
                            .builder
                            .build_call(self.release_fn, &[elem.into()], "release_free_array_elem")
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
                            "free_free_array_elem_block",
                        );
                        let merge_bb = self.context.append_basic_block(
                            self.builder
                                .get_insert_block()
                                .unwrap()
                                .get_parent()
                                .unwrap(),
                            "merge_free_free_array_elem",
                        );
                        self.builder
                            .build_conditional_branch(do_free_bool, then_bb, merge_bb)
                            .unwrap();
                        self.builder.position_at_end(then_bb);
                        let _ = self.builder.build_call(
                            self.rest_free_fn,
                            &[elem.into()],
                            "free_array_elem",
                        );
                        self.builder.build_unconditional_branch(merge_bb).unwrap();
                        self.builder.position_at_end(merge_bb);
                    }
                }
            } else if let Type::Struct(sname) = elem_type
                && let Ok(struct_ptr) =
                    self.builder
                        .build_load(self.ptr_ty(), gep, "array_struct_elem")
            {
                self.free_struct_ptr(struct_ptr.into_pointer_value(), sname, struct_field_types);
            }
        }
        {
            let do_free = self
                .builder
                .build_call(self.release_fn, &[arr_ptr.into()], "release_free_array")
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
                "free_free_array_block",
            );
            let merge_bb = self.context.append_basic_block(
                self.builder
                    .get_insert_block()
                    .unwrap()
                    .get_parent()
                    .unwrap(),
                "merge_free_free_array",
            );
            self.builder
                .build_conditional_branch(do_free_bool, then_bb, merge_bb)
                .unwrap();
            self.builder.position_at_end(then_bb);
            let _ = self
                .builder
                .build_call(self.rest_free_fn, &[arr_ptr.into()], "free_array");
            self.builder.build_unconditional_branch(merge_bb).unwrap();
            self.builder.position_at_end(merge_bb);
        }
    }
    pub(crate) fn free_owner_string(&mut self, owner_name: &str) {
        if let Some(&(ptr_alloca, _)) = self.lookup_value(owner_name) {
            if let Ok(loaded) = self
                .builder
                .build_load(self.ptr_ty(), ptr_alloca, owner_name)
            {
                self.builder
                    .build_call(
                        self.rest_release_string_fn,
                        &[loaded.into()],
                        "release_string",
                    )
                    .unwrap();
            }
        }
    }
    pub(crate) fn free_owner(&mut self, owner: &Owner, struct_field_types: &StructFieldTypes) {
        match owner {
            Owner::Struct(name, struct_name) => {
                self.free_owner_struct(name, struct_name, struct_field_types);
            }
            Owner::Array(name, elem_type, count) => {
                self.free_owner_array(name, elem_type, *count, struct_field_types);
            }
            Owner::String(name) => {
                self.free_owner_string(name);
            }
        }
    }
    pub(crate) fn free_owners_since(&mut self, depth: usize, struct_field_types: &StructFieldTypes) {
        let to_free: Vec<Owner> = self.owner_tracking[depth..]
            .iter()
            .rev()
            .flat_map(|v| v.iter().rev().cloned().collect::<Vec<_>>())
            .collect();
        for owner in &to_free {
            self.free_owner(owner, struct_field_types);
        }
        for entry in &mut self.owner_tracking[depth..] {
            entry.clear();
        }
    }
    pub(crate) fn is_owner(&self, name: &str) -> bool {
        self.owner_tracking.iter().any(|scope| {
            scope.iter().any(|owner| match owner {
                Owner::Struct(n, _) | Owner::Array(n, _, _) | Owner::String(n) => n == name,
            })
        })
    }
    pub(crate) fn bounds_check_array(
        &mut self,
        idx_val: IntValue<'ctx>,
        len: usize,
        label: &str,
    ) -> Result<()> {
        let len_val = self.context.i64_type().const_int(len as u64, false);
        let zero_val = self.context.i64_type().const_zero();
        let idx_ext = self.builder.build_int_z_extend_or_bit_cast(
            idx_val,
            self.context.i64_type(),
            &format!("{label}_idx_ext"),
        )?;
        let lt_zero = self.builder.build_int_compare(
            IntPredicate::SLT,
            idx_ext,
            zero_val,
            &format!("{label}_lt_zero"),
        )?;
        let ge_len = self.builder.build_int_compare(
            IntPredicate::SGE,
            idx_ext,
            len_val,
            &format!("{label}_ge_len"),
        )?;
        let oob = self
            .builder
            .build_or(lt_zero, ge_len, &format!("{label}_oob"))?;
        let current_fn = self.current_fn_val();
        let cont_block = self
            .context
            .append_basic_block(current_fn, &format!("{label}_cont"));
        let abort_block = self
            .context
            .append_basic_block(current_fn, &format!("{label}_oob_abort"));
        self.builder
            .build_conditional_branch(oob, abort_block, cont_block)?;
        self.builder.position_at_end(abort_block);
        self.builder
            .build_call(self.abort_fn, &[], &format!("{label}_abort"))?;
        self.builder.build_unreachable()?;
        self.builder.position_at_end(cont_block);
        Ok(())
    }
    pub(crate) fn transfer_ownership(&mut self, src_name: &str, dst_name: &str, struct_name: &str) {
        self.owner_scope_mut()
            .push(Owner::Struct(dst_name.to_string(), struct_name.to_string()));
        for scope in &mut self.owner_tracking {
            scope.retain(|owner| !matches ! (owner , Owner :: Struct (n , _) if n == src_name));
        }
    }
    pub(crate) fn remove_struct_owner(&mut self, name: &str) {
        for scope in &mut self.owner_tracking {
            scope.retain(|owner| !matches ! (owner , Owner :: Struct (n , _) if n == name));
        }
    }
    pub(crate) fn lookup_value(&self, name: &str) -> Option<&(PointerValue<'ctx>, BasicTypeEnum<'ctx>)> {
        for scope in self.values.iter().rev() {
            if let Some(v) = scope.get(name) {
                return Some(v);
            }
        }
        self.globals.get(name)
    }
    pub(crate) fn lookup_array_info(&self, name: &str) -> Option<&(Type, usize)> {
        for scope in self.array_info.iter().rev() {
            if let Some(v) = scope.get(name) {
                return Some(v);
            }
        }
        None
    }
    pub(crate) fn insert_value(&mut self, name: String, val: (PointerValue<'ctx>, BasicTypeEnum<'ctx>)) {
        self.values_mut().insert(name, val);
    }
    pub(crate) fn insert_array_info(&mut self, name: String, info: (Type, usize)) {
        self.array_info_mut().insert(name, info);
    }
    pub(crate) fn exit_scope(&mut self, struct_field_types: &StructFieldTypes) -> Result<()> {
        if let Some(owners) = self.owner_tracking.pop() {
            let to_free: Vec<Owner> = owners.iter().rev().cloned().collect();
            for owner in &to_free {
                self.free_owner(owner, struct_field_types);
            }
        }
        self.values.pop();
        self.array_info.pop();
        self.var_types.pop();
        Ok(())
    }
    pub fn compile(
        &mut self,
        stmts: &[HirStmt],
        struct_field_types: &StructFieldTypes,
    ) -> Result<()> {
        self.compile_runtime_fns()?;
        self.build_struct_types(stmts, struct_field_types);
        self.has_routes = stmts.iter().any(|stmt| {
            if let HirStmt::Fn { decorators, .. } = stmt {
                !decorators.is_empty()
            } else {
                false
            }
        });
        for stmt in stmts {
            match stmt {
                HirStmt::Fn {
                    name, params, ret, ..
                } => {
                    self.declare_function(name, params, false, ret, struct_field_types);
                }
                HirStmt::ExternFn {
                    name, params, is_variadic, ret, ..
                } => {
                    self.declare_function(name, params, *is_variadic, ret, struct_field_types);
                }
                HirStmt::GlobalAsm(asm, _) => {
                    self.module.set_inline_assembly(asm);
                }
                HirStmt::Const { name, ty, init, .. } => {
                    let basic_ty = self.hir_type_to_basic(ty, struct_field_types);
                    let global = self.module.add_global(basic_ty, None, name);
                    global.set_constant(true);
                    
                    // We need to evaluate the init expr to a constant
                    // If it's a simple literal, we can do it directly.
                    // Instead of full compilation, we just compile it into the global init.
                    // Wait, compile_expr needs a builder positioned in a basic block!
                    // Let's create a dummy function, compile it, extract const, then delete it?
                    // No, simpler: just match the literal manually here:
                    let init_val: inkwell::values::BasicValueEnum<'ctx> = match init {
                        crate::ir::HirExpr::Int(i, _, _) => self.context.i64_type().const_int(*i as u64, false).into(),
                        crate::ir::HirExpr::Bool(b, _) => self.context.bool_type().const_int(if *b { 1 } else { 0 }, false).into(),
                        crate::ir::HirExpr::String(s, _) => {
                            let init_str = self.context.const_string(s.as_bytes(), true);
                            let global_str = self.module.add_global(init_str.get_type(), None, ".str.const");
                            global_str.set_initializer(&init_str);
                            global_str.set_constant(true);
                            global_str.as_pointer_value().into()
                        }
                        _ => panic!("constants must be simple literals"),
                    };
                    global.set_initializer(&init_val);
                    self.globals.insert(name.clone(), (global.as_pointer_value(), basic_ty));
                    // Also store its type in var_types so lookups (like arrays) can find it
                    if self.var_types.is_empty() {
                        self.var_types.push(HashMap::new());
                    }
                    self.var_types[0].insert(name.clone(), ty.clone());
                }
                _ => {}
            }
        }
        for stmt in stmts {
            if let HirStmt::Fn {
                name,
                params,
                ret,
                body,
                decorators,
                span: _,
            } = stmt
            {
                let fn_val = self
                    .functions
                    .get(name)
                    .copied()
                    .ok_or_else(|| anyhow::anyhow!("undefined function `{}`", name))?;
                if !decorators.is_empty() {
                    let wrapper_val = self.generate_http_wrapper(name, fn_val, params, ret)?;
                    for dec in decorators {
                        if let Some(path) = &dec.arg {
                            self.routes
                                .push((dec.name.clone(), path.clone(), wrapper_val));
                        }
                    }
                }
                self.compile_fn_body(fn_val, params, ret, body, struct_field_types)?;
            }
        }
        if self.has_routes {
            self.generate_rest_main()?;
        }
        Ok(())
    }
    pub(crate) fn generate_rest_main(&mut self) -> Result<()> {
        let i32_ty = self.context.i32_type();
        let main_type = i32_ty.fn_type(&[], false);
        let main_fn = self.module.add_function("main", main_type, None);
        let bb = self.context.append_basic_block(main_fn, "entry");
        self.builder.position_at_end(bb);
        let i8_ptr_ty = self
            .context
            .i8_type()
            .ptr_type(inkwell::AddressSpace::default());
        for (method, path, handler_fn) in &self.routes {
            let method_global = self
                .builder
                .build_global_string_ptr(method, "method")
                .unwrap();
            let path_global = self.builder.build_global_string_ptr(path, "path").unwrap();
            let handler_ptr = self
                .builder
                .build_pointer_cast(
                    handler_fn.as_global_value().as_pointer_value(),
                    i8_ptr_ty,
                    "handler_cast",
                )
                .unwrap();
            self.builder
                .build_call(
                    self.rest_register_route_fn.unwrap(),
                    &[
                        method_global.as_pointer_value().into(),
                        path_global.as_pointer_value().into(),
                        handler_ptr.into(),
                    ],
                    "reg",
                )
                .unwrap();
        }
        if let Some(user_main) = self.functions.get("main") {
            self.builder
                .build_call(*user_main, &[], "call_user_main")
                .unwrap();
        }
        self.builder
            .build_call(
                self.rest_start_server_fn.unwrap(),
                &[i32_ty.const_int(8080, false).into()],
                "start",
            )
            .unwrap();
        self.builder
            .build_return(Some(&i32_ty.const_int(0, false)))
            .unwrap();
        Ok(())
    }
    pub(crate) fn generate_http_wrapper(
        &mut self,
        name: &str,
        original_fn: FunctionValue<'ctx>,
        params: &[(String, Type)],
        ret: &Type,
    ) -> Result<FunctionValue<'ctx>> {
        let i8_ptr_type = self
            .context
            .i8_type()
            .ptr_type(inkwell::AddressSpace::default());
        let wrapper_type = i8_ptr_type.fn_type(&[i8_ptr_type.into()], false);
        let wrapper_name = format!("__rest_http_wrapper_{}", name);
        let wrapper_fn = self.module.add_function(
            &wrapper_name,
            wrapper_type,
            Some(inkwell::module::Linkage::Internal),
        );
        let bb = self.context.append_basic_block(wrapper_fn, "entry");
        let prev_bb = self.builder.get_insert_block();
        self.builder.position_at_end(bb);
        let mut args: Vec<inkwell::values::BasicMetadataValueEnum<'ctx>> = Vec::new();
        let body_param = wrapper_fn.get_nth_param(0).unwrap();
        if params.len() == 1 && params[0].1 == Type::String {
            self.builder
                .build_call(
                    self.rest_retain_string_fn,
                    &[body_param.into()],
                    "retain_body",
                )
                .unwrap();
            args.push(body_param.into());
        }
        let call_site = self
            .builder
            .build_call(original_fn, &args, "call_orig")
            .unwrap();
        self.builder
            .build_call(
                self.rest_release_string_fn,
                &[body_param.into()],
                "release_body",
            )
            .unwrap();
        if *ret == Type::String {
            let res_val = call_site.try_as_basic_value().basic().unwrap();
            self.builder.build_return(Some(&res_val)).unwrap();
        } else {
            self.builder
                .build_return(Some(&i8_ptr_type.const_null()))
                .unwrap();
        }
        if let Some(pbb) = prev_bb {
            self.builder.position_at_end(pbb);
        }
        Ok(wrapper_fn)
    }
    pub(crate) fn owner_scope_mut(&mut self) -> &mut Vec<Owner> {
        self.owner_tracking
            .last_mut()
            .expect("owner_tracking: enter_scope() must be called before use")
    }
    pub(crate) fn values_mut(&mut self) -> &mut HashMap<String, (PointerValue<'ctx>, BasicTypeEnum<'ctx>)> {
        self.values
            .last_mut()
            .expect("values: enter_scope() must be called before use")
    }
    pub(crate) fn array_info_mut(&mut self) -> &mut HashMap<String, (Type, usize)> {
        self.array_info
            .last_mut()
            .expect("array_info: enter_scope() must be called before use")
    }
    pub(crate) fn current_fn_val(&self) -> FunctionValue<'ctx> {
        self.current_fn
            .expect("current_fn: must be set inside compile_fn_body")
    }
    pub(crate) fn declare_function(
        &mut self,
        name: &str,
        params: &[(String, Type)],
        is_variadic: bool,
        ret: &Type,
        struct_field_types: &StructFieldTypes,
    ) {
        let i8_ptr = self.ptr_ty();
        let llvm_param_tys: Vec<BasicMetadataTypeEnum> = params
            .iter()
            .map(|(_, ty)| {
                if matches!(ty, Type::Struct(_)) {
                    i8_ptr.into()
                } else {
                    self.hir_type_to_basic(ty, struct_field_types).into()
                }
            })
            .collect();
        let fn_type = if *ret == Type::Void && name == "main" {
            self.context.i32_type().fn_type(&llvm_param_tys, is_variadic)
        } else if *ret == Type::Void {
            self.context.void_type().fn_type(&llvm_param_tys, is_variadic)
        } else if matches!(ret, Type::Struct(_)) {
            i8_ptr.fn_type(&llvm_param_tys, is_variadic)
        } else {
            let llvm_ret = self.hir_type_to_basic(ret, struct_field_types);
            llvm_ret.fn_type(&llvm_param_tys, is_variadic)
        };
        let actual_name = if name == "main" && self.has_routes {
            "__rest_user_main"
        } else {
            name
        };
        if let Some(existing) = self.module.get_function(actual_name) {
            self.functions.insert(name.to_string(), existing);
            return;
        }
        let fn_val = self.module.add_function(actual_name, fn_type, None);
        self.functions.insert(name.to_string(), fn_val);
    }
    pub(crate) fn compile_fn_body(
        &mut self,
        fn_val: FunctionValue<'ctx>,
        params: &[(String, Type)],
        ret: &Type,
        body: &[HirStmt],
        struct_field_types: &StructFieldTypes,
    ) -> Result<()> {
        let entry = self.context.append_basic_block(fn_val, "entry");
        self.builder.position_at_end(entry);
        self.current_fn = Some(fn_val);
        self.current_fn_name = fn_val.get_name().to_str().unwrap_or("").to_string();
        self.enter_scope();
        let i8_ptr = self.ptr_ty();
        for (i, (name, param_ty)) in params.iter().enumerate() {
            let param = fn_val.get_nth_param(i as u32).with_context(|| {
                format!(
                    "parameter index {} out of bounds for fn `{}`",
                    i, self.current_fn_name
                )
            })?;
            let (alloca, llvm_ty) = if matches!(param_ty, Type::Struct(_)) {
                let a = self.builder.build_alloca(i8_ptr, name)?;
                (a, i8_ptr.into())
            } else {
                let llvm_ty = self.hir_type_to_basic(param_ty, struct_field_types);
                let a = self.builder.build_alloca(llvm_ty, name)?;
                (a, llvm_ty)
            };
            self.builder.build_store(alloca, param)?;
            self.insert_value(name.clone(), (alloca, llvm_ty));
        }
        for stmt in body {
            self.compile_hir_stmt(stmt, struct_field_types)?;
        }
        self.exit_scope(struct_field_types)?;
        let needs_return = self
            .builder
            .get_insert_block()
            .map(|bb| bb.get_terminator().is_none())
            .unwrap_or(false);
        if needs_return {
            if self.current_fn_name == "main" && *ret == Type::Void {
                self.builder
                    .build_return(Some(&self.context.i32_type().const_int(0, false)))?;
            } else if *ret == Type::Void {
                self.builder.build_return(None)?;
            } else if matches!(ret, Type::Struct(_)) {
                let zero: BasicValueEnum = self.ptr_ty().const_zero().into();
                self.builder.build_return(Some(&zero))?;
            } else {
                let zero = self.hir_type_to_basic(ret, struct_field_types).const_zero();
                self.builder.build_return(Some(&zero))?;
            }
        }
        Ok(())
    }
    pub(crate) fn compile_ref(
        &mut self,
        inner: &HirExpr,
        struct_field_types: &StructFieldTypes,
    ) -> Result<BasicValueEnum<'ctx>> {
        match inner {
            HirExpr::Ident { name, .. } => {
                let &(alloca, load_ty) = self
                    .lookup_value(name)
                    .ok_or_else(|| anyhow::anyhow!("undefined variable `{}`", name))?;
                if load_ty.is_pointer_type() {
                    let loaded = self.builder.build_load(self.ptr_ty(), alloca, name)?;
                    Ok(loaded)
                } else {
                    let casted = self.builder.build_pointer_cast(
                        alloca,
                        self.ptr_ty(),
                        &format!("{}_ref", name),
                    )?;
                    Ok(casted.into())
                }
            }
            _ => {
                let val = self.compile_expr(inner, struct_field_types)?;
                let val_ty = val.get_type();
                let alloca = self.builder.build_alloca(val_ty, "ref_tmp")?;
                self.builder.build_store(alloca, val)?;
                let casted =
                    self.builder
                        .build_pointer_cast(alloca, self.ptr_ty(), "ref_tmp_cast")?;
                Ok(casted.into())
            }
        }
    }
    pub(crate) fn compile_print(
        &mut self,
        arg: &HirExpr,
        struct_field_types: &StructFieldTypes,
    ) -> Result<BasicValueEnum<'ctx>> {
        let val = self.compile_expr(arg, struct_field_types)?;
        if self.type_of_expr(arg, struct_field_types) == Type::String {
            self.builder
                .build_call(self.rest_print_string_fn, &[val.into()], "print_str")?;
            return Ok(self.context.i32_type().const_zero().into());
        }
        let i8_ptr = self.ptr_ty();
        let i32_ty = self.context.i32_type();
        let (arg_val, fmt_str) = if val.is_float_value() {
            let ft = val.into_float_value();
            let ft_ty = ft.get_type();
            let is_f32 = ft_ty.get_bit_width() == 32;
            let promoted: BasicValueEnum = if is_f32 {
                self.builder
                    .build_float_ext(ft, self.context.f64_type(), "f32_to_f64")?
                    .into()
            } else {
                ft.into()
            };
            (
                promoted,
                self.builder.build_global_string_ptr("%f\n", "fmt_f64")?,
            )
        } else if val.is_pointer_value() {
            (
                val,
                self.builder.build_global_string_ptr("%p\n", "fmt_ptr")?,
            )
        } else {
            let iv = val.into_int_value();
            let iv_ty = iv.get_type();
            let width = iv_ty.get_bit_width();
            let is_unsigned = match arg {
                HirExpr::Bool(_, _) => true,
                HirExpr::Int(_, ty, _) => Self::is_unsigned_type(ty),
                HirExpr::Ident { name, .. } => self
                    .lookup_var_type(name)
                    .map(Self::is_unsigned_type)
                    .unwrap_or(false),
                _ => false,
            };
            if width < 32 {
                let ext = if is_unsigned {
                    self.builder.build_int_z_extend(iv, i32_ty, "zext")?
                } else {
                    self.builder.build_int_s_extend(iv, i32_ty, "sext")?
                };
                let fmt = if is_unsigned { "%u\n" } else { "%d\n" };
                (
                    ext.into(),
                    self.builder.build_global_string_ptr(fmt, "fmt_i32")?,
                )
            } else if width == 32 {
                let fmt = if is_unsigned { "%u\n" } else { "%d\n" };
                (
                    iv.into(),
                    self.builder.build_global_string_ptr(fmt, "fmt_i32")?,
                )
            } else {
                let fmt = if is_unsigned { "%lu\n" } else { "%ld\n" };
                (
                    iv.into(),
                    self.builder.build_global_string_ptr(fmt, "fmt_i64")?,
                )
            }
        };
        let fmt_ptr = fmt_str.as_pointer_value();
        let casted = self
            .builder
            .build_pointer_cast(fmt_ptr, i8_ptr, "fmt_cast")?;
        self.builder.build_call(
            self.printf_fn,
            &[casted.into(), arg_val.into()],
            "printf_call",
        )?;
        Ok(self.context.i32_type().const_zero().into())
    }
}
pub fn generate(
    output: &Path,
    hir: &[HirStmt],
    struct_field_types: StructFieldTypes,
    opt_level: OptimizationLevel,
) -> Result<()> {
    let context = Context::create();
    let mut codegen = Codegen::new(&context, "ref_module");
    codegen.compile(hir, &struct_field_types)?;
    match output.extension().and_then(|e| e.to_str()) {
        Some("bc") => codegen.write_bitcode(output),
        Some("o") => codegen.emit_object(output, opt_level),
        _ => {
            let ir = codegen.generate_ir();
            std::fs::write(output, ir)?;
            Ok(())
        }
    }
}
