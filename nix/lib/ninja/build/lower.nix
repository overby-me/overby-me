# Lower a Ninja build graph (from `rust-ninja -t graph-json`) to Nix
# derivations: one derivation per edge, no import-from-derivation beyond the
# single graph-extraction step. A sibling to nix/lib/buck2/build/lower.nix.
#
# Model: a virtual build tree rooted at the Ninja build directory. Every edge
# output has a stable build-dir-relative path (exactly the string Ninja uses).
# Each edge's derivation stages its inputs into a working tree — a producer
# edge's whole `$out` tree is symlinked in (cp -rs, so transitive files and
# symlink targets travel along and large trees are never copied), while source
# inputs are copied as real files so relative `#include` and sibling lookups
# resolve — then runs the fully-expanded command (which references build-dir-
# relative paths) and exports the resulting tree as `$out`. Dependencies flow
# through store-path interpolation in the staging commands only, so an edge
# that merely names a peer's path creates no derivation dependency.
#
# mkLower { pkgs; src; toolchain; } -> { lowerGraph }
# lowerGraph graph -> { drvForOutput; producerOf; edgeDrvs; ... }
{
  pkgs,
  src,
  # Packages placed on PATH for every edge command. CMake-generated Ninja uses
  # absolute compiler paths so needs little; hand-written fixtures invoking
  # `cc`/`ar`/... need a toolchain here.
  toolchain ? [pkgs.stdenv.cc pkgs.coreutils],
  # Store paths every edge must have mounted (e.g. the configured build dir),
  # for absolute references we do not rewrite. Needed because we discard the
  # graph JSON's string context (see buildNinjaProject).
  extraInputs ? [],
  # Store paths CMake baked absolute references to (the source tree, and the
  # configured build dir for generated headers). For each, an edge's absolute
  # references under it are rewritten to build-dir-relative paths and the
  # individual files / include dirs are staged content-addressed, so a compile
  # depends only on the sources it actually reads — editing one `.c` rebuilds
  # only its object, not its siblings. When empty (hand-written manifests),
  # relative source inputs are staged instead.
  rewriteRoots ? [],
  # Whole-tree store-path substitutions applied to each command, as a list of
  # { from; to } strings. Used for the configured build dir: `from` is CMake's
  # build-dir store path (which changes on every reconfigure), `to` a
  # content-addressed copy of it (stable when only sources change), mounted via
  # `extraInputs`. Generated headers under `-I<builddir>` thus resolve while
  # staying cached across source edits.
  subs ? [],
  # Store paths (real source tree + configured build dir) to mount whole for the
  # per-compile dependency scan. When non-empty, compile edges (deps=gcc/msvc)
  # are staged depfile-precisely: a `-M` preprocess scan with these mounted
  # discovers the exact headers read (including source-relative `#include`s that
  # `-I` cannot express), and only those individual files are staged. Requires
  # `rewriteRoots` to cover the same trees. Empty = the include-dir heuristic.
  scanMounts ? [],
}: let
  inherit (pkgs) lib;
  inherit (builtins) filter concatMap listToAttrs elemAt length genList elem;

  esc = lib.escapeShellArg;

  # Derivation names must not carry string context. Graph strings inherit the
  # graph-json readFile's context (which transitively references the configured
  # build dir), so strip it before using a path as a name; the command strings
  # keep their context so Nix still mounts the referenced store paths.
  sanDrv = s:
    "ninja-"
    + lib.strings.sanitizeDerivationName
    (builtins.replaceStrings ["/" ":"] ["-" "-"]
      (builtins.unsafeDiscardStringContext s));

  indices = xs: genList (i: i) (length xs);

  # A source artifact: a build-dir-relative path not produced by any edge.
  # Turn it into a content-addressed store path so edits re-key only its
  # consumers.
  srcStorePath = rel:
    builtins.path {
      path = src + "/${rel}";
      name = "ninja-src-" + lib.strings.sanitizeDerivationName rel;
    };

  # CMake (and generated wrappers like mig) hardcode a handful of host tool
  # paths (/bin/mkdir, /usr/bin/env, ...) that do not exist in the pure edge
  # sandbox. Rewrite them to PATH-relative so the edge toolchain provides them.
  # /bin/sh is intentionally left alone (Nix mounts it). rmdir is listed before
  # rm so the longer path wins the left-to-right replaceStrings scan.
  toolPathSubs = concatMap (t: [
    {
      from = "/usr/bin/${t}";
      to = t;
    }
    {
      from = "/bin/${t}";
      to = t;
    }
  ]) ["mkdir" "rmdir" "rm" "mv" "cp" "ln" "cat" "chmod" "chown" "touch" "test" "true" "false" "env" "sed"];
in {
  lowerGraph = graph: let
    inherit (graph) edges;

    edgeOutputs = e: e.outputs ++ e.implicit_outputs;
    edgeInputs = e: e.inputs ++ e.implicit_inputs ++ e.order_only_inputs;

    # An edge with no command produces nothing to run: `phony` aliases and
    # CMake's ordering helper edges (e.g. `cmake_object_order_depends_*`). We
    # flatten these away when resolving a consumer's dependencies rather than
    # giving them a derivation.
    isNoOp = e: e.phony || e.command == null || e.command == "";

    # Absolute paths (store paths for sources/toolchains, as CMake bakes) are
    # available in every edge's sandbox automatically once the command string
    # references them — Nix mounts referenced store paths. Only *relative*
    # inputs are project sources we must stage. `.` (the build dir) and other
    # non-file order-only markers are ignored.
    isAbsolute = p: lib.hasPrefix "/" p;
    isStageableSource = p: !(isAbsolute p) && p != "." && p != "";

    # output path -> producing edge index
    producerOf = listToAttrs (concatMap (i:
      map (o: {
        name = o;
        value = i;
      }) (edgeOutputs (elemAt edges i)))
    (indices edges));

    isProduced = p: producerOf ? ${p};

    # Resolve an input to the set of *real* (command-bearing) producer edge
    # indices it depends on, flattening no-op aliases/ordering edges.
    realProducers = p:
      if !(isProduced p)
      then []
      else let
        i = producerOf.${p};
        e = elemAt edges i;
      in
        if isNoOp e
        then lib.unique (concatMap realProducers (edgeInputs e))
        else [i];

    # Relative source inputs to stage, flattening through no-op edges.
    realSources = p:
      if isProduced p
      then let
        e = elemAt edges producerOf.${p};
      in
        if isNoOp e
        then lib.unique (concatMap realSources (edgeInputs e))
        else []
      else lib.optionals (isStageableSource p) [p];

    # ---- content-addressed rewriting of CMake's absolute references --------
    # The rewrite roots are disjoint store paths (source tree, configured build
    # dir), so an absolute path is under at most one.
    rootFor = p: let
      hits = filter (r: lib.hasPrefix (toString r + "/") p) rewriteRoots;
    in
      if hits == []
      then null
      else builtins.head hits;
    underAnyRoot = p: rootFor p != null;
    relUnder = p: lib.removePrefix (toString (rootFor p) + "/") p;

    # An individual content-addressed copy of the file/dir at absolute path `p`,
    # so an edge depends only on the specific inputs it reads.
    indivOf = p:
      builtins.path {
        path = (rootFor p) + "/${relUnder p}";
        name = "src-" + lib.strings.sanitizeDerivationName (relUnder p);
      };

    # True if `p` is itself a symlink. `builtins.readFileType` follows symlinks
    # (so it never returns "symlink"); the parent's `readDir` is lstat-based and
    # does report "symlink". Staging a symlink via `builtins.path` aborts eval,
    # so callers skip these — the real target is reachable under its own path.
    # `builtins.path`/`readFileType` *abort* (and NOT tryEval-catchable — the
    # error propagates through `tryEval`) when any component of the path is a
    # symlink. CMake produces such paths (e.g. `.../libsyscall/foo` where
    # `libsyscall` is a symlink to the top-level libsyscall). So detect symlink
    # components without ever traversing one: walk from the (real) rewrite root
    # down via `readDir` of real dirs, stopping at the first symlink.
    hasSymlinkComponent = p: let
      root = rootFor p;
      parts = filter (x: x != "") (lib.splitString "/" (relUnder p));
      walk = dir: ps:
        if ps == []
        then false
        else let
          h = builtins.head ps;
          entries = builtins.readDir dir;
        in
          if !(entries ? ${h})
          then false # missing; pathExists handles it
          else if entries.${h} == "symlink"
          then true
          else if entries.${h} == "directory"
          then walk (dir + "/${h}") (builtins.tail ps)
          else false; # regular file component
    in
      if root == null
      then false
      else walk (toString root) parts;

    # Stageable via `builtins.path` only if it exists and neither it nor any
    # ancestor is a symlink; `safeRegular` additionally requires a regular file.
    safeNotSymlink = p: builtins.pathExists p && !(hasSymlinkComponent p);
    safeRegular = p: safeNotSymlink p && builtins.readFileType p == "regular";

    # Rewrite every root prefix in a command to the edge's own `$out` (the
    # merged working tree the edge runs in): `<root>/x` -> `$out/x`, and a bare
    # `<root>` (e.g. `-I<builddir>`) -> `$out`. The `/` form is listed first so
    # it wins where both could match. Using the absolute `$out` (expanded by the
    # builder shell) rather than a relative "" keeps output paths and `cd`
    # targets consistent: CMake custom commands that `cd` into a subdirectory
    # before writing a build-root-relative output would otherwise have that
    # output resolved against the subdirectory and doubled
    # (`sub/dir/sub/dir/out`). `$out`-absolute paths are immune to the `cd`.
    stripRoots = cmd:
      builtins.replaceStrings
      (concatMap (r: [(toString r + "/") (toString r)]) rewriteRoots)
      (concatMap (_: ["$out/" "$out"]) rewriteRoots)
      cmd;

    # Absolute include directories the command references under a root
    # (`-I<root>/...`, joined form as CMake emits).
    incAbsDirs = cmd:
      lib.unique (
        filter underAnyRoot
        (map (t: lib.removePrefix "-I" t)
          (filter (t: lib.hasPrefix "-I/" t) (lib.splitString " " cmd)))
      );

    # ---- depfile-precise header discovery (compile edges) ------------------
    depfilePrecise = scanMounts != [];
    # A compile edge: emits a depfile, declares a gcc/msvc deps mode, and takes
    # a source file as input. The source-input test excludes link edges, which
    # (in CMake) also carry a gcc deps mode and a `link.d` depfile but take
    # object/archive inputs, so a `-M` scan of them is meaningless.
    sourceExts = [".c" ".cc" ".cpp" ".cxx" ".c++" ".m" ".mm" ".s" ".S"];
    hasSourceInput = e:
      lib.any (i: lib.any (ext: lib.hasSuffix ext i) sourceExts) (edgeInputs e);
    isCompile = e:
      e.depfile
      != null
      && e.depfile != ""
      && (e.deps == "gcc" || e.deps == "msvc")
      && hasSourceInput e;

    # Turn a compile command into a preprocess-only `-M` dependency scan: drop
    # the object-output and codegen flags, append `-M -MF "$DEPS_OUT"`.
    scanDropArg = ["-o" "-MF" "-MT" "-MQ" "-MJ"];
    scanDrop = ["-c" "-MD" "-MMD" "-MP" "-M" "-MM" "-MG"];
    scanCommand = cmd: let
      toks = filter (t: t != "") (lib.splitString " " cmd);
      step = acc: t:
        if acc.skip
        then {
          inherit (acc) out;
          skip = false;
        }
        else if elem t scanDropArg
        then {
          inherit (acc) out;
          skip = true;
        }
        else if elem t scanDrop
        then acc
        else {
          out = acc.out ++ [t];
          skip = false;
        };
      kept =
        (builtins.foldl' step {
            out = [];
            skip = false;
          }
          toks).out;
    in
      builtins.concatStringsSep " " (kept ++ ["-M" "-MF" ''"$DEPS_OUT"'']);

    # Parse a makefile-style depfile (`target.o: a.h b.h \` + continuations)
    # into its prerequisite paths. Assumes no spaces in paths (the common case).
    parseDepfile = content: let
      joined = builtins.replaceStrings ["\\\n"] [" "] content;
      parts = lib.splitString ":" joined;
      afterColon =
        if length parts < 2
        then ""
        else builtins.concatStringsSep ":" (lib.drop 1 parts);
      toks =
        lib.splitString " "
        (builtins.replaceStrings ["\n" "\r" "\t"] [" " " " " "] afterColon);
    in
      lib.unique (filter (t: t != "" && t != "\\") toks);

    # The exact project files (under a rewrite root) a compile edge reads,
    # discovered by scanning. System headers (toolchain store paths) are already
    # mounted and need no staging, so they are filtered out here.
    #
    # A generated input (a source/header produced by another edge, e.g. a bison
    # `parser.c` or a mig header) exists in neither mounted tree — it is built,
    # not configured — so the raw command's absolute `<root>/<gen>` reference
    # would be a missing file and the scan would abort. Rewrite each such
    # reference to the producing edge's output (which the string then pulls in
    # as a dependency) so the preprocessor can read it. Non-generated inputs are
    # untouched and still resolve through the mounted source/configured trees.
    scanDepsOf = e: let
      # Each generated input (produced by another edge), normalised to the
      # build-dir-relative path the producer writes plus that producer's drv.
      # An input may be listed absolutely (`<root>/rel`, CMake's usual form and
      # also the producer's implicit output) or relatively (`rel`).
      genPairs =
        concatMap (
          g: let
            ids = realProducers g;
          in
            lib.optionals (ids != []) [
              {
                rel =
                  if underAnyRoot g
                  then relUnder g
                  else g;
                pdrv = edgeDrvs.${toString (builtins.head ids)};
              }
            ]
        )
        (filter isProduced (edgeInputs e));
      # (a) Rewrite every `<root>/rel` the command could use to the producer's
      # copy, so a generated file referenced by path resolves (and the string
      # pulls the producer in as a dependency).
      genSubs =
        concatMap (
          x:
            map (r: {
              from = toString r + "/" + x.rel;
              to = "${x.pdrv}/" + x.rel;
            })
            rewriteRoots
        )
        genPairs;
      # (b) Add each producer output's directory as an include path, so a
      # generated header pulled in by name (`#include "parser.h"`, a bison `-d`
      # output next to a flex `lexer.c`) resolves during the preprocess even
      # though it lives in a different producer's output.
      genIncs =
        lib.unique (map (x: "-I${x.pdrv}/" + builtins.dirOf x.rel) genPairs);
      scanCmd =
        builtins.replaceStrings
        (map (s: s.from) genSubs) (map (s: s.to) genSubs)
        (scanCommand e.command);
      scanDrv =
        pkgs.runCommand "ninja-scan-${sanDrv (builtins.head (edgeOutputs e))}" {
          nativeBuildInputs = toolchain ++ scanMounts;
        } ''
          export DEPS_OUT=$out
          ${scanCmd} ${lib.concatStringsSep " " genIncs}
        '';
    in
      filter underAnyRoot (parseDepfile (builtins.readFile scanDrv));

    # ---- one derivation per (non-phony) edge -------------------------------
    mkEdge = i: let
      e = elemAt edges i;
      ins = edgeInputs e;
      depIds = lib.unique (concatMap realProducers ins);
      outs = edgeOutputs e;

      # Files to stage individually under a rewrite root. For compile edges with
      # a depfile scan available, that is the exact set the compiler reads
      # (source + all headers, including source-relative ones); otherwise the
      # explicit inputs plus a copy of each `-I` directory.
      useScan = depfilePrecise && isCompile e;
      relSrcs = lib.unique (filter
        (r: safeNotSymlink (src + "/${r}"))
        (concatMap realSources ins));
      # An `-I` dir or explicit input can point at a path that is absent from the
      # store copy of a root (an empty dir Nix does not preserve, or an optional
      # include CMake emits unconditionally). Staging it via `builtins.path`
      # would abort eval, so skip anything that no longer exists; clang tolerates
      # a missing `-I` dir.
      rootSrcs =
        if useScan
        then filter builtins.pathExists (scanDepsOf e)
        else
          # Declared under-root inputs, plus under-root *files named in the
          # command* that CMake did not declare (custom commands often reference
          # a helper script/template like `awk -f .../mig.awk` without listing it
          # in DEPENDS). Directories and not-yet-produced outputs are excluded.
          lib.unique (
            (filter (p: underAnyRoot p && safeNotSymlink p) ins)
            ++ (filter
              (p: underAnyRoot p && safeRegular p)
              (lib.splitString " " e.command))
          );
      # `builtins.path` aborts on a symlink root, and some CMake `-I` dirs are
      # symlinks (e.g. libsystem_kernel/libsyscall -> the top-level libsyscall);
      # skip them — the real directory is reachable and staged under its own path.
      rootIncs =
        if useScan
        then []
        else filter safeNotSymlink (incAbsDirs e.command);

      command = let
        stripped =
          if rewriteRoots == []
          then e.command
          else stripRoots e.command;
        withSubs =
          builtins.replaceStrings (map (s: s.from) subs) (map (s: s.to) subs) stripped;
      in
        builtins.replaceStrings
        (map (s: s.from) toolPathSubs) (map (s: s.to) toolPathSubs)
        withSubs;

      stageDeps =
        lib.concatMapStringsSep "\n"
        (id: "cp -rsf --no-preserve=mode ${edgeDrvs.${toString id}}/. ./")
        depIds;
      stageRelSrcs =
        lib.concatMapStringsSep "\n"
        (s: "install -Dm644 ${srcStorePath s} ${esc s}")
        relSrcs;
      stageRootSrcs =
        lib.concatMapStringsSep "\n"
        (p: "install -Dm644 ${indivOf p} ${esc (relUnder p)}")
        rootSrcs;
      stageIncs =
        lib.concatMapStringsSep "\n"
        (p: ''
          # A prior dir copy (a peer `-I` whose tree contains this path as a
          # child symlink) may have recreated this target as a dangling symlink;
          # `mkdir -p` then fails "File exists". Drop it first (only if it is a
          # broken symlink — never a real dir or a valid link).
          if [ -L ${esc (relUnder p)} ] && [ ! -e ${esc (relUnder p)} ]; then rm -f ${esc (relUnder p)}; fi
          mkdir -p ${esc (relUnder p)}
          cp -rsf --no-preserve=mode ${indivOf p}/. ${esc (relUnder p)}/
        '')
        rootIncs;
      mkOutDirs =
        lib.concatMapStringsSep "\n"
        (o: ''realize_writable "$(dirname ${esc o})"'')
        outs;
      rspStage = lib.optionalString (e.rspfile != null && e.rspfile != "") ''
        mkdir -p "$(dirname ${esc e.rspfile})"
        printf '%s' ${esc (e.rspfile_content or "")} > ${esc e.rspfile}
      '';
      rspClean =
        lib.optionalString (e.rspfile != null && e.rspfile != "")
        ''rm -f ${esc e.rspfile}'';
      # A generated script output (e.g. the mig `build-mig` wrapper) carries an
      # absolute `#!/bin/bash` shebang that the pure edge sandbox lacks; rewrite
      # it to the toolchain bash so a downstream edge can exec it. `/bin/sh` is
      # left alone (Nix mounts it). We use a direct sed rather than
      # `patchShebangs`, which silently leaves the line when it cannot resolve
      # the interpreter in the edge's minimal PATH. No-op for non-script outputs.
      patchOutShebangs =
        lib.concatMapStringsSep "\n"
        (o: let
          # An output may be named absolutely (`<root>/rel`, the implicit-output
          # form) or relatively; the command writes it at `$out/rel`, so target
          # that (cwd is $out).
          rel =
            if underAnyRoot o
            then relUnder o
            else o;
        in ''
          # Absolute $out path: the edge command may have cd'd into a
          # WORKING_DIRECTORY subdir and not returned, so a relative path would
          # miss the file.
          if [ -f "$out/${rel}" ] && [ "$(head -c2 "$out/${rel}" 2>/dev/null)" = "#!" ]; then
            chmod u+w "$out/${rel}" || true
            sed -i \
              -e "1s|^#! *\(/usr\)\?/bin/bash|#!${pkgs.bash}/bin/bash|" \
              -e "1s|^#! */usr/bin/env  *bash|#!${pkgs.bash}/bin/bash|" \
              "$out/${rel}"
          fi
        '')
        outs;
    in
      pkgs.runCommand (sanDrv (builtins.head outs)) {
        nativeBuildInputs = toolchain ++ extraInputs;
        # Ninja commands are plain shell; keep the working tree as $out.
        preferLocalBuild = true;
        passthru = {edgeIndex = i;};
      } ''
        mkdir -p $out
        cd $out
        # Make an output directory real and writable. cp -rs stages -I dirs as
        # read-only symlinks into the source/configured store; when an edge (e.g.
        # mig) both reads inputs from and writes outputs into such a dir, writing
        # fails EACCES. Walk the path, and for each symlinked component replace it
        # with a real dir that re-links the original target's content (inputs stay
        # readable, new outputs are writable).
        realize_writable() {
          local p="$1" cur="" comp tgt oldIFS="$IFS"
          IFS='/'; set -- $p; IFS="$oldIFS"
          for comp in "$@"; do
            [ -z "$comp" ] && continue
            if [ -z "$cur" ]; then cur="$comp"; else cur="$cur/$comp"; fi
            if [ -L "$cur" ]; then
              tgt="$(readlink -f "$cur" 2>/dev/null || true)"
              rm -f "$cur"; mkdir -p "$cur"
              if [ -n "$tgt" ] && [ -d "$tgt" ]; then
                cp -rsf --no-preserve=mode "$tgt"/. "$cur"/ 2>/dev/null || true
              fi
            else
              mkdir -p "$cur"
            fi
          done
        }
        ${stageDeps}
        ${stageRelSrcs}
        ${stageRootSrcs}
        ${stageIncs}
        # Prune broken symlinks the cp -rs staging carried in from copied dir
        # trees (a child symlink whose target was not itself staged); left in
        # place they break the edge command's own mkdir/cd on those paths.
        find . -xtype l -delete 2>/dev/null || true
        ${mkOutDirs}
        ${rspStage}
        ${command}
        ${patchOutShebangs}
        ${rspClean}
      '';

    # Memoized derivations, keyed by stringified edge index. No-op edges
    # (phony / commandless ordering edges) get no derivation.
    edgeDrvs = listToAttrs (map (i: {
        name = toString i;
        value = mkEdge i;
      })
      (filter (i: !(isNoOp (elemAt edges i))) (indices edges)));

    # The derivation that produces a given output path (resolving phony).
    drvForOutput = p: let
      ids = realProducers p;
    in
      if ids == []
      then throw "nix-ninja: no edge produces '${p}'"
      else edgeDrvs.${toString (builtins.head ids)};
  in {
    inherit producerOf edgeDrvs drvForOutput edges;
    inherit (graph) defaults;
  };
}
