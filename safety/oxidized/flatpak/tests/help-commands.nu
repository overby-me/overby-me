#!/usr/bin/env nu

# Commands that accept --help or at least don't crash when called with it.
# We verify the binary doesn't segfault (exits with some code) for each command.
# Some commands (like ps, kill, history) don't support --help and may produce
# no output: that's fine as long as they don't crash.

let commands = [
  run list info install uninstall update
  override remotes remote-add remote-delete remote-ls
  ps kill enter search history config repair
  build-init build build-finish build-export build-bundle
  build-import-bundle build-sign build-update-repo build-commit-from
  repo create-usb mask pin
]

for cmd in $commands {
  # Run the command with --help, allow non-zero exit
  let rc = (do { ^$env.FLATPAK $cmd --help } | complete).exit_code

  # A segfault gives rc=139. Anything else is acceptable.
  if $rc == 139 or $rc == 134 or $rc == 136 {
    print $"FAIL: '($cmd) --help' crashed with signal \(rc=($rc)\)"
    exit 1
  }

  print $"OK: ($cmd) --help exited with rc=($rc)"
}

print "PASS: all subcommand --help calls completed without crashing"
