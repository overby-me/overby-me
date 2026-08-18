source ../helpers.nu

# Same structural test as `structural`, but with `-i 4` indentation.
let dump = $env.TMPDIR | path join dump.json
let derr = $env.TMPDIR | path join dump.err
let exit_code = (do { ^$env.RUST -i 4 o> $dump e> $derr } | complete).exit_code
if $exit_code != 0 {
  print $"FAIL: pw-dump exited with ($exit_code)"
  print (open --raw $derr)
  exit 1
}

assert_grep "Core type"    '"PipeWire:Interface:Core"'    $dump
# At indent=4, each nesting level is 4 spaces. Array > object > property
# = 8 spaces before `"id":`.
assert_grep "8-space id key" '^        "id":' $dump

let first_byte = (open --raw $dump | str substring 0..<1)
if $first_byte != "[" {
  print "FAIL: pw-dump output does not start with '['"
  exit 1
}
print "PASS: pw-dump --indent 4 structural"
