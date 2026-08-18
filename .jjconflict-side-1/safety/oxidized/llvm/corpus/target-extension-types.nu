#!/usr/bin/env nu
# Derives what each target extension type is allowed to be.
#
#   nu target-extension-types.nu <llvm-source-tree> [out-file]
#
# `target("name")` names a type the target defines, and upstream registers
# each one with properties that say where it may appear: whether it has a
# size, whether it may be a global, whether `alloca` may name it, whether
# `zeroinitializer` is a value of it, and whether a vector may have it as an
# element. LangRef says those properties exist and then points at a C++
# header for the list, so the list is not readable here.
#
# The assembler knows it and answers one question at a time, which is the
# same trick the intrinsic attribute table uses: LangRef supplies the
# question and the assembler supplies the answer.
#
# Two things make a probe easy to get wrong, and both were got wrong first.
#
# A probe has to isolate the property it asks about. Asking whether
# `zeroinitializer` is a value by writing `@g = global T zeroinitializer`
# also asks whether `T` may be a global, so `aarch64.svcount` answered no to
# a question about zero and was recorded as taking no `zeroinitializer` when
# it does. The shape below passes the constant to a call instead, which needs
# nothing of the type but that it be a parameter.
#
# And a name has to be spelled the way its target spells it. Some names carry
# parameters and are not a type without them, so `target("riscv.vector.tuple")`
# is not a worse `riscv.vector.tuple`, it is not one at all, and every
# property probed through it reads false. The spellings are therefore
# harvested whole from upstream's tests and each name is probed through one
# the assembler accepts.
#
# The answer has two levels. A namespace has defaults, so `target("spirv.x")`
# is sized and global for any `x` at all, and a registered name may then
# override them: `spirv.Image` takes no `zeroinitializer` where its namespace
# does. Probing an invented name in a namespace is what separates the two,
# since no override can apply to a name nobody registered.
#
# The name list is as complete as upstream's tests are: a registered name no
# test mentions falls back to its namespace's defaults, and one in no
# namespace is treated as unregistered, which is what upstream does with a
# name it does not know.

const PROPERTIES = ["sized" "global" "alloca" "zeroinit" "vector"]

# The spellings a type parameter is tried as. A cell of the shape grid is
# accepted when any of them assembles, so a name wanting a vector is not
# recorded as wanting no parameters at all.
const TYPE_SPELLINGS = ["i32" "<4 x float>" "{ float }" "<vscale x 4 x i32>"]

# One shape per property, each assembling only when the type has it. `ty` is
# a whole type spelling, parameters and all.
def probe [ty: string, property: string] {
  match $property {
    "sized" => $"define void @f\(ptr %p) {\n  %v = load ($ty), ptr %p\n  ret void\n}\n"
    "global" => $"@g = global ($ty) undef\n"
    "alloca" => $"define void @f\() {\n  %a = alloca ($ty)\n  ret void\n}\n"
    # Through a call, so that a type which may not be a global can still be
    # asked whether zero is a value of it.
    "zeroinit" => $"declare void @g\(($ty))\ndefine void @f\() {\n  call void @g\(($ty) zeroinitializer)\n  ret void\n}\n"
    "vector" => $"define void @f\() {\n  %a = alloca <2 x ($ty)>\n  ret void\n}\n"
    _ => { error make {msg: $"no probe for ($property)"} }
  }
}

# Whether the spelling names a type at all, which is what a name carrying
# required parameters fails without them.
def spelled [ty: string] {
  $"declare void @f\(($ty))\n"
}

def assembles [llvm_as: string, text: string] {
  let file = (mktemp -t --suffix .ll)
  $text | save -f $file
  let code = (try {
    (do { ^$llvm_as $file -o /dev/null } | complete).exit_code
  } catch {
    # An assertion failure is a refusal like any other: the exit code is the
    # whole of the answer and the signal that carried it does not matter.
    134
  })
  rm -f $file
  $code == 0
}

# The parameter shape a name insists on, or null where it takes whatever it
# is given.
#
# Upstream says so in as many words: "should have no parameters", "should
# have one type parameter and one integer parameter", "should have no type
# parameters and one integer parameter". Three of the forty names its own
# tests spell have such a rule and the rest, an unregistered name included,
# take anything, so the grid is swept and a name accepting every cell has no
# rule to record.
#
# Counts rather than types, which is what upstream checks here. Whether the
# type parameter of `riscv.vector.tuple` has to be a particular type is a
# further question this does not ask.
def shape [llvm_as: string, name: string] {
  mut accepted = []
  for types in 0..2 {
    for ints in 0..3 {
      let cell = ($TYPE_SPELLINGS | any {|spelling|
        let params = (
          (0..<$types | each {|_| $spelling}) ++ (0..<$ints | each {|_| "4"})
        )
        let written = if ($params | is-empty) { "" } else { ", " + ($params | str join ", ") }
        assembles $llvm_as (spelled ('target("' + $name + '"' + $written + ')'))
      })
      if $cell {
        $accepted = ($accepted | append {types: $types, ints: $ints})
      }
    }
  }
  if ($accepted | length) == 12 {
    return null
  }
  if ($accepted | length) == 1 {
    return ($accepted | first)
  }
  error make {msg: $"($name) takes ($accepted | length) parameter shapes, which is neither one nor all"}
}

def measure [llvm_as: string, ty: string, name: string] {
  let properties = ($PROPERTIES | reduce --fold {} {|property, acc|
    $acc | insert $property (assembles $llvm_as (probe $ty $property))
  })
  $properties | insert shape (shape $llvm_as $name)
}

# The namespace a name belongs to, which is everything up to the first dot.
# A name with no dot is in no namespace and can only be unregistered.
def namespace [name: string] {
  let parts = ($name | split row ".")
  if ($parts | length) < 2 { null } else { $parts | first }
}

def rust-bool [value: bool] {
  if $value { "true" } else { "false" }
}

def row [name: string, properties: record] {
  let fields = (
    $PROPERTIES
    | each {|p| $p + ": " + (rust-bool ($properties | get $p)) }
    | str join ", "
  )
  let shape = if $properties.shape == null {
    "None"
  } else {
    $"Some\(\(($properties.shape.types), ($properties.shape.ints)))"
  }
  '    ("' + $name + '", Properties { ' + $fields + ", params: " + $shape + " }),"
}

def main [tree: path, out: path = "target_extension.rs"] {
  let tests = ([$tree "llvm" "test"] | path join)
  if not ($tests | path exists) {
    error make {msg: $"no test tree at ($tests)"}
  }
  let llvm_as = (which llvm-as | get path.0)

  # Whole spellings, so that a name needing parameters is probed with some.
  let spellings = (
    ^grep -rhoE 'target\("[^"]+"[^)]*\)' $tests
    | lines
    | uniq
    | sort
  )
  let names = (
    $spellings
    | each {|s| $s | parse --regex 'target\("(?<name>[^"]+)"' | get name.0 }
    | uniq
    | sort
  )
  print $"($spellings | length) spellings of ($names | length) names harvested"

  # For each name, a spelling the assembler takes. The bare form is preferred
  # so that a name needing no parameters is probed without any.
  mut usable = {}
  for name in $names {
    let bare = 'target("' + $name + '")'
    let candidates = (
      [$bare] ++ ($spellings | where {|s| $s | str starts-with ('target("' + $name + '"') })
    )
    let good = ($candidates | where {|c| assembles $llvm_as (spelled $c) })
    if not ($good | is-empty) {
      $usable = ($usable | insert $name ($good | first))
    }
  }
  print $"($usable | columns | length) names the assembler spells"

  # A namespace's defaults come from a name nobody registered, so no
  # override can be answering instead.
  let spaces = ($usable | columns | each {|n| namespace $n } | compact | uniq | sort)
  let defaults = ($spaces | reduce --fold {} {|space, acc|
    $acc | insert $space (measure $llvm_as ('target("' + $space + '.zzunregisteredzz")') ($space + ".zzunregisteredzz"))
  })
  let nothing = ($PROPERTIES | reduce --fold {} {|p, acc| $acc | insert $p false } | insert shape null)
  let interesting = ($spaces | where {|space| ($defaults | get $space) != $nothing })
  print $"($interesting | length) namespaces with defaults of their own"

  # A name is worth recording only where it differs from what its namespace
  # would already give it.
  mut overrides = []
  for name in ($usable | columns) {
    let space = (namespace $name)
    let inherited = if $space != null and ($space in $interesting) {
      $defaults | get $space
    } else {
      $nothing
    }
    let measured = (measure $llvm_as ($usable | get $name) $name)
    if $measured != $inherited {
      $overrides = ($overrides | append {name: $name, properties: $measured})
    }
  }
  print $"($overrides | length) names that differ from their namespace"

  let header = "//! What each target extension type is allowed to be.
//!
//! Generated by `corpus/target-extension-types.nu`; do not edit by hand.
//!
//! `target(\"name\")` names a type the target defines, and upstream registers
//! each one with properties saying where it may appear. LangRef says the
//! properties exist and points at a C++ header for the list, so this is
//! measured against the assembler instead: each row is an answer it gave.
//!
//! A namespace carries defaults and a registered name may override them, so
//! the lookup asks for the name first and falls back to the namespace. A name
//! in neither table is one upstream does not know, and an unknown target
//! extension type has no properties at all: it has no size, so it cannot be
//! loaded, stored, allocated or held in a global.

/// Where a target extension type may appear.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Properties {
    /// Whether the type has a size, which is what lets it be loaded and
    /// stored.
    pub sized: bool,
    /// Whether a global variable may have this type.
    pub global: bool,
    /// Whether `alloca` may name it.
    pub alloca: bool,
    /// Whether `zeroinitializer` is a value of it. `undef` and `poison` are
    /// values of every one of them and are not asked about.
    pub zeroinit: bool,
    /// Whether a vector may have it as an element. Most may not, which makes
    /// `<2 x target(\"spirv.Image\")>` an invalid vector element type where
    /// `<2 x target(\"llvm.test.vectorelement\")>` is a vector.
    pub vector: bool,
    /// How many type parameters and how many integer ones the name insists
    /// on, or `None` where it takes whatever it is given. Three of the names
    /// upstream's own tests spell have such a rule and everything else,
    /// an unregistered name included, takes any shape.
    pub params: Option<(u8, u8)>,
}
"

  let namespace_rows = ($interesting | each {|space| row $space ($defaults | get $space) } | str join "\n")
  let name_rows = ($overrides | each {|entry| row $entry.name $entry.properties } | str join "\n")

  let body = ("
/// What a namespace gives every name in it that is not listed below.
static NAMESPACES: &[(&str, Properties)] = &[
" + $namespace_rows + "
];

/// The names that differ from what their namespace would give them.
static NAMES: &[(&str, Properties)] = &[
" + $name_rows + "
];

/// The properties of a target extension type, which are none at all for a
/// name upstream does not know.
pub fn properties(name: &str) -> Properties {
    if let Some((_, found)) = NAMES.iter().find(|(listed, _)| *listed == name) {
        return *found;
    }
    let Some((space, _)) = name.split_once('.') else {
        return Properties::default();
    };
    NAMESPACES
        .iter()
        .find(|(listed, _)| *listed == space)
        .map(|(_, found)| *found)
        .unwrap_or_default()
}
")

  let path = if ($out | str starts-with "/") { $out } else { ([$env.PWD ".." "crates" "llvm-ir" "src" $out] | path join | path expand) }
  ($header + $body) | save -f $path
  # A row is wider than rustfmt's line limit, so the file is canonicalised
  # here rather than left for `cargo fmt --check` to fail on. Regenerating
  # then reproduces the file in the tree byte for byte.
  ^rustfmt --edition 2024 $path
  print $"wrote ($path)"
}
