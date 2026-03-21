# Frontend

The frontend transforms C source text into a type-annotated abstract syntax tree
suitable for IR lowering. It is organized as a four-phase pipeline where each
phase has a single, well-defined responsibility and communicates with the next
through an explicit interface type.

## Pipeline Overview

```text
                         +-----------------+
   Source text (.c)  --> | 1. Preprocessor | --> Expanded text (String)
                         +-----------------+
                                 |
                                 v
                         +-----------------+
   Expanded text     --> |    2. Lexer     | --> Token stream (Vec<Token>)
                         +-----------------+
                                 |
                                 v
                         +-----------------+
   Token stream      --> |    3. Parser    | --> AST (TranslationUnit)
                         +-----------------+
                                 |
                                 v
                         +-----------------+
   AST               --> |     4. Sema     | --> AST + SemaResult
                         +-----------------+
                                 |
                                 v
                       (consumed by IR lowerer)
```

Each phase is a self-contained Rust module under `src/frontend/`. The top-level
`mod.rs` re-exports the four submodules with `pub(crate)` visibility so that the
rest of the compiler can access them without reaching into internal details.

---

## Phase 1: Preprocessor (`preprocessor/`)

The preprocessor is a text-to-text pass. It consumes raw C source code as a
`&str` and produces a fully expanded `String` with all directives resolved,
macros expanded, and comments removed. The expanded output contains embedded
line markers (`# line "file"`) so that downstream phases can map byte offsets
back to original source locations.

### Public interface

- **`Preprocessor::new()`** -- creates a preprocessor instance with predefined
  macros (`__GNUC__`, `__x86_64__`, `__LINE__`, `__FILE__`, etc.) and builtin
  function-like macros already registered.
- **`Preprocessor::preprocess(&mut self, source: &str) -> String`** -- the main
  entry point. Strips comments, joins line continuations, processes directives
  (`#include`, `#define`, `#if`/`#ifdef`/`#elif`/`#else`/`#endif`, `#pragma`,
  `#error`, `#warning`, `#line`), and expands macros.
- Configuration methods (`add_include_path`, `set_target`, `define_macro`, etc.)
  allow the driver to set up search paths and target-specific state before
  preprocessing begins.

### Output contract

The preprocessor produces a single `String` of expanded C source code. This
string is syntactically valid C (modulo line markers) and is ready to be
tokenized. It also populates side-channel data:

| Field | Type | Purpose |
|-------|------|---------|
| `errors` | `Vec<PreprocessorDiagnostic>` | Errors from `#error`, missing includes, etc. |
| `warnings` | `Vec<PreprocessorDiagnostic>` | Warnings from `#warning` directives |
| `weak_pragmas` | `Vec<(String, Option<String>)>` | `#pragma weak` directives for the linker |
| `redefine_extname_pragmas` | `Vec<(String, String)>` | `#pragma redefine_extname` directives |
| `macro_expansion_info` | `Vec<MacroExpansionInfo>` | Per-line macro expansion metadata for diagnostics |

### Files

`pipeline.rs`, `macro_defs.rs`, `conditionals.rs`, `expr_eval.rs`,
`includes.rs`, `builtin_macros.rs`, `predefined_macros.rs`, `pragmas.rs`,
`text_processing.rs`, `utils.rs`.

For detailed per-file documentation, see [preprocessor/README.md](preprocessor/README.md).

---

## Phase 2: Lexer (`lexer/`)

The lexer tokenizes the preprocessed text into a flat `Vec<Token>`. Each token
carries a `TokenKind` discriminant and a `Span` (byte-offset range plus file
ID) for source location tracking.

### Public interface

- **`Lexer::new(input: &str, file_id: u32) -> Self`** -- creates a lexer over
  the preprocessed text. The `file_id` is embedded in every `Span` so the
  source manager can map offsets back to file names.
- **`Lexer::tokenize(&mut self) -> Vec<Token>`** -- scans the entire input and
  returns all tokens, terminated by `TokenKind::Eof`. Uses a capacity heuristic
  of one token per five bytes to minimize reallocation.
- **`Lexer::set_gnu_extensions(&mut self, enabled: bool)`** -- toggles GNU
  extension support (enabled by default). When enabled, the bare keywords
  `typeof` and `asm` (without double-underscore prefix) are recognized.
  Note: `$` in identifiers is always permitted regardless of this flag.

### Output contract

The token stream is a `Vec<Token>` where each `Token` is a struct with
`kind: TokenKind` and `span: Span` fields. `TokenKind` covers:

- **Integer literals**: six variants distinguished by suffix --
  `IntLiteral`, `UIntLiteral`, `LongLiteral`, `ULongLiteral`,
  `LongLongLiteral`, `ULongLongLiteral`.
- **Floating-point literals**: `FloatLiteral` (double), `FloatLiteralF32`
  (float with `f`/`F` suffix), `FloatLiteralLongDouble` (long double with
  `l`/`L` suffix, carrying both an `f64` approximation and full IEEE 754
  binary128 bytes).
- **Imaginary literals** (GCC extension): `ImaginaryLiteral` (double imaginary),
  `ImaginaryLiteralF32` (float imaginary), `ImaginaryLiteralLongDouble`
  (long double imaginary).
- **String literals**: `StringLiteral` (narrow), `WideStringLiteral` (`L"..."`),
  `Char16StringLiteral` (`u"..."`). Character literals use `CharLiteral`.
- **Identifiers and keywords**: all C11 keywords plus GNU/GCC extensions
  (`__attribute__`, `__builtin_*`, `__extension__`, `__int128`, `__seg_gs`,
  `__seg_fs`, `__auto_type`, `__label__`, etc.).
- **Pragma tokens**: the preprocessor injects synthetic `TokenKind` variants
  for pragma directives that affect parsing: `PragmaPackSet`, `PragmaPackPush`,
  `PragmaPackPushOnly`, `PragmaPackPop`, `PragmaPackReset`,
  `PragmaVisibilityPush`, and `PragmaVisibilityPop`. These appear inline in
  the token stream and are consumed by the parser to adjust struct packing
  and symbol visibility state.
- **Operators and punctuation**: the full set of C operators, braces, and
  delimiters.

Non-UTF-8 source bytes are handled through a private-use-area (PUA) encoding
scheme (see `common/encoding.rs`) so that the lexer can operate on the input as
a byte slice without rejecting arbitrary source files.

### Files

`scan.rs`, `token.rs`.

For detailed per-file documentation, see [lexer/README.md](lexer/README.md).

---

## Phase 3: Parser (`parser/`)

The parser is a hand-written recursive descent parser that consumes the token
stream and produces a typed AST rooted at `TranslationUnit`.

### Public interface

- **`Parser::new(tokens: Vec<Token>) -> Self`** -- takes ownership of the
  token stream. The `DiagnosticEngine` is configured after construction via
  a separate `set_diagnostics()` method. Typedef names are seeded from a
  hardcoded `builtin_typedefs()` list of approximately 90 common C standard
  library and system types (`size_t`, `int32_t`, `FILE`, `pthread_t`, `va_list`,
  etc.), not from the preprocessor's macro table. This is necessary because the
  compiler does not always process real system headers.
- **`Parser::parse(&mut self) -> TranslationUnit`** -- the main entry point.
  Parses the entire translation unit and returns the AST.

### Internal organization

The parser is split across six `impl Parser` blocks: the core in `parse.rs`
and five extension files, each extending the same `Parser` struct with
`pub(super)` methods:

| Module | Scope |
|--------|-------|
| `expressions.rs` | Operator precedence climbing from comma through primary expressions |
| `types.rs` | Type specifier collection and resolution, struct/union/enum bodies |
| `statements.rs` | All statement forms including inline assembly (GCC extended syntax) |
| `declarations.rs` | External and local declarations, K&R parameters, initializers |
| `declarators.rs` | C declarator syntax (the inside-out rule), parameter lists |

### Output contract

The parser produces a `TranslationUnit` -- a list of `ExternalDecl` nodes, each
of which is one of:

- **`FunctionDef`** -- a function definition with return type, parameters, body
  (as a `CompoundStmt`), and a packed `FunctionAttributes` bitfield carrying
  storage class, inline hints, and GCC `__attribute__` flags.
- **`Declaration`** -- a variable, typedef, struct/union/enum definition, or
  forward declaration. `_Static_assert` is also handled within declaration
  processing. Uses a packed `flags: u16` bitfield for storage class and
  qualifier booleans.
- **`TopLevelAsm`** -- a top-level `asm("...")` directive passed through
  verbatim.

AST nodes use `Span` annotations for source locations and packed bitfields
(rather than individual `bool` fields) for memory efficiency. Attributes like
`section`, `visibility`, and `alignment` that carry non-boolean payloads remain
as `Option<String>` or `Option<usize>`.

### Files

`parse.rs`, `expressions.rs`, `types.rs`, `statements.rs`, `declarations.rs`,
`declarators.rs`, `ast.rs`.

For detailed per-file documentation, see [parser/README.md](parser/README.md).

---

## Phase 4: Semantic Analysis (`sema/`)

Sema walks the AST to collect type information, build a scoped symbol table,
resolve typedefs and `typeof(expr)`, evaluate compile-time constants, and check
for common errors. It does not transform the AST; instead, it produces a
`SemaResult` structure that accompanies the AST into the IR lowering phase.

`SemanticAnalyzer` implements the `TypeConvertContext` trait (defined in
`common/type_builder`), which provides the shared `resolve_type_spec_to_ctype`
method for converting AST `TypeSpecifier` nodes to `CType` values. The trait
callbacks delegate typedef, struct/union/enum, and constant-expression
resolution back into sema's own state.

### Public interface

- **`SemanticAnalyzer::new() -> Self`** -- creates an analyzer with a fresh
  symbol table and pre-populated implicit function declarations (common libc
  functions like `memcpy`, `printf`, etc.).
- **`SemanticAnalyzer::analyze(&mut self, tu: &TranslationUnit) -> Result<(), usize>`**
  -- walks the entire translation unit. Returns `Ok(())` on success or
  `Err(error_count)` if hard errors were emitted through the diagnostic engine.
- **`SemanticAnalyzer::into_result(self) -> SemaResult`** -- consumes the
  analyzer and returns the collected semantic information.

### Output contract

`SemaResult` bundles everything the IR lowerer needs:

| Field | Type | Contents |
|-------|------|----------|
| `functions` | `FxHashMap<String, FunctionInfo>` | Return type, parameter types, variadic flag, definition status, and noreturn attribute for every function encountered |
| `type_context` | `TypeContext` | Struct/union layouts, typedef resolutions, enum constants, function typedef metadata, packed enum types, and a type cache |
| `expr_types` | `ExprTypeMap` | Per-expression type annotations keyed by `ExprId` (AST node identity) |
| `const_values` | `ConstMap` | Pre-evaluated compile-time constants (float literals, sizeof, cast chains, binary ops with correct signedness) |

The `TypeContext` is the central type-system data structure shared between sema
and the lowerer. It holds:

- Struct/union layouts (`Rc<StructLayout>`) indexed by tag name
- Typedef mappings (name to resolved `CType`)
- Per-typedef alignment overrides from `__attribute__((aligned(N)))`
- Enum constant values
- Function typedef info (bare function typedefs and function pointer typedefs)
- Packed enum type info for forward references
- A per-function return-type cache
- A `CType` cache for named struct/union types

### Files

`analysis.rs`, `builtins.rs`, `type_context.rs`, `type_checker.rs`, `const_eval.rs`.

For detailed per-file documentation, see [sema/README.md](sema/README.md).

---

## Data Flow Between Phases

The following diagram shows the concrete Rust types that flow between phases:

```text
  [Source: &str]
       |
       | Preprocessor::preprocess()
       v
  [Expanded: String]  +  [PreprocessorDiagnostic]  +  [#pragma side data]
       |
       | Lexer::tokenize()
       v
  [Tokens: Vec<Token>]        Token = struct { kind: TokenKind, span: Span }
       |
       | Parser::parse()
       v
  [AST: TranslationUnit]      TranslationUnit = struct { decls: Vec<ExternalDecl> }
       |
       | SemanticAnalyzer::analyze()
       v
  [AST: &TranslationUnit]  +  [SemaResult]
       |                          |--- functions: FxHashMap<String, FunctionInfo>
       |                          |--- type_context: TypeContext
       |                          |--- expr_types: FxHashMap<ExprId, CType>
       |                          '--- const_values: FxHashMap<ExprId, IrConst>
       v
  (IR lowering consumes both the AST and SemaResult by ownership)
```

Two points of cross-phase information flow deserve special mention:

1. **Typedef names in the parser.** The C grammar is ambiguous:
   `T * x;` is either a multiplication expression or a pointer declaration,
   depending on whether `T` is a typedef name. The parser maintains its own set
   of typedef names, seeded from a hardcoded `builtin_typedefs()` list of
   approximately 90 common C standard library and system types (`size_t`,
   `int32_t`, `FILE`, `pthread_t`, `va_list`, `__Float32x4_t`, etc.) and
   extended during parsing as `typedef` declarations are encountered. No
   preprocessor state flows to the parser -- only the expanded source text
   (indirectly, through the lexer's token stream).

2. **Expression identity from parser to sema.** Sema annotates expressions by
   keying on `ExprId`, which is derived from the identity (address) of each AST
   `Expr` node. This works because the AST is heap-allocated during parsing and
   is never moved or reallocated before the lowerer consumes it, so node
   addresses are stable across the entire compilation pipeline.

---

## Key Design Decisions

### Text-based preprocessor

The preprocessor operates on raw text rather than tokens. This is the simpler
approach (matching GCC's historical architecture) but means that precise source
locations for macro-expanded code are approximations. The line markers embedded
in the output (`# line "file"`) provide file-level and line-level accuracy, but
column information within macro expansions is lost. This trade-off was chosen
for implementation simplicity: a token-based preprocessor (like Clang's) would
provide better diagnostics at the cost of significantly more complex
architecture.

### Eager tokenization

The lexer produces a complete `Vec<Token>` rather than exposing an iterator or
lazy stream. This simplifies the parser, which needs arbitrary lookahead and
backtracking for C's notoriously context-sensitive grammar (declarators,
type names, K&R parameter lists). The memory cost is modest -- the capacity
heuristic of one token per five bytes of source keeps allocation tight.

### Recursive descent with split modules

The parser uses recursive descent with operator precedence climbing for
expressions, rather than a parser generator. This gives full control over error
recovery and makes it straightforward to handle C's context-sensitive parsing
(e.g., the typedef/identifier ambiguity, declarator syntax, statement
expressions). The parser is split across multiple files that each add methods to
the same `Parser` struct via `impl Parser` blocks with `pub(super)` visibility.
This avoids trait dispatch overhead while keeping each file focused on a single
syntactic domain.

### Packed bitfields for AST attributes

Attribute booleans on `FunctionAttributes`, `Declaration`, and related AST nodes
are stored as packed bitfields (`u16` or `u32`) rather than individual `bool`
fields. `FunctionAttributes` packs 13 boolean flags into a `u16`;
`Declaration` packs 9 boolean flags into a `u16`. Accessor methods
(getter/setter pairs) provide the same ergonomic API as named struct fields.

### Sema as an information-gathering pass

Sema is primarily an information-gathering pass rather than a strict type
checker. It walks the AST to populate the `SemaResult` (function signatures,
type context, expression types, constant values) that the lowerer needs. While
it does emit diagnostics for detectable errors (e.g., undeclared identifiers,
incompatible types in return statements, `-Wreturn-type` warnings), it does not
yet reject all type errors. The lowerer still performs some type resolution as a
fallback. This is a pragmatic choice that allowed the compiler to handle
real-world C code early; the intent is to move more checking into sema over
time.

### Undo-log scoping in TypeContext

`TypeContext` uses an undo-log pattern for scope management rather than cloning
entire hash maps. Each scope push creates a `TypeScopeFrame` that records newly
added keys and shadowed key-value pairs. Scope pop replays the undo log to
restore the previous state. This gives O(changes-in-scope) cost instead of
O(total-map-size), which matters for deeply nested scopes in large translation
units. `Rc<StructLayout>` is used for struct layouts so that save/restore
operations are cheap reference-count bumps.

### Multi-line macro argument accumulation

When the preprocessor encounters a line with unbalanced parentheses during macro
invocation, it accumulates subsequent lines before expanding. This handles
macro calls spanning many lines, including extreme cases like QEMU's generated
`QLIT_QLIST()` invocations that span tens of thousands of lines. A safety limit
(`MAX_PENDING_NEWLINES = 100_000`) prevents runaway accumulation from genuinely
unbalanced parentheses.

### Include guard optimization

After preprocessing an included file, the preprocessor scans for the classic
include guard pattern (`#ifndef GUARD` / `#define GUARD` / `#endif`). On
subsequent `#include` of the same file, if the guard macro is still defined, the
file is skipped entirely without re-reading or re-processing. This matches the
optimization performed by GCC and Clang and significantly reduces preprocessing
time for header-heavy translation units.

---

## Known Limitations

- **Source locations in macro expansions.** Because the preprocessor is
  text-based, column-level source locations within macro expansions are
  approximate. Diagnostics point to the correct file and line but may not
  pinpoint the exact column within an expanded macro.

- **`_Atomic` qualifier.** `_Atomic(type)` is parsed and resolved to the
  underlying type, but the atomic qualifier itself is not tracked in the type
  system. Atomic operations are not enforced.

- **`_Generic` const tracking.** `_Generic` selection can distinguish
  const-qualified pointer types via `is_const` flags on associations and
  parameters, but `const` on global variables and complex expressions (casts,
  array subscripts) is not yet tracked in the type system. `CType` does not
  carry qualifiers.

- **Incomplete type checking.** Sema does not yet perform full C type checking.
  Many type errors are caught during IR lowering rather than during semantic
  analysis. The `expr_types` map may not cover every expression node.

- **Conservative `-Wreturn-type` analysis.** The control-flow analysis for
  detecting non-void functions that may not return a value is intraprocedural
  and conservative. It may produce false positives in complex control flow
  patterns involving multiple gotos or computed jumps, and does not perform
  value-range analysis on conditions.

- **Preprocessor macro location loss.** The text-to-text design means that the
  preprocessor cannot provide a "macro expansion backtrace" showing which macro
  produced a given token. The `macro_expansion_info` side channel provides
  per-line expansion metadata, but it is less precise than what a token-based
  preprocessor could offer.

---

## Module Index

| Submodule | Entry point | README |
|-----------|-------------|--------|
| `preprocessor/` | `Preprocessor::preprocess()` | [preprocessor/README.md](preprocessor/README.md) |
| `lexer/` | `Lexer::tokenize()` | [lexer/README.md](lexer/README.md) |
| `parser/` | `Parser::parse()` | [parser/README.md](parser/README.md) |
| `sema/` | `SemanticAnalyzer::analyze()` | [sema/README.md](sema/README.md) |
