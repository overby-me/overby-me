source ../helpers.nu

# pw-mididump on a non-existent file: same error message and exit code
# (255, from C's return -1) as upstream.
let ec_ref = (try { ^$env.REF /nonexistent.mid o+e> ($env.TMPDIR | path join expected); 0 } catch {|e| $e.exit_code? | default 1 })
let ec_rust = (try { ^$env.RUST /nonexistent.mid o+e> ($env.TMPDIR | path join actual); 0 } catch {|e| $e.exit_code? | default 1 })
if $ec_ref != $ec_rust {
    print $"FAIL: exit codes differ \(REF=($ec_ref) RUST=($ec_rust))"
    exit 1
}
compare "pw-mididump /nonexistent.mid"
