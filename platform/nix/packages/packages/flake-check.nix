# Bounded-memory stand-in for `nix flake check`.
#
# `nix flake check` instantiates every check in one evaluator and keeps the
# whole value graph reachable until it exits, so on this tree it passes 30 GiB
# and is OOM-killed. `--max-jobs` does not help: it caps build concurrency, and
# the memory goes before a single build starts.
#
# nix-eval-jobs restarts a worker once it exceeds --max-memory-size, bounding
# peak memory by (workers * max-memory-size) instead of by the size of the
# check set. Keep that generous: a restart re-evaluates nixpkgs from scratch,
# so a tight cap trades memory for a large slowdown.
{
  writeShellApplication,
  nix-eval-jobs,
  jq,
}:
writeShellApplication {
  name = "flake-check";
  runtimeInputs = [nix-eval-jobs jq];
  text = ''
    system="''${SYSTEM:-$(nix eval --raw --impure --expr builtins.currentSystem)}"
    workers="''${WORKERS:-2}"
    max_memory_mb="''${MAX_MEMORY_MB:-5120}"
    batch="''${BATCH:-20}"
    # Override to realise a different fragment, e.g. CHECK_FRAGMENT=devShells.$system
    check_fragment="''${CHECK_FRAGMENT:-checks.$system}"

    out="$(mktemp -d)"

    # Nothing pushes check outputs to the cache today - CI's `nix flake archive`
    # uploads only the flake source and its inputs - so every run rebuilds from
    # scratch. Record what we realise and let the caller push it. Written even
    # when a check fails, so a partial run still makes the next one cheaper.
    cleanup() {
      if [ -n "''${OUT_PATHS_FILE:-}" ] && [ -s "$out/built-paths.txt" ]; then
        sort -u "$out/built-paths.txt" > "$OUT_PATHS_FILE"
      fi
      rm -rf "$out"
    }
    trap cleanup EXIT

    # Any remaining arguments are forwarded to the evaluation, so callers can
    # pass flake options such as --override-input.
    eval_args=("$@")

    # Set by `evaluate` when an attribute fails to evaluate, and read once at
    # the end so the run reports everything before it fails.
    eval_failed=0

    # Evaluate a fragment into $out/<label>.jsonl. Attributes that fail to
    # evaluate are reported and set `eval_failed`; the return value is
    # nix-eval-jobs' own, so the run carries on to what did evaluate.
    evaluate() {
      local fragment="$1" label="$2" rc=0
      echo ">> evaluating .#$fragment (workers=$workers, max-memory=''${max_memory_mb}MiB)"
      nix-eval-jobs \
        --flake ".#$fragment" \
        --workers "$workers" \
        --max-memory-size "$max_memory_mb" \
        --check-cache-status \
        ''${eval_args[@]+"''${eval_args[@]}"} \
        > "$out/$label.jsonl" || rc=$?

      if jq -e 'select(.error != null)' "$out/$label.jsonl" > /dev/null 2>&1; then
        echo ">> evaluation errors in $fragment:" >&2
        jq -r 'select(.error != null) | "  \(.attr): \(.error | rtrimstr("\n") | split("\n") | last)"' \
          "$out/$label.jsonl" >&2
        # Recorded, not fatal. One attribute that cannot be evaluated used to
        # end the run before a single check was built, so the answer to "does
        # this tree build" was withheld by whichever check was most broken -
        # `_ninja-darling-launcher` names a store path and can only be
        # evaluated with --impure, and on its own it hid the state of 5481
        # others. The run still fails at the end.
        eval_failed=1
      fi
      return "$rc"
    }

    # `nix flake check` also instantiates these; it never builds them.
    for fragment in "packages.$system" "devShells.$system"; do
      label="''${fragment%%.*}"
      evaluate "$fragment" "$label"
    done

    evaluate "$check_fragment" checks

    mapfile -t drvs < <(jq -r 'select(.isCached == false) | .drvPath' "$out/checks.jsonl")
    cached="$(jq -r 'select(.isCached == true) | .drvPath' "$out/checks.jsonl" | wc -l)"
    total=$(( ''${#drvs[@]} + cached ))
    echo ">> checks: $total  cached: $cached  to build: ''${#drvs[@]}"

    if [ "''${#drvs[@]}" -eq 0 ]; then
      echo ">> all checks already cached"
      exit "$eval_failed"
    fi

    # One build job at a time: several checks are NixOS VM tests, which are
    # memory-hungry to run even though they are cheap to schedule.
    failures="$out/build-failures.txt"
    built="$out/built-paths.txt"
    : > "$failures"
    : > "$built"
    i=0
    while [ "$i" -lt "''${#drvs[@]}" ]; do
      targets=()
      for drv in "''${drvs[@]:i:batch}"; do targets+=("$drv^*"); done
      if ! nix build --max-jobs 1 --cores 2 --keep-going --no-link --print-out-paths \
             "''${targets[@]}" >> "$built"; then
        # --keep-going reports the batch as failed; re-run singly to name them.
        for drv in "''${drvs[@]:i:batch}"; do
          nix build --max-jobs 1 --cores 2 --no-link --print-out-paths "$drv^*" >> "$built" 2>/dev/null \
            || echo "$drv" >> "$failures"
        done
      fi
      i=$(( i + batch ))
      echo ">> processed $(( i < ''${#drvs[@]} ? i : ''${#drvs[@]} ))/''${#drvs[@]}"
    done

    if [ -s "$failures" ]; then
      echo ">> failed checks:" >&2
      while read -r drv; do
        # Report the check name; the drv path alone says little.
        attr="$(jq -r --arg d "$drv" 'select(.drvPath == $d) | .attr' "$out/checks.jsonl" | head -1)"
        echo "  ''${attr:-?} ($drv)" >&2
      done < "$failures"
      exit 1
    fi
    if [ "$eval_failed" -ne 0 ]; then
      echo ">> checks built, but some attributes did not evaluate" >&2
      exit 1
    fi
    echo ">> all checks passed"
  '';
}
