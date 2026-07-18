# Run: nix eval -f nix/cargo/tests/semver.nix
let
  semver = import ../lib/semver.nix;

  # [ req version expected ]
  cases = [
    # caret (default)
    ["1.2.3" "1.2.3" true]
    ["1.2.3" "1.9.0" true]
    ["1.2.3" "2.0.0" false]
    ["1.2.3" "1.2.2" false]
    ["^1.2.3" "1.4.0" true]
    ["1.2" "1.9.9" true]
    ["1" "1.0.0" true]
    ["1" "2.0.0" false]
    ["0.2" "0.2.186" true]
    ["0.2" "0.3.0" false]
    ["0.0.3" "0.0.3" true]
    ["0.0.3" "0.0.4" false]
    ["^0.0" "0.0.9" true]
    ["^0.0" "0.1.0" false]
    ["^0" "0.9.9" true]
    ["^0" "1.0.0" false]
    # tilde
    ["~1.2.3" "1.2.9" true]
    ["~1.2.3" "1.3.0" false]
    ["~1.2" "1.2.0" true]
    ["~1.2" "1.3.0" false]
    ["~1" "1.9.9" true]
    ["~1" "2.0.0" false]
    # wildcard
    ["*" "3.4.5" true]
    ["1.*" "1.9.9" true]
    ["1.*" "2.0.0" false]
    ["1.2.*" "1.2.7" true]
    ["1.2.*" "1.3.0" false]
    ["1.x" "1.4.0" true]
    # exact
    ["=1.2.3" "1.2.3" true]
    ["=1.2.3" "1.2.4" false]
    ["=1.2" "1.2.9" true]
    ["=1.2" "1.3.0" false]
    # ranges
    [">=1.2.0" "1.2.0" true]
    [">=1.2.0" "1.1.9" false]
    [">1" "2.0.0" true]
    [">1" "1.5.0" false]
    [">1.2.3" "1.2.4" true]
    [">1.2.3" "1.2.3" false]
    ["<2" "1.9.9" true]
    ["<2" "2.0.0" false]
    ["<=1.2" "1.2.9" true]
    ["<=1.2" "1.3.0" false]
    ["<=1.2.3" "1.2.3" true]
    # conjunction
    [">=1.2, <1.5" "1.4.9" true]
    [">=1.2, <1.5" "1.5.0" false]
    [">=1.2, <1.5" "1.1.0" false]
    # pre-release
    ["1.0.0-alpha" "1.0.0-alpha" true]
    ["1.0.0-alpha" "1.0.0-alpha.1" true]
    ["1.0.0-alpha" "1.0.0" true]
    ["1.0.0-alpha.2" "1.0.0-alpha.1" false]
    ["1.2.3" "1.2.4-alpha" false]
    ["1.0.0-rc.1" "1.0.0-rc.2" true]
  ];

  results =
    map (
      c: let
        req = builtins.elemAt c 0;
        ver = builtins.elemAt c 1;
        expected = builtins.elemAt c 2;
        actual = semver.matches req ver;
      in {
        inherit req ver expected actual;
        ok = actual == expected;
      }
    )
    cases;

  failures = builtins.filter (r: !r.ok) results;

  cmpCases = [
    [(semver.cmp "1.2.3" "1.2.3") 0]
    [(semver.cmp "1.2.3" "1.2.4") (-1)]
    [(semver.cmp "1.10.0" "1.9.0") 1]
    [(semver.cmp "1.0.0-alpha" "1.0.0") (-1)]
    [(semver.cmp "1.0.0-alpha" "1.0.0-alpha.1") (-1)]
    [(semver.cmp "1.0.0-alpha.9" "1.0.0-alpha.10") (-1)]
    [(semver.cmp "1.0.0-1" "1.0.0-alpha") (-1)]
  ];

  cmpFailures = builtins.filter (c: builtins.elemAt c 0 != builtins.elemAt c 1) cmpCases;
in
  if failures == [] && cmpFailures == []
  then "ok: ${toString (builtins.length cases + builtins.length cmpCases)} cases"
  else throw "semver test failures: ${builtins.toJSON {inherit failures cmpFailures;}}"
