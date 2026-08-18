//! External tool invocation: assembler, linker, and dependency files.
//!
//! By default, the compiler uses built-in assembler and linker implementations.
//! When the `gcc_assembler` or `gcc_linker` Cargo features are enabled, GCC
//! can be used as a fallback. The `gcc_m16` feature enables GCC passthrough
//! for -m16 mode (16-bit real-mode boot code).

use super::Driver;
use crate::backend::Target;
#[cfg(feature = "gcc_assembler")]
use std::sync::Once;

/// Print a one-time warning when using a GCC-backed assembler for
/// source .s/.S files.
#[cfg(feature = "gcc_assembler")]
fn warn_gcc_source_assembler(command: &str) {
    static WARN_ONCE: Once = Once::new();
    WARN_ONCE.call_once(|| {
        eprintln!("WARNING: Using GCC-backed assembler for source files ({}) [gcc_assembler feature enabled]", command);
    });
}

/// Output mode for GCC -m16 passthrough compilation.
#[cfg(feature = "gcc_m16")]
pub(super) enum GccM16Mode {
    /// Generate assembly output (-S)
    Assembly,
    /// Generate object file (-c)
    Object,
}

impl Driver {
    /// Compile a C source file using GCC instead of the internal compiler.
    ///
    /// Only available when the `gcc_m16` Cargo feature is enabled.
    ///
    /// This is a hack for -m16 mode: the internal i686 backend produces code
    /// that is too large for the 32KB real-mode limit in Linux kernel boot code.
    /// Until our code size is competitive with GCC, we delegate -m16 compilation
    /// to GCC so the kernel can boot.
    ///
    /// TODO: Remove this once i686 code size optimizations bring boot code under 32KB.
    #[cfg(feature = "gcc_m16")]
    pub(super) fn compile_with_gcc_m16(
        &self,
        input_file: &str,
        output_path: &str,
        mode: GccM16Mode,
    ) -> Result<Option<String>, String> {
        // Warn prominently about using GCC for -m16 compilation
        static WARN_ONCE: std::sync::Once = std::sync::Once::new();
        WARN_ONCE.call_once(|| {
            eprintln!("WARNING: *** Using GCC for -m16 compilation (boot code) ***");
            eprintln!("WARNING: *** This delegates the ENTIRE compilation to GCC ***");
            eprintln!("WARNING: *** The gcc_m16 feature flag is enabled ***");
        });

        let mut cmd = std::process::Command::new("gcc");

        // Forward raw args, skipping -o <path>, -c, -S flags (we set those ourselves)
        let mut skip_next = false;
        for arg in &self.raw_args {
            if skip_next {
                skip_next = false;
                continue;
            }
            match arg.as_str() {
                "-o" => {
                    skip_next = true;
                    continue;
                }
                "-c" | "-S" => continue,
                _ => {}
            }
            cmd.arg(arg);
        }

        // Suppress warnings (GCC may warn about flags it doesn't recognize from our CLI)
        cmd.arg("-w");

        match mode {
            GccM16Mode::Assembly => {
                cmd.arg("-S");
                cmd.arg("-o").arg(output_path);
                cmd.arg(input_file);
                let result = cmd
                    .output()
                    .map_err(|e| format!("Failed to run GCC for -m16: {}", e))?;
                if !result.status.success() {
                    let stderr = String::from_utf8_lossy(&result.stderr);
                    return Err(format!(
                        "GCC -m16 compilation of {} failed: {}",
                        input_file, stderr
                    ));
                }
                let asm = std::fs::read_to_string(output_path).map_err(|e| {
                    format!("Cannot read GCC assembly output {}: {}", output_path, e)
                })?;
                Ok(Some(asm))
            }
            GccM16Mode::Object => {
                cmd.arg("-c");
                cmd.arg("-o").arg(output_path);
                cmd.arg(input_file);
                let result = cmd
                    .output()
                    .map_err(|e| format!("Failed to run GCC for -m16: {}", e))?;
                if !result.status.success() {
                    let stderr = String::from_utf8_lossy(&result.stderr);
                    return Err(format!(
                        "GCC -m16 compilation of {} failed: {}",
                        input_file, stderr
                    ));
                }
                Ok(None)
            }
        }
    }

    /// Assemble a .s or .S file to an object file.
    ///
    /// When the `gcc_assembler` Cargo feature is enabled, uses GCC for
    /// assembling (with a warning). When disabled (default), uses the
    /// built-in assembler with built-in C preprocessor for .S files.
    pub(super) fn assemble_source_file(
        &self,
        input_file: &str,
        output_path: &str,
    ) -> Result<(), String> {
        // When gcc_assembler feature is enabled, use GCC for assembling
        #[cfg(feature = "gcc_assembler")]
        {
            self.assemble_source_file_gcc(input_file, output_path, None)
        }

        // Default (gcc_assembler disabled): use built-in assembler
        #[cfg(not(feature = "gcc_assembler"))]
        {
            // Handle -Wa,--version: print GNU-compatible version string
            if self.assembler_extra_args.iter().any(|a| a == "--version") {
                println!("GNU assembler (rust-gcc built-in) 2.42");
                return Ok(());
            }
            self.assemble_source_file_builtin(input_file, output_path)
        }
    }

    /// Assemble a source file using an external GCC-backed assembler.
    ///
    /// Only compiled when the `gcc_assembler` feature is enabled.
    #[cfg(feature = "gcc_assembler")]
    fn assemble_source_file_gcc(
        &self,
        input_file: &str,
        output_path: &str,
        custom_command: Option<&str>,
    ) -> Result<(), String> {
        let config = self.target.assembler_config();
        let asm_command = custom_command.unwrap_or(config.command);

        // Warn when using GCC-backed assembler (custom path is OK)
        if custom_command.is_none() {
            warn_gcc_source_assembler(config.command);
        }

        let mut cmd = std::process::Command::new(asm_command);
        cmd.args(config.extra_args);

        let extra_asm_args = self.build_asm_extra_args();
        cmd.args(&extra_asm_args);

        // Pass through include paths and defines for .S preprocessing
        for path in &self.include_paths {
            cmd.arg("-I").arg(path);
        }
        for path in &self.quote_include_paths {
            cmd.arg("-iquote").arg(path);
        }
        for path in &self.isystem_include_paths {
            cmd.arg("-isystem").arg(path);
        }
        for path in &self.after_include_paths {
            cmd.arg("-idirafter").arg(path);
        }
        for def in &self.defines {
            if def.value == "1" {
                cmd.arg(format!("-D{}", def.name));
            } else {
                cmd.arg(format!("-D{}={}", def.name, def.value));
            }
        }
        for inc in &self.force_includes {
            cmd.arg("-include").arg(inc);
        }
        if self.nostdinc {
            cmd.arg("-nostdinc");
        }
        for undef in &self.undef_macros {
            cmd.arg(format!("-U{}", undef));
        }

        for flag in &self.assembler_extra_args {
            cmd.arg(format!("-Wa,{}", flag));
        }

        if let Some(ref lang) = self.explicit_language {
            cmd.arg("-x").arg(lang);
        }

        cmd.args(["-c", "-o", output_path, input_file]);

        if !self.assembler_extra_args.is_empty() {
            cmd.stdout(std::process::Stdio::inherit());
        }

        let result = cmd
            .output()
            .map_err(|e| format!("Failed to run assembler for {}: {}", input_file, e))?;

        if !result.status.success() {
            let stderr = String::from_utf8_lossy(&result.stderr);
            return Err(format!("Assembly of {} failed: {}", input_file, stderr));
        }

        Ok(())
    }

    /// Assemble a .s or .S source file using the built-in assembler.
    ///
    /// For .S files (assembly with C preprocessor directives), runs our built-in
    /// C preprocessor first to expand macros, includes, and conditionals, then
    /// passes the result to the target's builtin assembler.
    ///
    /// For .s files (pure assembly), reads the file directly and passes it
    /// to the builtin assembler.
    #[cfg_attr(feature = "gcc_assembler", allow(dead_code))] // Only called in standalone (non-gcc) assembler mode
    fn assemble_source_file_builtin(
        &self,
        input_file: &str,
        output_path: &str,
    ) -> Result<(), String> {
        let needs_cpp = input_file.ends_with(".S")
            || self.explicit_language.as_deref() == Some("assembler-with-cpp");
        let asm_text = if needs_cpp {
            // .S files (or -x assembler-with-cpp) need C preprocessing before assembly
            let source = Self::read_source(input_file)?;
            let mut preprocessor = crate::frontend::preprocessor::Preprocessor::new();
            self.configure_preprocessor(&mut preprocessor);
            // GCC defines __ASSEMBLER__ when preprocessing assembly source files (.S).
            // This is needed for headers like <cet.h> which gate assembly-specific
            // macro definitions (e.g. _CET_ENDBR) behind #ifdef __ASSEMBLER__.
            preprocessor.define_macro("__ASSEMBLER__", "1");
            // The built-in preprocessor doesn't ship with GCC's cet.h header.
            // When __CET__ is defined, ffitarget.h does `#include <cet.h>` which
            // would fail. We prevent this by pre-defining _CET_H_INCLUDED (the
            // include guard) so cet.h is skipped if encountered, then manually
            // define _CET_ENDBR and _CET_NOTRACK with the correct values.
            preprocessor.define_macro("_CET_H_INCLUDED", "1");
            if self.target == crate::backend::Target::X86_64 {
                preprocessor.define_macro("_CET_ENDBR", "endbr64");
            } else {
                preprocessor.define_macro("_CET_ENDBR", "endbr32");
            }
            preprocessor.define_macro("_CET_NOTRACK", "notrack");
            // In assembly mode, '$' is the AT&T immediate prefix, not part of
            // identifiers. Without this, `$FOO` is tokenized as one identifier
            // and the macro `FOO` is never expanded.
            preprocessor.set_asm_mode(true);
            preprocessor.set_filename(input_file);
            self.process_force_includes(&mut preprocessor)
                .map_err(|e| format!("Preprocessing {} failed: {}", input_file, e))?;
            preprocessor.preprocess(&source)
        } else {
            // .s files are pure assembly - read directly
            Self::read_source(input_file)?
        };

        // Debug: dump preprocessed assembly to /tmp/asm_debug_<basename>.s
        if std::env::var("CCC_ASM_DEBUG").is_ok() {
            let basename = std::path::Path::new(input_file)
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("unknown");
            let _ = std::fs::write(format!("/tmp/asm_debug_{}.s", basename), &asm_text);
        }

        let extra = self.build_asm_extra_args();
        self.target
            .assemble_with_extra(&asm_text, output_path, &extra)
    }

    /// Build extra assembler arguments for RISC-V ABI/arch overrides.
    ///
    /// When -mabi= or -march= are specified on the CLI, these override the
    /// defaults hardcoded in the assembler config. This is critical for the
    /// Linux kernel which uses -mabi=lp64 (soft-float) instead of the default
    /// lp64d (double-float), and -march=rv64imac... instead of rv64gc.
    /// The assembler uses these flags to set ELF e_flags (float ABI, RVC, etc.).
    pub(super) fn build_asm_extra_args(&self) -> Vec<String> {
        let mut args = Vec::new();
        // Only pass RISC-V flags to the RISC-V assembler. Passing -mabi/-march
        // to x86/ARM gcc would cause warnings or errors.
        if self.target == Target::Riscv64 {
            if let Some(ref abi) = self.riscv_abi {
                args.push(format!("-mabi={}", abi));
            }
            if let Some(ref march) = self.riscv_march {
                args.push(format!("-march={}", march));
            }
            if self.riscv_no_relax {
                args.push("-mno-relax".to_string());
            }
            // The RISC-V GNU assembler defaults to PIC mode, which causes
            // `la` pseudo-instructions to expand with R_RISCV_GOT_HI20 (GOT
            // indirection) instead of R_RISCV_PCREL_HI20 (direct PC-relative).
            // The Linux kernel does not have a GOT and expects PCREL relocations,
            // so we must explicitly pass -fno-pic when PIC is not requested.
            if !self.pic {
                args.push("-fno-pic".to_string());
            }
        }
        // Pass through any -Wa, flags from the command line. These are needed
        // when compiling C code that contains inline asm requiring specific
        // assembler settings (e.g., -Wa,-misa-spec=2.2 for RISC-V to enable
        // implicit zicsr in the old ISA spec, required by Linux kernel vDSO).
        for flag in &self.assembler_extra_args {
            args.push(format!("-Wa,{}", flag));
        }
        args
    }

    /// Build linker args from collected flags, preserving command-line ordering.
    ///
    /// Order-independent flags (-shared, -static, -nostdlib, -L paths) go first.
    /// Then linker_ordered_items provides the original CLI ordering of positional
    /// object/archive files, -l flags, and -Wl, pass-through flags. This ordering
    /// is critical for flags like -Wl,--whole-archive which must appear before
    /// the archive they affect.
    pub(super) fn build_linker_args(&self) -> Vec<String> {
        let mut args = Vec::new();
        if self.relocatable {
            // Relocatable link: merge .o files into a single .o without final linking.
            // -nostdlib prevents CRT startup files, -r tells ld to produce a .o.
            args.push("-nostdlib".to_string());
            args.push("-r".to_string());
        }
        if self.shared_lib {
            args.push("-shared".to_string());
        }
        if self.static_link {
            args.push("-static".to_string());
        }
        if self.nostdlib {
            args.push("-nostdlib".to_string());
        }
        for path in &self.linker_paths {
            args.push(format!("-L{}", path));
        }
        // Emit objects, -l flags, and -Wl, flags in their original command-line order.
        args.extend_from_slice(&self.linker_ordered_items);
        args
    }

    /// Write a Make-compatible dependency file for the given input/output.
    /// Format: "output: input\n"
    /// This is a minimal dependency file that tells make the object depends
    /// on its source file. A full implementation would also list included headers.
    pub(super) fn write_dep_file(&self, input_file: &str, output_file: &str) {
        if let Some(ref dep_path) = self.dep_file {
            let dep_path = if dep_path.is_empty() {
                // Derive from output: replace extension with .d
                let p = std::path::Path::new(output_file);
                p.with_extension("d").to_string_lossy().into_owned()
            } else {
                dep_path.clone()
            };
            let input_name = if input_file == "-" {
                "<stdin>"
            } else {
                input_file
            };
            let content = format!("{}: {}\n", output_file, input_name);
            let _ = std::fs::write(&dep_path, content);
        }
    }
}
