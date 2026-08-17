# Check the flakes this tree publishes, against the tree's own build of them.
#
# A published project ships a flake of its own, and for as long as nothing
# here read it, it could break without anything noticing: that is what made
# the generated ones inert, and moving them under publish/generated/ tidied
# the tree without changing it. A project taken as a root input is built by
# this check, so a change to nix-workspace's module API that breaks a published
# repo fails here rather than in someone's clone.
#
# It builds the crate a second time, which is deliberate. The two builds do
# not share a builder and should not: the tree builds with
# `lib.buildCargoProject`, per-crate derivations against a committed 7.3 MB
# registry index shared by all 39 projects, and a published repo builds with
# `rustPlatform.buildRustPackage`, one derivation and nothing bespoke.
# Converging them would mean either slicing that index into every published
# repo or making each one do IFD against the sparse index on every `nix flake
# check` - paying, in 39 repos an outsider clones, for a property only the
# monorepo benefits from.
#
# So the duplication is kept and turned into the check: two independent
# builders over one source should produce the same program, and that is worth
# more than either build alone. It is a differential, in the same spirit as
# the oracles the ports are held to.
{
  config,
  lib,
  ...
}: let
  # What publish/checks declares, which is the list itself: that flake takes
  # one input per published project and exports them minus itself and the
  # framework, so adding a project is one entry there and nothing here.
  #
  # Discovered rather than named, unlike everything else that reads an input.
  # `input-usage` proves a root input has a consumer by grepping for
  # `inputs.<name>`, and there is one root input here to find - the twenty-two
  # behind it are not this flake's to justify.
  published = config.inputs.publish-checks.published;

  check = name: input: pkgs: let
    tree = pkgs.${name};
    flake = input.packages.${pkgs.stdenv.hostPlatform.system}.default;
  in
    pkgs.runCommand "check-project-flake-${name}" {} ''
      fail=0
      volatile=""

      # What the published repo ships must exist in the tree's build too. Not
      # the reverse: the tree's default.nix adds the aliases these tools are
      # drop-in replacements for - gawk beside awk, sh beside bash, gmake
      # beside make, texi2any beside makeinfo - and a published repo builds
      # only what its Cargo.toml declares as a [[bin]]. That is a difference
      # in packaging, not in the build, so the check is containment rather
      # than equality. A binary the published build ships and the tree does
      # not would be the real divergence, and still fails.
      tree_bins=$(cd ${tree}/bin 2>/dev/null && ls | sort || true)
      flake_bins=$(cd ${flake}/bin 2>/dev/null && ls | sort || true)

      if [ -z "$flake_bins" ]; then
        echo "${name}: the published build ships no binaries at all" >&2
        fail=1
      fi

      for bin in $flake_bins; do
        if [ ! -x "${tree}/bin/$bin" ]; then
          echo "${name}: the published build ships $bin and the tree does not" >&2
          fail=1
          continue
        fi

        # Same behaviour, as far as running them without input can show. Both
        # `|| true` because a program may exit non-zero on either flag, and
        # stdenv runs this under `set -o pipefail`: what is compared is the
        # output, and disagreement about the exit status shows up in it.
        #
        # Each output has its own store path substituted out first. A program
        # that reports where it is - pipewire prints argv[0] for --version -
        # cannot agree with a second build of itself, and that is not the kind
        # of difference this is looking for.
        #
        # Then the tree's binary is run twice and compared with itself. Output
        # that is not reproducible cannot say anything about the two builds:
        # pw-loopback puts its pid in the default node name it prints for
        # --help, so comparing it would fail at random. Asking each program
        # whether it is deterministic beats keeping a list of the patterns
        # that are not, which is a list nobody would maintain.
        for flag in --version --help; do
          a=$(${tree}/bin/$bin $flag 2>&1 | sed "s|${tree}|@self@|g" || true)
          again=$(${tree}/bin/$bin $flag 2>&1 | sed "s|${tree}|@self@|g" || true)
          if [ "$a" != "$again" ]; then
            volatile="$volatile $bin:$flag"
            continue
          fi

          b=$(${flake}/bin/$bin $flag 2>&1 | sed "s|${flake}|@self@|g" || true)
          if [ "$a" != "$b" ]; then
            echo "${name}: $bin $flag differs between the two builds" >&2
            echo "  tree:      $a" >&2
            echo "  published: $b" >&2
            fail=1
          fi
        done
      done

      [ "$fail" -eq 0 ] || exit 1
      # Unquoted, so a project with several binaries reports them on one line.
      echo "${name}: published ships [$(echo $flake_bins)], all agreeing with the tree's build" > $out
      echo "  tree also ships: [$(echo $tree_bins)]" >> $out
      # Named, not silently dropped: a check that quietly stopped comparing
      # things would read exactly like one that had nothing to compare.
      if [ -n "$volatile" ]; then
        echo "  skipped as non-reproducible: [$(echo $volatile)]" >> $out
      fi
    '';
in {
  checks = lib.mapAttrs' (name: input:
    lib.nameValuePair "project-flake-${name}" (check name input))
  published;
}
