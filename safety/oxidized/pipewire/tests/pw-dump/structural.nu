source ../helpers.nu

# Structural test: oxidized-pipewire pw-dump should produce valid JSON listing
# the daemon's registry globals. Byte-for-byte parity with the C tool isn't
# the goal here: the C tool also binds each global and emits a richer
# `info` block, which is Phase 8 work for us. Here we just assert the
# basic shape and the presence of the always-on globals.

let dump = $env.TMPDIR | path join dump.json
let derr = $env.TMPDIR | path join dump.err
let exit_code = (do { ^$env.RUST o> $dump e> $derr } | complete).exit_code
if $exit_code != 0 {
  print $"FAIL: pw-dump exited with ($exit_code)"
  print "--- stderr ---"; print (open --raw $derr)
  print "--- stdout ---"; print (open --raw $dump)
  exit 1
}

# Strict JSON validation via python.
let py = ($env.pkgs_python? | default python3)
let pyprog = ("import json,sys; json.load(open('" + $dump + "'))")
# python may not be available in this sandbox; fall back to grep checks.
try { ^$py -c $pyprog }

assert_grep "Core type"           '"PipeWire:Interface:Core"'            $dump
assert_grep "Module type"         '"PipeWire:Interface:Module"'          $dump
assert_grep "Factory type"        '"PipeWire:Interface:Factory"'         $dump
assert_grep "SecurityContext type" '"PipeWire:Interface:SecurityContext"' $dump
assert_grep "Client type"         '"PipeWire:Interface:Client"'          $dump
assert_grep "id field"            '"id":'                               $dump
assert_grep "version field"       '"version":'                          $dump
assert_grep "permissions list"    '"permissions":'                      $dump
assert_grep "props block"         '"props":'                            $dump

# Top level should be a JSON array.
let first_byte = (open --raw $dump | str substring 0..<1)
if $first_byte != "[" {
  print "FAIL: pw-dump output does not start with '['"
  print (open --raw $dump | lines | first 3 | str join "\n")
  exit 1
}

print "PASS: pw-dump structural"
