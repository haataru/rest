use std::process::Command;

use inkwell::OptimizationLevel;
use tempfile::TempDir;

fn compile(source: &str) -> Result<TempDir, String> {
    let dir = TempDir::new().map_err(|e| e.to_string())?;
    let output = dir.path().join("out.ll");
    restc::driver::run(source, &output, OptimizationLevel::None).map_err(|e| e.to_string())?;
    Ok(dir)
}

fn compile_and_run(source: &str) -> Result<String, String> {
    let dir = TempDir::new().map_err(|e| e.to_string())?;
    let o_path = dir.path().join("out.o");
    let exe_path = dir.path().join("a.out");
    restc::driver::run(source, &o_path, OptimizationLevel::None).map_err(|e| e.to_string())?;
    let link_result = Command::new("cc")
        .args(["-no-pie", "-o", &exe_path.to_string_lossy(), &o_path.to_string_lossy()])
        .output()
        .map_err(|e| e.to_string())?;
    if !link_result.status.success() {
        let stderr = String::from_utf8_lossy(&link_result.stderr).to_string();
        return Err(format!("linker failed: {}", stderr));
    }
    let result = Command::new(&exe_path).output().map_err(|e| e.to_string())?;
    if !result.status.success() {
        let stderr = String::from_utf8_lossy(&result.stderr).to_string();
        return Err(format!("exit code {:?}: {}", result.status.code(), stderr));
    }
    Ok(String::from_utf8_lossy(&result.stdout).to_string())
}

fn compile_error(source: &str) -> String {
    let dir = TempDir::new().unwrap();
    let output = dir.path().join("out.ll");
    match restc::driver::run(source, &output, OptimizationLevel::None) {
        Ok(()) => panic!("expected compilation error, but got success"),
        Err(e) => e.to_string(),
    }
}

// ===== SUCCESS TESTS =====

#[test]
fn test_hello_world() {
    compile(r#"def main() { print("hello"); }"#).unwrap();
}

#[test]
fn test_print_int() {
    let out = compile_and_run(r#"def main() { print(42); }"#).unwrap();
    assert_eq!(out, "42\n");
}

#[test]
fn test_print_string() {
    let out = compile_and_run(r#"def main() { print("hello"); }"#).unwrap();
    assert_eq!(out, "hello\n");
}

#[test]
fn test_print_bool() {
    let out = compile_and_run(r#"def main() { print(true); print(false); }"#).unwrap();
    assert_eq!(out, "1\n0\n");
}

#[test]
fn test_print_float() {
    let out = compile_and_run(r#"def main() { print(3.14); }"#).unwrap();
    assert_eq!(out, "3.140000\n");
}

#[test]
fn test_variable() {
    let out = compile_and_run(r#"def main() { let x = 10; print(x); }"#).unwrap();
    assert_eq!(out, "10\n");
}

#[test]
fn test_arithmetic() {
    let out = compile_and_run(r#"def main() { print(1 + 2 * 3); }"#).unwrap();
    assert_eq!(out, "7\n");
}

#[test]
fn test_if_else() {
    let out = compile_and_run(r#"
def main() {
    if true {
        print(1);
    } else {
        print(0);
    }
}"#).unwrap();
    assert_eq!(out, "1\n");
}

#[test]
fn test_while_loop() {
    let out = compile_and_run(r#"
def main() {
    let i = 0;
    while i < 3 {
        print(i);
        i = i + 1;
    }
}"#).unwrap();
    assert_eq!(out, "0\n1\n2\n");
}

#[test]
fn test_for_loop() {
    let out = compile_and_run(r#"
def main() {
    for i in 0..3 {
        print(i);
    }
}"#).unwrap();
    assert_eq!(out, "0\n1\n2\n");
}

#[test]
fn test_function_call() {
    let out = compile_and_run(r#"
def add(a: i32, b: i32) -> i32 {
    return a + b;
}
def main() {
    print(add(2, 3));
}"#).unwrap();
    assert_eq!(out, "5\n");
}

#[test]
fn test_recursion() {
    let out = compile_and_run(r#"
def factorial(n: i32) -> i32 {
    if n <= 1 {
        return 1;
    }
    return n * factorial(n - 1);
}
def main() {
    print(factorial(5));
}"#).unwrap();
    assert_eq!(out, "120\n");
}

#[test]
fn test_mutual_recursion() {
    let out = compile_and_run(r#"
def is_even(n: i32) -> bool {
    if n == 0 { return true; }
    return is_odd(n - 1);
}
def is_odd(n: i32) -> bool {
    if n == 0 { return false; }
    return is_even(n - 1);
}
def main() {
    if is_even(4) { print("even"); }
    if is_odd(3) { print("odd"); }
}"#).unwrap();
    assert_eq!(out, "even\nodd\n");
}

#[test]
fn test_string_concat() {
    let out = compile_and_run(r#"
def main() {
    print("Hello, " + "World!");
}"#).unwrap();
    assert_eq!(out, "Hello, World!\n");
}

#[test]
fn test_string_variable() {
    let out = compile_and_run(r#"
def main() {
    let s = "hello";
    print(s);
}"#).unwrap();
    assert_eq!(out, "hello\n");
}

#[test]
fn test_let_struct_move() {
    let out = compile_and_run(r#"
model Point { x: i32, y: i32 }
def main() {
    let a = Point { x: 1, y: 2 };
    let b = a;
    print(b.x);
}"#).unwrap();
    assert_eq!(out, "1\n");
}

#[test]
fn test_struct_reassign() {
    let out = compile_and_run(r#"
model Point { x: i32, y: i32 }
def main() {
    let p = Point { x: 1, y: 2 };
    let q = Point { x: 3, y: 4 };
    q = p;
    print(q.x);
}"#).unwrap();
    assert_eq!(out, "1\n");
}

#[test]
fn test_string_reassign() {
    let out = compile_and_run(r#"
def main() {
    let s = "hello";
    let t = "world";
    t = s;
    print(t);
}"#).unwrap();
    assert_eq!(out, "hello\n");
}

#[test]
fn test_function_defined_first() {
    let out = compile_and_run(r#"
def main() {
    print(greet());
}
def greet() -> string {
    return "hi";
}"#).unwrap();
    assert_eq!(out, "hi\n");
}

#[test]
fn test_return_nested_if() {
    let out = compile_and_run(r#"
def test(x: i32) -> i32 {
    if x > 0 {
        if x > 10 {
            return 100;
        }
        return 10;
    }
    return 0;
}
def main() {
    print(test(5));
    print(test(15));
    print(test(-1));
}"#).unwrap();
    assert_eq!(out, "10\n100\n0\n");
}

#[test]
fn test_array_literal() {
    compile(r#"
def main() {
    let arr = i32{1, 2, 3};
    print(arr[0]);
}"#).unwrap();
}

#[test]
fn test_struct_field_access() {
    let out = compile_and_run(r#"
model Foo { a: i32, b: i32 }
def main() {
    let f = Foo { a: 7, b: 8 };
    print(f.a);
    print(f.b);
}"#).unwrap();
    assert_eq!(out, "7\n8\n");
}

#[test]
fn test_nested_struct() {
    let out = compile_and_run(r#"
model Inner { x: i32, y: i32 }
model Outer { inner: Inner, name: string }
def main() {
    let inner = Inner { x: 1, y: 2 };
    let outer = Outer { inner: inner, name: "hello" };
    print(outer.inner.x);
    print(outer.inner.y);
    print(outer.name);
}"#).unwrap();
    assert_eq!(out, "1\n2\nhello\n");
}

#[test]
fn test_nested_struct_string_field() {
    let out = compile_and_run(r#"
model Inner { value: string }
model Outer { inner: Inner, label: string }
def main() {
    let inner = Inner { value: "world" };
    let outer = Outer { inner: inner, label: "hello" };
    print(outer.label);
    print(outer.inner.value);
}"#).unwrap();
    assert_eq!(out, "hello\nworld\n");
}

#[test]
fn test_struct_field_assign_ident_rhs() {
    // L2 fix: assigning a whole struct Ident to a struct field must
    // deep-copy, so the source variable remains usable after the assignment.
    let out = compile_and_run(r#"
model Inner { v: i32 }
model Outer { inner: Inner }
def main() {
    let o1 = Outer { inner: Inner { v: 1 } };
    let o2 = Inner { v: 2 };
    o1.inner = o2;
    print(o1.inner.v);
    print(o2.v);
}"#).unwrap();
    assert_eq!(out, "2\n2\n");
}

#[test]
fn ok_print_temp_struct_field() {
    // L1 fix: `print(Person{...}.name)` must not leak the underlying
    // struct (which owns the strdup'd name string).
    let out = compile_and_run(r#"
model Person { name: string }
def main() {
    print(Person { name: "alice" }.name);
    print(Person { name: "bob" }.name);
}"#).unwrap();
    assert_eq!(out, "alice\nbob\n");
}

#[test]
fn ok_print_call_field() {
    // L1 fix: `print(call().field)` must free the call result.
    let out = compile_and_run(r#"
model Person { name: string }
def mk() -> Person { return Person { name: "carol" }; }
def main() {
    print(mk().name);
}"#).unwrap();
    assert_eq!(out, "carol\n");
}

#[test]
fn ok_print_temp_array_element_field() {
    // L1 fix: `print(arr[0].field)` for ArrayLiteral of structs must
    // free the array (which owns the strdup'd fields).
    let out = compile_and_run(r#"
model Item { name: string }
def main() {
    print([Item { name: "x" }, Item { name: "y" }][0].name);
}"#).unwrap();
    assert_eq!(out, "x\n");
}

#[test]
fn ok_print_inline_int_array_index() {
    // L1 fix + ArrayIndex typed-GEP fix: inline `print([1,2,3][2])`
    // must print the int and not segfault.
    let out = compile_and_run(r#"
def main() {
    print([10, 20, 30][2]);
    print(i32{1, 2, 3}[0]);
}"#).unwrap();
    assert_eq!(out, "30\n1\n");
}



#[test]
fn ok_reassign_revives_moved() {
    // Reassigning a variable gives it a fresh value, reviving it.
    let out = compile_and_run(r#"
def main() {
    let x = 1;
    let y = x;
    x = 42;
    print(x);
}"#).unwrap();
    assert_eq!(out, "42\n");
}












#[test]
fn ok_self_assign() {
    // `x = x;` is a strange but valid statement — the RHS reads
    // the current value of `x` and writes it back. It should not
    // be flagged as a move or use-after-move.
    let out = compile_and_run(r#"
def main() {
    let x = 1;
    x = x;
    print(x);
}
"#).unwrap();
    assert_eq!(out, "1\n");
}





#[test]
fn test_break() {
    let out = compile_and_run(r#"
def main() {
    for i in 0..10 {
        if i == 2 {
            break;
        }
        print(i);
    }
}"#).unwrap();
    assert_eq!(out, "0\n1\n");
}

#[test]
fn test_continue() {
    let out = compile_and_run(r#"
def main() {
    for i in 0..5 {
        if i == 2 {
            continue;
        }
        print(i);
    }
}"#).unwrap();
    assert_eq!(out, "0\n1\n3\n4\n");
}

#[test]
fn test_struct_string_field_assign() {
    let out = compile_and_run(r#"
model Person { name: string }
def main() {
    let a = Person { name: "Alice" };
    let b = Person { name: "Bob" };
    b.name = a.name;
    print(b.name);
}"#).unwrap();
    assert_eq!(out, "Alice\n");
}

#[test]
fn test_struct_string_field_assign_from_var() {
    let out = compile_and_run(r#"
model Person { name: string }
def main() {
    let s = "Charlie";
    let p = Person { name: "Diana" };
    p.name = s;
    print(p.name);
}"#).unwrap();
    assert_eq!(out, "Charlie\n");
}

#[test]
fn test_struct_string_field_assign_from_call() {
    let out = compile_and_run(r#"
model Person { name: string }
def greet() -> string {
    return "Hello";
}
def main() {
    let p = Person { name: "x" };
    p.name = greet();
    print(p.name);
}"#).unwrap();
    assert_eq!(out, "Hello\n");
}

// ===== CRITICAL FIX VERIFICATION TESTS =====

#[test]
fn err_print_empty() {
    let err = compile_error(r#"def main() { print(); }"#);
    assert!(err.contains("expected 1"), "got: {}", err);
}

#[test]
fn err_return_type_mismatch() {
    let err = compile_error(r#"
def foo() -> i32 {
    return "hello";
}
def main() { foo(); }"#);
    assert!(err.contains("type mismatch") || err.contains("expected"), "got: {}", err);
}

#[test]
fn err_struct_field_type_mismatch() {
    let err = compile_error(r#"
model Point { x: i32, y: i32 }
def main() {
    let p = Point { x: "hello", y: 1 };
}"#);
    assert!(err.contains("type mismatch") || err.contains("expected"), "got: {}", err);
}

#[test]
fn err_assign_type_mismatch() {
    let err = compile_error(r#"
def main() {
    let x: i32 = 1;
    x = "hello";
}"#);
    assert!(err.contains("type mismatch") || err.contains("expected"), "got: {}", err);
}

#[test]
fn err_binary_op_mismatch() {
    let err = compile_error(r#"
def main() {
    print(1 + true);
}"#);
    assert!(err.contains("expected") || err.contains("requires"), "got: {}", err);
}

#[test]
fn err_array_index_not_int() {
    let err = compile_error(r#"
def main() {
    let a = i32{1, 2, 3};
    print(a["hello"]);
}"#);
    assert!(err.contains("integer"), "got: {}", err);
}

#[test]
fn err_array_element_mismatch() {
    let err = compile_error(r#"
def main() {
    let a: i32[3] = [1, "hello", 3];
}"#);
    assert!(err.contains("type mismatch") || err.contains("expected"), "got: {}", err);
}

#[test]
fn err_inferred_array_element_mismatch() {
    let err = compile_error(r#"
def main() {
    let a = [1, "hello"];
}"#);
    assert!(err.contains("type mismatch") || err.contains("expected"), "got: {}", err);
}

#[test]
fn err_if_condition_not_bool() {
    let err = compile_error(r#"
def main() {
    if 42 { print(1); }
}"#);
    assert!(err.contains("bool"), "got: {}", err);
}

// ===== ERROR TESTS =====

#[test]
fn err_undefined_variable() {
    let err = compile_error(r#"def main() { print(x); }"#);
    assert!(err.contains("undefined variable"), "got: {}", err);
}

#[test]
fn err_undefined_function() {
    let err = compile_error(r#"def main() { foo(); }"#);
    assert!(err.contains("undefined function"), "got: {}", err);
}

#[test]
fn err_type_mismatch() {
    let err = compile_error(r#"
def main() {
    let x: i32 = "hello";
}"#);
    assert!(err.contains("type mismatch") || err.contains("expected"), "got: {}", err);
}

#[test]
fn err_missing_return() {
    let err = compile_error(r#"
def foo() -> i32 {
    let x = 1;
}"#);
    assert!(err.contains("missing return"), "got: {}", err);
}

#[test]
fn err_unexpected_return_value() {
    let err = compile_error(r#"
def main() {
    return 42;
}"#);
    assert!(err.contains("unexpected return value"), "got: {}", err);
}

#[test]
fn err_duplicate_function() {
    let err = compile_error(r#"
def foo() {}
def foo() {}
def main() {}"#);
    assert!(err.contains("duplicate"), "got: {}", err);
}

#[test]
fn err_duplicate_struct() {
    let err = compile_error(r#"
model A { x: i32 }
model A { y: i32 }
def main() {}"#);
    assert!(err.contains("duplicate"), "got: {}", err);
}

#[test]
fn err_duplicate_field() {
    let err = compile_error(r#"
model Foo { x: i32, x: i32 }
def main() {}"#);
    assert!(err.contains("duplicate field"), "got: {}", err);
}

#[test]
fn err_break_outside_loop() {
    let err = compile_error(r#"
def main() {
    break;
}"#);
    assert!(err.contains("break outside loop"), "got: {}", err);
}

#[test]
fn err_return_outside_function() {
    let err = compile_error(r#"return 1;"#);
    assert!(err.contains("return outside function"), "got: {}", err);
}

#[test]
fn err_wrong_arg_count() {
    let err = compile_error(r#"
def foo(x: i32) {}
def main() {
    foo(1, 2);
}"#);
    assert!(err.contains("expected 1 arguments"), "got: {}", err);
}

#[test]
fn err_unassignable_literal() {
    let err = compile_error(r#"
def main() {
    42 = 1;
}"#);
    assert!(err.contains("expression is not assignable"), "got: {}", err);
}

#[test]
fn err_missing_type_annotation() {
    let err = compile_error(r#"
def main() {
    let x;
}"#);
    assert!(err.contains("type annotation required"), "got: {}", err);
}

#[test]
fn err_void_variable() {
    let err = compile_error(r#"
def main() {
    let x: void = 1;
}"#);
    assert!(err.contains("cannot have type `void`"), "got: {}", err);
}

#[test]
fn err_not_a_struct() {
    let err = compile_error(r#"
def main() {
    let x = 1;
    let y = x.foo;
}"#);
    assert!(err.contains("not a struct"), "got: {}", err);
}

#[test]
fn err_no_such_field() {
    let err = compile_error(r#"
model Foo { x: i32 }
def main() {
    let f = Foo { x: 1 };
    let y = f.z;
}"#);
    assert!(err.contains("no field"), "got: {}", err);
}

#[test]
fn err_missing_struct_fields() {
    let err = compile_error(r#"
model Point { x: i32, y: i32 }
def main() {
    let a = Point { x: 1 };
}"#);
    assert!(err.contains("missing fields"), "got: {}", err);
}

#[test]
fn err_void_struct_field() {
    let err = compile_error(r#"
model Foo { x: void }
def main() { }
"#);
    assert!(err.contains("void"), "got: {}", err);
}

#[test]
fn err_struct_addition() {
    let err = compile_error(r#"
model Point { x: i32, y: i32 }
def main() {
    let a = Point { x: 1, y: 2 };
    let b = a + a;
}
"#);
    assert!(err.contains("found") || err.contains("not"), "got: {}", err);
}

#[test]
fn err_assign_to_temp_struct() {
    let err = compile_error(r#"
model Point { x: i32, y: i32 }
def main() {
    Point { x: 1, y: 2 }.x = 5;
}
"#);
    assert!(err.contains("not assignable"), "got: {}", err);
}

#[test]
fn test_return_in_loop() {
    let out = compile_and_run(r#"
def f() -> i32 {
    while true { return 42; }
    0;
}
def main() {
    print(f());
}
"#).unwrap();
    assert_eq!(out, "42\n");
}

#[test]
fn test_shift_different_types() {
    let out = compile_and_run(r#"
def main() {
    let x: i32 = 16;
    let y: i8 = 3i8;
    print(x << y);
}
"#).unwrap();
    assert_eq!(out, "128\n");
}

#[test]
fn err_not_a_function() {
    let err = compile_error(r#"
def main() {
    let x = 1;
    x();
}"#);
    assert!(err.contains("not a function") || err.contains("undefined function"), "got: {}", err);
}

#[test]
fn test_bool_assign() {
    let out = compile_and_run(r#"
def main() {
    let a = true;
    let b = false;
    let x = a && b;
    let y = a || b;
    if x == true { print(0); } else { print(1); }
    if y == true { print(1); } else { print(0); }
}
"#).unwrap();
    assert_eq!(out, "1\n1\n");
}

#[test]
fn test_string_array_element() {
    let out = compile_and_run(r#"
def main() {
    let arr = string{ "a", "b", "c" };
    let s = arr[0];
    print(s);
}
"#).unwrap();
    assert_eq!(out, "a\n");
}

#[test]
fn test_string_array_assign() {
    let out = compile_and_run(r#"
def main() {
    let arr = string{ "a", "b", "c" };
    arr[1] = "z";
    let s = arr[0];
    let t = arr[1];
    let u = arr[2];
    print(s);
    print(t);
    print(u);
}
"#).unwrap();
    assert_eq!(out, "a\nz\nc\n");
}

#[test]
fn test_if_with_parens() {
    let out = compile_and_run(r#"
def main() {
    let x = 1;
    if (x == 1) { print(42); }
}"#).unwrap();
    assert_eq!(out, "42\n");
}

#[test]
fn test_string_default_not_null() {
    let out = compile_and_run(r#"
def main() {
    let s: string;
    print(s);
}"#).unwrap();
    assert_eq!(out, "\n");
}

#[test]
fn test_utf8_string_literal() {
    // Multi-byte UTF-8 sequences must round-trip correctly,
    // not be split into individual Latin-1 bytes via `c as char`.
    let out = compile_and_run(r#"
def main() {
    print("café");
    print("привет");
    print("日本語");
}"#).unwrap();
    assert_eq!(out, "café\nпривет\n日本語\n");
}

#[test]
fn test_string_escape_null() {
    // \0 is a valid escape and the resulting string contains the NUL byte.
    // printf("%s") stops at NUL, so we can only verify the program compiles
    // and that the part before NUL is printed. The escape must not cause
    // a lex error.
    let out = compile_and_run(r#"def main() { print("a\0b"); }"#).unwrap();
    assert_eq!(out, "a\n");
}

#[test]
fn err_let_struct_no_init() {
    // `let x: Foo;` previously lowered to `Int(0, I8)` (a garbage HIR
    // node) and crashed in codegen. Now it should produce a clean error.
    let err = compile_error(r#"
model Foo { a: i32 }
def main() {
    let f: Foo;
}"#);
    assert!(err.contains("cannot be default-initialized"), "got: {}", err);
}

#[test]
fn err_let_array_no_init() {
    let err = compile_error(r#"
def main() {
    let a: i32[3];
}"#);
    assert!(err.contains("cannot be default-initialized"), "got: {}", err);
}

#[test]
fn err_float_exponent_no_digits() {
    // `1e` and `1.5e+` must produce a clear, specific error about
    // the missing digits, not a generic "invalid float" complaint.
    let err = compile_error("fn main() { let x = 1e; }");
    assert!(err.contains("missing digits after exponent"), "got: {}", err);

    let err2 = compile_error("fn main() { let x = 1.5e+; }");
    assert!(err2.contains("missing digits after exponent"), "got: {}", err2);
}

#[test]
fn err_assignment_in_if_condition() {
    // `if (x = 1)` is almost always a bug — user meant `==`.
    // Reject at parse time with a helpful suggestion.
    let err = compile_error(r#"
def main() {
    let x: i32 = 0;
    if (x = 1) {
        print(x);
    }
}"#);
    assert!(err.contains("assignment is not allowed"), "got: {}", err);
    assert!(err.contains("=="), "got: {}", err);
}

#[test]
fn err_assignment_in_while_condition() {
    let err = compile_error(r#"
def main() {
    let x: i32 = 0;
    while (x = 1) {
        print(x);
    }
}"#);
    assert!(err.contains("assignment is not allowed"), "got: {}", err);
}

#[test]
fn ok_normal_assignment_statement() {
    // Assignment at statement level (the intended use) must still work.
    let out = compile_and_run(r#"
def main() {
    let x: i32 = 0;
    x = 1;
    print(x);
}"#).unwrap();
    assert_eq!(out, "1\n");
}

#[test]
fn ok_implicit_struct_field_access() {
    // Implicit struct (no `struct` declaration, just used in a literal)
    // must be recognized and accepted by codegen.
    let out = compile_and_run(r#"
def main() {
    let p = Point { x: 1, y: 2 };
    print(p.x);
    print(p.y);
}"#).unwrap();
    assert_eq!(out, "1\n2\n");
}

#[test]
fn ok_field_to_field_struct_assign() {
    // Field-to-field struct assignment must deep-copy (not share pointer),
    // otherwise the struct's inner field is freed twice at scope exit.
    let out = compile_and_run(r#"
model Inner { v: i32 }
model Outer { inner: Inner, other: Inner }

def main() {
    let o1 = Outer { inner: Inner { v: 1 }, other: Inner { v: 2 } };
    let o2 = Outer { inner: Inner { v: 10 }, other: Inner { v: 20 } };
    o1.inner = o2.inner;
    o1.other = o2.other;
    print(o1.inner.v);
    print(o1.other.v);
    print(o2.inner.v);
    print(o2.other.v);
}"#).unwrap();
    assert_eq!(out, "10\n20\n10\n20\n");
}

#[test]
fn ok_inferred_array_literal_with_field_access() {
    // Inferred array literal `[a, b, c]` (no explicit type) used as
    // object of a field access must not panic in hir_type_to_basic.
    let out = compile_and_run(r#"
model Item { name: string }
def main() {
    let arr = [Item { name: "alpha" }, Item { name: "beta" }];
    print(arr[0].name);
    print(arr[1].name);
}"#).unwrap();
    assert_eq!(out, "alpha\nbeta\n");
}

#[test]
fn ok_inferred_array_literal_int() {
    // Inferred int array literal must work.
    let out = compile_and_run(r#"
def main() {
    let arr = [10, 20, 30];
    print(arr[0]);
    print(arr[1]);
    print(arr[2]);
}"#).unwrap();
    assert_eq!(out, "10\n20\n30\n");
}

// ===== C1: borrow through call arg =====
//
// `take(&s); let y = s;` is LEGAL: the borrow is scoped to the call,
// not to the rest of the function. After `take` returns, the borrow
// is gone and `s` can be moved. The corresponding error tests would
// be wrong — they would assert that a sound use-after-move is
// rejected. Positive tests covering the *legal* cases are added
// elsewhere (e.g. `ok_borrow_field_legal_use`).

// ===== C2: borrow of struct field =====



// ===== C3: double free on field let binding =====

#[test]
fn ok_let_string_field_no_double_free() {
    // C3: `let s = p.name;` must strdup the string, preventing
    // double-free when both `p` and `s` go out of scope.
    let out = compile_and_run(r#"
model P { name: string }
def main() {
    let p = P { name: "alice" };
    let s = p.name;
    print(s);
    print(p.name);
}"#).unwrap();
    assert_eq!(out, "alice\nalice\n");
}

// ===== C4: UAF on return of string field =====

#[test]
fn ok_return_string_field() {
    // C4: `return s.name;` must strdup the field before freeing the
    // struct, so the caller receives a valid owned string.
    let out = compile_and_run(r#"
model S { name: string }
def f() -> string {
    let s = S { name: "hello" };
    return s.name;
}
def main() {
    let r = f();
    print(r);
}"#).unwrap();
    assert_eq!(out, "hello\n");
}

// ===== H1: empty array literal =====

#[test]
fn err_empty_array_literal() {
    // H1: `let x = [];` must produce a type error, not a panic.
    let err = compile_error(r#"
def main() { let x = []; print(x); }
"#);
    assert!(err.contains("type"), "got: {}", err);
}

// ===== H2: integer overflow for typed literals =====

#[test]
fn err_literal_out_of_range_i8() {
    // H2: `1000i8` must produce a type error (silent wrap).
    let err = compile_error(r#"
def main() { let x: i8 = 1000i8; print(x); }
"#);
    assert!(err.contains("does not fit") || err.contains("LiteralOutOfRange"), "got: {}", err);
}

#[test]
fn err_literal_out_of_range_u8() {
    let err = compile_error(r#"
def main() { let x: u8 = 256u8; print(x); }
"#);
    assert!(err.contains("does not fit") || err.contains("LiteralOutOfRange"), "got: {}", err);
}

// ===== Postfix -- and ++ =====

#[test]
fn test_postfix_decrement() {
    let out = compile_and_run(r#"
def main() {
    let i = 5;
    i--;
    print(i);
}"#).unwrap();
    assert_eq!(out, "4\n");
}

#[test]
fn test_postfix_increment() {
    let out = compile_and_run(r#"
def main() {
    let i = 5;
    i++;
    print(i);
}"#).unwrap();
    assert_eq!(out, "6\n");
}

#[test]
fn test_postfix_increment_in_while() {
    let out = compile_and_run(r#"
def main() {
    let i = 0;
    while i < 5 {
        i++;
    }
    print(i);
}"#).unwrap();
    assert_eq!(out, "5\n");
}

#[test]
fn test_postfix_decrement_in_while() {
    let out = compile_and_run(r#"
def main() {
    let i = 5;
    while i > 0 {
        i--;
    }
    print(i);
}"#).unwrap();
    assert_eq!(out, "0\n");
}

// ===== For-loop variable type inference =====

#[test]
fn test_for_loop_u64() {
    // The loop variable must adopt the boundary type, not be hardcoded
    // to i32. Summing 0u64..5u64 yields 10, not silently truncated.
    let out = compile_and_run(r#"
def main() {
    let sum: u64 = 0u64;
    for i in 0u64..5u64 {
        sum = sum + i;
    }
    print(sum);
}"#).unwrap();
    assert_eq!(out, "10\n");
}

#[test]
fn test_for_loop_i8() {
    let out = compile_and_run(r#"
def main() {
    let sum: i8 = 0i8;
    for i in 0i8..5i8 {
        sum = sum + i;
    }
    print(sum);
}"#).unwrap();
    assert_eq!(out, "10\n");
}

#[test]
fn err_for_loop_mismatched_bounds() {
    // The for-range must require both boundaries to share a type.
    let err = compile_error(r#"
def main() {
    for i in 0i32..5u64 {
        print(i);
    }
}"#);
    assert!(err.contains("type mismatch") || err.contains("expected"), "got: {}", err);
}
