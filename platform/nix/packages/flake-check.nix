# Bounded-memory stand-in for `nix flake check`.
#
# `nix flake check` instantiates every check in a single evaluator process, and
# Nix keeps the whole value graph reachable until that process exits. With 3265
# checks (686 of them NixOS VM tests) peak evaluation memory passes 30 GiB and
# the run is OOM-killed. `--max-jobs` does not help: it caps build concurrency,
# and the memory is spent before a single build starts. `nix flake check
# --no-build --max-jobs 1` dies just the same, after ~490 of 3265 checks.
#
# nix-eval-jobs evaluates each attribute in a worker process that it restarts
# once the worker exceeds --max-memory-size, so peak memory is bounded by
# (workers * max-memory-size) rather than by the size of the check set. Keep
# max-memory-size generous: a worker restart re-evaluates nixpkgs from scratch,
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

    # Nothing pushes check outputs to the binary cache today: CI's `nix flake
    # archive` only uploads the flake source and its inputs, so every run
    # rebuilds all checks from scratch. Record what we realise here, and let the
    # caller push it. Written even when a check fails, so a partial run still
    # makes the next one cheaper.
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

    # Evaluate a fragment into $out/<label>.jsonl. Returns non-zero if any
    # attribute failed to evaluate.
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
        return 1
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
      exit 0
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
    echo ">> all checks passed"
  '';
}
