//! Tail call optimization: convert `call X; <epilogue>; ret` into `<epilogue>; jmp X`.
//!
//! This pass detects the pattern where a function call's return value is directly
//! returned by the enclosing function. The sequence:
//!     call TARGET      (or call *%r10)
//!     <callee-save restores from rbp>
//!     movq %rbp, %rsp
//!     popq %rbp
//!     ret
//! is transformed to:
//!     <callee-save restores from rbp>
//!     movq %rbp, %rsp
//!     popq %rbp
//!     jmp TARGET       (or jmp *%r10)
//!
//! This is critical for threaded interpreters (like wasm3) that use indirect
//! tail calls to dispatch between opcode handlers without overflowing the stack.
//!
//! SAFETY: We must NOT apply this optimization when:
//! 1. The function passes a pointer to a local variable to the callee.
//!    After frame teardown, such pointers become dangling. Detected by checking
//!    for `leaq offset(%rbp), %reg` instructions (address-of-local).
//! 2. The function uses dynamic stack allocation (__builtin_alloca / DynAlloca).
//!    Alloca'd memory lives below %rsp. After frame teardown (movq %rbp, %rsp),
//!    that memory is in unowned space and may be clobbered by the tail-called
//!    function's stack frame. Detected by checking for `subq %reg, %rsp`.

use super::super::types::*;

/// Scan the assembly for tail call opportunities and convert them.
/// Returns true if any changes were made.
pub(super) fn optimize_tail_calls(store: &mut LineStore, infos: &mut [LineInfo]) -> bool {
    let len = infos.len();
    let mut changed = false;

    // Track whether the function has unsafe stack usage that prevents tail calls:
    // address-of-local (lea from rbp/rsp) or dynamic alloca (subq %reg, %rsp).
    let mut func_suppress_tailcall = false;
    // Track whether we're inside a function (seen pushq %rbp or label)
    let mut in_function = false;

    let mut i = 0;
    while i < len {
        if infos[i].is_nop() {
            i += 1;
            continue;
        }

        // Detect function boundaries to reset the suppression flag
        match infos[i].kind {
            LineKind::Label => {
                let line = store.get(i);
                let trimmed = &line[infos[i].trim_start as usize..];
                // A global label (not starting with .L) indicates a new function
                if !trimmed.starts_with(".L") {
                    func_suppress_tailcall = false;
                    in_function = true;
                }
                i += 1;
                continue;
            }
            LineKind::Directive => {
                let line = store.get(i);
                let trimmed = &line[infos[i].trim_start as usize..];
                if trimmed == ".cfi_startproc" {
                    func_suppress_tailcall = false;
                    in_function = true;
                }
                i += 1;
                continue;
            }
            _ => {}
        }

        // Check for lea-of-local instructions: leaq offset(%rbp), %reg
        // or leaq offset(%rsp), %reg.
        // Also check for dynamic stack allocation (subq %reg, %rsp) which is
        // emitted by __builtin_alloca/DynAlloca. After frame teardown, alloca'd
        // memory lives below %rsp and may be clobbered by the tail-called function.
        if in_function && !func_suppress_tailcall {
            if let LineKind::Other { .. } = infos[i].kind {
                let line = store.get(i);
                let trimmed = &line[infos[i].trim_start as usize..];
                if (trimmed.starts_with("leaq ")
                    || trimmed.starts_with("leal ")
                    || trimmed.starts_with("lea "))
                    && (trimmed.contains("(%rbp)") || trimmed.contains("(%rsp)"))
                {
                    func_suppress_tailcall = true;
                }
                // Detect dynamic alloca: subq %rax, %rsp (or any register subtracted from rsp)
                if trimmed.starts_with("subq %") && trimmed.ends_with(", %rsp") {
                    func_suppress_tailcall = true;
                }
            }
        }

        if infos[i].kind != LineKind::Call {
            i += 1;
            continue;
        }

        // We found a call instruction. Check if it can be tail-call-optimized.
        // Skip if the function has unsafe stack usage (lea-of-local or alloca).
        if func_suppress_tailcall {
            i += 1;
            continue;
        }

        // Check if the sequence after it is purely epilogue
        // (callee-save restores + frame teardown + ret).
        if let Some(ret_idx) = is_tail_call_candidate(store, infos, i, len) {
            // Extract the call target
            let call_line = store.get(i);
            let trimmed = &call_line[infos[i].trim_start as usize..];

            if let Some(jmp_text) = convert_call_to_jmp(trimmed) {
                // NOP the call
                mark_nop(&mut infos[i]);

                // Replace the `ret` with `jmp TARGET`
                replace_line(
                    store,
                    &mut infos[ret_idx],
                    ret_idx,
                    format!("    {}", jmp_text),
                );

                changed = true;
            }
        }

        i += 1;
    }

    changed
}

/// Check if the instructions after a call at position `call_idx` form a pure
/// epilogue sequence ending in `ret`. Returns the index of the `ret` if so.
///
/// The allowed pattern between call and ret:
/// - LoadRbp (callee-save restores): movq offset(%rbp), %REG
/// - Other with text "movq %rbp, %rsp" (stack frame teardown)
/// - Pop with reg being rbp (popq %rbp)
/// - Directive lines (.cfi_*)
/// - Nop/Empty lines
/// - NOTHING that writes to %rax (the return value must pass through)
fn is_tail_call_candidate(
    store: &LineStore,
    infos: &[LineInfo],
    call_idx: usize,
    len: usize,
) -> Option<usize> {
    // Limit how far we scan forward
    let limit = (call_idx + 30).min(len);

    let mut found_frame_teardown = false;
    let mut found_pop_rbp = false;
    let mut j = call_idx + 1;

    while j < limit {
        if infos[j].is_nop() {
            j += 1;
            continue;
        }

        match infos[j].kind {
            LineKind::Empty => {
                j += 1;
                continue;
            }
            LineKind::Directive => {
                j += 1;
                continue;
            }
            LineKind::LoadRbp { reg, .. } => {
                // Callee-save restore from stack - OK, but must not restore to %rax (reg 0)
                if reg == 0 {
                    return None; // Writing to %rax would clobber the return value
                }
                j += 1;
                continue;
            }
            LineKind::Other { dest_reg } => {
                let trimmed = &store.get(j)[infos[j].trim_start as usize..];
                if trimmed == "movq %rbp, %rsp" {
                    found_frame_teardown = true;
                    j += 1;
                    continue;
                }
                // Any other instruction that writes a register - check if it's rax
                if dest_reg == 0 {
                    return None; // Writes to %rax
                }
                // Any other instruction is suspicious - bail out
                return None;
            }
            LineKind::Pop { reg } => {
                // popq %rbp is part of the epilogue
                if reg == 5 {
                    // rbp = register family 5
                    found_pop_rbp = true;
                    j += 1;
                    continue;
                }
                // Any other pop is suspicious
                return None;
            }
            LineKind::Ret => {
                // Found the ret! Make sure we saw the frame teardown
                if found_frame_teardown && found_pop_rbp {
                    return Some(j);
                }
                return None;
            }
            // Any other instruction kind (labels, jumps, calls, etc.) breaks the pattern
            _ => return None,
        }
    }

    None
}

/// Convert a `call TARGET` instruction text into `jmp TARGET`.
/// Returns None if the call format is not recognized.
fn convert_call_to_jmp(trimmed_call: &str) -> Option<String> {
    // Direct call: "call foo" or "call foo@PLT" or "callq foo"
    // Indirect call: "call *%r10" or "callq *%r10"
    let rest = if let Some(r) = trimmed_call.strip_prefix("callq ") {
        r
    } else if let Some(r) = trimmed_call.strip_prefix("call ") {
        r
    } else {
        return None;
    };

    if rest.starts_with('*') {
        // Indirect call: call *%r10 -> jmp *%r10
        Some(format!("jmp {}", rest))
    } else if rest.starts_with("__x86_indirect_thunk_") {
        // Retpoline thunk - skip for safety
        None
    } else {
        // Direct call: call foo@PLT -> jmp foo@PLT
        Some(format!("jmp {}", rest))
    }
}

#[cfg(test)]
mod tests {
    use super::super::super::peephole_optimize;

    #[test]
    fn test_tail_call_direct() {
        let asm = [
            "func:",
            "    pushq %rbp",
            "    .cfi_def_cfa_offset 16",
            "    .cfi_offset %rbp, -16",
            "    movq %rsp, %rbp",
            "    .cfi_def_cfa_register %rbp",
            "    subq $16, %rsp",
            "    movq %rbx, -16(%rbp)",
            "    movq %r12, -8(%rbp)",
            "    movq %rdi, %rbx",
            "    call foo",
            "    movq -16(%rbp), %rbx",
            "    movq -8(%rbp), %r12",
            "    movq %rbp, %rsp",
            "    popq %rbp",
            "    ret",
            ".size func, .-func",
        ]
        .join("\n")
            + "\n";
        let result = peephole_optimize(asm);
        assert!(
            result.contains("jmp foo"),
            "should convert call to jmp: {}",
            result
        );
        assert!(
            !result.contains("call foo"),
            "should not have call: {}",
            result
        );
        assert!(
            !result.contains("ret"),
            "should not have ret (replaced by jmp): {}",
            result
        );
    }

    #[test]
    fn test_tail_call_indirect() {
        let asm = [
            "func:",
            "    pushq %rbp",
            "    .cfi_def_cfa_offset 16",
            "    .cfi_offset %rbp, -16",
            "    movq %rsp, %rbp",
            "    .cfi_def_cfa_register %rbp",
            "    subq $48, %rsp",
            "    movq %rbx, -48(%rbp)",
            "    movq %r12, -40(%rbp)",
            "    movq %r13, -32(%rbp)",
            "    movq %r14, -24(%rbp)",
            "    movq %r15, -16(%rbp)",
            "    movq %rdi, %r10",
            "    call *%r10",
            "    movq -48(%rbp), %rbx",
            "    movq -40(%rbp), %r12",
            "    movq -32(%rbp), %r13",
            "    movq -24(%rbp), %r14",
            "    movq -16(%rbp), %r15",
            "    movq %rbp, %rsp",
            "    popq %rbp",
            "    ret",
            ".size func, .-func",
        ]
        .join("\n")
            + "\n";
        let result = peephole_optimize(asm);
        assert!(
            result.contains("jmp *%r10"),
            "should convert call *%r10 to jmp *%r10: {}",
            result
        );
        assert!(
            !result.contains("call *%r10"),
            "should not have call: {}",
            result
        );
        assert!(!result.contains("ret"), "should not have ret: {}", result);
    }

    #[test]
    fn test_no_tail_call_when_rax_used() {
        // If something between call and ret writes to %rax, it's not a tail call
        let asm = [
            "func:",
            "    pushq %rbp",
            "    movq %rsp, %rbp",
            "    subq $16, %rsp",
            "    call foo",
            "    movq %rax, %r12", // uses %rax result - but stores to r12
            "    movl $42, %eax",  // overwrites %rax!
            "    movq %rbp, %rsp",
            "    popq %rbp",
            "    ret",
            ".size func, .-func",
        ]
        .join("\n")
            + "\n";
        let result = peephole_optimize(asm);
        assert!(
            result.contains("call foo"),
            "should NOT convert when %rax is modified: {}",
            result
        );
        assert!(result.contains("ret"), "should keep ret: {}", result);
    }

    #[test]
    fn test_no_tail_call_when_extra_call() {
        let asm = [
            "func:",
            "    pushq %rbp",
            "    movq %rsp, %rbp",
            "    call foo",
            "    call bar",
            "    movq %rbp, %rsp",
            "    popq %rbp",
            "    ret",
            ".size func, .-func",
        ]
        .join("\n")
            + "\n";
        let result = peephole_optimize(asm);
        // Only the LAST call should be converted
        assert!(
            result.contains("call foo"),
            "first call should remain: {}",
            result
        );
        assert!(
            result.contains("jmp bar"),
            "second call should be tail-optimized: {}",
            result
        );
    }

    #[test]
    fn test_tail_call_plt() {
        let asm = [
            "func:",
            "    pushq %rbp",
            "    movq %rsp, %rbp",
            "    subq $16, %rsp",
            "    call foo@PLT",
            "    movq %rbp, %rsp",
            "    popq %rbp",
            "    ret",
            ".size func, .-func",
        ]
        .join("\n")
            + "\n";
        let result = peephole_optimize(asm);
        assert!(
            result.contains("jmp foo@PLT"),
            "should convert PLT call to jmp: {}",
            result
        );
    }

    #[test]
    fn test_no_tail_call_with_dyn_alloca() {
        // If the function uses dynamic stack allocation (alloca), the tail call
        // could clobber the alloca'd memory after frame teardown.
        let asm = [
            "test_alloca:",
            "    pushq %rbp",
            "    .cfi_def_cfa_offset 16",
            "    .cfi_offset %rbp, -16",
            "    movq %rsp, %rbp",
            "    .cfi_def_cfa_register %rbp",
            "    subq $32, %rsp",
            "    movq %rbx, -32(%rbp)",
            "    movq %r12, -24(%rbp)",
            "    addq $15, %rax",
            "    andq $-16, %rax",
            "    subq %rax, %rsp", // dynamic alloca!
            "    movq %rsp, %rax",
            "    movq %rax, -16(%rbp)",
            "    call memset",
            "    call printf",
            "    movq -32(%rbp), %rbx",
            "    movq -24(%rbp), %r12",
            "    movq %rbp, %rsp",
            "    popq %rbp",
            "    ret",
            ".size test_alloca, .-test_alloca",
        ]
        .join("\n")
            + "\n";
        let result = peephole_optimize(asm);
        assert!(
            result.contains("call printf"),
            "should NOT convert when alloca exists: {}",
            result
        );
        assert!(
            result.contains("ret"),
            "should keep ret when alloca exists: {}",
            result
        );
    }

    #[test]
    fn test_no_tail_call_with_lea_local() {
        // If the function takes address of a local (leaq offset(%rbp), %reg),
        // the tail call could pass a dangling stack pointer.
        let asm = [
            "func:",
            "    pushq %rbp",
            "    .cfi_def_cfa_offset 16",
            "    .cfi_offset %rbp, -16",
            "    movq %rsp, %rbp",
            "    .cfi_def_cfa_register %rbp",
            "    subq $32, %rsp",
            "    movq %rbx, -32(%rbp)",
            "    leaq -16(%rbp), %rsi", // takes address of local!
            "    movq %rdi, %rbx",
            "    call bar", // bar receives pointer to our local
            "    movq -32(%rbp), %rbx",
            "    movq %rbp, %rsp",
            "    popq %rbp",
            "    ret",
            ".size func, .-func",
        ]
        .join("\n")
            + "\n";
        let result = peephole_optimize(asm);
        assert!(
            result.contains("call bar"),
            "should NOT convert when lea of local exists: {}",
            result
        );
        assert!(
            result.contains("ret"),
            "should keep ret when lea of local exists: {}",
            result
        );
    }
}
