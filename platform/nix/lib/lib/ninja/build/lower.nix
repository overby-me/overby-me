# Lower a Ninja build graph (from `oxidized-ninja -t graph-json`) to one Nix
# derivation per edge, with no import-from-derivation past the single
# graph-extraction step. Sibling to ../../buck2/build/lower.nix.
#
# Each edge runs in a virtual build tree rooted at the Ninja build directory,
# where every output keeps the build-dir-relative path Ninja names it by. A
# producer's whole `$out` is symlinked in (cp -rs, so large trees are never
# copied); source inputs are copied as real files so relative `#include` and
# sibling lookups resolve; the resulting tree becomes this edge's `$out`.
# Dependencies flow only through store-path interpolation in the staging
# commands, so naming a peer's path is not itself a dependency.
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
  # The build graph as a file. Read at build time by each group derivation under
  # `buildTimeLowering`, so it is required there and unused otherwise.
  graphDrv ? null,
  # Build each group by running lower_group.py in the sandbox (it reads graphDrv
  # and rewrites/stages/runs the group's edges) rather than building every edge's
  # command as a Nix string during evaluation: 1-2 minutes of eval instead of
  # 15-40. False keeps the eval-time mkGroup.
  buildTimeLowering ? false,
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

    # The *real* (command-bearing) producer edges an input depends on, flattening
    # no-op alias/ordering edges. Memoised per produced path: unmemoised it
    # recurses through alias chains with a per-level lib.unique and is recomputed
    # at each of its dozen call sites, which on the 17e3-edge Darling graph never
    # finished. One cached thunk per produced path makes it linear.
    realProducersMemo = builtins.mapAttrs (_p: i: let
      e = elemAt edges i;
    in
      if isNoOp e
      then lib.unique (concatMap (inp: realProducersMemo.${inp} or []) (edgeInputs e))
      else [i])
    producerOf;
    realProducers = p: realProducersMemo.${p} or [];

    # Producers of paths named in an edge's command but never declared as inputs:
    # the undeclared cross-component link deps Darling's ninja graph omits (e.g.
    # system_duct's command names libsystem_notify.dylib). Only under-root tokens
    # count, so a bare word never matches, and arguments of the flags below are
    # dropped: those are Mach-O header strings, not build deps, and counting them
    # forges the whole libSystem umbrella into one un-orderable SCC.
    cmdMetaFlags = [
      "-o"
      "-install_name"
      "-umbrella"
      "-reexport_library"
      "-sub_library"
      "-sub_umbrella"
      "-reexported_symbols_list"
      "-exported_symbols_list"
      "-unexported_symbols_list"
      "-order_file"
    ];
    cmdProducersOf = i: let
      e = elemAt edges i;
      # Split on `=` as well, because a produced tool path can be a flag argument:
      # cctools' ar bakes `-DRANLIB=<...>/ranlib`, built by a separate group, so ar
      # must stage that group for its baked path to resolve when ar runs elsewhere.
      # The `-Xlinker`/`-Wl` wrappers go so the flag before a path stays visible.
      toks =
        filter (t: t != "-Xlinker" && t != "-Wl")
        (concatMap (lib.splitString "=")
          (concatMap (lib.splitString ",") (lib.splitString " " (e.command or ""))));
      kept = lib.imap0 (idx: t: let
        prev =
          if idx == 0
          then ""
          else elemAt toks (idx - 1);
      in
        lib.optionals (underAnyRoot t && isProduced t && !(elem prev cmdMetaFlags)) (realProducers t))
      toks;
    in
      builtins.concatLists kept;

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
        # The context of the rewrite root this is built from would otherwise ride
        # along into the copy, making the whole source tree an input of every edge
        # and defeating the per-input isolation this exists for. The file is there
        # at eval time, so it imports fresh with no reference back.
        path = builtins.unsafeDiscardStringContext (toString (rootFor p) + "/" + relUnder p);
        name = "src-" + lib.strings.sanitizeDerivationName (relUnder p);
      };

    # Whether any component of `p` is a symlink. CMake produces such paths (e.g.
    # `.../libsyscall/foo`, where `libsyscall` links to the top-level libsyscall)
    # and `builtins.path` aborts on them - past `tryEval`, so it cannot be caught.
    # Hence the walk: `readDir` is lstat-based and does report "symlink", where
    # `readFileType` follows links and never would, so descend real dirs from the
    # rewrite root and stop at the first link rather than traversing one.
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

    # The Mach/kernel RPC interface directories in an SDK `usr/include`: their
    # `.defs` and `.h` are symlinks into the tree's osfmk, which a mig edge reads
    # through `<mach/...>` / `<device/...>` includes but whose target dir it does
    # not itself stage as an -I.
    ifaceDirs = ["mach" "mach_debug" "device" "servers" "machine"];
    # Every symlink under one of `ifaceDirs`, relative to real dir `base`. Their
    # followed content is what gets staged, and staying inside the interface dirs
    # (a couple hundred files, content-addressed) rather than cp -rL of the whole
    # SDK (thousands of framework symlinks) is what keeps caching fine-grained.
    # Compile edges find their headers through the depfile scan instead; mig edges
    # have no scan, so this is where their `<mach/*>` includes resolve.
    ifaceSymlinksUnder = base: let
      collectAll = sub: let
        dir = toString base + "/${sub}";
        entries = builtins.readDir dir;
      in
        concatMap (
          n: let
            rel = "${sub}/${n}";
            t = entries.${n};
          in
            if t == "symlink"
            then [rel]
            else if t == "directory"
            then collectAll rel
            else []
        )
        (builtins.attrNames entries);
      top = builtins.readDir (toString base);
    in
      concatMap (
        n:
          if (top.${n} or "") == "directory" && elem n ifaceDirs
          then collectAll n
          else []
      )
      (builtins.attrNames top);

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

    # A generated header may be `#include <...>`d by a compile edge that declares
    # no dependency on it: the monolithic build satisfies the ordering by luck and
    # the ninja graph never encodes it (libsyscall's mach_init.c includes the
    # generated `darlingserver/rpc.h`, yet the libsyscall target's order-only
    # barrier lists no darlingserver output). So collect every generated header's
    # containing directories graph-wide as -I flags, and any scan resolves such an
    # include whatever prefix names it. clang ignores -I dirs that do not exist,
    # so the over-inclusion is deliberate and harmless.
    headerExts = [".h" ".hpp" ".hxx" ".hh" ".ipp" ".inc" ".def" ".defs"];
    isHeaderPath = o: lib.any (ext: lib.hasSuffix ext o) headerExts;
    # Whether edge `i` reaches a compile edge through its real producers. A
    # compile's derivation forces a scan that references `generatedHeaderIncs`, so
    # a header producer that transitively depends on a compile (a mig edge, via
    # migcom) cannot join that set without closing an eval cycle. Pure graph
    # analysis over indices, never a derivation, so it is itself cycle-free. Pure
    # generators (python/awk codegen reading only sources, like the darlingserver
    # rpc.h generator) are false, and theirs are exactly the undeclared headers no
    # scan can otherwise reach; a mig header already resolves through its producer.
    dependsOnCompileMemo = listToAttrs (map (i: {
        name = toString i;
        value = let
          e = elemAt edges i;
        in
          isCompile e
          || lib.any (j: dependsOnCompileMemo.${toString j})
          (concatMap realProducers (edgeInputs e));
      })
      (indices edges));
    generatedHeaderIncs = lib.unique (concatMap (
        i: let
          e = elemAt edges i;
          pdrv = edgeDrvs.${toString i};
        in
          concatMap (
            o: let
              rel =
                if underAnyRoot o
                then relUnder o
                else o;
              dirs = lib.init (filter (x: x != "") (lib.splitString "/" rel));
              ancestors =
                lib.genList (n: builtins.concatStringsSep "/" (lib.take n dirs))
                (length dirs + 1);
            in
              map (a: "-I${pdrv}" + lib.optionalString (a != "") "/${a}") ancestors
          )
          (filter isHeaderPath (edgeOutputs e))
      )
      (filter
        (i:
          !(isNoOp (elemAt edges i))
          && !dependsOnCompileMemo.${toString i}
          && lib.any isHeaderPath (edgeOutputs (elemAt edges i)))
        (indices edges)));

    # The group variant of migHeaderIncs, scoped to a compile's own source module.
    # Graph-wide would pollute the search path with unrelated generated dirs that
    # shadow standard headers (libsyscall/mach/string.h over <string.h>, for
    # migcom). Inside a group these headers are already materialised in $out, and
    # `-I$out` carries no derivation reference, so there is no edgeDrvs cycle to
    # filter against. Covers non-mig generated headers too, which migHeadersByModule
    # does not, hence a fresh module map over every header-producing edge.
    genHeaderDirsByModule =
      lib.foldl' (
        acc: i:
          lib.foldl' (
            acc2: o: let
              rel =
                if underAnyRoot o
                then relUnder o
                else o;
              mod = moduleKey rel;
              # ALL ancestor dirs of the header, not just its immediate dir: a
              # `<darlingserver/rpc.h>` produced at src/external/darlingserver/rpc.h needs
              # -I src/external (the parent), which only an ancestor supplies.
              dirs = lib.init (filter (x: x != "") (lib.splitString "/" rel));
              ancestry = lib.genList (n: builtins.concatStringsSep "/" (lib.take n dirs)) (length dirs + 1);
            in
              acc2 // {${mod} = lib.unique ((acc2.${mod} or []) ++ ancestry);}
          )
          acc (filter isHeaderPath (edgeOutputs (elemAt edges i)))
      ) {}
      (filter (i: !(isNoOp (elemAt edges i)) && lib.any isHeaderPath (edgeOutputs (elemAt edges i)))
        (indices edges));
    genIncsOutFor = i: let
      e = elemAt edges i;
      outs = edgeOutputs e;
      mod =
        if outs == []
        then ""
        else
          moduleKey (let
            o = builtins.head outs;
          in
            if underAnyRoot o
            then relUnder o
            else o);
      # Same-module generated-header dirs (covers mig headers reached via a cmake
      # target-include that the graph does not declare at all -- e.g. asl.c/<asl_ipc.h>).
      moduleDirs = genHeaderDirsByModule.${mod} or [];
      # Plus the header-output dirs of this compile's actual producers (declared +
      # implicit + order-only) -- covers CROSS-module generated headers the graph DOES
      # declare a dependency on, without pulling in unrelated dirs that could shadow.
      prodDirs = concatMap (j:
        concatMap (o: let
          rel =
            if underAnyRoot o
            then relUnder o
            else o;
          dirs = lib.init (filter (x: x != "") (lib.splitString "/" rel));
        in
          lib.genList (n: builtins.concatStringsSep "/" (lib.take n dirs)) (length dirs + 1))
        (filter isHeaderPath (edgeOutputs (elemAt edges j))))
      (lib.unique (concatMap realProducers (edgeInputs e)));
    in
      map (d: "-I$out" + lib.optionalString (d != "" && d != ".") "/${d}") (lib.unique (moduleDirs ++ prodDirs));

    # A generated mig header may be `#include <...>`d by a compile in the same
    # source module with the ninja graph declaring nothing, and (unlike rpc.h)
    # without the compile carrying even a literal `-I` to the mig output dir, cmake
    # having wired it through object-library includes: syslog's asl.c reaches
    # <asl_ipc.h>, generated in aslcommon. `generatedHeaderIncs` excludes mig
    # headers to avoid an eval cycle and per-edge `genIncs` covers only declared
    # producers, so neither resolves it. Each compile therefore takes the mig-header
    # dirs of producers in its own source module: lighter than a graph-wide set, and
    # keyed on the module rather than on a literal -I dir.
    migHeaderProducerIdxs =
      filter
      (i:
        !(isNoOp (elemAt edges i))
        && dependsOnCompileMemo.${toString i}
        && lib.any isHeaderPath (edgeOutputs (elemAt edges i)))
      (indices edges);
    # The source-module key of a build-dir-relative path: src/external/<m>, src/<m>,
    # or the first component. Used to relate a compile to mig headers near it.
    moduleKey = p: let
      cs = filter (x: x != "") (lib.splitString "/" p);
      n = length cs;
    in
      if n >= 3 && elemAt cs 0 == "src" && elemAt cs 1 == "external"
      then "src/external/" + elemAt cs 2
      else if n >= 2 && elemAt cs 0 == "src"
      then "src/" + elemAt cs 1
      else if n >= 1
      then elemAt cs 0
      else "";
    # Map: source module -> [{ p = mig producer index; dir = header's build-dir-rel
    # dir }]. Pure string/index analysis over declared outputs (references no drvs).
    migHeadersByModule =
      lib.foldl' (
        acc: i:
          lib.foldl' (
            acc2: o: let
              rel =
                if underAnyRoot o
                then relUnder o
                else o;
              mod = moduleKey rel;
            in
              acc2
              // {
                ${mod} =
                  (acc2.${mod} or [])
                  ++ [
                    {
                      p = i;
                      dir = builtins.dirOf rel;
                    }
                  ];
              }
          )
          acc (filter isHeaderPath (edgeOutputs (elemAt edges i)))
      ) {}
      migHeaderProducerIdxs;
    # The full transitive input closure of each mig producer, order-only inputs
    # included: those are staged as real derivation deps too, so any of them can
    # close an eval cycle. The BFS is over the acyclic ninja graph, so it ends.
    #
    # The cycle to avoid: giving compile i the flag -I<M> makes edgeDrvs.i depend
    # on edgeDrvs.M, so if M transitively depends on i that recurses forever. i may
    # therefore take M's mig -I only when i is not in M's closure. A global "any mig
    # producer reaches i" exclusion also breaks the cycle but over-excludes at
    # full-graph scope (syslog's asl.c lost its aslcommon inc to a producer it only
    # order-only reached); excluding data deps only let the order-only cycle back
    # in. Per-pair against the full closure is what does both.
    migProducerClosure = let
      producersOf = j: concatMap realProducers (edgeInputs (elemAt edges j));
      closureOf = m: let
        go = frontier: acc:
          if frontier == []
          then acc
          else let
            fresh = filter (j: !(acc ? ${toString j})) (lib.unique (concatMap producersOf frontier));
          in
            go fresh (acc
              // listToAttrs (map (j: {
                  name = toString j;
                  value = true;
                })
                fresh));
      in
        go [m] {};
    in
      listToAttrs (map (m: {
          name = toString m;
          value = closureOf m;
        })
        migHeaderProducerIdxs);
    # Mig-header -I flags for compile edge i: the dirs of mig headers produced in i's
    # own source module, minus any producer whose closure contains i (cycle safety).
    migHeaderIncsFor = i: let
      outs = edgeOutputs (elemAt edges i);
      mod =
        if outs == []
        then ""
        else
          moduleKey (let
            o = builtins.head outs;
          in
            if underAnyRoot o
            then relUnder o
            else o);
      safe =
        filter
        (h: !((migProducerClosure.${toString h.p} or {}) ? ${toString i}))
        (migHeadersByModule.${mod} or []);
    in
      lib.unique (map (h: "-I${edgeDrvs.${toString h.p}}/${h.dir}") safe);

    # The exact project files under a rewrite root a compile edge reads, found by
    # scanning. System headers are already mounted, so they are filtered out.
    #
    # A generated input (a bison `parser.c`, a mig header) is built rather than
    # configured, so it exists in neither mounted tree and the command's absolute
    # `<root>/<gen>` reference would abort the scan on a missing file. Each such
    # reference is rewritten to the producing edge's output, which the string then
    # pulls in as a dependency. Non-generated inputs are untouched.
    scanDrvOf = i: let
      e = elemAt edges i;
      # Each generated input, as the build-dir-relative path its producer writes
      # plus that producer's drv. An input may be listed absolutely (CMake's usual
      # form, and the producer's implicit output) or relatively.
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
      # Every `<root>/rel` the command could use, pointed at the producer's copy.
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
      # Producer output directories as include paths, so a generated header pulled
      # in by name resolves during the preprocess: beside another producer output
      # (a bison `-d` header next to a flex `lexer.c`), and through `<...>` by
      # mirroring this compile's own under-root -I dirs onto each of its declared
      # producers, so `<mach/mach_port_internal.h>` resolves from the mig edge that
      # wrote it. An edge's declared producers are upstream of it, so this stays a
      # DAG and needs no compile-dependency filter, unlike generatedHeaderIncs.
      genDrvs =
        lib.unique (map (i: edgeDrvs.${toString i})
          (concatMap realProducers (filter isProduced (edgeInputs e))));
      genIncs = lib.unique (
        (map (x: "-I${x.pdrv}/" + builtins.dirOf x.rel) genPairs)
        ++ concatMap
        (pd: map (d: "-I${pd}/${relUnder d}") (filter underAnyRoot (incAbsDirs e.command)))
        genDrvs
      );
      scanCmd =
        builtins.replaceStrings
        (map (s: s.from) genSubs) (map (s: s.to) genSubs)
        (scanCommand e.command);
      scanDrv =
        pkgs.runCommand "ninja-scan-${sanDrv (builtins.head (edgeOutputs e))}" {
          nativeBuildInputs = toolchain ++ scanMounts;
        } ''
          export DEPS_OUT=$out
          ${scanCmd} ${lib.concatStringsSep " " (genIncs ++ generatedHeaderIncs ++ migHeaderIncsFor i)}
        '';
    in
      scanDrv;

    # On parallelism: each scan is an import-from-derivation and Nix forces those
    # serially, so at CMake scale the scans dominate first-build wall-clock. They
    # cannot be batched into one parallel realization: a compile's scan resolves
    # `<generated.h>` includes, which needs that header's producer built, and a
    # producer that is itself a compile needs its own scan first, so forcing them
    # all up front closes a scan -> producer -> scan cycle. Only this eval-time
    # discovery is serial, and each scan is content-addressed, so it is paid once.
    # The real fix is grouped builds, which resolve headers at build time instead.
    scanDepsOf = i:
    # The depfile paths are substrings of the scan's output, whose context reaches
    # the mounted source tree. Left on, that context rides every `relUnder p` into
    # the staging script and makes the whole source tree an inputSrc of each
    # consuming derivation, so an edit to any source rehashes every edge. indivOf
    # re-imports each header content-addressed, keeping the per-file dependency.
      filter underAnyRoot
      (parseDepfile (builtins.unsafeDiscardStringContext (builtins.readFile (scanDrvOf i))));

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
      # An `-I` dir or explicit input can name a path absent from the store copy of
      # a root: an empty dir Nix does not preserve, or an optional include CMake
      # emits unconditionally. `builtins.path` would abort eval on those, and clang
      # tolerates a missing `-I` dir, so anything gone is skipped.
      rootSrcs =
        if useScan
        then filter builtins.pathExists (scanDepsOf i)
        else
          # Declared under-root inputs, plus under-root files the command names
          # that CMake did not declare: custom commands reference a helper script
          # like `awk -f .../mig.awk` without listing it in DEPENDS, and a linker
          # names an alias list inside a comma-joined `-Wl,-alias_list,<path>`,
          # hence splitting on commas as well as spaces.
          lib.unique (
            (filter (p: underAnyRoot p && safeNotSymlink p) ins)
            ++ (filter
              (p: underAnyRoot p && safeRegular p)
              (concatMap (lib.splitString ",") (lib.splitString " " e.command)))
          );
      # `builtins.path` aborts on a symlink root, and some CMake `-I` dirs are
      # symlinks (e.g. libsystem_kernel/libsyscall -> the top-level libsyscall);
      # skip them — the real directory is reachable and staged under its own path.
      rootIncs =
        if useScan
        then []
        else filter safeNotSymlink (incAbsDirs e.command);
      # -I dirs, inputs and command-named paths that traverse a symlink, such as
      # `.../libsyscall/mach/x.defs` where libsyscall links out of the tree. The
      # symlink they pass through is pruned as broken, its target not being staged,
      # so the reference would dangle. `builtins.path` does follow a symlink whose
      # target exists, so the followed content is staged at the reference's own
      # path; pathExists filters the broken ones, which would abort eval.
      symlinkTargets = lib.unique (filter
        (p: underAnyRoot p && hasSymlinkComponent p && builtins.pathExists p)
        (incAbsDirs e.command
          ++ ins
          ++ concatMap (lib.splitString ",") (lib.splitString " " e.command)));

      command = let
        stripped =
          if rewriteRoots == []
          then e.command
          else stripRoots e.command;
        withSubs =
          builtins.replaceStrings (map (s: s.from) subs) (map (s: s.to) subs) stripped;
        base =
          builtins.replaceStrings
          (map (s: s.from) toolPathSubs) (map (s: s.to) toolPathSubs)
          withSubs;
      in
        # A compile may `#include` a generated header ninja never declared, so it
        # is not staged into $out and the command's own $out-relative -I cannot
        # find it. The producer-output include dirs, the same set the scan uses,
        # let the compiler read it straight from the producer, and the store path
        # in the flag is what makes Nix mount that output.
        base
        + lib.optionalString (useScan && generatedHeaderIncs != [])
        (" " + lib.concatStringsSep " " generatedHeaderIncs)
        + lib.optionalString (useScan && migHeaderIncsFor i != [])
        (" " + lib.concatStringsSep " " (migHeaderIncsFor i));

      # `cp -rs` each producer's whole output tree in. A path one producer provides
      # as a real dir may already be a symlink, or sit under a symlinked parent,
      # from an earlier producer, and cp cannot overwrite a non-dir with a dir - so
      # de-symlink such a destination first and let cp merge into a real dir. The
      # test is cheap and realize_writable runs only on the rare conflict.
      stageDeps =
        lib.concatMapStringsSep "\n"
        (id: let
          d = edgeDrvs.${toString id};
        in ''
          (cd ${d} && find . -mindepth 1 -type d) | while IFS= read -r sub; do
            s=''${sub#./}
            if [ -L "$s" ] || { [ -e "$s" ] && [ ! -d "$s" ]; }; then realize_writable "$s"; fi
          done
          cp -rsf --no-preserve=mode ${d}/. ./ || true
          # Staged executable tools become real copies: one that resolves its own
          # argv[0] and execs a sibling by that dir would otherwise look inside the
          # producer store path, which holds no siblings from other producers
          # (cctools `ar` execs a co-located `ranlib`, built by a separate edge).
          # Libraries are excluded, being linked rather than run, and large.
          (cd ${d} && find . -type f -perm -u+x ! -name '*.dylib' ! -name '*.so' ! -name '*.so.*' ! -name '*.a' ! -name '*.o') | while IFS= read -r f; do
            g=''${f#./}
            if [ -L "$g" ]; then
              t=$(readlink -f "$g" 2>/dev/null) || continue
              [ -f "$t" ] && { rm -f "$g"; cp --no-preserve=mode "$t" "$g" && chmod +x "$g"; } || true
            fi
          done
        '')
        depIds;
      # Skip if the path is already staged: the same header can be both a scanned
      # input here and a producer output copied in by stageDeps (a source header
      # `install`ed into the SDK), whose read-only symlinked parent then makes a
      # second `install` fail "cannot remove". The first copy is authoritative. The
      # execute bit is preserved because a command may run a staged file directly,
      # the rpc.h generator being one; `builtins.path` keeps the source's bit, so
      # the content-addressed copy is what to test.
      stageRelSrcs =
        lib.concatMapStringsSep "\n"
        (s: ''
          if [ ! -e ${esc s} ]; then
            install -Dm644 ${srcStorePath s} ${esc s}
            if [ -x ${srcStorePath s} ]; then chmod +x ${esc s}; ${shebangSed (esc s)} fi
          fi
        '')
        relSrcs;
      stageRootSrcs =
        lib.concatMapStringsSep "\n"
        (p: ''
          if [ ! -e ${esc (relUnder p)} ]; then
            install -Dm644 ${indivOf p} ${esc (relUnder p)}
            if [ -x ${indivOf p} ]; then chmod +x ${esc (relUnder p)}; ${shebangSed (esc (relUnder p))} fi
          fi
        '')
        rootSrcs;
      stageIncs =
        lib.concatMapStringsSep "\n"
        (p: ''
          # A peer `-I` whose tree holds this path as a child symlink may have
          # recreated it dangling, and `mkdir -p` then fails "File exists". Only a
          # broken symlink is dropped, never a real dir or a valid link.
          if [ -L ${esc (relUnder p)} ] && [ ! -e ${esc (relUnder p)} ]; then rm -f ${esc (relUnder p)}; fi
          mkdir -p ${esc (relUnder p)}
          # A real dir already staged here, a producer output tree or another -I
          # copy, must win over an incoming symlink of the same name. cp reports
          # that one conflict but copies the rest, so its exit is tolerated rather
          # than fatal, and the diagnostics stay visible.
          cp -rsf --no-preserve=mode ${indivOf p}/. ${esc (relUnder p)}/ || true
          # `cp -rs` turns each source symlink into a link into the read-only
          # copy, whose own relative target then points outside it and dangles (an
          # SDK `mach/*.defs` to the tree's osfmk). Re-created with their original
          # relative target, they resolve against the merged $out tree, where
          # another -I copy stages the target - and the broken-link prune below
          # leaves them alone. A path already present as a real dir wins.
          (cd ${indivOf p} && find . -type l 2>/dev/null) | while IFS= read -r l; do
            d=${esc (relUnder p)}/"$l"
            if [ -d "$d" ] && [ ! -L "$d" ]; then continue; fi
            t=$(readlink "${indivOf p}/$l" 2>/dev/null) || continue
            ln -sfn "$t" "$d" 2>/dev/null || true
          done
        '')
        rootIncs;
      # The followed content of each Mach/kernel interface file reached through a
      # symlink whose osfmk target dir this edge does not stage itself, so a mig
      # `<mach/...>` include resolves. Runs after the prune and the -I copies so
      # its real files win over a peer -I's dangling symlink of the same name.
      # Only the source tree is walked: the configured tree's interface dirs hold
      # symlinks to not-yet-generated headers that `builtins.path` aborts on.
      stageIfaceDeref =
        lib.concatMapStringsSep "\n"
        (p:
          lib.optionalString (rewriteRoots != [] && rootFor p == builtins.head rewriteRoots)
          (lib.concatMapStringsSep "\n" (
              rel: let
                orig = toString (rootFor p) + "/" + relUnder p + "/" + rel;
                content = builtins.path {
                  path = orig;
                  name = "iref-" + lib.strings.sanitizeDerivationName rel;
                };
              in
                lib.optionalString
                (builtins.pathExists orig && builtins.readFileType content == "regular")
                ''
                  rm -f ${esc (relUnder p + "/" + rel)}
                  install -Dm644 ${content} ${esc (relUnder p + "/" + rel)}
                ''
            )
            (ifaceSymlinksUnder (toString (rootFor p) + "/" + relUnder p))))
        rootIncs;
      # The followed content of each symlinked *file* reference, a mig `.defs` say,
      # at its own through-symlink path, replacing the pruned dangling symlink.
      #
      # Directories are deliberately not staged wholesale: a symlinked include dir
      # often holds child symlinks pointing outside it, which a store copy carries
      # as broken links, and cp -rs'ing that over the tree would clobber real files
      # the header scan already staged. Compile edges get their exact headers from
      # the scan; other edges get each file they name here.
      stageSymlinkTargets =
        lib.concatMapStringsSep "\n"
        (p: let
          r = relUnder p;
          cp = indivOf p;
        in
          lib.optionalString (builtins.readFileType cp == "regular") ''
            if [ -L ${esc r} ]; then rm -f ${esc r}; fi
            install -Dm644 ${cp} ${esc r}
            if [ -x ${cp} ]; then chmod +x ${esc r}; ${shebangSed (esc r)} fi
          '')
        symlinkTargets;
      mkOutDirs =
        lib.concatMapStringsSep "\n"
        (o: ''
          realize_writable "$(dirname ${esc o})"
          # The output path itself may be a staged read-only symlink, a committed
          # source file mapping to the same merged $out path as this edge's
          # generated output. Dropping it makes the command write a fresh real file
          # rather than follow the link into the store, where fopen gets EACCES.
          if [ -L ${esc o} ]; then rm -f ${esc o}; fi
        '')
        outs;
      rspStage = lib.optionalString (e.rspfile != null && e.rspfile != "") ''
        mkdir -p "$(dirname ${esc e.rspfile})"
        printf '%s' ${esc (e.rspfile_content or "")} > ${esc e.rspfile}
      '';
      rspClean =
        lib.optionalString (e.rspfile != null && e.rspfile != "")
        ''rm -f ${esc e.rspfile}'';
      # Point a script's absolute shebang at the toolchain, since the pure edge
      # sandbox has neither /bin/bash nor /usr/bin/env. `/bin/sh` is left alone,
      # Nix mounting it. A direct sed rather than `patchShebangs`, which silently
      # leaves the line when it cannot resolve the interpreter in the minimal PATH.
      # `p` is a shell-quoted path expression; a no-op for non-scripts.
      shebangSed = p: ''
        if [ -f ${p} ] && [ "$(head -c2 ${p} 2>/dev/null)" = "#!" ]; then
          chmod u+w ${p} 2>/dev/null || true
          sed -i \
            -e "1s|^#! *\(/usr\)\?/bin/bash|#!${pkgs.bash}/bin/bash|" \
            -e "1s|^#! */usr/bin/env  *bash|#!${pkgs.bash}/bin/bash|" \
            -e "1s|^#! */usr/bin/env  *|#!${pkgs.coreutils}/bin/env |" \
            ${p}
        fi
      '';
      # A generated script output (e.g. the mig `build-mig` wrapper) carries such
      # a shebang; rewrite each. Absolute `$out/<rel>` because the edge command
      # may have cd'd into a WORKING_DIRECTORY subdir and not returned.
      patchOutShebangs =
        lib.concatMapStringsSep "\n"
        (o: let
          rel =
            if underAnyRoot o
            then relUnder o
            else o;
        in
          shebangSed ''"$out/${rel}"'')
        outs;
      # A link command whose linker step fails does not abort the edge: the body
      # has no `set -e` and cmake link rules often end `&& :`, so it exits 0 with
      # no artifact and the miss surfaces far downstream. Failing here instead
      # keeps the real linker error in this edge's log. Only declared final
      # artifacts count - anything with an extension other than .dylib/.a is an
      # object, depfile or generated source, and an edge that skips one of those
      # implicit outputs must not be tripped.
      checkOutputs =
        lib.concatMapStringsSep "\n"
        (o: let
          rel =
            if underAnyRoot o
            then relUnder o
            else o;
          base = builtins.baseNameOf o;
          isFinal =
            lib.hasSuffix ".dylib" o
            || lib.hasSuffix ".a" o
            || (!(lib.hasInfix "." base) && !(lib.hasInfix "CMakeFiles" o));
        in
          lib.optionalString isFinal ''
            if [ ! -e "$out/${rel}" ]; then
              echo "nix-ninja: edge produced no output ${rel}" >&2
              exit 1
            fi
          '')
        e.outputs;
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
        # read-only symlinks into the store, so an edge that both reads inputs from
        # and writes outputs into such a dir gets EACCES. Each symlinked component
        # becomes a real dir re-linking the original target's content: inputs stay
        # readable, new outputs writable.
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
        ${stageIfaceDeref}
        ${stageSymlinkTargets}
        ${mkOutDirs}
        ${rspStage}
        ${command}
        ${patchOutShebangs}
        ${rspClean}
        ${checkOutputs}
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

    # Whether `p` is produced by a phony / no-op edge -- an aggregate alias
    # like the top-level `all`, which has no file of its own but transitively
    # names every real target.
    isPhonyTarget = p: isProduced p && isNoOp (elemAt edges producerOf.${p});

    # The real file outputs to stage for target `p`, as `{ path; drv; }`. For a
    # real output that is just `p` from its producer; for a phony aggregate it is
    # every declared output of every real edge the phony resolves to (e.g. `all`
    # -> each sublibrary/tool's final artifact). Lets a caller materialize a
    # whole-graph build (`target = null` -> `default` -> `all`) instead of
    # trying to `cp` a nonexistent file named after the phony.
    realOutputsForTarget = p:
      lib.concatMap
      (i:
        map (o: {
          path = o;
          drv = edgeDrvs.${toString i};
        }) (edgeOutputs (elemAt edges i)))
      (realProducers p);

    # ---- grouped lowering (per-component): one derivation per edge GROUP -----
    # A group is a set of edges (a CMake subproject, from component-dag). Its
    # internal producer->consumer order and internal generated headers (mig/rpc)
    # are resolved by an emitted mini `build.ninja` run inside the group, so the
    # per-edge generated-header/cycle bridging is unnecessary here. Isolation
    # comes from staging only: the group's own declared sources, the include dirs
    # its commands name, and its EXTERNAL dependency groups' outputs (symlinked).
    #   groupOf : edgeIndex(int) -> groupId(string).  `groupOf = toString` recovers
    #   the per-edge behaviour (each edge its own group).
    lowerGroupsBy = groupOf: let
      realIds = filter (i: !(isNoOp (elemAt edges i))) (indices edges);
      # groupOf receives the edge, not its index, so a caller can group by rule or
      # by output path.
      rawGidOf = listToAttrs (map (i: {
          name = toString i;
          value = groupOf (elemAt edges i);
        })
        realIds);
      rawGid = i: rawGidOf.${toString i};
      # groupBy, one linear pass. A filter per group is O(groups*edges) and
      # lib.unique O(edges*distinct); at 1e3 groups over 38e3 edges either
      # quadratic form ran for tens of minutes.
      rawIdsInGroup = builtins.groupBy rawGid realIds;
      rawGroupIds = builtins.attrNames rawIdsInGroup;
      # Dedup by attrset key rather than lib.unique's O(n*distinct) elem-scan: the
      # lists deduped below are one per group and as long as that group's
      # (edges * inputs) before dedup. Insertion order is not meaningful for the
      # dep and reach sets this builds, so losing it costs nothing.
      fastUniq = xs: builtins.attrNames (builtins.groupBy (x: x) xs);
      # Groups staged into *every* group, so that a cross-cutting generated header
      # (darlingserver/rpc.h) reached only through a literal -I, with no declared
      # ninja dependency for realProducers to find, is always materialised in $out.
      #
      # A group qualifies only if the whole of it is a pure generator - codegen
      # that reads sources and nothing a compile produced. That is what makes the
      # role cycle-free: with no compile in its closure it can never reach a group
      # it was staged into. Testing per edge instead admits mixed groups (the
      # mig-wrapper group is one), and a mixed universal dep cycles with its own
      # compile deps into the build-mig mega-SCC that swallows
      # duct-tape/bootstrap_cmds/darlingserver. Their headers go through
      # migByCompDir below instead, and undeclared link deps through cmdProducersOf.
      rawHeaderProducerGroups = filter (g:
        lib.all (i: !dependsOnCompileMemo.${toString i}) (rawIdsInGroup.${g} or [])
        && lib.any (i:
          !(isNoOp (elemAt edges i))
          && lib.any isHeaderPath (edgeOutputs (elemAt edges i)))
        (rawIdsInGroup.${g} or []))
      rawGroupIds;
      pureHdrGroupSet = listToAttrs (map (g: {
          name = g;
          value = true;
        })
        rawHeaderProducerGroups);
      # Header producers whose group did not qualify above (mixed or compile-
      # dependent, like the mig-wrapper group producing xnu/osfmk/mach/notify.h).
      # Consumers in the same source subtree still reach their headers through an
      # undeclared -I, so each is staged as a targeted dep of the compile groups
      # whose component directory owns the header path. Staging them universally
      # instead would make their groups universal deps and re-form the mega-SCC;
      # targeted stays a per-component DAG.
      migHeaderProducerIds = filter (i:
        !(isNoOp (elemAt edges i))
        && !(isCompile (elemAt edges i))
        && lib.any isHeaderPath (edgeOutputs (elemAt edges i))
        && !(pureHdrGroupSet ? ${rawGid i}))
      (indices edges);
      compDirToGids = builtins.groupBy (g: builtins.head (lib.splitString "::" g)) rawGroupIds;
      # The longest ancestor directory of a produced header that names an actual compile group's
      # component directory (the group that "owns" and -I-includes that subtree).
      ownerCompDir = p: let
        rel =
          if underAnyRoot p
          then relUnder p
          else p;
        dirs = lib.init (filter (x: x != "") (lib.splitString "/" rel));
        ancestors =
          lib.genList
          (n: builtins.concatStringsSep "/" (lib.take (length dirs - n) dirs))
          (length dirs);
      in
        lib.findFirst (a: compDirToGids ? ${a}) null ancestors;
      # When the source rewrite-root holds a real file at the same relative path,
      # that one is authoritative and the mounted srcHeaders base already serves
      # it: the hand-written xnu/osfmk/mach/notify.h defines MACH_NOTIFY_* and the
      # notify structs, while mig emits a different notify.h from notify.defs
      # without them. Targeting such a header would clobber the real one in the
      # consumer's sandbox and add a spurious producer -> consumer dep that, against
      # a real reverse link dep, is the duct-tape <-> darlingserver cycle.
      srcRoot =
        if rewriteRoots == []
        then null
        else builtins.head rewriteRoots;
      hasSourceVersion = o: let
        rel =
          if underAnyRoot o
          then relUnder o
          else o;
      in
        srcRoot != null && builtins.pathExists (srcRoot + "/" + rel);
      # componentDir -> [ producer rawGid ] : mig-header producers targeted at that component.
      migByCompDir =
        builtins.mapAttrs (_cd: lst: fastUniq (map (x: x.gid) lst))
        (builtins.groupBy (x: x.compDir)
          (concatMap (i: let
            prodGid = rawGid i;
            owners = fastUniq (filter (x: x != null)
              (map ownerCompDir
                (filter (o: isHeaderPath o && !(hasSourceVersion o)) (edgeOutputs (elemAt edges i)))));
          in
            map (cd: {
              compDir = cd;
              gid = prodGid;
            })
            owners)
          migHeaderProducerIds));
      # g -> h when an edge in g consumes an output an edge in h produces, plus the
      # two undeclared-header routes above.
      rawGroupDeps = listToAttrs (map (g: {
          name = g;
          value =
            filter (h: h != g)
            (fastUniq ((map rawGid (concatMap (i:
                (concatMap realProducers (edgeInputs (elemAt edges i))) ++ cmdProducersOf i)
              rawIdsInGroup.${g}))
              ++ rawHeaderProducerGroups
              ++ (migByCompDir.${builtins.head (lib.splitString "::" g)} or [])));
        })
        rawGroupIds);
      # A group derivation depends on its external groups' derivations, so the
      # group graph must be acyclic or eval recurses forever - and a path-based
      # grouping does produce cycles, two CMake targets linking each other being
      # routine in libc/libsystem. Merging each strongly-connected set into one
      # derivation fixes that, since a condensation is always a DAG.
      #
      # Only a group on a cycle can merge, and all-pairs reachability over the 1e3
      # groups is O(G * reachSize^2), so peel the DAG fringes first: drop any live
      # group with no live successor or no live predecessor, to a fixpoint. What
      # remains is exactly the union of the non-trivial SCCs. The predecessor test
      # reads the live nodes' successor lists, so no predecessor map is built, and
      # alive' is a subset each round, so comparing lengths is a valid fixpoint.
      cyclicSet = let
        toSet = xs:
          listToAttrs (map (x: {
              name = x;
              value = true;
            })
            xs);
        anyIn = set: xs: lib.any (x: set ? ${x}) xs;
        succ = rawGroupDeps;
        peel = aliveSet: let
          alive = builtins.attrNames aliveSet;
          liveTargetSet = toSet (concatMap (g: filter (h: aliveSet ? ${h}) (succ.${g} or [])) alive);
          alive' = filter (g: (liveTargetSet ? ${g}) && anyIn aliveSet (succ.${g} or [])) alive;
        in
          if length alive' == length alive
          then aliveSet
          else peel (toSet alive');
      in
        peel (toSet rawGroupIds);
      cyclic = builtins.attrNames cyclicSet;
      # Reachability within the residual only (successors restricted to cyclic). Any mutual-
      # reachability path between two SCC members stays inside the residual (a peeled
      # intermediate is acyclic, so cannot close a cycle), so this is exact for them. norm =
      # sorted-unique so list equality is order-independent (else the fixpoint oscillates).
      sccRep = let
        norm = xs: builtins.sort (a: b: a < b) (fastUniq xs);
        succ = rawGroupDeps;
        reachC = let
          seed = listToAttrs (map (g: {
              name = g;
              value = norm (filter (h: cyclicSet ? ${h}) (succ.${g} or []));
            })
            cyclic);
          expand = acc: let
            acc' = lib.mapAttrs (_g: rs: norm (rs ++ concatMap (h: acc.${h} or []) rs)) acc;
          in
            if acc' == acc
            then acc
            else expand acc';
        in
          expand seed;
        repOf = g:
          if !(cyclicSet ? ${g})
          then g
          else
            builtins.head (builtins.sort (a: b: a < b)
              ([g] ++ filter (h: h != g && elem h (reachC.${g} or []) && elem g (reachC.${h} or [])) cyclic));
      in
        listToAttrs (map (g: {
            name = g;
            value = repOf g;
          })
          rawGroupIds);
      # Effective grouping = raw grouping condensed through SCC representatives.
      gid = i: sccRep.${rawGid i};
      # Single-pass O(edges) grouping (see rawIdsInGroup) -- keys are exactly groupIds.
      idsInGroup = builtins.groupBy gid realIds;
      groupIds = builtins.attrNames idsInGroup;
      # Condensed groups that produce cross-cutting generated headers (rpc.h etc.), staged
      # into EVERY group. Group-independent, so computed once here -- not per mkGroup call.
      headerProducerReps = fastUniq (map (h: sccRep.${h}) rawHeaderProducerGroups);
      groupOfOutput = p: let
        ids = realProducers p;
      in
        if ids == []
        then null
        else gid (builtins.head ids);
      shebangSedG = p: ''
        if [ -f ${p} ] && [ "$(head -c2 ${p} 2>/dev/null)" = "#!" ]; then
          chmod u+w ${p} 2>/dev/null || true
          sed -i -e "1s|^#! *\(/usr\)\?/bin/bash|#!${pkgs.bash}/bin/bash|" \
                 -e "1s|^#! */usr/bin/env  *bash|#!${pkgs.bash}/bin/bash|" \
                 -e "1s|^#! */usr/bin/env  *|#!${pkgs.coreutils}/bin/env |" ${p}
        fi
      '';
      # One shared copy of the whole source tree filtered to headers, symlinks and
      # dir structure: the entire include namespace with the darling/include -> SDK
      # -> xnu shim maze intact. Every group mounts it and resolves its includes at
      # build time, which is what removes the per-edge -M scan and with it the
      # dominant first-build cost. Leaving the compiled sources out is what keeps
      # the base stable across source edits, so editing a .c rebuilds only its own
      # group; a header edit rehashes it, as it should.
      hdrExts = [
        "h"
        "hpp"
        "hh"
        "hxx"
        "h++"
        "inc"
        "def"
        "defs"
        "modulemap"
        "apinotes"
        "tbd"
        "pch"
        # libcxx inline-include fragments: exception.cpp/typeinfo.cpp `#include`
        # "support/runtime/*.ipp" (and support/atomic/*.ipp). Never compiled
        # directly, so they belong in the shared header base, not per-group srcs.
        "ipp"
        # export-list templates (security/OSX preprocesses Security.exp-in, which
        # #includes Security/SecExports.exp-in -> Security/SecPolicy.list): put the
        # .exp / .list export-list source family in the base.
        "exp"
        "exp-in"
        "list"
      ];
      srcExts = ["c" "cc" "cpp" "cxx" "c++" "m" "mm"];
      findNames = exts: lib.concatMapStringsSep " -o " (e: ''-name "*.${e}"'') exts;
      # A derivation, not builtins.path: builtins.path re-walks and re-hashes the
      # whole 4G source tree on every evaluation, minutes of eval-blocking work.
      # Input-addressed by the source store path, this is built once and reused
      # across every edit here that leaves the source alone, as a normal cached
      # graph node.
      srcHeaders =
        if rewriteRoots == []
        then null
        else
          pkgs.runCommand "darling-src-headers" {nativeBuildInputs = [pkgs.cpio];} ''
            mkdir -p "$out"
            # Both rewrite roots: the source tree for headers, sources, assembly and
            # the shim maze, and the configured build dir for cmake-generated headers.
            # Everything but the large framework binaries is kept. Size does not
            # matter per group, which mounts this as directory symlinks and never
            # copies it, and holding compiled sources back buys no isolation here
            # (the derivation is already input-addressed by the source tree) - while
            # keeping them covers the ones under symlink dirs, like
            # libsyscall/mach/mach_traps.S, that per-file staging cannot reach.
            # 1. Source tree (bulk cpio): headers + all sources + assembly + the shim maze.
            cd ${builtins.head rewriteRoots}
            {
              find . -type d
              find . -type l
              find . -type f \( ${findNames (hdrExts ++ srcExts ++ ["s" "S" "asm" "sub"])} \)
              find . -type f ! -name "*.*" \( -path "*/include/*" -o -path "*/Headers/*" \)
            } | LC_ALL=C sort -u | cpio -pdm --quiet "$out" 2>/dev/null || true
            # cpio preserves the read-only store mode on the copied dirs; make them
            # writable so the per-file ninjaRoot pass below can add generated headers.
            chmod -R u+w "$out" 2>/dev/null || true
            # 2. Configured build dir (ninjaRoot): cmake-generated headers (darling-config.h
            # from configure_file, mig/rpc outputs). These overlay onto dirs the source pass
            # may have created as SYMLINKS (darling has symlink dirs like src/include), which
            # makes a bulk cpio silently fail to write there -- so copy per file, de-symlinking
            # each parent first. The generated-header set is small, so per-file is cheap.
            ${lib.optionalString (length rewriteRoots > 1) ''
              cd ${builtins.elemAt rewriteRoots 1}
              find . -type f \( ${findNames hdrExts} \) 2>/dev/null | while IFS= read -r f; do
                rel=''${f#./}; d=$(dirname "$rel"); cur="$out"
                if [ "$d" != "." ]; then
                  oldIFS="$IFS"; IFS='/'; set -- $d; IFS="$oldIFS"
                  for comp in "$@"; do
                    cur="$cur/$comp"
                    if [ -L "$cur" ]; then t=$(readlink -f "$cur" 2>/dev/null); rm -f "$cur"; mkdir -p "$cur"; [ -n "$t" ] && [ -d "$t" ] && cp -rs "$t"/. "$cur"/ 2>/dev/null || true; else mkdir -p "$cur"; fi
                  done
                fi
                cp -Lf "$f" "$out/$rel" 2>/dev/null || true
              done''}
          '';
      mkGroup = g: let
        myIds = idsInGroup.${g};
        mySet = listToAttrs (map (i: {
            name = toString i;
            value = true;
          })
          myIds);
        allIns = concatMap (i: edgeInputs (elemAt edges i)) myIds;
        # The undeclared tool-generated headers this group's compiles reach through
        # an -I. Keyed off member component dirs so the targeting survives SCC
        # merging, then mapped through sccRep like the reps below.
        migGids = map (h: sccRep.${h}) (fastUniq (concatMap
          (cd: migByCompDir.${cd} or [])
          (fastUniq (map (i: builtins.head (lib.splitString "::" (rawGid i))) myIds))));
        # Producers of this group's inputs that live in another condensed group,
        # plus the cross-cutting header-producer groups. Deduped at the group level:
        # lib.unique on the raw producer-edge list is O(n^2) in this group's total
        # inputs and dominated eval at libSystem scale. Mapping to gid first also
        # collapses same-group producers to g, which `h != g` then drops.
        extGids =
          filter (h: h != g)
          (fastUniq ((map gid (concatMap realProducers allIns))
            ++ (map gid (concatMap cmdProducersOf myIds))
            ++ headerProducerReps
            ++ migGids));
        extGroupDrvs = map (h: groupDrvs.${h}) extGids;
        relSrcs =
          fastUniq (filter (r: safeNotSymlink (src + "/${r}"))
            (concatMap (i: concatMap realSources (edgeInputs (elemAt edges i))) myIds));
        # No per-edge scan here: headers come from the mounted srcHeaders base at
        # build time, so this stages only each edge's declared under-root inputs
        # plus the under-root files its command names (a linker's alias list, a
        # custom command's template).
        rootSrcs = fastUniq (concatMap (i: let
          e = elemAt edges i;
        in
          (filter (p: underAnyRoot p && safeNotSymlink p) (edgeInputs e))
          ++ (filter (p: underAnyRoot p && safeRegular p)
            (concatMap (lib.splitString ",") (lib.splitString " " e.command))))
        myIds);
        # The group's internal edges in topological order, producers first. It is
        # SCC-condensed and so acyclic; the ready == [] arm is a defensive
        # residue-dump, not an expected path.
        topo = let
          # Once per edge, not recomputed on every layer.
          intDepsOf = listToAttrs (map (i: {
              name = toString i;
              value =
                filter (j: mySet ? ${toString j})
                (concatMap realProducers (edgeInputs (elemAt edges i)));
            })
            myIds);
          # Kahn by layers. The residue filter tests attrset membership rather than
          # `lib.elem i batch`, which made the whole sort O(edges^2) and was the
          # dominant per-group eval cost at libSystem scale. Layers accumulate and
          # concat once.
          go = remaining: doneSet: acc:
            if remaining == []
            then acc
            else let
              ready = filter (i: lib.all (d: doneSet ? ${toString d}) intDepsOf.${toString i}) remaining;
              batch =
                if ready == []
                then remaining
                else ready;
              batchSet = listToAttrs (map (i: {
                  name = toString i;
                  value = true;
                })
                batch);
            in
              go (filter (i: !(batchSet ? ${toString i})) remaining) (doneSet // batchSet) (acc ++ [batch]);
        in
          lib.concatLists (go myIds {} []);
        relOf = o:
          if underAnyRoot o
          then relUnder o
          else o;
        # Run one internal edge DIRECTLY (not via ninja): stripRoots gives
        # $out-absolute paths (cd-immune, and $out is the shell env var here --
        # no ninja `$out` variable to collide with). Reuses mkEdge's command
        # construction; internal generated headers are already produced by earlier
        # edges in this topo order, so no generated-header -I bridging is needed.
        runEdge = i: let
          e = elemAt edges i;
          outs = edgeOutputs e;
          rsp = e.rspfile or null;
          cmd = let
            stripped =
              if rewriteRoots == []
              then e.command
              else stripRoots e.command;
            withSubs = builtins.replaceStrings (map (s: s.from) subs) (map (s: s.to) subs) stripped;
            base = builtins.replaceStrings (map (s: s.from) toolPathSubs) (map (s: s.to) toolPathSubs) withSubs;
            # Append this compile's OWN-MODULE $out-relative generated-header -I dirs so a
            # `<generated.h>` reached via a cmake target-include (not a literal -I / declared
            # input) resolves from where topo/external staging materialised it in $out --
            # module-scoped so unrelated generated dirs cannot shadow standard headers.
          in
            base + lib.optionalString (isCompile e) (" " + lib.concatStringsSep " " (genIncsOutFor i));
        in ''
          # Subshell resetting to $out: edges run sequentially in one shell, and a
          # compile command that `cd`s into its WORKING_DIRECTORY (and does not
          # return) would otherwise leave the next edge -- e.g. a link with a
          # relative -o and no cd of its own -- writing to a doubled path.
          ( cd "$out"
          ${lib.concatMapStringsSep "\n" (o: ''
            realize_writable "$(dirname ${esc (relOf o)})"
            if [ -L ${esc (relOf o)} ]; then rm -f ${esc (relOf o)}; fi'')
          outs}
          ${lib.optionalString (rsp != null && rsp != "") ''
            mkdir -p "$(dirname ${esc rsp})"
            printf '%s' ${esc (e.rspfile_content or "")} > ${esc rsp}''}
          ${cmd}
          ${lib.concatMapStringsSep "\n" (o: shebangSedG ''"$out/${relOf o}"'') outs}
          ${lib.optionalString (rsp != null && rsp != "") "rm -f ${esc rsp}"} )
        '';
      in
        pkgs.runCommand "ninja-group-${lib.strings.sanitizeDerivationName g}" {
          nativeBuildInputs = toolchain ++ extraInputs;
          preferLocalBuild = true;
          passthru = {
            groupId = g;
            edgeIndices = myIds;
          };
        } ''
          mkdir -p $out; cd $out
          realize_writable() {
            local p="$1" cur="" comp tgt oldIFS="$IFS"
            IFS='/'; set -- $p; IFS="$oldIFS"
            for comp in "$@"; do
              [ -z "$comp" ] && continue
              if [ -z "$cur" ]; then cur="$comp"; else cur="$cur/$comp"; fi
              if [ -L "$cur" ]; then
                # Lazy one-level de-symlink: replace the dir symlink with a real dir
                # whose entries are symlinks to the target's DIRECT children (not a deep
                # cp of the whole subtree). Writing deeper just recurses this, so a group
                # only ever materialises real dirs along the exact paths it writes.
                tgt="$(readlink -f "$cur" 2>/dev/null || true)"; rm -f "$cur"; mkdir -p "$cur"
                if [ -n "$tgt" ] && [ -d "$tgt" ]; then
                  find "$tgt" -mindepth 1 -maxdepth 1 2>/dev/null | while IFS= read -r e; do
                    if [ ! -e "$cur/''${e##*/}" ]; then ln -s "$e" "$cur/''${e##*/}" 2>/dev/null || true; fi
                  done
                fi
              else mkdir -p "$cur"; fi
            done
          }
          # The shared source-header namespace goes in as a handful of top-level
          # directory symlinks rather than a 1.3GB deep cp -rs: a compile's
          # `-I $out/src/.../include` resolves straight through into the base, and
          # realize_writable materialises real dirs only along paths this group
          # writes.
          ${lib.optionalString (srcHeaders != null) ''
            find ${srcHeaders} -mindepth 1 -maxdepth 1 2>/dev/null | while IFS= read -r e; do
              if [ ! -e "''${e##*/}" ]; then ln -s "$e" "''${e##*/}" 2>/dev/null || true; fi
            done''}
          ${lib.concatMapStringsSep "\n" (d: ''
            (cd ${d} && find . -mindepth 1 -type d 2>/dev/null) | while IFS= read -r sub; do
              s=''${sub#./}
              if [ -L "$s" ] || { [ -e "$s" ] && [ ! -d "$s" ]; }; then realize_writable "$s"; fi
            done
            cp -rsf --no-preserve=mode ${d}/. ./ 2>/dev/null || true'')
          extGroupDrvs}
          ${lib.concatMapStringsSep "\n" (s: ''
            if [ ! -e ${esc s} ] || [ -L ${esc s} ]; then realize_writable "$(dirname ${esc s})"; rm -f ${esc s}; install -Dm644 ${srcStorePath s} ${esc s}; if [ -x ${srcStorePath s} ]; then chmod +x ${esc s}; ${shebangSedG (esc s)} fi; fi'')
          relSrcs}
          ${lib.concatMapStringsSep "\n" (p: ''
            if [ ! -e ${esc (relUnder p)} ] || [ -L ${esc (relUnder p)} ]; then realize_writable "$(dirname ${esc (relUnder p)})"; rm -f ${esc (relUnder p)}; install -Dm644 ${indivOf p} ${esc (relUnder p)}; if [ -x ${indivOf p} ]; then chmod +x ${esc (relUnder p)}; ${shebangSedG (esc (relUnder p))} fi; fi'')
          rootSrcs}
          # No -I staging or broken-symlink prune here, unlike mkEdge: srcHeaders
          # already provides every source header and the intact shim maze through
          # the lazy symlink tree, and copying over that would clobber it.
          ${lib.concatMapStringsSep "\n" runEdge topo}
        '';
      # The build-time variant of mkGroup: eval computes only this group's edge list
      # and external-group drvs, and lower_group.py does the per-edge rewrite, stage
      # and run inside the sandbox. The dep wiring is the same, since referencing
      # rewriteRoots / srcHeaders / extGroupDrvs is what makes Nix mount them.
      mkGroupViaTool = g: let
        myIds = idsInGroup.${g};
        allIns = concatMap (i: edgeInputs (elemAt edges i)) myIds;
        migGids = map (h: sccRep.${h}) (fastUniq (concatMap
          (cd: migByCompDir.${cd} or [])
          (fastUniq (map (i: builtins.head (lib.splitString "::" (rawGid i))) myIds))));
        extGids =
          filter (h: h != g)
          (fastUniq ((map gid (concatMap realProducers allIns))
            ++ (map gid (concatMap cmdProducersOf myIds))
            ++ headerProducerReps
            ++ migGids));
        extGroupDrvs = map (h: groupDrvs.${h}) extGids;
      in
        pkgs.runCommand "ninja-group-${lib.strings.sanitizeDerivationName g}" {
          nativeBuildInputs = toolchain ++ extraInputs ++ [pkgs.python3];
          preferLocalBuild = true;
          passthru = {
            groupId = g;
            edgeIndices = myIds;
          };
        } ''
          ${pkgs.python3}/bin/python3 ${./lower_group.py} \
            --graph ${graphDrv} \
            --edges ${esc (lib.concatStringsSep "," (map toString myIds))} \
            --out $out \
            --bash-path ${pkgs.bash}/bin/bash --env-path ${pkgs.coreutils}/bin/env \
            ${lib.concatMapStringsSep " " (r: "--rewrite-root ${r}") rewriteRoots} \
            ${lib.optionalString (srcHeaders != null) "--src-headers ${srcHeaders}"} \
            ${lib.concatMapStringsSep " " (d: "--ext-dir ${d}") extGroupDrvs} \
            ${lib.concatMapStringsSep " " (s: "--toolsub ${esc s.from}=${esc s.to}") toolPathSubs}
        '';
      groupDrvs = listToAttrs (map (g: {
          name = g;
          value =
            (
              if buildTimeLowering
              then mkGroupViaTool
              else mkGroup
            )
            g;
        })
        groupIds);
      groupDrvForOutput = p: groupDrvs.${groupOfOutput p};
      # realOutputsForTarget against group derivations, so buildOne can materialize
      # a whole-graph build from groups: each final output's group and its
      # dependency groups build transitively, and copying them all gives the tree.
      realOutputsForTargetG = p:
        lib.concatMap
        (i:
          map (o: {
            path = o;
            drv = groupDrvForOutput o;
          }) (edgeOutputs (elemAt edges i)))
        (realProducers p);
      # DEBUG: forces the whole SCC condensation (peel + reachC) but NOT the per-group
      # derivation construction, to isolate where eval time goes.
      groupStats = {
        nEdges = length realIds;
        nRaw = length rawGroupIds;
        nDepEdges = lib.foldl' (a: g: a + length (rawGroupDeps.${g} or [])) 0 rawGroupIds;
        nCyclic = length cyclic;
        nGroups = length groupIds;
      };
    in {inherit groupDrvs groupDrvForOutput idsInGroup realOutputsForTargetG groupStats;};
  in {
    inherit producerOf edgeDrvs drvForOutput edges;
    inherit isPhonyTarget realOutputsForTarget lowerGroupsBy;
    inherit (graph) defaults;
  };
}
