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
  # Named here rather than discovered, because an input cannot be discovered:
  # nix resolves `inputs` from flake.nix before any of this is evaluated, so
  # this and the list in flake.nix are kept in step by hand. Getting that
  # wrong fails either way - input-usage catches an input nothing reads, and
  # this fails on an input that is not there.
  #
  # Each is named literally rather than looped over, because input-usage
  # proves an input has a consumer by grepping for `inputs.<name>`, and a
  # reference built from a variable is invisible to it. Writing them out is
  # what makes the use findable, by that check and by anyone else grepping
  # the tree, so the repetition is the point rather than a cost.
  #
  # The attribute name is the project's name in projects.nuon, which is also
  # the tree's package name, so one string addresses both sides.
  #
  # Not all 38. This is the cheap end by crate count, and grows deliberately
  # because each entry costs a second build of the crate. The two projects
  # with sibling dependencies are absent for a different reason: they publish
  # as several directories with the crate one level down, which their own
  # directory is not, so there is no in-tree flake to compare.
  published = {
    oxidized-awk = config.inputs.oxidized-awk;
    oxidized-bash = config.inputs.oxidized-bash;
    oxidized-binutils = config.inputs.oxidized-binutils;
    oxidized-bison = config.inputs.oxidized-bison;
    oxidized-bubblewrap = config.inputs.oxidized-bubblewrap;
    oxidized-bzip2 = config.inputs.oxidized-bzip2;
    oxidized-diffutils = config.inputs.oxidized-diffutils;
    oxidized-file = config.inputs.oxidized-file;
    oxidized-gcc = config.inputs.oxidized-gcc;
    oxidized-gzip = config.inputs.oxidized-gzip;
    oxidized-help2man = config.inputs.oxidized-help2man;
    oxidized-llvm = config.inputs.oxidized-llvm;
    oxidized-make = config.inputs.oxidized-make;
    oxidized-ninja = config.inputs.oxidized-ninja;
    oxidized-patch = config.inputs.oxidized-patch;
    oxidized-perl = config.inputs.oxidized-perl;
    oxidized-pipewire = config.inputs.oxidized-pipewire;
    oxidized-patchelf = config.inputs.oxidized-patchelf;
    oxidized-pcre2 = config.inputs.oxidized-pcre2;
    oxidized-sed = config.inputs.oxidized-sed;
    oxidized-texinfo = config.inputs.oxidized-texinfo;
    wclip = config.inputs.wclip;
  };
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
