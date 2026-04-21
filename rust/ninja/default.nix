{
  packages = {
    rust-ninja = {
      lib,
      rustPlatform,
    }:
      rustPlatform.buildRustPackage {
        pname = "rust-ninja";
        version = "0.1.0";

        src = lib.fileset.toSource {
          root = ./.;
          fileset = lib.fileset.unions [
            ./Cargo.toml
            ./Cargo.lock
            ./src
          ];
        };

        cargoLock.lockFile = ./Cargo.lock;

        meta = {
          description = "A Ninja-compatible build system written in Rust";
          homepage = "https://tangled.org/overby.me/overby.me/tree/main/rust/ninja";
          license = lib.licenses.asl20;
          mainProgram = "ninja";
        };
      };

    rust-ninja-dev = {
      lib,
      rustPlatform,
    }:
      rustPlatform.buildRustPackage {
        pname = "rust-ninja-dev";
        version = "0.1.0";

        src = lib.fileset.toSource {
          root = ./.;
          fileset = lib.fileset.unions [
            ./Cargo.toml
            ./Cargo.lock
            ./src
          ];
        };

        cargoLock.lockFile = ./Cargo.lock;

        buildType = "debug";

        meta = {
          description = "A Ninja-compatible build system written in Rust (dev build, fast compile)";
          homepage = "https://tangled.org/overby.me/overby.me/tree/main/rust/ninja";
          license = lib.licenses.asl20;
          mainProgram = "ninja";
        };
      };
  };

  checks = let
    # Names of test methods inside the upstream Output class in
    # misc/output_test.py. Each is run individually so that failures are
    # localized and Nix caches per-test results.
    testNames = [
      "test_issue_1418"
      "test_issue_1214"
      "test_issue_1966"
      "test_issue_2499"
      "test_pr_1685"
      "test_issue_2048"
      "test_pr_2540"
      "test_depfile_directory_creation"
      "test_status"
      "test_ninja_status_default"
      "test_ninja_status_quiet"
      "test_entering_directory_on_stdout"
      "test_tool_inputs"
      "test_tool_compdb_targets"
      "test_tool_multi_inputs"
      "test_explain_output"
      "test_issue_2586"
      "test_issue_2621"
    ];
    # Phase 4 differential roundtrip checks: build a small C project
    # with both rust-ninja and the reference `pkgs.ninja` and `cmp` the
    # produced artifacts. Catches scheduling/depfile bugs that
    # output_test.py can't surface.
    roundtripNames = [
      "cold-build"
      "incremental-noop"
      "incremental-modify"
      "depfile-header-change"
      "cmake-cold-build"
      "cmake-incremental-modify"
      "cmake-clean-rebuild"
    ];
    # Phase 3 jobserver tests from misc/jobserver_test.py. The full
    # GNU make jobserver client protocol is implemented for the FIFO
    # variant; the FD-pair (`--jobserver-fds=R,W`) variant is rejected
    # with the canonical "Pipe-based protocol is not supported!"
    # warning, matching reference ninja.
    jobserverTestNames = [
      "test_no_jobserver_client"
      "test_jobserver_client_with_posix_fifo"
      "test_jobserver_client_with_posix_pipe"
      "test_client_passes_MAKEFLAGS"
    ];
  in
    builtins.listToAttrs (
      (map (name: {
          name = "rust-ninja-test-${name}";
          value = pkgs: import ./testsuite.nix {inherit pkgs name;};
        })
        testNames)
      ++ (map (name: {
          name = "rust-ninja-roundtrip-${name}";
          value = pkgs: import ./roundtrip.nix {inherit pkgs name;};
        })
        roundtripNames)
      ++ (map (name: {
          name = "rust-ninja-jobserver-${name}";
          value = pkgs:
            import ./testsuite.nix {
              inherit pkgs name;
              module = "jobserver_test";
              className = "JobserverTest";
            };
        })
        jobserverTestNames)
    );
}
