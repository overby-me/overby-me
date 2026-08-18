pub(crate) mod asm_expr; // Shared assembly expression evaluator (arithmetic, bitwise, parens)
pub(crate) mod asm_preprocess; // Shared GAS preprocessing: comments, macros, rept, conditionals
pub(crate) mod common;
#[allow(dead_code)] // Defines ELF standard constants/helpers; not all used by every backend
pub(crate) mod elf;
pub(crate) mod elf_writer_common; // Shared x86/i686 assembler ELF writer
#[cfg_attr(feature = "gcc_linker", allow(dead_code))]
// Built-in linker code unused when gcc handles linking
pub(crate) mod linker_common;
pub(crate) mod peephole_common; // Shared peephole optimizer utilities (word matching, LineStore)

// Shared codegen framework, split into focused modules:
pub(crate) mod call_abi; // Unified ABI classification: call args + callee params, stack computation
pub(crate) mod cast; // Cast and float operation classification
pub(crate) mod f128_softfloat; // Shared F128 soft-float orchestration (ARM + RISC-V)
pub(crate) mod generation; // Module/function/instruction dispatch
pub(crate) mod inline_asm; // InlineAsmEmitter trait and shared framework
pub(crate) mod stack_layout; // Stack layout: slot assignment, alloca coalescing, regalloc helpers
pub(crate) mod state; // CodegenState, StackSlot, SlotAddr
pub(crate) mod traits; // ArchCodegen trait with default implementations
pub(crate) mod x86_common; // Shared x86/i686 register names, condition codes, asm template parsing

// Register allocation and liveness analysis
pub(crate) mod liveness; // Live interval computation
pub(crate) mod regalloc; // Linear scan register allocator

pub(crate) mod arm;
pub(crate) mod i686;
pub(crate) mod riscv;
pub(crate) mod x86;

use crate::ir::reexports::IrModule;

/// Options that control code generation, parsed from CLI flags.
#[derive(Debug, Clone, Default)]
pub(crate) struct CodegenOptions {
    /// Whether to generate position-independent code (-fPIC/-fpic)
    pub(crate) pic: bool,
    /// Whether to replace `ret` with `jmp __x86_return_thunk` (-mfunction-return=thunk-extern)
    pub(crate) function_return_thunk: bool,
    /// Whether to replace indirect calls/jumps with retpoline thunks (-mindirect-branch=thunk-extern)
    pub(crate) indirect_branch_thunk: bool,
    /// Patchable function entry: (total_nops, nops_before_entry).
    /// -fpatchable-function-entry=N[,M] emits NOP padding around function entry points
    /// and records them in __patchable_function_entries for runtime patching (ftrace).
    pub(crate) patchable_function_entry: Option<(u32, u32)>,
    /// Whether to emit endbr64 at function entry points (-fcf-protection=branch).
    /// Required for Intel CET/IBT (Indirect Branch Tracking).
    pub(crate) cf_protection_branch: bool,
    /// Whether SSE is disabled (-mno-sse). When true, the x86 codegen avoids
    /// SSE/XMM instructions in variadic prologues (XMM register saving) and
    /// va_start sets fp_offset to overflow so va_arg never uses XMM regs.
    /// TODO: Full -mno-sse support would also need to avoid SSE in float
    /// operations, casts, and other FP codegen paths. Currently only the
    /// variadic ABI path is gated, which is sufficient for the Linux kernel.
    pub(crate) no_sse: bool,
    /// Whether to use only general-purpose registers (-mgeneral-regs-only).
    /// On AArch64, this prevents FP/SIMD register usage in variadic function
    /// prologues (no q0-q7 saves) and sets __vr_offs=0 in va_start.
    /// The Linux kernel uses this to avoid touching NEON/FP state.
    /// TODO: Full -mgeneral-regs-only support would also need to avoid NEON/FP in
    /// popcount, byte-swap, float casts, and other FP codegen paths. Currently only
    /// the variadic ABI path is gated, which is sufficient for the Linux kernel
    /// (kernel code doesn't use floats or popcount builtins in hot paths).
    pub(crate) general_regs_only: bool,
    /// Whether to use the kernel code model (-mcmodel=kernel). All symbols
    /// are assumed to be in the negative 2GB of the virtual address space.
    /// Uses absolute sign-extended 32-bit addressing (movq $symbol) for
    /// global address references, producing R_X86_64_32S relocations.
    pub(crate) code_model_kernel: bool,
    /// Whether to disable jump table emission for switch statements (-fno-jump-tables).
    /// When true, all switch statements use compare-and-branch chains instead of
    /// indirect jumps through a jump table. Required by the Linux kernel when building
    /// with retpoline (-mindirect-branch=thunk-extern) to avoid indirect jumps that
    /// objtool would reject.
    pub(crate) no_jump_tables: bool,
    /// Whether to suppress linker relaxation (-mno-relax, RISC-V only).
    /// When true, the codegen emits `.option norelax` at the top of the
    /// assembly output, which prevents the GNU assembler from generating
    /// R_RISCV_RELAX relocation entries. This is required for the Linux
    /// kernel's EFI stub, which uses -fpic -mno-relax to ensure no
    /// absolute symbol references are introduced by linker relaxation.
    pub(crate) no_relax: bool,
    /// Whether to emit debug info (.file/.loc directives) when compiling with -g.
    /// When true, the codegen emits DWARF line number directives based on
    /// source_spans attached to each IR instruction during lowering.
    pub(crate) debug_info: bool,
    /// Whether to place each function in its own ELF section (-ffunction-sections).
    /// When true, each function is emitted into `.text.funcname` instead of `.text`.
    /// This enables the linker's `--gc-sections` to discard unreferenced functions.
    pub(crate) function_sections: bool,
    /// Whether to place each data object in its own ELF section (-fdata-sections).
    /// When true, each global variable is emitted into its own section
    /// (e.g., `.data.varname`, `.rodata.varname`, `.bss.varname`).
    /// This enables the linker's `--gc-sections` to discard unreferenced data.
    pub(crate) data_sections: bool,
    /// Whether to prepend `.code16gcc` to the assembly output (-m16).
    /// When true, the GNU assembler treats the 32-bit instructions as code
    /// that will run in 16-bit real mode, adding operand/address-size override
    /// prefixes as needed. Used by the Linux kernel boot code.
    pub(crate) code16gcc: bool,
    /// Number of integer arguments passed in registers (i686 only, -mregparm=N).
    /// 0 = standard cdecl (all args on stack), 1-3 = pass first N integer args
    /// in EAX, EDX, ECX respectively. Used by the Linux kernel boot code
    /// (-mregparm=3) to reduce code size in 16-bit real mode.
    pub(crate) regparm: u8,
    /// Whether to omit the frame pointer (-fomit-frame-pointer).
    /// When true, functions do not set up EBP as a frame pointer, freeing it
    /// as a general register and saving prologue/epilogue instructions.
    /// Used by the Linux kernel boot code to reduce code size.
    pub(crate) omit_frame_pointer: bool,
    /// Whether to emit CFI directives (.cfi_startproc, .cfi_endproc, etc.)
    /// for generating .eh_frame unwind tables. Enabled by default (like GCC).
    /// Disabled by -fno-asynchronous-unwind-tables or -fno-unwind-tables.
    /// Many programs (LuaJIT, libunwind users) require .eh_frame for exception
    /// handling and stack unwinding.
    pub(crate) emit_cfi: bool,
}

/// Target architecture.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    X86_64,
    I686,
    Aarch64,
    Riscv64,
}

impl Target {
    /// Return the GCC-style target triple for this architecture.
    /// Used by configure scripts (via -dumpmachine) to detect the target.
    pub fn triple(&self) -> &'static str {
        match self {
            Target::X86_64 => "x86_64-linux-gnu",
            Target::I686 => "i686-linux-gnu",
            Target::Aarch64 => "aarch64-linux-gnu",
            Target::Riscv64 => "riscv64-linux-gnu",
        }
    }

    /// Return the dynamic linker path for this target.
    pub(crate) fn dynamic_linker(&self) -> &'static str {
        match self {
            Target::X86_64 => "/lib64/ld-linux-x86-64.so.2",
            Target::I686 => "/lib/ld-linux.so.2",
            Target::Aarch64 => "/lib/ld-linux-aarch64.so.1",
            Target::Riscv64 => "/lib/ld-linux-riscv64-lp64d.so.1",
        }
    }

    /// Return the implicit library search directories for this target.
    /// This is used by the driver to emit `LIBRARY_PATH=...` during verbose
    /// linking, which CMake parses to discover implicit link directories
    /// (needed for `find_library()` to locate libraries like libm in
    /// multiarch paths like /usr/lib/x86_64-linux-gnu/).
    pub(crate) fn implicit_library_paths(&self) -> String {
        let triple = self.triple();

        // GCC lib base paths and versions to probe
        let gcc_bases: &[&str] = match self {
            Target::X86_64 => &[
                "/usr/lib/gcc/x86_64-linux-gnu",
                "/usr/lib/gcc/x86_64-redhat-linux",
                "/usr/lib64/gcc/x86_64-linux-gnu",
            ],
            Target::I686 => &[
                "/usr/lib/gcc-cross/i686-linux-gnu",
                "/usr/lib/gcc/i686-linux-gnu",
                "/usr/lib/gcc/i386-linux-gnu",
            ],
            Target::Aarch64 => &[
                "/usr/lib/gcc-cross/aarch64-linux-gnu",
                "/usr/lib/gcc/aarch64-linux-gnu",
            ],
            Target::Riscv64 => &[
                "/usr/lib/gcc-cross/riscv64-linux-gnu",
                "/usr/lib/gcc/riscv64-linux-gnu",
            ],
        };
        let gcc_versions: &[&str] = &["14", "13", "12", "11", "10", "9", "8", "7"];

        let mut paths: Vec<String> = Vec::new();

        // Find GCC lib dir (contains crtbegin.o)
        'outer: for base in gcc_bases {
            for ver in gcc_versions {
                let dir = format!("{}/{}", base, ver);
                if std::path::Path::new(&format!("{}/crtbegin.o", dir)).exists() {
                    paths.push(dir);
                    break 'outer;
                }
            }
        }

        // Multiarch lib dirs
        let lib_dir = format!("/usr/lib/{}", triple);
        if std::path::Path::new(&lib_dir).exists() {
            paths.push(lib_dir);
        }
        let lib_alt = format!("/lib/{}", triple);
        if std::path::Path::new(&lib_alt).exists() {
            paths.push(lib_alt);
        }

        // Cross-compiler lib dirs
        let cross_lib = format!("/usr/{}/lib", triple);
        if std::path::Path::new(&cross_lib).exists() {
            paths.push(cross_lib);
        }

        // Generic fallback dirs
        for dir in &["/usr/lib", "/lib"] {
            if std::path::Path::new(dir).exists() {
                paths.push(dir.to_string());
            }
        }

        paths.join(":")
    }

    /// Whether this target uses 32-bit pointers (ILP32 data model).
    pub(crate) fn is_32bit(&self) -> bool {
        matches!(self, Target::I686)
    }

    /// Pointer size in bytes for this target.
    pub(crate) fn ptr_size(&self) -> usize {
        if self.is_32bit() {
            4
        } else {
            8
        }
    }

    /// Get the assembler config for this target.
    /// Only used when the `gcc_assembler` feature is enabled for GCC fallback.
    #[cfg_attr(not(feature = "gcc_assembler"), allow(dead_code))]
    pub(crate) fn assembler_config(&self) -> common::AssemblerConfig {
        match self {
            Target::X86_64 => common::AssemblerConfig {
                command: "gcc",
                extra_args: &[],
            },
            Target::I686 => common::AssemblerConfig {
                command: "i686-linux-gnu-gcc",
                extra_args: &["-m32"],
            },
            Target::Aarch64 => common::AssemblerConfig {
                command: "aarch64-linux-gnu-gcc",
                extra_args: &["-march=armv8-a+crc+crypto"],
            },
            Target::Riscv64 => common::AssemblerConfig {
                command: "riscv64-linux-gnu-gcc",
                extra_args: &["-march=rv64gc", "-mabi=lp64d"],
            },
        }
    }

    /// Get the linker config for this target.
    pub(crate) fn linker_config(&self) -> common::LinkerConfig {
        // ELF e_machine constants (from elf.h):
        // EM_386 = 3, EM_AARCH64 = 183, EM_X86_64 = 62, EM_RISCV = 243
        match self {
            Target::X86_64 => common::LinkerConfig {
                command: "gcc",
                extra_args: &["-no-pie"],
                expected_elf_machine: 62, // EM_X86_64
                arch_name: "x86-64",
            },
            Target::I686 => common::LinkerConfig {
                command: "i686-linux-gnu-gcc",
                extra_args: &["-m32", "-no-pie"],
                expected_elf_machine: 3, // EM_386
                arch_name: "i686",
            },
            Target::Aarch64 => common::LinkerConfig {
                command: "aarch64-linux-gnu-gcc",
                // Use -no-pie to match non-PIC code generation.  The previous
                // default of -static prevented dlopen() of shared libraries
                // at runtime, breaking postgres extension loading.  The unit
                // test harness passes -static explicitly for QEMU user-mode.
                extra_args: &["-no-pie"],
                expected_elf_machine: 183, // EM_AARCH64
                arch_name: "aarch64",
            },
            Target::Riscv64 => common::LinkerConfig {
                command: "riscv64-linux-gnu-gcc",
                extra_args: &["-no-pie"],
                expected_elf_machine: 243, // EM_RISCV
                arch_name: "riscv64",
            },
        }
    }

    /// Generate assembly with full codegen options and optional source manager for debug info.
    /// When `source_mgr` is provided and `opts.debug_info` is true, the codegen emits
    /// .file/.loc directives for DWARF line number information.
    pub(crate) fn generate_assembly_with_opts_and_debug(
        &self,
        module: &IrModule,
        opts: &CodegenOptions,
        source_mgr: Option<&crate::common::source::SourceManager>,
    ) -> String {
        match self {
            Target::X86_64 => {
                let mut cg = x86::X86Codegen::new();
                cg.apply_options(opts);
                cg.state.function_sections = opts.function_sections;
                cg.state.data_sections = opts.data_sections;
                let raw = generation::generate_module_with_debug(
                    &mut cg,
                    module,
                    opts.debug_info,
                    source_mgr,
                );
                x86::codegen::peephole::peephole_optimize(raw)
            }
            Target::I686 => {
                let mut cg = i686::I686Codegen::new();
                cg.apply_options(opts);
                cg.state.function_sections = opts.function_sections;
                cg.state.data_sections = opts.data_sections;
                let raw = generation::generate_module_with_debug(
                    &mut cg,
                    module,
                    opts.debug_info,
                    source_mgr,
                );
                let optimized = i686::codegen::peephole::peephole_optimize(raw);
                if opts.code16gcc {
                    format!(".code16gcc\n{}", optimized)
                } else {
                    optimized
                }
            }
            Target::Aarch64 => {
                let mut cg = arm::ArmCodegen::new();
                cg.apply_options(opts);
                cg.state.function_sections = opts.function_sections;
                cg.state.data_sections = opts.data_sections;
                let raw = generation::generate_module_with_debug(
                    &mut cg,
                    module,
                    opts.debug_info,
                    source_mgr,
                );
                arm::codegen::peephole::peephole_optimize(raw)
            }
            Target::Riscv64 => {
                let mut cg = riscv::RiscvCodegen::new();
                cg.apply_options(opts);
                cg.state.function_sections = opts.function_sections;
                cg.state.data_sections = opts.data_sections;
                cg.emit_pre_directives();
                let raw = generation::generate_module_with_debug(
                    &mut cg,
                    module,
                    opts.debug_info,
                    source_mgr,
                );
                riscv::codegen::peephole::peephole_optimize(raw)
            }
        }
    }

    /// Assemble text to object file with dynamic extra arguments.
    /// Used to pass through -mabi= and -march= flags from the CLI.
    ///
    /// When the `gcc_assembler` Cargo feature is enabled, uses GCC for assembling
    /// (with a warning). When disabled (default), uses the built-in assembler.
    pub(crate) fn assemble_with_extra(
        &self,
        asm_text: &str,
        output_path: &str,
        extra_args: &[String],
    ) -> Result<(), String> {
        // When gcc_assembler feature is enabled, use GCC for assembling
        #[cfg(feature = "gcc_assembler")]
        {
            common::assemble_with_extra(&self.assembler_config(), asm_text, output_path, extra_args)
        }

        // Default (gcc_assembler disabled): use the built-in assembler
        #[cfg(not(feature = "gcc_assembler"))]
        {
            // Handle -Wa,--version: print GNU-compatible version string for
            // kernel build system's as-version.sh probe.
            if extra_args.iter().any(|a| a == "--version") {
                println!("GNU assembler (rust-gcc built-in) 2.42");
                return Ok(());
            }

            match self {
                Target::Aarch64 => arm::assembler::assemble(asm_text, output_path),
                Target::X86_64 => x86::assembler::assemble(asm_text, output_path),
                Target::Riscv64 => {
                    riscv::assembler::assemble_with_args(asm_text, output_path, extra_args)
                }
                Target::I686 => i686::assembler::assemble(asm_text, output_path),
            }
        }
    }

    /// Link object files into executable.
    pub(crate) fn link(&self, object_files: &[&str], output_path: &str) -> Result<(), String> {
        self.link_with_args(object_files, output_path, &[])
    }

    /// Link object files with additional user-provided linker args.
    ///
    /// By default, uses the built-in native linker for all architectures.
    /// When the `gcc_linker` Cargo feature is enabled, GCC can be used as a
    /// fallback for operations the built-in linker doesn't support (e.g.,
    /// -shared, -r).
    pub(crate) fn link_with_args(
        &self,
        object_files: &[&str],
        output_path: &str,
        user_args: &[String],
    ) -> Result<(), String> {
        common::link_with_args(&self.linker_config(), object_files, output_path, user_args)
    }
}
