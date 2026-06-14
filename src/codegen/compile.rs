use std::collections::HashMap;
use std::path::Path;
use std::sync::Once;

use anyhow::{Context as _, Result, bail};
use inkwell::IntPredicate;
use inkwell::FloatPredicate;
use inkwell::OptimizationLevel;
use inkwell::builder::Builder;
use inkwell::context::Context;
use inkwell::module::Module;
use inkwell::targets::{
    CodeModel, FileType, InitializationConfig, RelocMode, Target, TargetMachine,
};
use inkwell::types::{BasicMetadataTypeEnum, BasicType, BasicTypeEnum, StructType};
use inkwell::values::{BasicMetadataValueEnum, BasicValueEnum, FunctionValue, IntValue, PointerValue};
use inkwell::basic_block::BasicBlock;

use crate::ir::{HirExpr, HirStmt};
use crate::ops::{BinOp, UnOp};
use crate::sema::Type;

type StructFieldTypes = HashMap<String, Vec<(String, Type)>>;

#[derive(Debug, Clone)]
enum Owner {
    Struct(String, String),
    Array(String, Type, usize),
    String(String),
}

pub(crate) struct Codegen<'ctx> {
    context: &'ctx Context,
    module: Module<'ctx>,
    builder: Builder<'ctx>,
    current_fn: Option<FunctionValue<'ctx>>,
    current_fn_name: String,
    values: Vec<HashMap<String, (PointerValue<'ctx>, BasicTypeEnum<'ctx>)>>,
    struct_types: HashMap<String, StructType<'ctx>>,
    printf_fn: FunctionValue<'ctx>,
    malloc_fn: FunctionValue<'ctx>,
    free_fn: FunctionValue<'ctx>,
    strlen_fn: FunctionValue<'ctx>,
    memcpy_fn: FunctionValue<'ctx>,
    retain_fn: FunctionValue<'ctx>,
    release_fn: FunctionValue<'ctx>,
    rest_alloc_fn: FunctionValue<'ctx>,
    rest_free_fn: FunctionValue<'ctx>,
    strcat_fn: FunctionValue<'ctx>,
    abort_fn: FunctionValue<'ctx>,
    rest_register_route_fn: Option<FunctionValue<'ctx>>,
    rest_start_server_fn: Option<FunctionValue<'ctx>>,
    functions: HashMap<String, FunctionValue<'ctx>>,
    routes: Vec<(String, String, FunctionValue<'ctx>)>,
    loop_stack: Vec<(BasicBlock<'ctx>, BasicBlock<'ctx>, usize)>,
    owner_tracking: Vec<Vec<Owner>>,
    array_info: Vec<HashMap<String, (Type, usize)>>,
    var_types: Vec<HashMap<String, Type>>,
    runtime_compiled: bool,
    has_routes: bool,
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

        let strlen_type = context.i64_type().fn_type(&[i8_ptr.into()], false);
        let strlen_fn = module.add_function("strlen", strlen_type, None);

        let memcpy_type = context.void_type().fn_type(
            &[i8_ptr.into(), i8_ptr.into(), context.i64_type().into(), context.bool_type().into()],
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
        let strcat_fn = module.add_function("__ref_strcat", strcat_type, None);

        let abort_type = context.void_type().fn_type(&[], false);
        let abort_fn = module.add_function("abort", abort_type, None);
        // Mark `abort` as noreturn so LLVM knows the call never falls
        // through. This prevents LLVM from treating code that follows
        // an abort branch as reachable UB during optimization.
        // We still emit `build_unreachable()` after each call as a
        // defensive terminator in case inkwell's `position_at_end`
        // doesn't recognize a noreturn call as a block terminator.
        let noreturn_kind = inkwell::attributes::Attribute::get_named_enum_kind_id("noreturn");
        let noreturn_attr = context.create_enum_attribute(noreturn_kind, 0);
        abort_fn.add_attribute(inkwell::attributes::AttributeLoc::Function, noreturn_attr);

        Self {
            context,
            module,
            builder,
            current_fn: None,
            current_fn_name: String::new(),
            values: vec![HashMap::new()],
            struct_types: HashMap::new(),
            printf_fn,
            malloc_fn,
            free_fn,
            strlen_fn,
            memcpy_fn,
            retain_fn,
            release_fn,
            rest_alloc_fn,
            rest_free_fn,
            strcat_fn,
            abort_fn,
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

    fn compile_runtime_fns(&mut self) -> Result<()> {
        if self.runtime_compiled {
            return Ok(());
        }

        let i64_ty = self.context.i64_type();
        let i32_ptr = self.context.i32_type().ptr_type(inkwell::AddressSpace::default());
        let bool_ty = self.context.bool_type();

        // __rest_alloc
        let entry = self.context.append_basic_block(self.rest_alloc_fn, "entry");
        self.builder.position_at_end(entry);
        let size = self.rest_alloc_fn.get_nth_param(0).unwrap().into_int_value();
        let total_size = self.builder.build_int_add(size, i64_ty.const_int(4, false), "total_size")?;
        let ptr = self.builder.build_call(self.malloc_fn, &[total_size.into()], "malloc")?.try_as_basic_value().basic().unwrap().into_pointer_value();
        let rc_ptr = self.builder.build_pointer_cast(ptr, i32_ptr, "rc_ptr")?;
        self.builder.build_store(rc_ptr, self.context.i32_type().const_int(1, false))?;
        let data_ptr = unsafe { self.builder.build_in_bounds_gep(self.context.i8_type(), ptr, &[i64_ty.const_int(4, false)], "data_ptr")? };
        self.builder.build_return(Some(&data_ptr))?;

        // __rest_retain
        let entry = self.context.append_basic_block(self.retain_fn, "entry");
        self.builder.position_at_end(entry);
        let s = self.retain_fn.get_nth_param(0).unwrap().into_pointer_value();
        let null_bb = self.context.append_basic_block(self.retain_fn, "null");
        let not_null_bb = self.context.append_basic_block(self.retain_fn, "not_null");
        let is_null = self.builder.build_is_null(s, "is_null")?;
        self.builder.build_conditional_branch(is_null, null_bb, not_null_bb)?;
        self.builder.position_at_end(null_bb);
        self.builder.build_return(Some(&s))?;
        self.builder.position_at_end(not_null_bb);
        let rc_ptr_i8 = unsafe { self.builder.build_in_bounds_gep(self.context.i8_type(), s, &[i64_ty.const_int(0xFFFFFFFFFFFFFFFC, true)], "rc_ptr_i8")? };
        let rc_ptr = self.builder.build_pointer_cast(rc_ptr_i8, i32_ptr, "rc_ptr")?;
        self.builder.build_atomicrmw(
            inkwell::AtomicRMWBinOp::Add,
            rc_ptr,
            self.context.i32_type().const_int(1, false),
            inkwell::AtomicOrdering::SequentiallyConsistent
        )?;
        self.builder.build_return(Some(&s))?;

        // __rest_release
        let entry = self.context.append_basic_block(self.release_fn, "entry");
        self.builder.position_at_end(entry);
        let s = self.release_fn.get_nth_param(0).unwrap().into_pointer_value();
        let null_bb = self.context.append_basic_block(self.release_fn, "null");
        let not_null_bb = self.context.append_basic_block(self.release_fn, "not_null");
        let is_null = self.builder.build_is_null(s, "is_null")?;
        self.builder.build_conditional_branch(is_null, null_bb, not_null_bb)?;
        self.builder.position_at_end(null_bb);
        self.builder.build_return(Some(&self.context.i32_type().const_zero()))?;
        self.builder.position_at_end(not_null_bb);
        let rc_ptr_i8 = unsafe { self.builder.build_in_bounds_gep(self.context.i8_type(), s, &[i64_ty.const_int(0xFFFFFFFFFFFFFFFC, true)], "rc_ptr_i8")? };
        let rc_ptr = self.builder.build_pointer_cast(rc_ptr_i8, i32_ptr, "rc_ptr")?;
        let old_rc = self.builder.build_atomicrmw(
            inkwell::AtomicRMWBinOp::Sub,
            rc_ptr,
            self.context.i32_type().const_int(1, false),
            inkwell::AtomicOrdering::SequentiallyConsistent
        )?;
        let is_one = self.builder.build_int_compare(
            inkwell::IntPredicate::EQ,
            old_rc,
            self.context.i32_type().const_int(1, false),
            "is_one"
        )?;
        let i32_is_one = self.builder.build_int_z_extend(is_one, self.context.i32_type(), "i32_is_one")?;
        self.builder.build_return(Some(&i32_is_one))?;

        // __rest_free
        let entry = self.context.append_basic_block(self.rest_free_fn, "entry");
        self.builder.position_at_end(entry);
        let s = self.rest_free_fn.get_nth_param(0).unwrap().into_pointer_value();
        let null_bb = self.context.append_basic_block(self.rest_free_fn, "null");
        let not_null_bb = self.context.append_basic_block(self.rest_free_fn, "not_null");
        let is_null = self.builder.build_is_null(s, "is_null")?;
        self.builder.build_conditional_branch(is_null, null_bb, not_null_bb)?;
        self.builder.position_at_end(null_bb);
        self.builder.build_return(None)?;
        self.builder.position_at_end(not_null_bb);
        let real_ptr = unsafe { self.builder.build_in_bounds_gep(self.context.i8_type(), s, &[i64_ty.const_int(0xFFFFFFFFFFFFFFFC, true)], "real_ptr")? };
        self.builder.build_call(self.free_fn, &[real_ptr.into()], "free")?;
        self.builder.build_return(None)?;

        // __ref_strcat(ptr %a, ptr %b) -> ptr
        let entry = self.context.append_basic_block(self.strcat_fn, "entry");
        self.builder.position_at_end(entry);
        let a = self.strcat_fn.get_nth_param(0)
            .expect("__ref_strcat should have exactly 2 parameters");
        let b = self.strcat_fn.get_nth_param(1)
            .expect("__ref_strcat should have exactly 2 parameters");

        let len_a = self
            .builder
            .build_call(self.strlen_fn, &[a.into()], "len_a")?
            .try_as_basic_value()
            .basic()
            .expect("strlen should return a basic int value")
            .into_int_value();
        let len_b = self
            .builder
            .build_call(self.strlen_fn, &[b.into()], "len_b")?
            .try_as_basic_value()
            .basic()
            .expect("strlen should return a basic int value")
            .into_int_value();
        let total = self
            .builder
            .build_int_add(len_a, len_b, "total")?;
        let plus_one = self
            .builder
            .build_int_add(total, i64_ty.const_int(1, false), "plus_one")?;
        let result = self
            .builder
            .build_call(self.rest_alloc_fn, &[plus_one.into()], "result")?
            .try_as_basic_value()
            .basic()
            .expect("malloc should return a basic pointer value")
            .into_pointer_value();
        self.builder.build_call(
            self.memcpy_fn,
            &[
                result.into(),
                a.into(),
                len_a.into(),
                bool_ty.const_zero().into(),
            ],
            "",
        )?;

        let i8_ty = self.context.i8_type();
        // SAFETY: GEP on i8* with i64 offset — i8 has size 1, so any offset is valid
        let offset = unsafe {
            self.builder
                .build_gep(i8_ty, result, &[len_a], "offset")?
        };
        let len_b_plus_one = self
            .builder
            .build_int_add(len_b, i64_ty.const_int(1, false), "len_b_plus_one")?;
        self.builder.build_call(
            self.memcpy_fn,
            &[
                offset.into(),
                b.into(),
                len_b_plus_one.into(),
                bool_ty.const_zero().into(),
            ],
            "",
        )?;
        let result_val: BasicValueEnum = result.into();
        self.builder.build_return(Some(&result_val))?;

        let void_ty = self.context.void_type();
        let i8_ptr_ty = self.context.i8_type().ptr_type(inkwell::AddressSpace::default());
        let handler_ty = i8_ptr_ty.fn_type(&[i8_ptr_ty.into()], false).ptr_type(inkwell::AddressSpace::default());
        let reg_route_ty = void_ty.fn_type(&[i8_ptr_ty.into(), i8_ptr_ty.into(), handler_ty.into()], false);
        self.rest_register_route_fn = Some(self.module.add_function("rest_register_route", reg_route_ty, Some(inkwell::module::Linkage::External)));

        let i32_ty = self.context.i32_type();
        let start_server_ty = void_ty.fn_type(&[i32_ty.into()], false);
        self.rest_start_server_fn = Some(self.module.add_function("rest_start_server", start_server_ty, Some(inkwell::module::Linkage::External)));

        self.runtime_compiled = true;
        Ok(())
    }

    // ---- Top-level ----

    fn enter_scope(&mut self) {
        self.values.push(HashMap::new());
        self.var_types.push(HashMap::new());
        self.array_info.push(HashMap::new());
        self.owner_tracking.push(Vec::new());
    }

    fn free_struct_ptr(
        &mut self,
        ptr: PointerValue<'ctx>,
        struct_name: &str,
        struct_field_types: &StructFieldTypes,
    ) {
        let Some(struct_ty) = self.struct_types.get(struct_name).copied() else { return };
        if let Some(fields) = struct_field_types.get(struct_name) {
            for (i, (_, ty)) in fields.iter().enumerate() {
                        if let Ok(gep) = self.builder.build_struct_gep(struct_ty, ptr, i as u32, "field")
                            && (matches!(ty, Type::String) || matches!(ty, Type::Struct(_)))
                                && let Ok(field_val) = self.builder.build_load(self.ptr_ty(), gep, "field_val") {
                                    if let Type::Struct(inner_name) = ty {
                                        let field_ptr = field_val.into_pointer_value();
                                        self.free_struct_ptr(field_ptr, inner_name, struct_field_types);
                                    } else {
                                        {
        let do_free = self.builder.build_call(self.release_fn, &[field_val.into()], "release_free_field").unwrap().try_as_basic_value().basic().unwrap().into_int_value();
        let do_free_bool = self.builder.build_int_compare(inkwell::IntPredicate::NE, do_free, self.context.i32_type().const_zero(), "do_free_bool").unwrap();
        let then_bb = self.context.append_basic_block(self.builder.get_insert_block().unwrap().get_parent().unwrap(), "free_free_field_block");
        let merge_bb = self.context.append_basic_block(self.builder.get_insert_block().unwrap().get_parent().unwrap(), "merge_free_free_field");
        self.builder.build_conditional_branch(do_free_bool, then_bb, merge_bb).unwrap();
        self.builder.position_at_end(then_bb);
        let _ = self.builder.build_call(self.rest_free_fn, &[field_val.into()], "free_field");
        self.builder.build_unconditional_branch(merge_bb).unwrap();
        self.builder.position_at_end(merge_bb);
    }
                                    }
                                }
            }
        }
        {
        let do_free = self.builder.build_call(self.release_fn, &[ptr.into()], "release_free_struct").unwrap().try_as_basic_value().basic().unwrap().into_int_value();
        let do_free_bool = self.builder.build_int_compare(inkwell::IntPredicate::NE, do_free, self.context.i32_type().const_zero(), "do_free_bool").unwrap();
        let then_bb = self.context.append_basic_block(self.builder.get_insert_block().unwrap().get_parent().unwrap(), "free_free_struct_block");
        let merge_bb = self.context.append_basic_block(self.builder.get_insert_block().unwrap().get_parent().unwrap(), "merge_free_free_struct");
        self.builder.build_conditional_branch(do_free_bool, then_bb, merge_bb).unwrap();
        self.builder.position_at_end(then_bb);
        let _ = self.builder.build_call(self.rest_free_fn, &[ptr.into()], "free_struct");
        self.builder.build_unconditional_branch(merge_bb).unwrap();
        self.builder.position_at_end(merge_bb);
    }
    }

    fn free_owner_struct(
        &mut self,
        owner_name: &str,
        struct_name: &str,
        struct_field_types: &StructFieldTypes,
    ) {
        let Some(&(ptr_val, _)) = self.lookup_value(owner_name) else { return };
        let Ok(struct_ptr) = self.builder.build_load(self.ptr_ty(), ptr_val, owner_name) else { return };
        self.free_struct_ptr(struct_ptr.into_pointer_value(), struct_name, struct_field_types);
    }

    fn free_owner_array(
        &mut self,
        owner_name: &str,
        elem_type: &Type,
        count: usize,
        struct_field_types: &StructFieldTypes,
    ) {
        let Some(&(ptr_val, _)) = self.lookup_value(owner_name) else { return };
        let Ok(arr_ptr_val) = self.builder.build_load(self.ptr_ty(), ptr_val, owner_name) else { return };
        let arr_ptr = arr_ptr_val.into_pointer_value();
        let elem_llvm_ty = self.hir_type_to_basic(elem_type, struct_field_types);
        for i in 0..count {
            // SAFETY: GEP on typed array pointer with i32 index — bounds-checked by loop range
            let gep = unsafe {
                let idx = self.context.i32_type().const_int(i as u64, false);
                self.builder.build_gep(elem_llvm_ty, arr_ptr, &[idx], "array_elem").ok()
            };
            let Some(gep) = gep else { continue };
            if matches!(elem_type, Type::String) {
                if let Ok(elem) = self.builder.build_load(self.ptr_ty(), gep, "array_elem_val") {
                    {
        let do_free = self.builder.build_call(self.release_fn, &[elem.into()], "release_free_array_elem").unwrap().try_as_basic_value().basic().unwrap().into_int_value();
        let do_free_bool = self.builder.build_int_compare(inkwell::IntPredicate::NE, do_free, self.context.i32_type().const_zero(), "do_free_bool").unwrap();
        let then_bb = self.context.append_basic_block(self.builder.get_insert_block().unwrap().get_parent().unwrap(), "free_free_array_elem_block");
        let merge_bb = self.context.append_basic_block(self.builder.get_insert_block().unwrap().get_parent().unwrap(), "merge_free_free_array_elem");
        self.builder.build_conditional_branch(do_free_bool, then_bb, merge_bb).unwrap();
        self.builder.position_at_end(then_bb);
        let _ = self.builder.build_call(self.rest_free_fn, &[elem.into()], "free_array_elem");
        self.builder.build_unconditional_branch(merge_bb).unwrap();
        self.builder.position_at_end(merge_bb);
    }
                }
            } else if let Type::Struct(sname) = elem_type
                && let Ok(struct_ptr) = self.builder.build_load(self.ptr_ty(), gep, "array_struct_elem") {
                    self.free_struct_ptr(struct_ptr.into_pointer_value(), sname, struct_field_types);
                }
        }
        {
        let do_free = self.builder.build_call(self.release_fn, &[arr_ptr.into()], "release_free_array").unwrap().try_as_basic_value().basic().unwrap().into_int_value();
        let do_free_bool = self.builder.build_int_compare(inkwell::IntPredicate::NE, do_free, self.context.i32_type().const_zero(), "do_free_bool").unwrap();
        let then_bb = self.context.append_basic_block(self.builder.get_insert_block().unwrap().get_parent().unwrap(), "free_free_array_block");
        let merge_bb = self.context.append_basic_block(self.builder.get_insert_block().unwrap().get_parent().unwrap(), "merge_free_free_array");
        self.builder.build_conditional_branch(do_free_bool, then_bb, merge_bb).unwrap();
        self.builder.position_at_end(then_bb);
        let _ = self.builder.build_call(self.rest_free_fn, &[arr_ptr.into()], "free_array");
        self.builder.build_unconditional_branch(merge_bb).unwrap();
        self.builder.position_at_end(merge_bb);
    }
    }

    fn free_owner_string(&mut self, owner_name: &str) {
        if let Some(&(ptr_alloca, _)) = self.lookup_value(owner_name)
            && let Ok(loaded) = self.builder.build_load(self.ptr_ty(), ptr_alloca, owner_name) {
                {
        let do_free = self.builder.build_call(self.release_fn, &[loaded.into()], "release_free_string").unwrap().try_as_basic_value().basic().unwrap().into_int_value();
        let do_free_bool = self.builder.build_int_compare(inkwell::IntPredicate::NE, do_free, self.context.i32_type().const_zero(), "do_free_bool").unwrap();
        let then_bb = self.context.append_basic_block(self.builder.get_insert_block().unwrap().get_parent().unwrap(), "free_free_string_block");
        let merge_bb = self.context.append_basic_block(self.builder.get_insert_block().unwrap().get_parent().unwrap(), "merge_free_free_string");
        self.builder.build_conditional_branch(do_free_bool, then_bb, merge_bb).unwrap();
        self.builder.position_at_end(then_bb);
        let _ = self.builder.build_call(self.rest_free_fn, &[loaded.into()], "free_string");
        self.builder.build_unconditional_branch(merge_bb).unwrap();
        self.builder.position_at_end(merge_bb);
    }
            }
    }

    fn free_owner(&mut self, owner: &Owner, struct_field_types: &StructFieldTypes) {
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

    fn free_owners_since(&mut self, depth: usize, struct_field_types: &StructFieldTypes) {
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

    fn is_owner(&self, name: &str) -> bool {
        self.owner_tracking.iter().any(|scope| {
            scope.iter().any(|owner| match owner {
                Owner::Struct(n, _) | Owner::Array(n, _, _) | Owner::String(n) => n == name,
            })
        })
    }

    fn bounds_check_array(
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
            IntPredicate::SLT, idx_ext, zero_val, &format!("{label}_lt_zero"),
        )?;
        let ge_len = self.builder.build_int_compare(
            IntPredicate::SGE, idx_ext, len_val, &format!("{label}_ge_len"),
        )?;
        let oob = self.builder.build_or(lt_zero, ge_len, &format!("{label}_oob"))?;
        let current_fn = self.current_fn_val();
        let cont_block = self.context.append_basic_block(current_fn, &format!("{label}_cont"));
        let abort_block = self.context.append_basic_block(current_fn, &format!("{label}_oob_abort"));
        self.builder.build_conditional_branch(oob, abort_block, cont_block)?;
        self.builder.position_at_end(abort_block);
        self.builder.build_call(self.abort_fn, &[], &format!("{label}_abort"))?;
        self.builder.build_unreachable()?;
        self.builder.position_at_end(cont_block);
        Ok(())
    }

    fn transfer_ownership(&mut self, src_name: &str, dst_name: &str, struct_name: &str) {
        self.owner_scope_mut().push(Owner::Struct(dst_name.to_string(), struct_name.to_string()));
        for scope in &mut self.owner_tracking {
            scope.retain(|owner| !matches!(owner, Owner::Struct(n, _) if n == src_name));
        }
    }

    fn remove_struct_owner(&mut self, name: &str) {
        for scope in &mut self.owner_tracking {
            scope.retain(|owner| !matches!(owner, Owner::Struct(n, _) if n == name));
        }
    }

    fn lookup_value(&self, name: &str) -> Option<&(PointerValue<'ctx>, BasicTypeEnum<'ctx>)> {
        for scope in self.values.iter().rev() {
            if let Some(v) = scope.get(name) {
                return Some(v);
            }
        }
        None
    }

    fn lookup_var_type(&self, name: &str) -> Option<&Type> {
        for scope in self.var_types.iter().rev() {
            if let Some(t) = scope.get(name) {
                return Some(t);
            }
        }
        None
    }

    fn lookup_array_info(&self, name: &str) -> Option<&(Type, usize)> {
        for scope in self.array_info.iter().rev() {
            if let Some(v) = scope.get(name) {
                return Some(v);
            }
        }
        None
    }

    fn insert_value(&mut self, name: String, val: (PointerValue<'ctx>, BasicTypeEnum<'ctx>)) {
        self.values_mut().insert(name, val);
    }

    fn insert_var_type(&mut self, name: String, ty: Type) {
        self.var_types_mut().insert(name, ty);
    }

    fn insert_array_info(&mut self, name: String, info: (Type, usize)) {
        self.array_info_mut().insert(name, info);
    }

    fn exit_scope(&mut self, struct_field_types: &StructFieldTypes) -> Result<()> {
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
            if let HirStmt::Fn {
                name, params, ret, ..
            } = stmt
            {
                self.declare_function(name, params, ret, struct_field_types);
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
                let fn_val = self.functions.get(name)
                    .copied()
                    .ok_or_else(|| anyhow::anyhow!("undefined function `{}`", name))?;
                
                if !decorators.is_empty() {
                    let wrapper_val = self.generate_http_wrapper(name, fn_val, params, ret)?;
                    for dec in decorators {
                        if let Some(path) = &dec.arg {
                            self.routes.push((dec.name.clone(), path.clone(), wrapper_val));
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

    fn generate_rest_main(&mut self) -> Result<()> {
        let i32_ty = self.context.i32_type();
        let main_type = i32_ty.fn_type(&[], false);
        let main_fn = self.module.add_function("main", main_type, None);
        let bb = self.context.append_basic_block(main_fn, "entry");
        self.builder.position_at_end(bb);

        let i8_ptr_ty = self.context.i8_type().ptr_type(inkwell::AddressSpace::default());

        for (method, path, handler_fn) in &self.routes {
            let method_global = self.builder.build_global_string_ptr(method, "method").unwrap();
            let path_global = self.builder.build_global_string_ptr(path, "path").unwrap();
            
            let handler_ptr = self.builder.build_pointer_cast(handler_fn.as_global_value().as_pointer_value(), i8_ptr_ty, "handler_cast").unwrap();

            self.builder.build_call(
                self.rest_register_route_fn.unwrap(),
                &[
                    method_global.as_pointer_value().into(),
                    path_global.as_pointer_value().into(),
                    handler_ptr.into(),
                ],
                "reg",
            ).unwrap();
        }

        // Call the user's main if it exists
        if let Some(user_main) = self.functions.get("main") {
            self.builder.build_call(*user_main, &[], "call_user_main").unwrap();
        }

        self.builder.build_call(
            self.rest_start_server_fn.unwrap(),
            &[i32_ty.const_int(8080, false).into()],
            "start",
        ).unwrap();

        self.builder.build_return(Some(&i32_ty.const_int(0, false))).unwrap();
        Ok(())
    }

    fn generate_http_wrapper(
        &mut self,
        name: &str,
        original_fn: FunctionValue<'ctx>,
        params: &[(String, Type)],
        ret: &Type,
    ) -> Result<FunctionValue<'ctx>> {
        let i8_ptr_type = self.context.i8_type().ptr_type(inkwell::AddressSpace::default());
        let wrapper_type = i8_ptr_type.fn_type(&[i8_ptr_type.into()], false);
        let wrapper_name = format!("__rest_http_wrapper_{}", name);
        let wrapper_fn = self.module.add_function(&wrapper_name, wrapper_type, Some(inkwell::module::Linkage::Internal));
        
        let bb = self.context.append_basic_block(wrapper_fn, "entry");
        let prev_bb = self.builder.get_insert_block();
        self.builder.position_at_end(bb);
        
        let mut args: Vec<inkwell::values::BasicMetadataValueEnum<'ctx>> = Vec::new();
        let body_param = wrapper_fn.get_nth_param(0).unwrap();
        if params.len() == 1 && params[0].1 == Type::String {
            // Retain before passing to original_fn, since original_fn will release it
            self.builder.build_call(self.retain_fn, &[body_param.into()], "retain_body").unwrap();
            args.push(body_param.into());
        }
        
        let call_site = self.builder.build_call(original_fn, &args, "call_orig").unwrap();
        
        // The wrapper owns body_param (from tiny_http), so it must release it before returning
        self.builder.build_call(self.release_fn, &[body_param.into()], "release_body").unwrap();

        if *ret == Type::String {
            let res_val = call_site.try_as_basic_value().basic().unwrap();
            self.builder.build_return(Some(&res_val)).unwrap();
        } else {
            self.builder.build_return(Some(&i8_ptr_type.const_null())).unwrap();
        }
        
        if let Some(pbb) = prev_bb {
            self.builder.position_at_end(pbb);
        }
        Ok(wrapper_fn)
    }

    // ---- Safe access helpers ----

    fn owner_scope_mut(&mut self) -> &mut Vec<Owner> {
        self.owner_tracking.last_mut()
            .expect("owner_tracking: enter_scope() must be called before use")
    }

    fn values_mut(&mut self) -> &mut HashMap<String, (PointerValue<'ctx>, BasicTypeEnum<'ctx>)> {
        self.values.last_mut()
            .expect("values: enter_scope() must be called before use")
    }

    fn var_types_mut(&mut self) -> &mut HashMap<String, Type> {
        self.var_types.last_mut()
            .expect("var_types: enter_scope() must be called before use")
    }

    fn array_info_mut(&mut self) -> &mut HashMap<String, (Type, usize)> {
        self.array_info.last_mut()
            .expect("array_info: enter_scope() must be called before use")
    }

    fn current_fn_val(&self) -> FunctionValue<'ctx> {
        self.current_fn
            .expect("current_fn: must be set inside compile_fn_body")
    }

    fn build_struct_types(&mut self, stmts: &[HirStmt], struct_field_types: &StructFieldTypes) {
        // First pass: declared structs
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
                self.struct_types.insert(name.clone(), struct_ty);
            }
        }
        // Second pass: implicit structs (used in literals without declaration)
        for (name, fields) in struct_field_types {
            if !self.struct_types.contains_key(name) {
                let field_tys: Vec<BasicTypeEnum> = fields
                    .iter()
                    .map(|(_, ty)| self.hir_type_to_basic(ty, struct_field_types))
                    .collect();
                let struct_ty = self.context.opaque_struct_type(name);
                struct_ty.set_body(&field_tys, false);
                self.struct_types.insert(name.clone(), struct_ty);
            }
        }
    }

    fn declare_function(
        &mut self,
        name: &str,
        params: &[(String, Type)],
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
            self.context.i32_type().fn_type(&llvm_param_tys, false)
        } else if *ret == Type::Void {
            self.context.void_type().fn_type(&llvm_param_tys, false)
        } else if matches!(ret, Type::Struct(_)) {
            // Structs are heap-allocated; return a pointer
            i8_ptr.fn_type(&llvm_param_tys, false)
        } else {
            let llvm_ret = self.hir_type_to_basic(ret, struct_field_types);
            llvm_ret.fn_type(&llvm_param_tys, false)
        };
        let actual_name = if name == "main" && self.has_routes { "__rest_user_main" } else { name };
        let fn_val = self.module.add_function(actual_name, fn_type, None);
        self.functions.insert(name.to_string(), fn_val);
    }

    fn compile_fn_body(
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
            let param = fn_val.get_nth_param(i as u32)
                .with_context(|| format!("parameter index {} out of bounds for fn `{}`", i, self.current_fn_name))?;
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
                self.builder.build_return(Some(&self.context.i32_type().const_int(0, false)))?;
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

    // ---- Statements ----

    fn compile_hir_stmt(
        &mut self,
        stmt: &HirStmt,
        struct_field_types: &StructFieldTypes,
    ) -> Result<()> {
        match stmt {
            HirStmt::Let { name, ty, init, owner, .. } => {
                self.compile_let(name, ty, init, *owner, struct_field_types)
            }
            HirStmt::Expr(expr, _) => {
                let result = self.compile_expr(expr, struct_field_types)?;
                if matches!(expr, HirExpr::AllocStruct(..) | HirExpr::ArrayLiteral(..) | HirExpr::Binary { .. } | HirExpr::Call(..))
                    && result.is_pointer_value() {
                        {
        let do_free = self.builder.build_call(self.release_fn, &[result.into()], "release_free_expr_tmp").unwrap().try_as_basic_value().basic().unwrap().into_int_value();
        let do_free_bool = self.builder.build_int_compare(inkwell::IntPredicate::NE, do_free, self.context.i32_type().const_zero(), "do_free_bool").unwrap();
        let then_bb = self.context.append_basic_block(self.builder.get_insert_block().unwrap().get_parent().unwrap(), "free_free_expr_tmp_block");
        let merge_bb = self.context.append_basic_block(self.builder.get_insert_block().unwrap().get_parent().unwrap(), "merge_free_free_expr_tmp");
        self.builder.build_conditional_branch(do_free_bool, then_bb, merge_bb).unwrap();
        self.builder.position_at_end(then_bb);
        let _ = self.builder.build_call(self.rest_free_fn, &[result.into()], "free_expr_tmp");
        self.builder.build_unconditional_branch(merge_bb).unwrap();
        self.builder.position_at_end(merge_bb);
    }
                    }
                Ok(())
            }
            HirStmt::Fn { .. } | HirStmt::Struct { .. } => Ok(()),
            HirStmt::If { cond, then, else_, .. } => {
                self.compile_if(cond, then, else_.as_deref(), struct_field_types)
            }
            HirStmt::While { cond, body, .. } => {
                self.compile_while(cond, body, struct_field_types)
            }
            HirStmt::For { var, var_ty, lo, hi, body, .. } => {
                self.compile_for(var, var_ty, lo, hi, body, struct_field_types)
            }
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
                            Owner::Struct(n, _) | Owner::Array(n, _, _) | Owner::String(n) => n != name,
                        });
                    }
                }
        let val = self.compile_expr(v, struct_field_types)?;
        // String literals produce global constants that are copied into new allocations by compile_string_literal;
        // The expression is already owned (rc=1), so no need to retain it again!
        let ret_val = match v {
                    HirExpr::FieldLoad { struct_name, index, .. } => {
                        if let Some(fields) = struct_field_types.get(struct_name)
                            && let Some((_, field_type)) = fields.get(*index)
                        {
                            if *field_type == Type::String {
                                // Strdup string field before parent struct is freed
                                self.builder.build_call(self.retain_fn, &[val.into()], "ret_field_retain")?
                                    .try_as_basic_value().basic().expect("__rest_retain should return a basic value")
                            } else if matches!(field_type, Type::Struct(_)) {
                                // Deep-copy struct field before parent is freed
                                self.deep_copy_loaded(val, field_type, struct_field_types)?
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

    fn compile_let(
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
            // owner is set only for Struct types in the Lowerer. If a
            // future change sets owner=true for a non-Struct type, we
            // skip the owner-tracking push rather than bailing out —
            // the codegen can still produce a working program.
            if let Type::Struct(struct_name) = ty {
                self.owner_scope_mut().push(Owner::Struct(name.to_string(), struct_name.clone()));
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
                let heap_ptr = self.compile_alloc_struct(struct_name, fields, struct_field_types)?;
                self.builder.build_store(alloca, heap_ptr)?;
            }
            HirExpr::ArrayLiteral(ty, elements, _) => {
                let heap_ptr = self.compile_array_literal(ty, elements, struct_field_types)?;
                self.builder.build_store(alloca, heap_ptr)?;
                self.insert_array_info(name.to_string(), (*ty.clone(), elements.len()));
                self.owner_scope_mut().push(Owner::Array(name.to_string(), *ty.clone(), elements.len()));
            }
            HirExpr::Ident { name: src_name, .. } => {
                if let Some(&(src_alloca, _)) = self.lookup_value(src_name) {
                    let src_is_owner = self.is_owner(src_name);
                    let loaded = self.builder.build_load(load_ty, src_alloca, src_name)?;
                    if is_string && src_is_owner {
                        let dup = self.builder.build_call(self.retain_fn, &[loaded.into()], "let_retain")?;
                        let dup_val = dup.try_as_basic_value().basic().expect("__rest_retain should return a basic value");
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
                    // Field loads extract owned data (string pointer) from parent → retain
                    // to prevent double-free when both parent and let-binding are freed.
                    let dup = self.builder.build_call(self.retain_fn, &[val.into()], "let_retain")?;
                    dup.try_as_basic_value().basic().expect("__rest_retain should return a basic value")
                } else if let Type::Struct(_) = ty {
                    // Struct values from field loads must be deep-copied
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

    fn bool_from_value(&self, val: BasicValueEnum<'ctx>) -> Result<inkwell::values::IntValue<'ctx>> {
        if val.is_int_value() {
            let iv = val.into_int_value();
            let zero = iv.get_type().const_zero();
            Ok(self.builder.build_int_compare(IntPredicate::NE, iv, zero, "boolval")?)
        } else if val.is_float_value() {
            let fv = val.into_float_value();
            let zero = fv.get_type().const_zero();
            Ok(self.builder.build_float_compare(FloatPredicate::ONE, fv, zero, "boolval")?)
        } else if val.is_pointer_value() {
            let pv = val.into_pointer_value();
            Ok(self.builder.build_is_not_null(pv, "boolval")?)
        } else {
            anyhow::bail!("value cannot be used as boolean condition")
        }
    }

    fn compile_if(
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
            self.builder.build_conditional_branch(i1, then_bb, else_bb)?;

            self.builder.position_at_end(then_bb);
            self.enter_scope();
            for stmt in then_stmts {
                self.compile_hir_stmt(stmt, struct_field_types)?;
            }
            self.exit_scope(struct_field_types)?;
            if self.builder.get_insert_block().map(|bb| bb.get_terminator().is_none()).unwrap_or(false) {
                self.builder.build_unconditional_branch(merge_bb)?;
            }

            self.builder.position_at_end(else_bb);
            self.enter_scope();
            for stmt in else_stmts {
                self.compile_hir_stmt(stmt, struct_field_types)?;
            }
            self.exit_scope(struct_field_types)?;
            if self.builder.get_insert_block().map(|bb| bb.get_terminator().is_none()).unwrap_or(false) {
                self.builder.build_unconditional_branch(merge_bb)?;
            }
        } else {
            self.builder.build_conditional_branch(i1, then_bb, merge_bb)?;

            self.builder.position_at_end(then_bb);
            self.enter_scope();
            for stmt in then_stmts {
                self.compile_hir_stmt(stmt, struct_field_types)?;
            }
            self.exit_scope(struct_field_types)?;
            if self.builder.get_insert_block().map(|bb| bb.get_terminator().is_none()).unwrap_or(false) {
                self.builder.build_unconditional_branch(merge_bb)?;
            }
        }

        self.builder.position_at_end(merge_bb);
        Ok(())
    }

    fn compile_while(
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
        let needs_back_edge = self.builder
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

    fn compile_for(
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
        self.builder.build_conditional_branch(has_work, body_bb, end_bb)?;

        self.insert_value(var.to_string(), (alloca, llvm_ty));

        self.builder.position_at_end(body_bb);
        self.enter_scope();
        for stmt in body {
            self.compile_hir_stmt(stmt, struct_field_types)?;
        }
        self.exit_scope(struct_field_types)?;
        let needs_inc = self.builder
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
        let inc = self.builder.build_int_add(next, int_ty.const_int(1, false), "inc")?;
        self.builder.build_store(alloca, inc)?;
        self.builder.build_unconditional_branch(cond_bb)?;

        self.loop_stack.pop();
        self.builder.position_at_end(end_bb);
        Ok(())
    }

    // ---- Expressions ----

    fn compile_expr(
        &mut self,
        expr: &HirExpr,
        struct_field_types: &StructFieldTypes,
    ) -> Result<BasicValueEnum<'ctx>> {
        match expr {
            HirExpr::Int(v, ty, _) => {
                let basic = self.hir_type_to_basic(ty, struct_field_types);
                // SAFETY: *v as u64 followed by const_int correctly produces two's complement
                // representation for both signed and unsigned types. LLVM truncates to the target
                // bit width (e.g., -1i64 → 0xFF..FF → const_int on i8 → 0xFF = -1i8).
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
            HirExpr::Bool(v, _) => Ok(self.context.bool_type().const_int(if *v { 1 } else { 0 }, false).into()),
            HirExpr::String(s, _) => Ok(self.compile_string_literal(s)),
            HirExpr::Ident { name, .. } => self.compile_ident(name),
            HirExpr::AllocStruct(struct_name, fields, _) => {
                self.compile_alloc_struct(struct_name, fields, struct_field_types)
            }
            HirExpr::Call(callee, args, _) => self.compile_call(callee, args, struct_field_types),
            HirExpr::FieldLoad { object, index, struct_name, .. } => {
                self.compile_field_load(object, *index, struct_name, struct_field_types)
            }
            HirExpr::Unary(op, inner, _) => self.compile_unary(*op, inner, struct_field_types),
            HirExpr::ArrayIndex { object, index, .. } => {
                self.compile_array_index(object, index, struct_field_types)
            }
            HirExpr::ArrayLiteral(ty, elements, _) => {
                self.compile_array_literal(ty, elements, struct_field_types)
            }
            HirExpr::Binary { lhs, op, rhs, ty, .. } => {
                self.compile_binary(lhs, *op, rhs, ty, struct_field_types)
            }
            HirExpr::Assign { lhs, rhs, .. } => self.compile_assign(lhs, rhs, struct_field_types),
            HirExpr::Print(arg, _) => self.compile_print(arg, struct_field_types),
        }
    }

    fn compile_unary(
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

    fn dup_string_expr(&mut self, expr: &HirExpr, field_type: &Type, struct_field_types: &StructFieldTypes) -> Result<Option<BasicValueEnum<'ctx>>> {
        if !matches!(field_type, Type::String) {
            return Ok(None);
        }
        let ptr_val = match expr {
            HirExpr::String(s, _) => self.compile_string_literal(s),
            _ => self.compile_expr(expr, struct_field_types)?
        };
        let result = self
            .builder
            .build_call(self.retain_fn, &[ptr_val.into()], "retain")?
            .try_as_basic_value();
        let bv = result
            .basic()
            .expect("__rest_retain should return a basic value");
        Ok(Some(bv))
    }

    fn deep_copy_loaded(
        &mut self,
        val: BasicValueEnum<'ctx>,
        elem_type: &Type,
        struct_field_types: &StructFieldTypes,
    ) -> Result<BasicValueEnum<'ctx>> {
        match elem_type {
            Type::String => {
                let result = self
                    .builder
                    .build_call(self.retain_fn, &[val.into()], "arr_retain")?
                    .try_as_basic_value()
                    .basic()
                    .expect("__rest_retain should return a basic value");
                Ok(result)
            }
            Type::Struct(struct_name) => {
                let struct_ty = *self.struct_types.get(struct_name)
                    .ok_or_else(|| anyhow::anyhow!("undefined struct `{}`", struct_name))?;
                let size = struct_ty.size_of()
                    .unwrap_or_else(|| self.context.i64_type().const_int(8, false));
                let malloc_args = &[size.into()];
                let heap_ptr = self.builder.build_call(self.rest_alloc_fn, malloc_args, "struct_copy_malloc")?
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
                        if let Ok(gep) = self.builder.build_struct_gep(struct_ty, ptr, i as u32, "copy_field")
                            && let Ok(field_val) = self.builder.build_load(self.ptr_ty(), gep, "copy_field_val")
                        {
                            if matches!(ty, Type::String) {
                                let dup = self.builder.build_call(self.retain_fn, &[field_val.into()], "copy_field_retain")?
                                    .try_as_basic_value().basic().expect("__rest_retain should return a basic value");
                                self.builder.build_store(gep, dup)?;
                            } else if let Type::Struct(..) = ty {
                                let inner_copy = self.deep_copy_loaded(field_val, ty, struct_field_types)?;
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

    fn compile_string_literal(&self, s: &str) -> BasicValueEnum<'ctx> {
        let global = self.builder.build_global_string_ptr(s, "str")
            .expect("build_global_string_ptr failed");
        let ptr = global.as_pointer_value();
        let i8_ptr_ty = self.ptr_ty();
        let casted = self
            .builder
            .build_pointer_cast(ptr, i8_ptr_ty, "str_cast")
            .expect("build_pointer_cast to i8* failed");

        let len = self.context.i64_type().const_int(s.len() as u64, false);
        let plus_one = self.builder.build_int_add(len, self.context.i64_type().const_int(1, false), "plus_one").unwrap();
        let alloc_ptr = self.builder.build_call(self.rest_alloc_fn, &[plus_one.into()], "alloc_str")
            .unwrap().try_as_basic_value().basic().unwrap().into_pointer_value();

        self.builder.build_call(self.memcpy_fn, &[alloc_ptr.into(), casted.into(), plus_one.into(), self.context.bool_type().const_zero().into()], "memcpy").unwrap();

        alloc_ptr.into()
    }

    fn compile_ident(
        &mut self,
        name: &str,
    ) -> Result<BasicValueEnum<'ctx>> {
        let &(alloca, load_ty) = self
            .lookup_value(name)
            .ok_or_else(|| anyhow::anyhow!("undefined variable `{}`", name))?;
        let loaded = self.builder.build_load(load_ty, alloca, name)?;
        Ok(loaded)
    }

    fn compile_ref(
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
                    // Heap-allocated (struct/string/array): alloca stores a ptr, load it
                    let loaded = self.builder.build_load(self.ptr_ty(), alloca, name)?;
                    Ok(loaded)
                } else {
                    // Stack-allocated (int/float/bool): alloca stores the value,
                    // return the address of the alloca itself
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
                // Allocate a temp of the correct type, store the value, return pointer to it
                let val_ty = val.get_type();
                let alloca = self.builder.build_alloca(val_ty, "ref_tmp")?;
                self.builder.build_store(alloca, val)?;
                let casted = self.builder.build_pointer_cast(
                    alloca,
                    self.ptr_ty(),
                    "ref_tmp_cast",
                )?;
                Ok(casted.into())
            }
        }
    }

    fn compile_alloc_struct(
        &mut self,
        struct_name: &str,
        fields: &[(String, HirExpr)],
        struct_field_types: &StructFieldTypes,
    ) -> Result<BasicValueEnum<'ctx>> {
        let struct_ty = *self.struct_types.get(struct_name)
            .ok_or_else(|| anyhow::anyhow!("undefined struct `{}`", struct_name))?;
        let size_val = struct_ty.size_of().unwrap_or_else(|| self.context.i64_type().const_int(8, false));
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
            let field_type = match struct_field_list
                .and_then(|f| f.get(i))
                .map(|(_, ty)| ty)
            {
                Some(ty) => ty.clone(),
                None => {
                    // typeck should have validated that the field exists.
                    // If a regression lets an out-of-bounds field through,
                    // skip the field rather than crashing the compiler.
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

    fn compile_array_literal(
        &mut self,
        ty: &Type,
        elements: &[HirExpr],
        struct_field_types: &StructFieldTypes,
    ) -> Result<BasicValueEnum<'ctx>> {
        let elem_ty = self.hir_type_to_basic(ty, struct_field_types);
        let elem_size = elem_ty.size_of().unwrap_or_else(|| self.context.i64_type().const_int(8, false));
        let count_val = self.context.i64_type().const_int(elements.len() as u64, false);
        let total_size = self.builder.build_int_mul(elem_size, count_val, "array_total_size")?;
        let malloc_args = &[total_size.into()];
        let heap_ptr = self
            .builder
            .build_call(self.rest_alloc_fn, malloc_args, "array_malloc")?
            .try_as_basic_value()
            .basic()
            .expect("malloc should return a basic pointer value")
            .into_pointer_value();
        let typed_ptr = self.builder.build_pointer_cast(
            heap_ptr,
            self.ptr_ty(),
            "array_typed",
        )?;
        for (i, elem) in elements.iter().enumerate() {
            // SAFETY: GEP on typed array pointer — element type matches, index is in bounds
            let gep = unsafe {
                self.builder.build_gep(elem_ty, typed_ptr, &[
                    self.context.i32_type().const_int(i as u64, false),
                ], "array_elt")?
            };
            // Duplicate string elements so each array entry owns its copy
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

    fn compile_array_index(
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
                    // SAFETY: GEP on typed array pointer — element type matches, index verified by bounds_check_array
                    let gep = unsafe {
                        self.builder.build_gep(elem_llvm_ty, typed_ptr, &[idx_val.into_int_value()], "array_idx")?
                    };
                    let loaded = self.builder.build_load(elem_llvm_ty, gep, "array_elt")?;
                    let result = self.deep_copy_loaded(loaded, &elem_type, struct_field_types)?;
                    Ok(result)
                } else {
                    // Fallback for non-heap arrays (e.g. param arrays)
                    let ptr = arr_ptr.into_pointer_value();
                    // SAFETY: GEP on void pointer — accessed as i8*, load treated as pointer
                    let gep = unsafe {
                        self.builder.build_gep(
                            self.context.i8_type(),
                            ptr,
                            &[idx_val.into_int_value()],
                            "array_idx",
                        )?
                    };
                    let loaded = self.builder.build_load(self.ptr_ty(), gep, "array_elt")?;
                    Ok(loaded)
                }
            }
            _ => {
                if let HirExpr::ArrayLiteral(_, elements, _) = object {
                    self.bounds_check_array(idx_val.into_int_value(), elements.len(), "array_idx_read")?;
                }
                let val = self.compile_expr(object, struct_field_types)?;
                let ptr = val.into_pointer_value();
                // Determine element type from HIR when possible (inline literals)
                let (elem_llvm_ty, elem_type, use_deep_copy) = match object {
                    HirExpr::ArrayLiteral(ty, _, _) => {
                        let ty_ref: &Type = ty.as_ref();
                        let llvm = self.hir_type_to_basic(ty_ref, struct_field_types);
                        (llvm, Some(ty_ref.clone()), matches!(ty_ref, Type::String | Type::Struct(_)))
                    }
                    HirExpr::AllocStruct(..) => {
                        // AllocStruct returns a pointer; indexing makes no sense here
                        (self.ptr_ty().into(), None, false)
                    }
                    _ => (self.context.i8_type().into(), None, false),
                };
                let typed_ptr = self.builder.build_pointer_cast(ptr, self.ptr_ty(), "array_idx_typed")?;
                let gep = unsafe {
                    self.builder.build_gep(elem_llvm_ty, typed_ptr, &[idx_val.into_int_value()], "array_idx")?
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

    fn compile_call(
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
        let mut temp_allocs: Vec<BasicValueEnum<'ctx>> = Vec::new(); // C2: track temp heap allocs
        for (i, arg) in args.iter().enumerate() {
            if let Some(param) = fn_val.get_nth_param(i as u32)
                && param.get_type().is_pointer_type()
                    && let HirExpr::Ident { name, .. } = arg
                        && let Some(&(alloca, _)) = self.lookup_value(name) {
                            let loaded = self.builder.build_load(self.ptr_ty(), alloca, name)?;
                            llvm_args.push(loaded.into());
                            continue;
                        }
            let val = self.compile_expr(arg, struct_field_types)?;
            if matches!(arg, HirExpr::AllocStruct(..) | HirExpr::ArrayLiteral(..) | HirExpr::Call(..) | HirExpr::Binary { .. })
                && val.is_pointer_value() {
                temp_allocs.push(val);
            }
            llvm_args.push(val.into());
        }

        let call_site = self.builder.build_call(fn_val, &llvm_args, callee)?;
        // C2: free temporary AllocStruct/ArrayLiteral args after call
        for val in temp_allocs {
            {
        let do_free = self.builder.build_call(self.release_fn, &[val.into()], "release_free_temp_arg").unwrap().try_as_basic_value().basic().unwrap().into_int_value();
        let do_free_bool = self.builder.build_int_compare(inkwell::IntPredicate::NE, do_free, self.context.i32_type().const_zero(), "do_free_bool").unwrap();
        let then_bb = self.context.append_basic_block(self.builder.get_insert_block().unwrap().get_parent().unwrap(), "free_free_temp_arg_block");
        let merge_bb = self.context.append_basic_block(self.builder.get_insert_block().unwrap().get_parent().unwrap(), "merge_free_free_temp_arg");
        self.builder.build_conditional_branch(do_free_bool, then_bb, merge_bb).unwrap();
        self.builder.position_at_end(then_bb);
        let _ = self.builder.build_call(self.rest_free_fn, &[val.into()], "free_temp_arg");
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

    fn compile_print(
        &mut self,
        arg: &HirExpr,
        struct_field_types: &StructFieldTypes,
    ) -> Result<BasicValueEnum<'ctx>> {
        let val = self.compile_expr(arg, struct_field_types)?;
        let i8_ptr = self.ptr_ty();
        let i32_ty = self.context.i32_type();

        let (arg, fmt_str) = if val.is_float_value() {
            let ft = val.into_float_value();
            let ft_ty = ft.get_type();
            let is_f32 = ft_ty.get_bit_width() == 32;
            let promoted: BasicValueEnum = if is_f32 {
                self.builder.build_float_ext(ft, self.context.f64_type(), "f32_to_f64")?.into()
            } else {
                ft.into()
            };
            (promoted, self.builder.build_global_string_ptr("%f\n", "fmt_f64")?)
        } else if val.is_pointer_value() {
            (val, self.builder.build_global_string_ptr("%s\n", "fmt_str")?)
        } else {
            let iv = val.into_int_value();
            let iv_ty = iv.get_type();
            let width = iv_ty.get_bit_width();
            let is_unsigned = match arg {
                HirExpr::Bool(_, _) => true,
                HirExpr::Int(_, ty, _) => Self::is_unsigned_type(ty),
                HirExpr::Ident { name, .. } => self.lookup_var_type(name)
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
                (ext.into(), self.builder.build_global_string_ptr(fmt, "fmt_i32")?)
            } else if width == 32 {
                let fmt = if is_unsigned { "%u\n" } else { "%d\n" };
                (iv.into(), self.builder.build_global_string_ptr(fmt, "fmt_i32")?)
            } else {
                let fmt = if is_unsigned { "%lu\n" } else { "%ld\n" };
                (iv.into(), self.builder.build_global_string_ptr(fmt, "fmt_i64")?)
            }
        };

        let fmt_ptr = fmt_str.as_pointer_value();
        let casted = self
            .builder
            .build_pointer_cast(fmt_ptr, i8_ptr, "fmt_cast")?;
        self.builder
            .build_call(self.printf_fn, &[casted.into(), arg.into()], "printf_call")?;

        Ok(self.context.i32_type().const_zero().into())
    }

    fn compile_field_load_ptr(
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
            _ => {
                let val = self.compile_expr(object, struct_field_types)?;
                val.into_pointer_value()
            }
        };
        Ok(struct_ptr)
    }

    fn compile_field_load(
        &mut self,
        object: &HirExpr,
        index: usize,
        struct_name: &str,
        struct_field_types: &StructFieldTypes,
    ) -> Result<BasicValueEnum<'ctx>> {
        let struct_ptr = self.compile_field_load_ptr(object, struct_name, struct_field_types)?;
        let struct_ty = *self.struct_types.get(struct_name)
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
                // typeck should have rejected out-of-bounds field
                // accesses. If a regression lets one through, return
                // a zero i32 rather than crashing the compiler.
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

    fn is_unsigned_type(ty: &Type) -> bool {
        matches!(ty, Type::U8 | Type::U16 | Type::U32 | Type::U64)
    }

    /// LLVM requires shift amount to have the same bit width as the value.
    /// Extend or truncate `rhs` to match `lhs`'s width (zero-extend, since shift amounts are unsigned).
    fn match_int_width(
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

    fn compile_binary(
        &mut self,
        lhs: &HirExpr,
        op: BinOp,
        rhs: &HirExpr,
        ty: &Type,
        struct_field_types: &StructFieldTypes,
    ) -> Result<BasicValueEnum<'ctx>> {
        let i32_ty = self.context.i32_type();

        if op == BinOp::And {
            // short-circuit &&
            let current_block = self.builder.get_insert_block()
                .expect("compile_binary &&: must be inside a function body");
            let current_fn = current_block.get_parent()
                .expect("basic block must belong to a function");

            let bool_ty = self.context.bool_type();
            let rhs_block = self.context.append_basic_block(current_fn, "and_rhs");
            let merge_block = self.context.append_basic_block(current_fn, "and_merge");
            let false_block = self.context.append_basic_block(current_fn, "and_false");

            let l = self.compile_expr(lhs, struct_field_types)?;
            let l_is_true = self.bool_from_value(l)?;
            self.builder.build_conditional_branch(l_is_true, rhs_block, false_block)?;

            // rhs block
            self.builder.position_at_end(rhs_block);
            let r = self.compile_expr(rhs, struct_field_types)?;
            let r_is_true = self.bool_from_value(r)?;
            self.builder.build_unconditional_branch(merge_block)?;

            // false block
            self.builder.position_at_end(false_block);
            self.builder.build_unconditional_branch(merge_block)?;

            // merge block
            self.builder.position_at_end(merge_block);
            let phi = self.builder.build_phi(bool_ty, "and_result")?;
            phi.add_incoming(&[
                (&r_is_true, rhs_block),
                (&bool_ty.const_zero(), false_block),
            ]);
            return Ok(phi.as_basic_value());
        }

        if op == BinOp::Or {
            // short-circuit ||
            let current_block = self.builder.get_insert_block()
                .expect("compile_binary ||: must be inside a function body");
            let current_fn = current_block.get_parent()
                .expect("basic block must belong to a function");

            let bool_ty = self.context.bool_type();
            let rhs_block = self.context.append_basic_block(current_fn, "or_rhs");
            let merge_block = self.context.append_basic_block(current_fn, "or_merge");
            let true_block = self.context.append_basic_block(current_fn, "or_true");

            let l = self.compile_expr(lhs, struct_field_types)?;
            let l_is_true = self.bool_from_value(l)?;
            self.builder.build_conditional_branch(l_is_true, true_block, rhs_block)?;

            // rhs block
            self.builder.position_at_end(rhs_block);
            let r = self.compile_expr(rhs, struct_field_types)?;
            let r_is_true = self.bool_from_value(r)?;
            self.builder.build_unconditional_branch(merge_block)?;

            // true block
            self.builder.position_at_end(true_block);
            self.builder.build_unconditional_branch(merge_block)?;

            // merge block
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
            let result = self.builder.build_call(
                self.strcat_fn,
                &[l.into(), r.into()],
                "strcat",
            )?;
            return Ok(result.try_as_basic_value().basic().expect("__ref_strcat should return a basic value"));
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
                    let cmp = self.builder.build_float_compare(FloatPredicate::OEQ, lf, rf, "eq")?;
                    self.builder.build_int_z_extend(cmp, i32_ty, "eq_ext")?.into()
                }
                BinOp::Ne => {
                    let cmp = self.builder.build_float_compare(FloatPredicate::ONE, lf, rf, "ne")?;
                    self.builder.build_int_z_extend(cmp, i32_ty, "ne_ext")?.into()
                }
                BinOp::Lt => {
                    let cmp = self.builder.build_float_compare(FloatPredicate::OLT, lf, rf, "lt")?;
                    self.builder.build_int_z_extend(cmp, i32_ty, "lt_ext")?.into()
                }
                BinOp::Le => {
                    let cmp = self.builder.build_float_compare(FloatPredicate::OLE, lf, rf, "le")?;
                    self.builder.build_int_z_extend(cmp, i32_ty, "le_ext")?.into()
                }
                BinOp::Gt => {
                    let cmp = self.builder.build_float_compare(FloatPredicate::OGT, lf, rf, "gt")?;
                    self.builder.build_int_z_extend(cmp, i32_ty, "gt_ext")?.into()
                }
                BinOp::Ge => {
                    let cmp = self.builder.build_float_compare(FloatPredicate::OGE, lf, rf, "ge")?;
                    self.builder.build_int_z_extend(cmp, i32_ty, "ge_ext")?.into()
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
                    let is_zero = self.builder.build_int_compare(IntPredicate::EQ, ri, zero, "div_zero_check")?;
                    let current_fn = self.current_fn_val();
                    let cont_block = self.context.append_basic_block(current_fn, "div_cont");
                    let abort_block = self.context.append_basic_block(current_fn, "div_abort");
                    self.builder.build_conditional_branch(is_zero, abort_block, cont_block)?;
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
                    let is_zero = self.builder.build_int_compare(IntPredicate::EQ, ri, zero, "rem_zero_check")?;
                    let current_fn = self.current_fn_val();
                    let cont_block = self.context.append_basic_block(current_fn, "rem_cont");
                    let abort_block = self.context.append_basic_block(current_fn, "rem_abort");
                    self.builder.build_conditional_branch(is_zero, abort_block, cont_block)?;
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
                    let cmp = self.builder.build_int_compare(IntPredicate::EQ, li, ri, "eq")?;
                    self.builder.build_int_z_extend(cmp, i32_ty, "eq_ext")?.into()
                }
                BinOp::Ne => {
                    let cmp = self.builder.build_int_compare(IntPredicate::NE, li, ri, "ne")?;
                    self.builder.build_int_z_extend(cmp, i32_ty, "ne_ext")?.into()
                }
                BinOp::Lt => {
                    let pred = if unsigned { IntPredicate::ULT } else { IntPredicate::SLT };
                    let cmp = self.builder.build_int_compare(pred, li, ri, "lt")?;
                    self.builder.build_int_z_extend(cmp, i32_ty, "lt_ext")?.into()
                }
                BinOp::Le => {
                    let pred = if unsigned { IntPredicate::ULE } else { IntPredicate::SLE };
                    let cmp = self.builder.build_int_compare(pred, li, ri, "le")?;
                    self.builder.build_int_z_extend(cmp, i32_ty, "le_ext")?.into()
                }
                BinOp::Gt => {
                    let pred = if unsigned { IntPredicate::UGT } else { IntPredicate::SGT };
                    let cmp = self.builder.build_int_compare(pred, li, ri, "gt")?;
                    self.builder.build_int_z_extend(cmp, i32_ty, "gt_ext")?.into()
                }
                BinOp::Ge => {
                    let pred = if unsigned { IntPredicate::UGE } else { IntPredicate::SGE };
                    let cmp = self.builder.build_int_compare(pred, li, ri, "ge")?;
                    self.builder.build_int_z_extend(cmp, i32_ty, "ge_ext")?.into()
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
                    self.builder.build_right_shift(li, ri, unsigned, "shr")?.into()
                }
                BinOp::And | BinOp::Or => {
                    // Unreachable: And/Or are intercepted by the
                    // short-circuit logic at the top of compile_binary.
                    // Returning a dummy i32 keeps the compiler alive
                    // even if a future regression lets one through;
                    // debug_assert catches it in development.
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

    fn compile_assign(
        &mut self,
        lhs: &HirExpr,
        rhs: &HirExpr,
        struct_field_types: &StructFieldTypes,
    ) -> Result<BasicValueEnum<'ctx>> {
        match lhs {
            HirExpr::Ident { name, .. } => {
                if let Some(&(alloca, _)) = self.lookup_value(name) {
                    // Read RHS FIRST to avoid use-after-free on self-assignment (x = x
                    // where x owns heap memory). The value is saved before freeing the
                    // old owner, then deep-copied below if needed.
                    let val = self.compile_expr(rhs, struct_field_types)?;

                    // Self-assignment (x = x) is a no-op — skip free + transfer
                    let is_self_assign = matches!(rhs, HirExpr::Ident { name: n, .. } if n == name);

                    if !is_self_assign {
                        let old_owners: Vec<Owner> = self.owner_tracking
                            .iter()
                            .flat_map(|scope| scope.iter())
                            .filter(|owner| match owner {
                                Owner::Struct(n, _) | Owner::Array(n, _, _) | Owner::String(n) => n == name,
                            })
                            .cloned()
                            .collect();
                        for owner in &old_owners {
                            self.free_owner(owner, struct_field_types);
                        }
                        for scope in &mut self.owner_tracking {
                            scope.retain(|owner| match owner {
                                Owner::Struct(n, _) | Owner::Array(n, _, _) | Owner::String(n) => n != name,
                            });
                        }
                    }

                    self.builder.build_store(alloca, val)?;

                    if !is_self_assign {
                        if let HirExpr::Ident { name: src_name, .. } = rhs {
                            if self.is_owner(src_name) {
                                let src_owners: Vec<Owner> = self.owner_tracking
                                    .iter()
                                    .flat_map(|scope| scope.iter())
                                    .filter(|owner| match owner {
                                        Owner::Struct(n, _) | Owner::Array(n, _, _) | Owner::String(n) => n == src_name,
                                    })
                                    .cloned()
                                    .collect();
                                for src_owner in &src_owners {
                                    match src_owner {
                                        Owner::Struct(_, struct_name) => {
                                            self.owner_scope_mut().push(Owner::Struct(name.to_string(), struct_name.clone()));
                                        }
                                        Owner::Array(_, elem_ty, count) => {
                                            self.owner_scope_mut().push(Owner::Array(name.to_string(), elem_ty.clone(), *count));
                                        }
                                        Owner::String(_) => {
                                            let dup = self.builder.build_call(self.retain_fn, &[val.into()], "assign_retain")?;
                                            let dup_val = dup.try_as_basic_value().basic().expect("__rest_retain should return a basic value");
                                            self.builder.build_store(alloca, dup_val)?;
                                            self.owner_scope_mut().push(Owner::String(name.to_string()));
                                        }
                                    }
                                }
                                if !src_owners.is_empty() {
                                    for scope in &mut self.owner_tracking {
                                        scope.retain(|owner| match owner {
                                            Owner::String(_) => true,
                                            Owner::Struct(n, _) | Owner::Array(n, _, _) => n != src_name,
                                        });
                                    }
                                }
                            }
                        } else if let Some(ty) = self.lookup_var_type(name).cloned() {
                            match ty {
                                Type::Struct(n) => self.owner_scope_mut().push(Owner::Struct(name.to_string(), n)),
                                Type::Array(e, c) => self.owner_scope_mut().push(Owner::Array(name.to_string(), *e, c)),
                                Type::String => self.owner_scope_mut().push(Owner::String(name.to_string())),
                                _ => {}
                            }
                        }
                    }
                }
            }
            HirExpr::FieldLoad { object, index, struct_name, .. } => {
                let struct_ptr = self.compile_field_load_ptr(object, struct_name, struct_field_types)?;
                let struct_ty = *self.struct_types.get(struct_name)
                    .ok_or_else(|| anyhow::anyhow!("undefined struct `{}`", struct_name))?;
                let gep = self.builder.build_struct_gep(struct_ty, struct_ptr, *index as u32, "field_set")?;
                // Free old owner field value before overwriting
                if let Some(fields) = struct_field_types.get(struct_name)
                    && let Some((_, field_type)) = fields.get(*index) {
                        if matches!(field_type, Type::String) {
                            if let Ok(old) = self.builder.build_load(self.ptr_ty(), gep, "old_field") {
                                {
        let do_free = self.builder.build_call(self.release_fn, &[old.into()], "release_free_old_field").unwrap().try_as_basic_value().basic().unwrap().into_int_value();
        let do_free_bool = self.builder.build_int_compare(inkwell::IntPredicate::NE, do_free, self.context.i32_type().const_zero(), "do_free_bool").unwrap();
        let then_bb = self.context.append_basic_block(self.builder.get_insert_block().unwrap().get_parent().unwrap(), "free_free_old_field_block");
        let merge_bb = self.context.append_basic_block(self.builder.get_insert_block().unwrap().get_parent().unwrap(), "merge_free_free_old_field");
        self.builder.build_conditional_branch(do_free_bool, then_bb, merge_bb).unwrap();
        self.builder.position_at_end(then_bb);
        let _ = self.builder.build_call(self.rest_free_fn, &[old.into()], "free_old_field");
        self.builder.build_unconditional_branch(merge_bb).unwrap();
        self.builder.position_at_end(merge_bb);
    }
                            }
                        } else if let Type::Struct(inner_name) = field_type
                            && let Ok(old) = self.builder.build_load(self.ptr_ty(), gep, "old_field") {
                                self.free_struct_ptr(old.into_pointer_value(), inner_name, struct_field_types);
                            }
                    }
                let val = self.compile_expr(rhs, struct_field_types)?;
                if let Some((_, field_type)) = struct_field_types.get(struct_name).and_then(|f| f.get(*index)) {
                    if matches!(field_type, Type::String) {
                        if matches!(rhs, HirExpr::Call(..) | HirExpr::Binary { op: BinOp::Add, .. }) {
                            // Call returns owned string, strcat returns new allocation → take ownership directly
                            self.builder.build_store(gep, val)?;
                        } else {
                            // Pointer to an existing allocation → retain to prevent double-free
                            let dup = self.builder.build_call(self.retain_fn, &[val.into()], "field_retain")?;
                            let dup_val = dup.try_as_basic_value().basic().expect("__rest_retain should return a basic value");
                            self.builder.build_store(gep, dup_val)?;
                        }
                    } else if let Type::Struct(_) = field_type {
                        // Deep-copy struct values to prevent double-free when both
                        // source and destination own the same heap allocation
                        let copied = self.deep_copy_loaded(val, field_type, struct_field_types)?;
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
                        && let Ok(arr_ptr) = self.builder.build_load(self.ptr_ty(), ptr_val, name) {
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
                                self.builder.build_pointer_cast(
                                    arr_ptr,
                                    self.ptr_ty(),
                                    "array_assign_typed",
                                ).ok().and_then(|typed_ptr| {
                                    // SAFETY: GEP on typed array ptr — elem type matches known array_info
                                    unsafe {
                                        self.builder.build_gep(elem_llvm_ty, typed_ptr, &[idx_val.into_int_value()], "array_assign_idx").ok()
                                    }
                                })
                            } else {
                                // SAFETY: GEP on void ptr fallback — accessed as i8*
                                unsafe {
                                    self.builder.build_gep(
                                        self.context.i8_type(),
                                        arr_ptr,
                                        &[idx_val.into_int_value()],
                                        "array_assign_idx",
                                    ).ok()
                                }
                            };
                            if let Some(gep) = gep {
                                // Free old element before overwriting (string/struct only)
                                if let Some(elem_type) = &elem_type
                                    && (matches!(elem_type, Type::String) || matches!(elem_type, Type::Struct(_)))
                                        && let Ok(old) = self.builder.build_load(self.ptr_ty(), gep, "old_elem") {
                                            if let Type::Struct(sname) = elem_type {
                                                self.free_struct_ptr(old.into_pointer_value(), sname, struct_field_types);
                                            } else {
                                                {
        let do_free = self.builder.build_call(self.release_fn, &[old.into()], "release_free_old_elem").unwrap().try_as_basic_value().basic().unwrap().into_int_value();
        let do_free_bool = self.builder.build_int_compare(inkwell::IntPredicate::NE, do_free, self.context.i32_type().const_zero(), "do_free_bool").unwrap();
        let then_bb = self.context.append_basic_block(self.builder.get_insert_block().unwrap().get_parent().unwrap(), "free_free_old_elem_block");
        let merge_bb = self.context.append_basic_block(self.builder.get_insert_block().unwrap().get_parent().unwrap(), "merge_free_free_old_elem");
        self.builder.build_conditional_branch(do_free_bool, then_bb, merge_bb).unwrap();
        self.builder.position_at_end(then_bb);
        let _ = self.builder.build_call(self.rest_free_fn, &[old.into()], "free_old_elem");
        self.builder.build_unconditional_branch(merge_bb).unwrap();
        self.builder.position_at_end(merge_bb);
    };
                                            }
                                        }
                                let val = self.compile_expr(rhs, struct_field_types)?;
                                if let Some(elem_type) = &elem_type
                                    && matches!(elem_type, Type::String) {
                                        // String values from literals/identifiers are pointers
                                        // into the static @str.N pool or to existing
                                        // allocations — NOT a fresh malloc. Strdup to
                                        // give the slot its own heap allocation so the
                                        // end-of-scope free doesn't try to free a
                                        // constant.
                                        // Exception: call/strcat results are already
                                        // freshly malloced by the callee, so take them
                                        // directly to avoid an extra retain round-trip.
                                        if matches!(rhs, HirExpr::Call(..) | HirExpr::Binary { op: BinOp::Add, .. }) {
                                            self.builder.build_store(gep, val)?;
                                        } else {
                                            let dup = self.builder.build_call(self.retain_fn, &[val.into()], "array_elem_retain")?;
                                            let dup_val = dup.try_as_basic_value().basic()
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

    // ---- Type mapping ----

    fn ptr_ty(&self) -> inkwell::types::PointerType<'ctx> {
        self.context.ptr_type(inkwell::AddressSpace::default())
    }

    #[allow(clippy::only_used_in_recursion)]
    fn hir_type_to_basic(
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
            Type::Fn(..) => self.ptr_ty().into(),
            Type::Void => {
                // Type::Void should never reach codegen: typeck rejects
                // Void-typed expressions/values before lowering. If a
                // future change lets it through, the safest fallback is
                // a zero-sized i32 — incorrect, but won't crash the
                // compiler. The debug_assert catches the regression in
                // development builds.
                debug_assert!(false, "Type::Void reached codegen — typeck bug");
                self.context.i32_type().into()
            }
        }
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
