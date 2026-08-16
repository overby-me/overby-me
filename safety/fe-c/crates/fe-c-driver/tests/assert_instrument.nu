#!/usr/bin/env nu
# Asserts the B2 MIR instrumentation: the instrumented harness fired a
# non-zero number of injected checks, the at-exit reporter printed, the
# program's own output is unchanged from the uninstrumented control, and
# the control fired zero checks.

def prog_line [lines: list] {
  $lines | where {|l| $l | str starts-with "v="} | first
}

def count [lines: list] {
  $lines
  | where {|l| $l | str starts-with "fec-count="}
  | first
  | str replace "fec-count=" ""
  | into int
}

def main [instrumented: path, control: path] {
  let ins = (open $instrumented | lines)
  let ctl = (open $control | lines)

  # Instrumentation must not change observable program behaviour.
  if (prog_line $ins) != (prog_line $ctl) {
    error make {msg: $"instrumentation changed program output: (prog_line $ins) vs (prog_line $ctl)"}
  }

  # Instrumented run fired checks and reported them.
  let n = (count $ins)
  if $n < 1 {
    error make {msg: $"expected a non-zero check count, got ($n)"}
  }
  if ($ins | where {|l| $l | str contains "deref checks executed"} | is-empty) {
    error make {msg: "the at-exit check report is missing"}
  }

  # Control was genuinely uninstrumented.
  let c = (count $ctl)
  if $c != 0 {
    error make {msg: $"control must be uninstrumented but reported ($c) checks"}
  }

  print $"instrument OK: ($n) checks fired, program output unchanged, control clean"
}
