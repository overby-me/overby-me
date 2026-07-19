source ../helpers.nu

# With --indent 0 the output collapses to a single line of JSON.
let dump = $env.TMPDIR | path join dump.json
let derr = $env.TMPDIR | path join dump.err
let ec = (do { ^$env.RUST -i 0 o> $dump e> $derr } | complete).exit_code
if $ec != 0 {
  print $"FAIL: pw-dump exited ($ec)"
  print (open --raw $derr)
  exit 1
}

# Should be exactly one line.
let lines = (open --raw $dump | ^wc -l | str trim | into int)
if $lines != 1 {
  print $"FAIL: expected 1 line, got ($lines)"
  print (open --raw $dump | lines | first 3 | str join "\n")
  exit 1
}

assert_grep "Core type" '"PipeWire:Interface:Core"' $dump
print "PASS: pw-dump --indent 0 structural"
