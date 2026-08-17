# Run: nix eval -f platform/nix/config/cargo/tests/cfg.nix
let
  cfg = import ../lib/cfg.nix;

  linux = cfg.platforms.x86_64-linux;
  darwin = cfg.platforms.aarch64-darwin;

  # [ platform target expected ]
  cases = [
    [linux "cfg(unix)" true]
    [linux "cfg(windows)" false]
    [darwin "cfg(unix)" true]
    [linux "cfg(target_os = \"linux\")" true]
    [linux "cfg(target_os = \"macos\")" false]
    [darwin "cfg(target_os = \"macos\")" true]
    [linux "cfg(any(target_os = \"macos\", target_os = \"ios\"))" false]
    [darwin "cfg(any(target_os = \"macos\", target_os = \"ios\"))" true]
    [linux "cfg(all(target_arch = \"x86_64\", target_os = \"linux\"))" true]
    [linux "cfg(all(target_arch = \"aarch64\", target_os = \"linux\"))" false]
    [linux "cfg(not(windows))" true]
    [linux "cfg(not(unix))" false]
    [linux "cfg(all(unix, not(target_os = \"macos\")))" true]
    [darwin "cfg(all(unix, not(target_os = \"macos\")))" false]
    [linux "cfg(target_pointer_width = \"64\")" true]
    [linux "cfg(target_pointer_width = \"32\")" false]
    [linux "cfg(target_env = \"gnu\")" true]
    [linux "cfg(target_env = \"musl\")" false]
    [linux "cfg(target_family = \"unix\")" true]
    [linux "cfg(target_arch = \"wasm32\")" false]
    [linux "cfg(target_has_atomic = \"64\")" true]
    [linux "cfg(debug_assertions)" false]
    [linux "cfg(test)" false]
    # whitespace tolerance
    [linux "cfg( all( unix , not( windows ) ) )" true]
    # literal triples
    [linux "x86_64-unknown-linux-gnu" true]
    [linux "x86_64-pc-windows-gnu" false]
    [darwin "aarch64-apple-darwin" true]
  ];

  results =
    map (
      c: let
        platform = builtins.elemAt c 0;
        target = builtins.elemAt c 1;
        expected = builtins.elemAt c 2;
        actual = cfg.matchesTarget platform target;
      in {
        inherit target expected actual;
        ok = actual == expected;
      }
    )
    cases;

  failures = builtins.filter (r: !r.ok) results;
in
  if failures == []
  then "ok: ${toString (builtins.length cases)} cases"
  else throw "cfg test failures: ${builtins.toJSON failures}"
