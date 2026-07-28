{
  name = "04-JOURNAL";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.journal-remote\\.sh$";
  };
  testTimeout = 300;
  extraPackages = pkgs: [pkgs.openssl pkgs.curl];
  # WHERE IT STOPS, measured 2026-07-28. Everything up to the upload step
  # passes. It dies on
  #
  #     systemctl restart systemd-journal-upload
  #
  # with "The unit systemd-journal-upload.service ... failed to start/stop
  # because these related units did not have the expected state:
  # [network-online.target]".
  #
  # This is a Wants= bug, not a journal one. The unit we ship in default.nix
  # declares `Wants=network-online.target` plus `After=network-online.target`.
  # In systemd, Wants= PULLS THE TARGET IN: starting the service enqueues
  # network-online.target too, and a Wants= that fails does not block the
  # depending unit. rust never activates it, so the target stays NeverStarted
  # and unstarted_deps (units/unitset_manipulation/activate.rs, the is_pull_dep
  # branch) blocks forever, because for a pulled dep it requires
  #
  #     matches!(status, UnitStatus::Started(_) | UnitStatus::Stopped(_, _))
  #
  # and NeverStarted satisfies neither. The target IS in the unit table -- a
  # dependency missing from the table is treated as ready and would not block
  # -- and it appears nowhere else in the VM log, confirming nothing pulls it.
  #
  # So the service is permanently unstartable. The fix belongs in activation:
  # an explicit start must enqueue the unit's Wants= dependencies the way
  # Requires= ones are. That is the central dependency engine, so it needs the
  # regression trio plus dependency-ordering coverage, not a quick patch.
  # Do NOT "fix" it by treating a NeverStarted Wants= as ready: that would
  # break After= ordering whenever the dep genuinely is being activated.
}
