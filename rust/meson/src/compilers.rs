use crate::objects::*;
use crate::vm::*;
use std::io::Write;
/// Compiler detection and compiler check functions.
use std::process::Command;

/// Detect a compiler for the given language and register it in the VM.
/// Returns true if found.
pub fn detect_compiler(vm: &mut VM, language: &str) -> bool {
    let candidates = match language {
        "c" => vec![
            ("cc", "gcc"),
            ("gcc", "gcc"),
            ("clang", "clang"),
            ("cc", "clang"),
            ("cl", "msvc"),
        ],
        "cpp" | "c++" => vec![
            ("c++", "gcc"),
            ("g++", "gcc"),
            ("clang++", "clang"),
            ("c++", "clang"),
            ("cl", "msvc"),
        ],
        "objc" => vec![("cc", "gcc"), ("clang", "clang")],
        "objcpp" => vec![("c++", "gcc"), ("clang++", "clang")],
        "rust" => vec![("rustc", "rustc")],
        "d" => vec![("ldc2", "llvm"), ("dmd", "dmd"), ("gdc", "gcc")],
        "fortran" => vec![("gfortran", "gcc"), ("flang", "flang")],
        "java" => vec![("javac", "javac")],
        "cs" | "csharp" => vec![("mcs", "mono"), ("csc", "csc")],
        "vala" => vec![("valac", "valac")],
        "swift" => vec![("swiftc", "swiftc")],
        "nasm" => vec![("nasm", "nasm")],
        "masm" => vec![("ml", "ml"), ("ml64", "ml")],
        "cython" => vec![("cython", "cython")],
        "cuda" => vec![("nvcc", "nvcc")],
        _ => return false,
    };

    for (cmd, compiler_id) in candidates {
        if let Some(version) = get_compiler_version(cmd, compiler_id) {
            let compiler = CompilerData {
                id: compiler_id.to_string(),
                language: language.to_string(),
                version: version.clone(),
                cmd: vec![cmd.to_string()],
                linker_id: detect_linker(cmd, compiler_id),
                full_version: version,
            };
            let key = format!("compiler_{}", language);
            vm.globals.insert(key, Object::Compiler(compiler));
            return true;
        }
    }
    false
}

fn get_compiler_version(cmd: &str, compiler_id: &str) -> Option<String> {
    let output = match compiler_id {
        "rustc" => Command::new(cmd).arg("--version").output().ok()?,
        "javac" => Command::new(cmd).arg("-version").output().ok()?,
        "valac" => Command::new(cmd).arg("--version").output().ok()?,
        _ => Command::new(cmd).arg("--version").output().ok()?,
    };

    if !output.status.success() && compiler_id != "javac" {
        return None;
    }

    let text = String::from_utf8_lossy(&output.stdout);
    let text = if text.is_empty() {
        String::from_utf8_lossy(&output.stderr)
    } else {
        text
    };

    // Extract version number
    for word in text.split_whitespace() {
        if word.chars().next().map_or(false, |c| c.is_ascii_digit()) {
            // Strip trailing junk
            let version: String = word
                .chars()
                .take_while(|c| c.is_ascii_digit() || *c == '.')
                .collect();
            if !version.is_empty() {
                return Some(version);
            }
        }
    }
    // Fallback: return first line
    text.lines().next().map(|s| s.trim().to_string())
}

fn detect_linker(cmd: &str, compiler_id: &str) -> String {
    match compiler_id {
        "msvc" => "link".to_string(),
        "rustc" => if cfg!(target_os = "linux") {
            "ld"
        } else if cfg!(target_os = "macos") {
            "ld64"
        } else {
            "link"
        }
        .to_string(),
        _ => {
            // Try to detect from -Wl,--version
            if let Ok(output) = Command::new(cmd)
                .args(["-Wl,--version", "-x", "c", "/dev/null", "-o", "/dev/null"])
                .output()
            {
                let text = String::from_utf8_lossy(&output.stderr);
                if text.contains("GNU ld") {
                    return "ld.bfd".to_string();
                }
                if text.contains("GNU gold") {
                    return "ld.gold".to_string();
                }
                if text.contains("LLD") {
                    return "ld.lld".to_string();
                }
                if text.contains("mold") {
                    return "ld.mold".to_string();
                }
            }
            if cfg!(target_os = "macos") {
                "ld64".to_string()
            } else {
                "ld".to_string()
            }
        }
    }
}

/// Check if a header exists
pub fn check_header(compiler: &CompilerData, header: &str) -> bool {
    let code = format!("#include <{}>", header);
    try_compile_code(compiler, &code, &[])
}

/// Check if a symbol exists in a header
pub fn check_header_symbol(
    compiler: &CompilerData,
    header: &str,
    symbol: &str,
    args: &[CallArg],
) -> bool {
    let prefix = VM::get_arg_str(args, "prefix", usize::MAX).unwrap_or("");
    let extra = extra_args_from_callargs(args);
    // First try: use the symbol directly (works for functions, variables, macros in C)
    let code1 = format!(
        "{}\n#include <{}>\nint main(void) {{\n  #ifndef {}\n    {};\n  #endif\n  return 0;\n}}",
        prefix, header, symbol, symbol
    );
    if try_compile_code(compiler, &code1, &extra) {
        return true;
    }
    // Second try: use sizeof (works for type names like int, FILE, etc.)
    let code2 = format!(
        "{}\n#include <{}>\nint main(void) {{\n  #ifndef {}\n    (void)sizeof({});\n  #endif\n  return 0;\n}}",
        prefix, header, symbol, symbol
    );
    if try_compile_code(compiler, &code2, &extra) {
        return true;
    }
    // Third try: for C++ templates like std::vector, try pointer instantiation
    if compiler.language == "cpp" || compiler.language == "c++" {
        let code3 = format!(
            "{}\n#include <{}>\nint main(void) {{\n  {}< int > *_meson_p = nullptr;\n  (void)_meson_p;\n  return 0;\n}}",
            prefix, header, symbol
        );
        if try_compile_code(compiler, &code3, &extra) {
            return true;
        }
    }
    false
}

/// Check if a function exists
pub fn check_function(compiler: &CompilerData, func: &str, args: &[CallArg]) -> bool {
    let prefix = VM::get_arg_str(args, "prefix", usize::MAX).unwrap_or("");

    // First try: compile test for builtins and macros
    let compile_code = format!(
        "{}\n\
         #ifdef __has_builtin\n\
         #if __has_builtin ({})\n\
         int main(void) {{ return 0; }}\n\
         #else\n\
         #error \"not a builtin\"\n\
         #endif\n\
         #elif defined({})\n\
         int main(void) {{ return 0; }}\n\
         #else\n\
         #error \"not found\"\n\
         #endif",
        prefix, func, func
    );
    let compile_result = try_compile_code(compiler, &compile_code, &extra_args_from_callargs(args));
    if compile_result {
        return true;
    }

    // Second try: link test
    let link_code = if prefix.is_empty() {
        // No prefix: declare the function ourselves and call it
        // Use extern "C" for C++ compatibility (linker needs C linkage for libc functions)
        format!(
            "#ifdef __cplusplus\nextern \"C\"\n#endif\nchar {}(void);\nint main(void) {{ return {}(); }}",
            func, func
        )
    } else {
        // With prefix: the function is declared by the prefix header.
        // Use address-of approach which works because the header provides
        // the correct declaration (and undeclared names cause compile errors).
        format!(
            "{}\nint main(void) {{ void *p = (void *){}; return p == 0; }}",
            prefix, func
        )
    };
    try_link_code(compiler, &link_code, args)
}

/// Check if a type has a member
pub fn check_member(
    compiler: &CompilerData,
    typename: &str,
    member: &str,
    args: &[CallArg],
) -> bool {
    let prefix = VM::get_arg_str(args, "prefix", usize::MAX).unwrap_or("");
    let code = format!(
        "{}\nint main(void) {{ {} x; (void)x.{}; return 0; }}",
        prefix, typename, member
    );
    try_compile_code(compiler, &code, &extra_args_from_callargs(args))
}

/// Check if a type exists
pub fn check_type(compiler: &CompilerData, typename: &str, args: &[CallArg]) -> bool {
    let prefix = VM::get_arg_str(args, "prefix", usize::MAX).unwrap_or("");
    let code = format!(
        "{}\nint main(void) {{ {} x; (void)x; return 0; }}",
        prefix, typename
    );
    try_compile_code(compiler, &code, &[])
}

/// Get sizeof a type
pub fn get_sizeof(compiler: &CompilerData, typename: &str, args: &[CallArg]) -> i64 {
    let prefix = VM::get_arg_str(args, "prefix", usize::MAX).unwrap_or("");
    let code = format!(
        "{}\n#include <stdio.h>\nint main(void) {{ printf(\"%zu\", sizeof({})); return 0; }}",
        prefix, typename
    );
    match try_run_code(compiler, &code, args) {
        Some(output) => output.parse().unwrap_or(-1),
        None => -1,
    }
}

/// Get alignment of a type
pub fn get_alignment(compiler: &CompilerData, typename: &str) -> i64 {
    let code = format!(
        "#include <stddef.h>\n#include <stdio.h>\nstruct __align_test {{ char c; {} x; }};\nint main(void) {{ printf(\"%zu\", offsetof(struct __align_test, x)); return 0; }}",
        typename
    );
    match try_run_code(compiler, &code, &[]) {
        Some(output) => output.parse().unwrap_or(0),
        None => 0,
    }
}

/// Try to compile code
pub fn try_compile(compiler: &CompilerData, code: &str, args: &[CallArg]) -> bool {
    let mut extra = extra_args_from_callargs(args);
    if VM::get_arg_bool(args, "werror", false) {
        extra.push("-Werror".to_string());
    }

    try_compile_code(compiler, code, &extra)
}

/// Try to compile and link code
pub fn try_link(compiler: &CompilerData, code: &str, args: &[CallArg]) -> bool {
    let werror = VM::get_arg_bool(args, "werror", false);
    let extra = extra_args_from_callargs(args);
    let mut link_extra: Vec<String> = Vec::new();
    if werror {
        link_extra.push("-Werror".to_string());
    }
    try_link_code_with_args(compiler, code, &extra, &link_extra)
}

/// Try to compile, link, and run code
pub fn try_run(compiler: &CompilerData, code: &str, args: &[CallArg]) -> RunResultData {
    let werror = VM::get_arg_bool(args, "werror", false);
    match try_run_code_werror(compiler, code, args, werror) {
        Some(stdout) => RunResultData {
            returncode: 0,
            stdout,
            stderr: String::new(),
        },
        None => RunResultData {
            returncode: 1,
            stdout: String::new(),
            stderr: "Failed to run".to_string(),
        },
    }
}

/// Check if the compiler supports an argument
pub fn has_argument(compiler: &CompilerData, arg: &str) -> bool {
    let code = "int main(void) { return 0; }";
    // GCC silently ignores unknown -Wno-* flags, so convert to positive form for testing
    let test_arg = convert_wno_arg(compiler, arg);
    try_compile_code(compiler, code, &[test_arg, "-Werror".to_string()])
}

/// Convert -Wno-foo to -Wfoo for testing (GCC silently ignores unknown -Wno- flags)
fn convert_wno_arg(compiler: &CompilerData, arg: &str) -> String {
    if (compiler.id == "gcc" || compiler.id == "clang") && arg.starts_with("-Wno-") {
        let rest = &arg[5..]; // everything after "-Wno-"
        // Special cases where the positive form is different or requires a value
        // -Wno-attributes=foo -> not convertible (no -Wattributes=foo)
        if rest.starts_with("attributes=") {
            return arg.to_string();
        }
        // Some flags like -Wno-frame-larger-than don't have valid positive counterparts
        // (the positive form requires a value like -Wframe-larger-than=N)
        let special_negative_only = [
            "frame-larger-than",
            "stack-usage",
            "alloc-size-larger-than",
            "alloca-larger-than",
            "vla-larger-than",
        ];
        for s in &special_negative_only {
            if rest == *s {
                return arg.to_string();
            }
        }
        format!("-W{}", rest)
    } else {
        arg.to_string()
    }
}

/// Check if the compiler supports multiple arguments
pub fn has_multi_arguments(compiler: &CompilerData, args: &[String]) -> bool {
    let code = "int main(void) { return 0; }";
    let mut all_args: Vec<String> = args.iter().map(|a| convert_wno_arg(compiler, a)).collect();
    all_args.push("-Werror".to_string());
    try_compile_code(compiler, code, &all_args)
}

/// Check if the linker supports an argument
pub fn has_link_argument(compiler: &CompilerData, arg: &str) -> bool {
    let code = "int main(void) { return 0; }";
    let args = [
        arg.to_string(),
        "-Werror".to_string(),
        "-Wl,--fatal-warnings".to_string(),
    ];
    try_link_code_with_args(compiler, code, &[], &args)
}

/// Check if the linker supports multiple arguments
pub fn has_multi_link_arguments(compiler: &CompilerData, args: &[String]) -> bool {
    let code = "int main(void) { return 0; }";
    let mut all_args: Vec<String> = args.to_vec();
    all_args.push("-Werror".to_string());
    all_args.push("-Wl,--fatal-warnings".to_string());
    try_link_code_with_args(compiler, code, &[], &all_args)
}

/// Get a preprocessor define value
pub fn get_define(compiler: &CompilerData, name: &str, args: &[CallArg]) -> String {
    let prefix = get_prefix_from_args(args);
    let extra = extra_args_from_callargs(args);
    let delim = "MESON_GET_DEFINE_DELIMITER";
    let code = format!(
        "{}\n#ifndef {}\n#define {}\n#endif\n{}\n{}\n",
        prefix, name, name, delim, name
    );
    if let Some(output) = try_preprocess_code(compiler, &code, &extra) {
        if let Some(pos) = output.find(delim) {
            let after = &output[pos + delim.len()..];
            let value = after.trim();
            // Post-process: concatenate adjacent C string literals
            return concatenate_string_literals(value);
        }
    }
    String::new()
}

/// Concatenate adjacent C string literals in preprocessor output.
/// e.g., \x22ab\x22 \x22cd\x22 → \x22abcd\x22
fn concatenate_string_literals(input: &str) -> String {
    if !input.contains('\x22') {
        return input.to_string();
    }
    let mut result = String::new();
    let mut chars = input.chars().peekable();
    let mut in_string = false;
    let mut current_string = String::new();
    let mut pending_close = false;

    while let Some(c) = chars.next() {
        if pending_close {
            // We just closed a string literal. Check if next non-space char is another quote.
            if c == '\x20' || c == '\t' || c == '\n' {
                // Skip whitespace between potential adjacent strings
                continue;
            } else if c == '\x22' {
                // Another string literal follows — continue accumulating
                pending_close = false;
                in_string = true;
                continue;
            } else {
                // Not an adjacent string — close the current string and emit
                result.push('\x22');
                result.push_str(&current_string);
                result.push('\x22');
                current_string.clear();
                pending_close = false;
                // Process this character normally
                result.push(c);
                continue;
            }
        }

        if !in_string && c == '\x22' {
            // Start of a string literal
            in_string = true;
            continue;
        }

        if in_string {
            if c == '\x5c' {
                // Backslash escape in string
                current_string.push(c);
                if let Some(next) = chars.next() {
                    current_string.push(next);
                }
            } else if c == '\x22' {
                // End of string literal — but might be followed by another
                pending_close = true;
                in_string = false;
            } else {
                current_string.push(c);
            }
        } else {
            result.push(c);
        }
    }

    if pending_close || in_string {
        // Flush remaining string
        result.push('\x22');
        result.push_str(&current_string);
        result.push('\x22');
    }

    result
}

/// Find a library
pub fn find_library(compiler: &CompilerData, name: &str, dirs: &[String]) -> Option<Object> {
    // Try linking with -l<name>
    let code = "int main(void) { return 0; }";
    let link_args = vec![format!("-l{}", name)];
    let mut extra = Vec::new();
    for dir in dirs {
        extra.push(format!("-L{}", dir));
    }
    extra.extend(link_args.clone());

    if try_link_code_with_args(compiler, code, &[], &extra) {
        Some(Object::Dependency(DependencyData {
            name: name.to_string(),
            found: true,
            version: String::new(),
            compile_args: Vec::new(),
            link_args,
            sources: Vec::new(),
            include_dirs: Vec::new(),
            dependencies: Vec::new(),
            variables: std::collections::HashMap::new(),
            is_internal: false,
        }))
    } else {
        None
    }
}

/// Check if a function attribute is supported
pub fn has_function_attribute(compiler: &CompilerData, attr: &str) -> bool {
    let code = match attr {
        "visibility" | "visibility:default" => {
            "int __attribute__((visibility(\"default\"))) func(void) { return 0; }"
        }
        "dllexport" => "__declspec(dllexport) int func(void) { return 0; }",
        "dllimport" => "__declspec(dllimport) int func(void);",
        "noreturn" => "void __attribute__((noreturn)) func(void) { while(1); }",
        "unused" => "int __attribute__((unused)) x;",
        "deprecated" => "int __attribute__((deprecated)) func(void);",
        "aligned" => "int __attribute__((aligned(16))) x;",
        "pure" => "int __attribute__((pure)) func(void);",
        "const" => "int __attribute__((const)) func(void);",
        "malloc" => "void *__attribute__((malloc)) func(void);",
        "warn_unused_result" => "int __attribute__((warn_unused_result)) func(void);",
        "weak" => "int __attribute__((weak)) func(void) { return 0; }",
        _ => return false,
    };
    try_compile_code(compiler, code, &["-Werror".to_string()])
}

/// Compute an integer expression
pub fn compute_int(compiler: &CompilerData, expr: &str, args: &[CallArg]) -> i64 {
    let prefix = VM::get_arg_str(args, "prefix", usize::MAX).unwrap_or("");
    let code = format!(
        "{}\n#include <stdio.h>\nint main(void) {{ printf(\"%ld\", (long)({})); return 0; }}",
        prefix, expr
    );
    try_run_code(compiler, &code, args)
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(0)
}

// ---- Internal helpers ----

pub fn extra_args_from_callargs(args: &[CallArg]) -> Vec<String> {
    let mut extra = Vec::new();
    match VM::get_arg_value(args, "args") {
        Some(Object::Array(arr)) => {
            for item in arr {
                extra.push(item.to_string_value());
            }
        }
        Some(Object::String(s)) => {
            extra.push(s.clone());
        }
        _ => {}
    }
    extra
}

/// Extract prefix from args, handling both string and array of strings.
pub fn get_prefix_from_args(args: &[CallArg]) -> String {
    if let Some(s) = VM::get_arg_str(args, "prefix", usize::MAX) {
        return s.to_string();
    }
    if let Some(Object::Array(arr)) = VM::get_arg_value(args, "prefix") {
        return arr
            .iter()
            .filter_map(|v| {
                if let Object::String(s) = v {
                    Some(s.as_str())
                } else {
                    None
                }
            })
            .collect::<Vec<_>>()
            .join("\n");
    }
    String::new()
}

fn try_preprocess_code(
    compiler: &CompilerData,
    code: &str,
    extra_args: &[String],
) -> Option<String> {
    let tmpdir = std::env::temp_dir();
    let suffix = get_source_suffix(compiler);
    let src_path = tmpdir.join(format!("meson_preprocess{}", suffix));

    std::fs::write(&src_path, code).ok()?;

    let mut cmd = Command::new(&compiler.cmd[0]);
    cmd.arg("-E").arg("-P").arg("-w").arg(&src_path);
    for arg in extra_args {
        cmd.arg(arg);
    }

    let output = cmd.output().ok()?;
    let _ = std::fs::remove_file(&src_path);

    if output.status.success() {
        Some(String::from_utf8_lossy(&output.stdout).to_string())
    } else {
        None
    }
}

/// Check if a preprocessor define exists using the actual compiler.
pub fn has_define(compiler: &CompilerData, name: &str, args: &[CallArg]) -> bool {
    let prefix = get_prefix_from_args(args);
    let extra = extra_args_from_callargs(args);
    let code = format!(
        "{}\n#ifndef {}\n#error \"not defined\"\n#endif\nint main(void) {{ return 0; }}\n",
        prefix, name
    );
    try_compile_code(compiler, &code, &extra)
}

fn get_source_suffix(compiler: &CompilerData) -> &str {
    match compiler.language.as_str() {
        "c" | "objc" => ".c",
        "cpp" | "c++" | "objcpp" => ".cpp",
        "fortran" => ".f90",
        "d" => ".d",
        "rust" => ".rs",
        "vala" => ".vala",
        "swift" => ".swift",
        "cuda" => ".cu",
        _ => ".c",
    }
}

fn try_compile_code(compiler: &CompilerData, code: &str, extra_args: &[String]) -> bool {
    let tmpdir = std::env::temp_dir();
    let suffix = get_source_suffix(compiler);
    let src_path = tmpdir.join(format!("meson_test{}", suffix));
    let obj_path = tmpdir.join("meson_test.o");

    if std::fs::write(&src_path, code).is_err() {
        return false;
    }

    let mut cmd = Command::new(&compiler.cmd[0]);
    cmd.arg("-c").arg(&src_path).arg("-o").arg(&obj_path);
    let has_werror = extra_args.iter().any(|a| a == "-Werror");
    if !has_werror {
        cmd.arg("-w"); // suppress warnings (but not when -Werror is requested)
    }
    for arg in extra_args {
        cmd.arg(arg);
    }

    let result = cmd.output().map(|o| o.status.success()).unwrap_or(false);
    let _ = std::fs::remove_file(&src_path);
    let _ = std::fs::remove_file(&obj_path);
    result
}

fn try_link_code(compiler: &CompilerData, code: &str, args: &[CallArg]) -> bool {
    let extra = extra_args_from_callargs(args);
    try_link_code_with_args(compiler, code, &extra, &[])
}

fn try_link_code_with_args(
    compiler: &CompilerData,
    code: &str,
    compile_args: &[String],
    link_args: &[String],
) -> bool {
    let tmpdir = std::env::temp_dir();
    let suffix = get_source_suffix(compiler);
    let src_path = tmpdir.join(format!("meson_test{}", suffix));
    let exe_path = tmpdir.join("meson_test_exe");

    if std::fs::write(&src_path, code).is_err() {
        return false;
    }

    let mut cmd = Command::new(&compiler.cmd[0]);
    cmd.arg(&src_path).arg("-o").arg(&exe_path);
    cmd.arg("-w");
    for arg in compile_args {
        cmd.arg(arg);
    }
    for arg in link_args {
        cmd.arg(arg);
    }

    let output = cmd.output();
    let result = output.as_ref().map(|o| o.status.success()).unwrap_or(false);
    if !result {
        if let Ok(ref o) = output {
            let stderr = String::from_utf8_lossy(&o.stderr);
            if !stderr.is_empty() && code.contains("statx") {}
        }
    }
    let _ = std::fs::remove_file(&src_path);
    let _ = std::fs::remove_file(&exe_path);
    result
}

fn try_run_code(compiler: &CompilerData, code: &str, args: &[CallArg]) -> Option<String> {
    try_run_code_werror(compiler, code, args, false)
}

fn try_run_code_werror(
    compiler: &CompilerData,
    code: &str,
    args: &[CallArg],
    werror: bool,
) -> Option<String> {
    let tmpdir = std::env::temp_dir();
    let suffix = get_source_suffix(compiler);
    let src_path = tmpdir.join(format!("meson_test{}", suffix));
    let exe_path = tmpdir.join("meson_test_exe");

    std::fs::write(&src_path, code).ok()?;

    let extra = extra_args_from_callargs(args);
    let mut cmd = Command::new(&compiler.cmd[0]);
    cmd.arg(&src_path).arg("-o").arg(&exe_path);
    if werror {
        cmd.arg("-Werror");
    } else {
        cmd.arg("-w");
    }
    for arg in &extra {
        cmd.arg(arg);
    }

    let compile_ok = cmd.output().map(|o| o.status.success()).unwrap_or(false);
    if !compile_ok {
        let _ = std::fs::remove_file(&src_path);
        return None;
    }

    let run_output = Command::new(&exe_path).output().ok()?;
    let _ = std::fs::remove_file(&src_path);
    let _ = std::fs::remove_file(&exe_path);

    if run_output.status.success() {
        Some(String::from_utf8_lossy(&run_output.stdout).to_string())
    } else {
        None
    }
}
