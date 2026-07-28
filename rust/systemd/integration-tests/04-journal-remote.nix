{
  name = "04-JOURNAL";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.journal-remote\\.sh$";
  };
  testTimeout = 300;
  extraPackages = pkgs: [pkgs.openssl pkgs.curl];
  # WHERE IT STOPS, re-measured 2026-07-28 after the restart-ordering fix.
  #
  # An earlier version of this note blamed `unstarted_deps` in
  # units/unitset_manipulation/activate.rs. THAT WAS WRONG, twice over, and the
  # corrected story is worth keeping because both errors were easy to make:
  #
  #   * The error came from somewhere else entirely. "these related units did
  #     not have the expected state" is UnitOperationErrorReason::DependencyError,
  #     built ONLY in units/unit.rs (1582/1734/1832) from the bad_ids that
  #     Unit::state_transition_starting returns. `unstarted_deps` has just two
  #     callers and neither constructs it.
  #   * "Wants= is never enqueued" was too strong. collect_unit_start_subgraph
  #     expands via Dependencies::start_before_this(), which DOES include After=
  #     deps that are also Wants=/Requires=/BindsTo=/Upholds=, so plain
  #     `systemctl start` always pulled the target in. Only RESTART was broken.
  #
  # The real defect was ORDERING, and the correct code was already present but
  # unreachable: the Restart handlers called reactivate_unit FIRST, that returned
  # DependencyError while network-online.target was still NeverStarted, the `?`
  # propagated out, and the block right below it -- which starts exactly those
  # NeverStarted Wants=/Requires= deps -- never ran. Starting the dependencies
  # first fixes it. Three handlers shared the shape; the RestartNoBlock one only
  # logged its error instead of propagating, so it failed silently.
  #
  # THAT PART NOW WORKS: the DependencyError is gone and the test runs about
  # 2560 lines further, through the whole TLS upload round trip (25 entries
  # uploaded and imported).
  #
  # IT STILL FAILS, on a later and unrelated assertion:
  #
  #     timeout 10 bash -xec 'while [[ "$(systemctl show -P ActiveState \
  #         systemd-journal-upload)" != failed ]]; do sleep 1; done'
  #
  # ActiveState reads `inactive`, never `failed`. Upstream stops journal-remote
  # immediately before this section and points journal-upload at the server's own
  # key/cert as if they were client certs, so the upload has nothing to talk to
  # and must fail: upstream expects "failed with code 401", then
  # "Main process exited, code=exited, status=1/FAILURE" and, with a Restart=no
  # drop-in, the unit settling in `failed`. In our run journal-upload logs no
  # error at all and PID 1 records "Deactivated successfully".
  #
  # THE C ORACLE CANNOT SETTLE THIS ONE. c-systemd-test-04-journal-remote dies
  # EARLIER than we now do, at `systemctl restart systemd-journal-remote.socket`,
  # and never reaches the assertion -- so it says nothing about it either way.
  # (Do not read that as a pass for us; it only means real systemd stops sooner
  # in this harness.)
  #
  # NOW ATTRIBUTED, measured 2026-07-28 with a temporary in-VM probe (since
  # removed). PID 1 is NOT at fault and the unit is NOT mis-loaded:
  #
  #     FragmentPath=/etc/systemd/system/systemd-journal-upload.service
  #     DropInPaths=/run/systemd/system/systemd-journal-upload.service.d/99-test.conf
  #     ExecStart={ path=/nix/store/...-systemd-journal-upload ... }
  #     after it exits: ActiveState=inactive SubState=dead
  #                     ExecMainStatus=0 Result=success
  #
  # So the fragment, the drop-in and ExecStart all resolve correctly, and PID 1
  # faithfully recorded what it was given. **Our systemd-journal-upload really
  # does exit 0 here.** `inactive` is the correct state for a service that exits
  # successfully -- the test wants `failed`, and nothing failed. This is
  # therefore NOT the exit-status-recording question that 59-RELOADING-RESTART is
  # stuck on; do not merge the two.
  #
  # WHY IT SUCCEEDS, now confirmed by reading the server: **our
  # systemd-journal-remote never verifies client certificates at all.**
  #
  # `TrustedCertificateFile=` (crates/journal-remote/src/main.rs:136) and
  # `--trust` (:57) are parsed, merged (:418) and stored into Config (:432) --
  # and then never read again. Every one of the nine occurrences of
  # `trusted_cert` is parse, store or plumb; none is a check. `read_ssl_config`
  # (:346-373) builds `SslConfig { certificate, private_key }`, i.e. the SERVER
  # key and cert only, so the trusted-CA value never reaches the TLS layer.
  # tiny_http's SslConfig has no client-authentication field, so with this
  # server stack mutual TLS cannot happen at all.
  #
  # That is exactly why the journal for this phase shows
  #
  #     systemd-journal-upload[...]: Uploading 17 entries to https://localhost:19532
  #     systemd-journal-upload[...]: Upload complete
  #
  # The upload is ACCEPTED, so there is no 401, so the client has nothing to
  # fail on, so it exits 0 and the unit lands `inactive`. The whole chain
  # follows from the missing authorization check.
  #
  # NOTE THIS IS SECURITY-RELEVANT, not merely a test gap: `--trust` /
  # `TrustedCertificateFile=` is an authorization control that silently does
  # nothing, and the option's own help text (:55) even advertises `"-" to
  # disable verification`, implying verification happens otherwise. Anyone
  # relying on it to restrict who may upload journals is unprotected.
  #
  # Greening this needs real client-certificate verification in journal-remote,
  # which tiny_http cannot express -- it would mean moving to a TLS stack that
  # exposes peer verification (rustls directly, with a ClientCertVerifier built
  # from the trusted CA). That is a genuine piece of work, not a patch.
  #
  # A SEPARATE, SMALLER DEFECT spotted by the same probe: `Description=` comes
  # back EMPTY for this unit, which is why the journal logs it as
  # "Started systemd-journal-upload.service" rather than
  # "Started Journal Remote Upload Service". Cosmetic, unrelated to the failure.
}
