# Run: nix eval -f platform/nix/lib/lib/cargo/tests/profile.nix
let
  profile = import ../lib/profile.nix;

  root = {
    profile = {
      dev = {
        debug = "line-tables-only";
        package = {
          "*" = {"opt-level" = 1;};
          special = {
            "opt-level" = 3;
            debug = false;
          };
        };
      };
      release = {
        lto = "thin";
        strip = true;
      };
    };
  };

  dev = profile.mkProfiles {
    rootManifest = root;
    release = false;
  };
  rel = profile.mkProfiles {
    rootManifest = root;
    release = true;
  };
  bare = profile.mkProfiles {
    rootManifest = {};
    release = true;
  };

  checks = {
    devBase = dev.base.optLevel == "0" && dev.base.debugInfo == "line-tables-only";
    starHitsDeps = (dev.forPackage "serde" false).optLevel == "1";
    starSkipsMembers = (dev.forPackage "my-member" true).optLevel == "0";
    namedBeatsStar = (dev.forPackage "special" false).optLevel == "3";
    namedDebug = (dev.forPackage "special" false).debugInfo == "0";
    relLto = rel.base.lto == "thin" && rel.base.strip == "symbols";
    bareDefaults = bare.base.optLevel == "3" && bare.base.lto == "off" && bare.base.panic == "unwind";
  };

  failures = builtins.filter (n: !checks.${n}) (builtins.attrNames checks);
in
  if failures == []
  then "ok: ${toString (builtins.length (builtins.attrNames checks))} checks"
  else throw "profile test failures: ${builtins.concatStringsSep ", " failures}"
