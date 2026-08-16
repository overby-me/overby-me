# Semantic Analysis (`sema`)

The semantic analysis pass sits between parsing and IR lowering in the compilation
pipeline. Its primary role is **information gathering**, not strict type checking:
it walks the AST to build a rich collection of type metadata, constant values, and
function signatures that the IR lowerer consumes. The pass does not reject programs
with type errors (with a few exceptions noted below); instead, it populates the
`SemaResult` structure so the lowerer can make informed decisions without
re-deriving type information from the AST.

```text
Parser AST ────> SemanticAnalyzer.analyze(&ast)
                        |
                        |-- symbol table (scoped)
                        |-- function signatures
                        |-- TypeContext (typedefs, struct layouts, enum constants)
                        |-- ExprTypeMap (expression -> CType annotations)
                        +-- ConstMap (expression -> compile-time IrConst values)
                        |
                 SemanticAnalyzer.into_result() --> SemaResult
                        |
                 Lowerer::with_type_context(sema_result.type_context,
                                            sema_result.functions,
                                            sema_result.expr_types,
                                            sema_result.const_values, ...)
```

---

## Module Layout

| File | Responsibility |
|---|---|
| `analysis.rs` | Main pass: AST walk, declaration/statement/expression analysis, `-Wreturn-type`, implicit declarations, `TypeConvertContext` trait impl |
| `type_context.rs` | `TypeContext` -- module-level type state (struct layouts, typedefs, enum constants, ctype cache) with undo-log scope management |
| `type_checker.rs` | `ExprTypeChecker` -- expression-level CType inference using only sema-available state |
| `const_eval.rs` | `SemaConstEval` -- compile-time constant evaluation returning `IrConst` values |
| `builtins.rs` | Static database mapping `__builtin_*` names and `_mm_*` intrinsics to their lowering behavior |

---

## Public Interface

### `SemanticAnalyzer`

The analyzer is created, configured, run, and consumed in a strict linear sequence:

```rust
let mut sema = SemanticAnalyzer::new();          // 1. Create
sema.set_diagnostics(engine);                     // 2. Configure (optional)
sema.analyze(&ast)?;                              // 3. Run -- returns Err(count) on errors
let mut diagnostics = sema.take_diagnostics();    // 4. Reclaim diagnostic engine
let result: SemaResult = sema.into_result();      // 5. Consume -- moves ownership
```

**`new()`** initializes the symbol table, creates an empty `SemaResult`, and
pre-populates implicit function declarations for common libc functions (`printf`,
`malloc`, `memcpy`, `strcmp`, etc.) so that C89-style code that calls functions
without prototypes produces warnings rather than errors.

**`analyze(&mut self, tu: &TranslationUnit) -> Result<(), usize>`** walks every
top-level declaration and function definition. Errors are emitted through the
`DiagnosticEngine` with source spans. The method returns `Ok(())` if no hard
errors occurred, or `Err(n)` with the error count.

**`into_result(self) -> SemaResult`** consumes the analyzer and returns the
accumulated results. The method simply returns `self.result`. Both sema's
declaration processing and expression type inference use
`TypeContext::next_anon_struct_id()` (a single shared counter), so no separate
counter synchronization is needed.

### Analyzer State

The `SemanticAnalyzer` struct holds the following fields:

| Field | Type | Purpose |
|---|---|---|
| `symbol_table` | `SymbolTable` | Scoped variable/function name resolution. |
| `result` | `SemaResult` | Accumulated output for the lowerer. |
| `enum_counter` | `i64` | Auto-incrementing counter for enum variant values. |
| `diagnostics` | `RefCell<DiagnosticEngine>` | Structured error/warning reporting. Uses `RefCell` so `&self` methods (e.g., `eval_const_expr_as_usize`) can emit diagnostics. |
| `defined_structs` | `RefCell<FxHashSet<String>>` | Set of struct/union keys that have full definitions (not just forward declarations). Used to distinguish `struct X { ... }` from `struct X;` for incomplete type checking. Uses `RefCell` since `resolve_struct_or_union` takes `&self`. |

### `TypeConvertContext` Trait Implementation

`SemanticAnalyzer` implements the `TypeConvertContext` trait from
`common::type_builder`. This trait is the mechanism for shared type-building
logic between sema and the lowerer. The trait provides a default
`resolve_type_spec_to_ctype` method that handles all 22 primitive C types,
pointers, arrays, and function pointers identically. `SemanticAnalyzer`
supplies the five required methods (four for type resolution, one for
constant expression evaluation):

| Trait Method | Sema Behavior |
|---|---|
| `resolve_typedef(name)` | Looks up `type_context.typedefs`; falls back to `CType::Int`. |
| `resolve_struct_or_union(...)` | Computes layout, inserts into `TypeContext`, tracks definitions in `defined_structs`. |
| `resolve_enum(...)` | Returns `CType::Enum` preserving variant values for packed-size computation. |
| `resolve_typeof_expr(expr)` | Delegates to `ExprTypeChecker::infer_expr_ctype`; falls back to `CType::Int`. |
| `eval_const_expr_as_usize(expr)` | Evaluates via `SemaConstEval`; emits "size of array is negative" error for negative values. |

---

## `SemaResult` -- Output Structure

`SemaResult` bundles everything the IR lowerer needs from semantic analysis:

```rust
pub struct SemaResult {
    pub functions:    FxHashMap<String, FunctionInfo>,
    pub type_context: TypeContext,
    pub expr_types:   ExprTypeMap,       // FxHashMap<ExprId, CType>
    pub const_values: ConstMap,          // FxHashMap<ExprId, IrConst>
}
```

### `functions: FxHashMap<String, FunctionInfo>`

Every function encountered during analysis -- whether defined, declared as a
prototype, or implicitly declared from a call site -- gets a `FunctionInfo`
entry:

```rust
pub struct FunctionInfo {
    pub return_type: CType,
    pub params: Vec<(CType, Option<String>)>,
    pub variadic: bool,
    pub is_defined: bool,
    pub is_noreturn: bool,
}
```

The `is_noreturn` flag is sticky: if a prior declaration carries
`__attribute__((noreturn))` or `_Noreturn`, the flag persists even when the
function definition does not repeat the attribute. Implicitly declared functions
(C89-style calls without prototypes) default to `return_type: CType::Int`,
`variadic: true`, `is_defined: false`.

### `type_context: TypeContext`

The shared type-system state. Described in detail below.

### `expr_types: ExprTypeMap`

Maps each AST expression node's `ExprId` (a stable identity derived from the
node's heap address) to its inferred `CType`. Populated bottom-up during the
AST walk: after recursing into sub-expressions, `annotate_expr_type` creates a
fresh `ExprTypeChecker` and records the result. The lowerer consults this map as
an O(1) fallback in `get_expr_ctype()` when its own IR-state-based inference
cannot determine the type.

### `const_values: ConstMap`

Maps each expression's `ExprId` to its pre-computed `IrConst` compile-time
constant value. Populated alongside `expr_types` by `annotate_const_value`.
The lowerer uses this as an O(1) fast path before falling back to its own
`eval_const_expr`, which handles additional cases requiring IR-level state
(global addresses, runtime values).

---

## TypeContext

`TypeContext` is the central type-system repository. Sema creates it, populates it
during the AST walk, and transfers it by ownership to the lowerer. The lowerer
continues to extend it (e.g., adding struct layouts discovered during lowering).

### Contents

| Field | Type | Purpose |
|---|---|---|
| `struct_layouts` | `RefCell<FxHashMap<String, RcLayout>>` | Struct/union field layouts indexed by tag name. Uses `Rc<StructLayout>` for cheap clone/lookup. |
| `enum_constants` | `FxHashMap<String, i64>` | Enum constant name-to-value mapping. |
| `typedefs` | `FxHashMap<String, CType>` | Typedef name-to-resolved-CType mapping. |
| `typedef_alignments` | `FxHashMap<String, usize>` | Per-typedef alignment overrides from `__attribute__((aligned(N)))`. |
| `function_typedefs` | `FxHashMap<String, FunctionTypedefInfo>` | Bare function typedefs (e.g., `typedef int func_t(int, int)`). |
| `func_ptr_typedefs` | `FxHashSet<String>` | Set of typedef names that are function pointer types. |
| `func_ptr_typedef_info` | `FxHashMap<String, FunctionTypedefInfo>` | Function pointer typedef metadata. |
| `enum_typedefs` | `FxHashSet<String>` | Typedef names that alias enum types (for unsigned bitfield treatment). |
| `packed_enum_types` | `FxHashMap<String, EnumType>` | Packed enum definitions for forward-reference size lookups. |
| `func_return_ctypes` | `FxHashMap<String, CType>` | Cached return types for known functions. |
| `ctype_cache` | `RefCell<FxHashMap<String, CType>>` | Cache for named struct/union CType resolution. |
| `scope_stack` | `RefCell<Vec<TypeScopeFrame>>` | Undo-log frames for scope management. |
| `anon_ctype_counter` | `Cell<u32>` | Counter for generating unique anonymous struct/union keys. Uses `Cell` for interior mutability since `type_spec_to_ctype` takes `&self`. |

### `FunctionTypedefInfo`

```rust
pub struct FunctionTypedefInfo {
    pub return_type: TypeSpecifier,    // AST type, NOT resolved CType
    pub params: Vec<ParamDecl>,        // AST declarations, NOT resolved types
    pub variadic: bool,
}
```

Note that `return_type` is a `TypeSpecifier` (an AST-level type representation)
and `params` is a `Vec<ParamDecl>` (AST parameter declarations), not resolved
`CType` values. This preserves the original AST syntax so the lowerer can
perform its own resolution with full lowering context. This applies to both
`function_typedefs` and `func_ptr_typedef_info` entries.

### Builtin Typedef Seeding

On construction, `TypeContext::seed_builtin_typedefs()` pre-populates typedef
mappings for standard C types (`size_t`, `uint64_t`, `pid_t`, `FILE`, `pthread_t`,
etc.) so that sema can resolve these types even when the source does not include
the relevant headers. The mappings are target-aware: on ILP32 (i686), `size_t`
maps to `CType::UInt` and `int64_t` maps to `CType::LongLong`; on LP64 (x86-64,
arm64, riscv64), they map to `CType::ULong` and `CType::Long` respectively.

### Scope Management via Undo-Log

Rather than cloning entire hash maps at scope boundaries (O(total-map-size) per
scope push), `TypeContext` uses an **undo-log** pattern. Each scope boundary
pushes a `TypeScopeFrame` that records:

- **Added keys** -- keys inserted during this scope (removed on pop).
- **Shadowed keys** -- keys that existed before and were overwritten, along with
  their previous values (restored on pop).

This gives O(changes-in-scope) cost for push/pop. The undo log tracks five
separate maps: `enum_constants`, `struct_layouts`, `ctype_cache`, `typedefs`,
and `typedef_alignments`.

```rust
pub struct TypeScopeFrame {
    pub enums_added:                Vec<String>,
    pub struct_layouts_added:       Vec<String>,
    pub struct_layouts_shadowed:    Vec<(String, RcLayout)>,
    pub ctype_cache_added:          Vec<String>,
    pub ctype_cache_shadowed:       Vec<(String, CType)>,
    pub typedefs_added:             Vec<String>,
    pub typedefs_shadowed:          Vec<(String, CType)>,
    pub typedef_alignments_added:   Vec<String>,
    pub typedef_alignments_shadowed:Vec<(String, usize)>,
}
```

Scope push/pop pairs are placed at:

- Function body entry/exit (both symbol table and type context).
- Compound statement (`{ ... }`) entry/exit.
- `for` loop init clause (to scope loop-local declarations).

A key subtlety in `pop_scope`: when restoring a shadowed struct layout, the
system avoids restoring an empty forward-declaration layout over a full definition.
This ensures that a struct defined inside a function body but forward-declared
in an outer scope retains its complete layout after the inner scope exits.

### Interior Mutability

Several `TypeContext` fields use `RefCell` (for maps) or `Cell` (for counters)
because type resolution methods that take `&self` -- particularly
`type_spec_to_ctype` called from the `TypeConvertContext` trait -- may need to
insert forward-declaration layouts or update the ctype cache. The scoped
insertion methods (`insert_struct_layout_scoped_from_ref`,
`invalidate_ctype_cache_scoped_from_ref`) borrow both the data map and the scope
stack to record undo entries.

---

## Expression Type Inference (`ExprTypeChecker`)

`ExprTypeChecker` infers the `CType` of any AST expression using only sema-available
state: `SymbolTable`, `TypeContext`, and the `FunctionInfo` map. It does not depend
on IR-level state (allocas, IR types, global variable metadata), which makes it
suitable for use during semantic analysis before IR lowering begins.

### Design

The checker operates on immutable references and does not modify any state. It is
created fresh for each call to `annotate_expr_type` and borrows:

```rust
pub struct ExprTypeChecker<'a> {
    pub symbols:    &'a SymbolTable,
    pub types:      &'a TypeContext,
    pub functions:  &'a FxHashMap<String, FunctionInfo>,
    pub expr_types: Option<&'a FxHashMap<ExprId, CType>>,  // memoization cache
}
```

The `expr_types` field enables memoization: because the AST walk in `analyze_expr`
is bottom-up, child expression types are already in the map when a parent expression
is analyzed. This turns what would be O(2^N) recursive re-evaluation on deeply nested
chains like `1+1+1+...+1` into O(N) with O(1) lookups.

### Coverage

The checker handles all expression forms in the AST:

- **Literals**: `IntLiteral` -> `CType::Int`, `FloatLiteral` -> `CType::Double`,
  `FloatLiteralF32` -> `CType::Float`, `FloatLiteralLongDouble` -> `CType::LongDouble`,
  `StringLiteral` -> `Pointer(Char)`, `WideStringLiteral` -> `Pointer(Int)`,
  `Char16StringLiteral` -> `Pointer(UShort)`. Integer literal types are
  target-aware: on ILP32, values exceeding `INT_MAX` promote to `LongLong`.
  Imaginary literal variants produce their complex type:
  `ImaginaryLiteral` -> `ComplexDouble`, `ImaginaryLiteralF32` -> `ComplexFloat`,
  `ImaginaryLiteralLongDouble` -> `ComplexLongDouble`.
- **Identifiers**: symbol table lookup, enum constant lookup (with GCC promotion
  rules: `int` -> `unsigned int` -> `long long` -> `unsigned long long`), function
  signature lookup.
- **Unary operators**: `&` wraps in `Pointer`, `*` peels off `Pointer`/`Array`,
  arithmetic operators apply integer promotion, `!` produces `int`.
  `__real__`/`__imag__` (`RealPart`/`ImagPart`) extract the component type of
  a complex number (e.g., `ComplexDouble` -> `Double`).
- **Binary operators**: comparison and logical operators produce `int`; shift
  operators produce the promoted type of the left operand; arithmetic operators
  apply the usual arithmetic conversions; pointer arithmetic preserves the pointer
  type (or produces `ptrdiff_t` for pointer-pointer subtraction). GCC vector
  extensions are handled: if either operand is a vector type, the result is that
  vector type.
- **Casts**: resolves the target `TypeSpecifier` to `CType`.
- **Function calls**: looks up the callee's return type from the function table,
  with special handling for `__builtin_choose_expr`.
- **`sizeof`/`_Alignof`**: always `CType::ULong`. Note: on ILP32, `ULong` is a
  4-byte type, which is the correct size for `size_t` on those platforms, even
  though `seed_builtin_typedefs()` maps `size_t` to `CType::UInt` on ILP32.
  Both are 4-byte unsigned types, so the practical impact is nil.
- **Member access**: resolves the struct/union type of the base expression, then
  looks up the field type in the struct layout.
- **Ternary conditional**: applies C11 6.5.15 composite type rules.
- **`GnuConditional`** (Elvis operator `a ?: b`): applies the same composite
  type rules between the condition and the else expression.
- **Statement expressions** (`({ ... })`): type is the type of the last expression
  statement, with fallback resolution from declarations within the compound.
- **`_Generic`**: resolves the controlling expression type and matches against
  the association list.
- **Compound literals**: type from the type specifier.
- **`VaArg`**: type from the target type specifier.
- **`LabelAddr`** (`&&label`): always `Pointer(Void)`.
- **`BuiltinTypesCompatibleP`**: always `CType::Int`.

---

## Compile-Time Constant Evaluation (`SemaConstEval`)

`SemaConstEval` evaluates constant expressions at compile time, returning `IrConst`
values that match the lowerer's richer result type. Like `ExprTypeChecker`, it
borrows sema state immutably and is created fresh per evaluation.

```rust
pub struct SemaConstEval<'a> {
    pub types:        &'a TypeContext,
    pub symbols:      &'a SymbolTable,
    pub functions:    &'a FxHashMap<String, FunctionInfo>,
    pub const_values: Option<&'a FxHashMap<ExprId, IrConst>>,  // memoization
    pub expr_types:   Option<&'a FxHashMap<ExprId, CType>>,    // type cache
}
```

### What It Evaluates

| Expression | Handling |
|---|---|
| Integer/float/char literals | Delegates to shared `common::const_eval::eval_literal` |
| Unary `+`, `-`, `~`, `!` | Sub-int promotion, then arithmetic via `common::const_arith` |
| Binary ops | Type-aware signedness semantics: derives operand CTypes for width/signedness, delegates to shared `eval_binop_with_types` |
| Cast chains | Tracks bit-widths through nested casts; handles float-to-int, int-to-float, and 128-bit conversions with proper sign extension |
| `sizeof(type)` / `sizeof(expr)` | Resolves through sema's type system |
| `_Alignof(type)` / `__alignof__(type)` | C11 standard vs. GCC preferred alignment |
| `__alignof__(expr)` | Checks for explicit alignment on variable declarations |
| Ternary `?:` | Evaluates condition, returns selected branch |
| `GnuConditional` (Elvis `a ?: b`) | Evaluates condition; if nonzero returns condition value, otherwise evaluates else branch |
| `__builtin_types_compatible_p` | Structural CType comparison |
| `offsetof` patterns (`&((T*)0)->member`) | Resolves struct field offsets, handles nested access and array subscripts |
| Enum constants | Direct `i64` lookup in `TypeContext.enum_constants` |
| Builtin function calls | Delegates to shared `common::const_eval::eval_builtin_call` |
| Logical `&&` / `||` | Short-circuit evaluation; handles always-nonzero expressions (string literals) |

### What It Does NOT Evaluate

The following require IR-level state and remain in the lowerer's `const_eval.rs`:

- Global address expressions (`&x`, function pointers as values, array decay)
- `func_state` const local values
- Pointer arithmetic on global addresses

### Memoization

Both `const_values` and `expr_types` caches prevent exponential blowup. For
deeply nested binary expressions (common in preprocessor-generated enum
initializers), each sub-expression is evaluated exactly once during the bottom-up
walk, and subsequent references are O(1) lookups.

---

## Builtin Function Database (`builtins.rs`)

A static `LazyLock<FxHashMap<&str, BuiltinInfo>>` maps GCC `__builtin_*` names
and Intel `_mm_*` intrinsic names to their lowering behavior. Each entry is a
`BuiltinInfo` with one of four `BuiltinKind` variants:

| Kind | Behavior | Examples |
|---|---|---|
| `LibcAlias(name)` | Emit a call to the named libc function | `__builtin_memcpy` -> `memcpy`, `__builtin_printf` -> `printf` |
| `Identity` | Return the first argument unchanged | `__builtin_expect`, `__builtin_assume_aligned` |
| `ConstantF64(val)` | Evaluate to a compile-time float constant | `__builtin_nan` -> `NaN`, `__builtin_inf` -> `Infinity` |
| `Intrinsic(kind)` | Requires target-specific codegen | `__builtin_clz`, `__builtin_bswap32`, `_mm_set1_epi8`, AES-NI, CRC32, etc. |

The `BuiltinIntrinsic` enum covers a wide range of operations:

- **Bit manipulation**: clz, ctz, popcount, bswap, ffs, parity, clrsb
- **Overflow arithmetic**: `__builtin_add_overflow`, `__builtin_sub_overflow`,
  `__builtin_mul_overflow` (and unsigned/signed/predicate variants)
- **Floating-point classification**: fpclassify, isnan, isinf, isfinite,
  isnormal, signbit, isinf_sign
- **Floating-point comparison**: `__builtin_isgreater`, `__builtin_isless`,
  `__builtin_islessgreater`, `__builtin_isunordered`, etc.
- **Atomics**: `__sync_synchronize` (fence)
- **Complex numbers**: creal, cimag, conj, `__builtin_complex`
- **Variadic arguments**: va_start, va_end, va_copy
- **Alloca**: `__builtin_alloca`, `__builtin_alloca_with_align` (dynamic stack allocation)
- **Return/frame address**: `__builtin_return_address`, `__builtin_frame_address`
- **Thread pointer**: `__builtin_thread_pointer` (TLS base address)
- **Compile-time queries**: `__builtin_constant_p`, `__builtin_object_size`,
  `__builtin_classify_type`
- **Fortification**: `__builtin___memcpy_chk`, `__builtin___strcpy_chk`,
  `__builtin___sprintf_chk`, etc. (glibc `_FORTIFY_SOURCE` wrappers forwarded
  to their `__*_chk` runtime functions)
- **Variadic arg forwarding**: `__builtin_va_arg_pack`, `__builtin_va_arg_pack_len`
  (used in always-inline fortification wrappers)
- **CPU feature detection**: `__builtin_cpu_init` (no-op), `__builtin_cpu_supports`
  (conservatively returns 0)
- **Prefetch**: `__builtin_prefetch` (lowered as a no-op)
- **x86 SSE/SSE2/SSE4.1/AES-NI/CLMUL**: complete 128-bit SIMD instruction coverage

The `is_builtin()` function extends the static map with direct recognition of
`__builtin_choose_expr`, `__builtin_unreachable`, and `__builtin_trap` (handled
by name in the lowerer's `try_lower_builtin_call` before the map lookup), plus
pattern-matched recognition of `__atomic_*` and `__sync_*` families (handled by
the lowerer's `expr_atomics.rs`). This prevents sema from emitting spurious
"implicit declaration" warnings for these builtins.

---

## Scope Management and Symbol Table Integration

The analyzer maintains two parallel scope stacks:

1. **`SymbolTable`** (in `common::symbol_table`) -- variable/function name resolution
   with lexical scoping. `push_scope()` / `pop_scope()` manage variable visibility.

2. **`TypeContext`** scope stack -- undo-log for type-system state (struct layouts,
   typedefs, enum constants). Ensures that local struct definitions inside a
   function body do not overwrite global layouts.

Both stacks are pushed/popped together at function body boundaries and compound
statement boundaries. `for` loops get their own scope pair (for C99 loop-local
declarations like `for (int i = 0; ...)`).

### Declaration Analysis Flow

When `analyze_declaration` encounters a declaration:

1. **Enum constants** are collected recursively from the type specifier, walking
   into struct/union field types to catch inline enum definitions.
2. The base type is resolved via `type_spec_to_ctype`. For `__auto_type`
   declarations (a GCC extension), the type is inferred from the first
   declarator's initializer expression using `ExprTypeChecker`.
3. **Typedefs** are handled specially: the resolved CType is stored in
   `TypeContext.typedefs`, function typedef info is extracted into
   `function_typedefs` or `func_ptr_typedef_info`, and alignment overrides
   are recorded in `typedef_alignments`.
4. **Variable declarations** build the full CType (including derived declarators
   like pointers and arrays), resolve incomplete array sizes from initializers
   (e.g., `int arr[] = {1,2,3}`), check for incomplete struct/union types, and
   register the symbol in the symbol table.
5. **Function prototypes** (declarations with `CType::Function`) register in the
   `functions` map with `is_defined: false`.

### Expression Analysis Flow

`analyze_expr` recursively walks the expression tree:

1. **Identifier resolution**: checks the symbol table, enum constants, builtin
   database, and function table. Emits an error for undeclared identifiers.
2. **Implicit function declarations**: when a function call targets an unknown
   name, sema emits a `-Wimplicit-function-declaration` warning and registers
   the function as `return_type: Int, variadic: true`.
3. **Sub-expression recursion**: descends into all child expressions.
4. **Type annotation** (`annotate_expr_type`): after sub-expressions are processed,
   creates an `ExprTypeChecker` and records the inferred CType in `expr_types`.
5. **Constant annotation** (`annotate_const_value`): creates a `SemaConstEval` and
   records any compile-time constant value in `const_values`.

The bottom-up order is critical: it ensures the memoization caches (`expr_types`,
`const_values`) are populated for child nodes before parent nodes are evaluated,
preventing exponential blowup on deep expression trees.

---

## Diagnostics

Sema emits structured diagnostics through the `DiagnosticEngine`, which supports:

- **Errors**:
  - Undeclared identifiers
  - Incomplete struct/union types in variable declarations
    ("storage size of 'struct/union X' isn't known")
  - Incomplete struct/union types in `sizeof` expressions
    (`check_sizeof_incomplete_type`)
  - Non-integer switch controlling expressions
  - Invalid struct/union field access ("'type' has no member named 'field'",
    via `check_member_exists`)
  - Negative array sizes ("size of array is negative", emitted from
    `eval_const_expr_as_usize`)
  - Incompatible pointer/float implicit conversions in assignments, variable
    initializers, and function call arguments (C11 6.5.16.1p1, via
    `check_pointer_float_conversion`)
  - Incompatible pointer subtraction operand types (C11 6.5.6p3, via
    `check_pointer_subtraction_compat`)
- **Warnings**: `-Wimplicit-function-declaration` (C89-style implicit declarations),
  `-Wreturn-type` (non-void functions that may not return a value).

The diagnostic engine is transferred to sema via `set_diagnostics` and reclaimed
via `take_diagnostics`, preserving the source manager for span resolution.

### `-Wreturn-type` Analysis

Sema implements a control-flow analysis to detect non-void functions where control
can reach the closing `}` without a `return` statement. The analysis:

- Tracks whether each statement can "fall through" to the next.
- Recognizes `return`, `goto`, `break`, `continue` as diverging statements.
- Handles infinite loops (`while(1)`, `for(;;)`) as non-fall-through.
- Analyzes `if/else` branches: both must diverge for the whole `if` to not fall through.
- Handles `switch` statements: requires a `default` label and checks that no segment
  breaks out of the switch (segments that fall through to the next case are allowed).
- Recognizes calls to noreturn functions (`abort`, `exit`, `__builtin_unreachable`,
  and user-declared `__attribute__((noreturn))` functions).
- Exempts `main()` (C99 5.1.2.2.3: reaching `}` is equivalent to `return 0`),
  noreturn functions, and naked functions (body is pure inline assembly).

---

## How Sema Output Flows to the IR Lowerer

The driver orchestrates the handoff:

```rust
let sema_result = sema.into_result();
let lowerer = Lowerer::with_type_context(
    target,
    sema_result.type_context,       // ownership transfer
    sema_result.functions,          // ownership transfer
    sema_result.expr_types,         // ownership transfer
    sema_result.const_values,       // ownership transfer
    diagnostics,
    gnu89_inline,
);
let (module, diagnostics) = lowerer.lower(&ast);
```

The lowerer receives all four components by ownership:

1. **`TypeContext`** becomes the lowerer's primary type-system state. It continues
   to extend it with struct layouts discovered during lowering.
2. **`functions`** provides authoritative C-level function signatures. The lowerer
   falls back to these when its own ABI-adjusted `FuncSig` lacks info.
3. **`expr_types`** is consulted as a fast O(1) fallback in `get_expr_ctype()` after
   the lowerer's own IR-state-based inference.
4. **`const_values`** is consulted as an O(1) fast path before the lowerer's own
   `eval_const_expr`, which handles additional cases requiring IR-level state.

---

## Design Decisions and Tradeoffs

### Information Gathering Over Strict Checking

Sema deliberately does **not** reject programs with type errors in most cases. The
design philosophy is pragmatic: collect as much information as possible so the
lowerer can do its job, rather than attempting full C type checking and risking
rejection of valid programs that the type checker does not yet fully understand.
The few hard errors sema does emit (undeclared identifiers, incomplete struct
types in variable definitions, invalid member access, negative array sizes) are
cases where the lowerer would definitely fail.

### Memoized Bottom-Up Walk

The choice to annotate every expression during a single bottom-up walk (rather
than on-demand top-down inference) was driven by performance. Preprocessor-generated
code frequently produces deeply nested expression chains (e.g., enum initializers
with hundreds of `+` operators). Without memoization, recursive type inference on
such chains exhibits O(2^N) behavior. The bottom-up walk with `ExprTypeMap` and
`ConstMap` caches guarantees O(N) total work.

### Undo-Log Scoping

The undo-log pattern for `TypeContext` scope management was chosen over
snapshot/clone because many scopes modify only a handful of entries while the
total map size can be large (hundreds of struct layouts, thousands of typedefs
from system headers). The undo-log makes scope push/pop proportional to the
number of changes within the scope rather than the total map size.

### `ExprId` Stability

Expression type and constant annotations are keyed by `ExprId`, a type-safe wrapper
around each `Expr` node's heap address. This works because the AST is allocated once
during parsing and is never moved or reallocated before the lowerer consumes it.
This approach avoids the overhead of assigning explicit IDs to every AST node while
still providing stable, hashable identities.

### Separation of `ExprTypeChecker` and `SemaConstEval`

Type inference and constant evaluation are separate modules because they serve
different consumers and have different coverage:

- `ExprTypeChecker` produces `CType` for every expression it can type.
- `SemaConstEval` produces `IrConst` only for expressions that are compile-time
  constants.

Both share the same borrowed state pattern (immutable references, created fresh per
invocation) and both use the same memoization caches, but they live in separate
modules because their logic is fundamentally different (type algebra vs. arithmetic
evaluation).

### Shared Evaluation Logic

Pure arithmetic operations (literal evaluation, binary op arithmetic, sub-int
promotion) live in `common::const_eval` and `common::const_arith`, shared between
sema's `SemaConstEval` and the lowerer's `const_eval.rs`. This avoids duplicating
tricky signedness and bit-width logic while allowing each module to handle its
own context-specific cases.

---

## Known Limitations

- **Limited type checking.** Sema validates pointer<->float type incompatibility
  in assignments, variable initializers, and function call arguments (C11 6.5.16.1p1),
  and checks pointer subtraction operand compatibility (C11 6.5.6p3), but does not
  perform full type compatibility checking. Return statement types, function pointer
  call arguments, and other implicit conversions are not validated. Programs with
  non-pointer/float type errors may pass sema and cause panics or incorrect codegen
  in the lowerer.

- **Incomplete `typeof(expr)` support.** The `ExprTypeChecker` returns `None` for
  expressions it cannot type (complex expression chains through typeof, expressions
  requiring lowering-specific state). In such cases, sema falls back to `CType::Int`.

- **No VLA support in constant evaluation.** Variable-length arrays and
  runtime-computed array sizes are not handled by `SemaConstEval`; they fall
  through to the lowerer.

- **Conservative `-Wreturn-type` analysis.** The control flow analysis is
  intraprocedural and conservative. It may produce false positives (warning when
  all paths actually do return) in complex control flow patterns involving multiple
  gotos or computed jumps. It does not perform value-range analysis on conditions.

- **Implicit function declarations.** Sema pre-populates a set of common libc
  functions and emits warnings for unknown calls rather than errors. This matches
  C89 behavior but may mask genuine "undefined function" bugs.

- **Single-pass limitation.** Sema makes a single forward pass over the AST. This
  means that a function called before its declaration (without a forward prototype)
  will be recorded as an implicit declaration with `int` return type, even if the
  actual definition later in the file has a different return type. The lowerer
  handles such cases with its own multi-pass approach.
