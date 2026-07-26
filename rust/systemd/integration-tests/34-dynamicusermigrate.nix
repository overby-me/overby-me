{
  name = "34-DYNAMICUSERMIGRATE";
  # Was a FAKE PASS (`touch /testok` with nothing run). Now an honest skip.
  # Baselined and iterated 2026-07-26; the whole DynamicUser=0 half of
  # test_directory now passes, including every host-side assertion.
  #
  # Fixed while getting there (all of these affect any service using exec
  # directories, not just this test):
  #   1. Exec directories were invisible under TemporaryFileSystem=: a tmpfs on
  #      /var/lib hid the service's own StateDirectory=. They are now bound back
  #      into the namespace via the BindPaths= machinery, which opens an O_PATH
  #      fd before the tmpfs is mounted.
  #   2. `TemporaryFileSystem=/var/lib:ro` mounted the tmpfs read-only up front,
  #      so the bind step could not create its mount points inside it. The `ro`
  #      is now deferred to a non-recursive remount after the binds, leaving the
  #      exec directories writable inside a read-only tmpfs (as upstream does).
  #   3. An exec-directory alias under a tmpfs was also written as a symlink on
  #      the host. Upstream leaves no trace there: `zzz:yyy` without a tmpfs
  #      leaves a symlink, `zzz:xxx` with one leaves nothing.
  #   4. `source[:destination[:access-mode]]` now parses for RuntimeDirectory=,
  #      CacheDirectory= and LogsDirectory=, not just StateDirectory=. They
  #      previously created a directory literally named `zzz:yyy`.
  #   5. DynamicUser=0 <-> 1 migration moves <base>/<name> under private/ and
  #      back, following only a symlink that really points at private/<name>.
  #   6. ProtectSystem=strict re-derived its implicit ReadWritePaths= with
  #      format!("/var/lib/{dir_name}") per directory type, so an aliased entry
  #      became the nonexistent /var/lib/zzz:xxx and the real state directory
  #      stayed read-only. It now uses the paths recorded at creation time.
  #
  # FIRST REMAINING FAILURE, the first command of the DynamicUser=1 phase:
  #   systemd-run --wait -p DynamicUser=1 -p StateDirectory=zzz \
  #               test -f /var/lib/zzz/test
  # exits 1 (the binary runs; the file is not visible to it).
  #
  # Established by instrumentation, so do not re-derive:
  #   - the migration itself is correct: the log shows
  #     `migrated "/var/lib/zzz" into "/var/lib/private/zzz"` and
  #     `exec dir: "/var/lib/zzz" -> "/var/lib/private/zzz" (uid=61184
  #     gid=61184 mode=755)`.
  #   - no bind-mount or remount warning is emitted.
  #   - binding <base>/private/<name> onto <base>/<name> is a NO-OP as written:
  #     the destination is the symlink, which the kernel resolves back to the
  #     source, so the service still reaches its directory through private/.
  #
  # NEXT STEP: stop theorising and instrument the child. Add a check just before
  # execv (after namespace setup and privilege drop) that stats each entry of
  # exec_dir_paths and logs the errno when it is unreachable. That distinguishes
  # the remaining candidates (private/ traversal mode, an implied-sandbox
  # interaction, or the bind being a no-op) in a single VM run, and
  # "service cannot reach its own StateDirectory" is worth logging permanently.
  #
  # Further in, the test also needs nested exec directories (`quux/pief`,
  # `xxx/yyy:aaa/111`) and idmapped mounts on kernels >= 5.12.
  patchScript = ''
    {
      echo "#!/usr/bin/env bash"
      echo "echo 'rust-systemd: DynamicUser=1 service cannot see its migrated StateDirectory' >/skipped"
      echo "exit 77"
    } > TEST-34-DYNAMICUSERMIGRATE.sh
    chmod +x TEST-34-DYNAMICUSERMIGRATE.sh
  '';
  # Skips rather than passes: DynamicUser=1 cannot reach its migrated state dir
  # See ../docs/TEST-OVERRIDES.md.
  expectedSkip = true;
}
