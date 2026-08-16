{
  # WHERE IT STOPS, 2026-07-28: the credentials section, and NOT because of
  # firstboot. The script runs
  #
  #     systemd-run --wait --pipe --service-type=exec \
  #         -p SetCredential=firstboot.locale:foo ... \
  #         systemd-firstboot --root="$ROOT" ...
  #
  # with ROOT=test-root, a RELATIVE path, and firstboot answers "Root directory
  # does not exist: test-root" while the very next at_exit `ls -lR test-root`
  # lists it happily. So the transient unit resolves that path against a
  # different working directory than the script does. Neither the test nor
  # rust's systemd-run sets WorkingDirectory=, so the unit gets whatever PID 1
  # hands it. Credential support in firstboot itself is already implemented and
  # is not the blocker.
  #
  # That is transient-unit/PID 1 cwd semantics rather than a firstboot bug, and
  # changing the default working directory for services has a wide blast
  # radius, so it is left as a decision rather than taken on the way past.
  #
  # Everything before it was fixed by walking the script forward: the test went
  # from 121 to 248 traced lines over eleven distinct defects, covering the
  # already-booted guard, passwd/shadow creation, provisioned-password
  # detection, root shell creation and overwrite protection, --reset, --copy,
  # verbatim file copying, verbatim root-password copying, --prompt-locale
  # consuming two answers, and a declined password still producing a locked
  # account.
  name = "74-AUX-UTILS";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.firstboot\\.sh$";
  };
}
