# Run a single test from the upstream ninja output_test.py suite against
# rust-ninja.
#
# The strategy mirrors rust/awk: copy the upstream ninja source tree, pin
# the rust-ninja-dev binary onto PATH as `ninja`, and let the official
# misc/output_test.py drive a single TestCase method via unittest.
#
# Run with: nix build .#checks.x86_64-linux.rust-ninja-test-{name}
# Example:  nix build .#checks.x86_64-linux.rust-ninja-test-test_status
{
  pkgs,
  name,
  # Which test file (without .py) and class to invoke. Defaults match
  # the original output_test.Output convention so existing call sites
  # keep working.
  module ? "output_test",
  className ? "Output",
}: let
  # Some upstream tests pass a custom env= dict to subprocess that omits
  # PATH (e.g. test_issue_2586 with env={'NINJA_STATUS':''}). In a normal
  # Linux environment `script` lives at /usr/bin/script which is found
  # regardless, but the nix sandbox has no /usr/bin, so we wrap
  # subprocess.check_output to inject PATH whenever a caller passes a
  # custom env without one.
  injectPathSitecustomize = pkgs.writeText "sitecustomize.py" ''
    import os
    import subprocess

    _ORIG_CHECK_OUTPUT = subprocess.check_output


    def _patched_check_output(*args, **kwargs):
        env = kwargs.get("env")
        if env is not None and "PATH" not in env:
            env = dict(env)
            env["PATH"] = os.environ.get("PATH", "")
            kwargs["env"] = env
        return _ORIG_CHECK_OUTPUT(*args, **kwargs)


    subprocess.check_output = _patched_check_output
  '';
in
  pkgs.runCommand "rust-ninja-test-${name}" {
    nativeBuildInputs = [
      pkgs.rust-ninja-dev
      pkgs.python3
      pkgs.util-linux # provides `script` used by output_test.py to fake a tty
      pkgs.coreutils
      pkgs.bash
    ];
    ninjaSrc = pkgs.ninja.src;
  } ''
    # pkgs.ninja.src may be a tarball or an unpacked source tree depending
    # on the channel — handle both.
    if [ -d "$ninjaSrc" ]; then
      cp -r "$ninjaSrc" ./ninja-src
      chmod -R u+w ./ninja-src
    else
      tar xf "$ninjaSrc"
      mv ninja-* ./ninja-src
    fi

    cd ./ninja-src

    # output_test.py uses NINJA_PATH = os.path.abspath('./ninja'), so symlink
    # the rust-ninja binary into the source root under that name.
    ln -sf ${pkgs.rust-ninja-dev}/bin/ninja ./ninja

    # Drop the sitecustomize hook on PYTHONPATH so it auto-loads and
    # transparently patches subprocess.check_output to preserve PATH.
    mkdir -p ./pyhook
    cp ${injectPathSitecustomize} ./pyhook/sitecustomize.py
    export PYTHONPATH="$PWD/pyhook:''${PYTHONPATH:-}"

    # Run only the named TestCase method. The {module}.{class}.{method}
    # naming below selects which test file and class to invoke — the
    # default targets misc/output_test.py's Output class, but jobserver
    # tests live in misc/jobserver_test.py's JobserverTest class.
    python3 misc/${module}.py "${className}.${name}" -v
    touch $out
  ''
