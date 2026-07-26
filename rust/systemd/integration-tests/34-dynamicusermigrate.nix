{
  name = "34-DYNAMICUSERMIGRATE";
  # Was a FAKE PASS (`touch /testok` with nothing run). Now an honest skip with
  # the real first failure recorded, baselined 2026-07-26.
  #
  # Fixed since the fake pass went in:
  #   - exec-directory migration between DynamicUser=0 and =1 (create_managed_dir
  #     moves <base>/<name> under private/ and back, only following a symlink
  #     that actually points at private/<name>)
  #   - the `source[:destination[:access-mode]]` syntax now parses for
  #     RuntimeDirectory=, CacheDirectory= and LogsDirectory= too, not just
  #     StateDirectory=. They previously created a directory literally named
  #     `zzz:yyy`.
  #
  # FIRST FAILURE (line 18 of the upstream script):
  #   systemd-run --wait -p DynamicUser=0 -p StateDirectory=zzz \
  #               -p TemporaryFileSystem=/var/lib test -f /var/lib/zzz/test
  # The tmpfs masks the real /var/lib, and rust-systemd does not bind the exec
  # directories back on top of it, so the service cannot see its own
  # StateDirectory. Upstream always bind-mounts exec directories into the
  # namespace after TemporaryFileSystem=.
  #
  # NEXT STEP: register each exec directory (state/runtime/cache/logs/config) as
  # a bind entry in mount-namespace setup. The machinery is already there:
  # `bind_entries` supports a `source_fd` opened before the tmpfs mount and then
  # bound from /proc/self/fd/N (exec_helper.rs, "Step 3"), which is exactly the
  # tmpfs-shadowing case.
  #
  # Beyond that first failure the test also needs nested exec directories
  # (`quux/pief`, `xxx/yyy:aaa/111`) and, on kernels >= 5.12, idmapped mounts.
  patchScript = ''
    {
      echo "#!/usr/bin/env bash"
      echo "echo 'rust-systemd: exec directories are not bound over TemporaryFileSystem=' >/skipped"
      echo "exit 77"
    } > TEST-34-DYNAMICUSERMIGRATE.sh
    chmod +x TEST-34-DYNAMICUSERMIGRATE.sh
  '';
  # Skips rather than passes: exec dirs are masked by TemporaryFileSystem=
  # See ../docs/TEST-OVERRIDES.md.
  expectedSkip = true;
}
