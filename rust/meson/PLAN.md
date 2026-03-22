# Rust Meson Rewrite Plan

Drop-in replacement for [Meson](https://github.com/mesonbuild/meson) build system in Rust.
Inspired by [muon](https://github.com/muon-build/muon) C implementation.

## Architecture

Like muon, we use a bytecode compiler + stack VM instead of AST-walking interpreter.

```text
meson.build → Lexer → Parser → AST → Compiler → Bytecode → VM → Build Graph → Backend → build.ninja
```

## Phases

### Phase 1: Core Language (Lexer, Parser, AST, Compiler, VM)

- [x] Lexer: tokenize meson.build DSL
- [x] Parser: produce AST from tokens
- [x] AST types: all node types for the language
- [x] Compiler: AST → bytecode
- [x] VM: execute bytecode (stack-based)
- [x] Types: string, int, bool, array, dict, disabler
- [x] Operators: arithmetic, comparison, logical, string/array concat
- [x] Control flow: if/elif/else/endif, foreach/endforeach, break, continue
- [x] String interpolation: f-strings with @var@
- [x] Method calls on built-in types

### Phase 2: Built-in Functions & Objects

- [x] project() — declare project metadata
- [x] message/warning/error — output functions
- [x] executable() — declare executable target
- [x] static_library() / shared_library() / library() / both_libraries()
- [x] dependency() — find external dependencies
- [x] find_program() — find programs on PATH
- [x] custom_target() — custom build steps
- [x] configure_file() — generate files from templates
- [x] install_headers/data/subdir/man/emptydir/symlink
- [x] declare_dependency() — create internal dependency
- [x] subdir() — process subdirectory
- [x] subproject() — process subproject
- [x] test() / benchmark()
- [x] environment() — environment variable manipulation
- [x] generator() — source generators
- [x] vcs_tag() — version control tag
- [x] run_command() — run external commands at configure time
- [x] include_directories()
- [x] import() — load modules
- [x] files() — mark files
- [x] join_paths() — path joining
- [x] get_option() — read project options
- [x] configuration_data() — config data dict
- [x] is_variable/get_variable/set_variable
- [x] assert/summary
- [x] range() — integer ranges
- [x] structured_sources() — grouped sources
- [x] meson object (meson.version(), source_root(), etc.)
- [x] build_machine / host_machine / target_machine objects

### Phase 3: Compiler & Dependency Detection

- [x] Compiler detection (C, C++, Rust, etc.)
- [x] Compiler object methods (check_header, has_function, sizeof, etc.)
- [x] Pkg-config dependency finder
- [x] CMake dependency finder
- [x] Config-tool dependency finder
- [x] System/library dependency finder
- [x] Framework dependency finder (macOS)
- [x] Dependency fallbacks and subproject wraps

### Phase 4: Ninja Backend

- [x] Build graph construction from interpreter state
- [x] Ninja file generation (build.ninja, rules, build edges)
- [x] Compile rules per language/compiler
- [x] Link rules (executable, shared lib, static lib)
- [x] Custom target rules
- [x] Install targets
- [x] Test targets
- [x] Rpath handling
- [x] Response files for long command lines
- [x] Unity builds
- [x] PCH (precompiled headers)
- [x] Cross-compilation support

### Phase 5: Options System

- [x] Built-in options (buildtype, warning_level, default_library, etc.)
- [x] meson_options.txt / meson.options parser
- [x] Option types: string, boolean, combo, integer, array, feature
- [x] Subproject options
- [x] Cross/native file parsing

### Phase 6: CLI & Commands

- [x] setup — configure project
- [x] compile — build (invoke ninja)
- [x] test — run tests
- [x] install — install artifacts
- [x] configure — reconfigure
- [x] introspect — JSON project queries
- [x] init — scaffold new project
- [x] dist — create release archives
- [x] wrap — manage wraps
- [x] subprojects — manage subprojects
- [x] rewrite — programmatic modifications
- [x] devenv — developer environment
- [x] env2mfile — generate machine files
- [x] format/fmt — format meson.build

### Phase 7: Modules

- [x] fs — filesystem operations
- [x] pkgconfig — generate .pc files
- [x] python — Python extension building
- [x] gnome — GLib/GNOME helpers
- [x] cmake — CMake integration
- [x] rust — Rust helpers
- [x] windows — Windows resource compilation
- [x] i18n — internationalization
- [x] qt4/qt5/qt6 — Qt helpers
- [x] sourceset — conditional source sets
- [x] keyval — key-value file parsing
- [x] wayland — Wayland protocol helpers
- [x] cuda — CUDA helpers
- [x] hotdoc — documentation generation
- [x] java — Java/JAR support
- [x] dlang — D language helpers
- [x] external_project — wrap external build systems
- [x] icestorm — FPGA toolchain

### Phase 8: Polish & Compatibility

- [ ] Meson test suite compatibility
- [ ] Edge cases and error messages matching Meson
- [ ] Performance optimization
- [ ] Documentation
