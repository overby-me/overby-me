# Cargo profile handling. Pure builtins.
#
# Reads [profile.release]/[profile.dev] from the workspace root manifest
# (the only place cargo honors them) plus per-package overrides
# ([profile.X.package."<name>"] and [profile.X.package."*"]). The wildcard
# applies to dependencies (registry/git packages); workspace members use
# the base profile unless named explicitly, which matches the common
# "deps at opt-level 1" pattern.
let
  normLto = v:
    if v == true || v == "fat"
    then "fat"
    else if v == "thin"
    then "thin"
    else "off";

  normStrip = v:
    if v == true || v == "symbols"
    then "symbols"
    else if v == "debuginfo"
    then "debuginfo"
    else "none";

  # Accepts bools, ints, and strings like "line-tables-only".
  normDebug = v:
    if v == true
    then "2"
    else if v == false
    then "0"
    else toString v;

  mkBase = release: toml: {
    optLevel = toString (toml."opt-level"
      or (
      if release
      then 3
      else 0
    ));
    debugInfo = normDebug (toml.debug
      or (
      if release
      then 0
      else 2
    ));
    lto = normLto (toml.lto or false);
    strip = normStrip (toml.strip or false);
    panic = toml.panic or "unwind";
    codegenUnits = toml."codegen-units" or null;
  };

  applyOverride = base: o:
    base
    // (
      if o ? "opt-level"
      then {optLevel = toString o."opt-level";}
      else {}
    )
    // (
      if o ? debug
      then {debugInfo = normDebug o.debug;}
      else {}
    )
    // (
      if o ? "codegen-units"
      then {codegenUnits = o."codegen-units";}
      else {}
    );

  # rootManifest -> { base; forPackage = name: isMember: profile; }
  mkProfiles = {
    rootManifest,
    release,
  }: let
    toml =
      (rootManifest.profile
        or {
      }).${
        if release
        then "release"
        else "dev"
      } or {
      };
    base = mkBase release toml;
    overrides = toml.package or {};
  in {
    inherit base;
    forPackage = name: isMember:
      if overrides ? ${name}
      then applyOverride base overrides.${name}
      else if !isMember && overrides ? "*"
      then applyOverride base overrides."*"
      else base;
  };
in {
  inherit mkProfiles;
}
