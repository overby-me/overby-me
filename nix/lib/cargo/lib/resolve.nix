# Dependency graph construction and feature resolution. Pure builtins.
#
# Joins the lock (exact versions), the index (dependency metadata), and
# workspace manifests into a build graph, then computes enabled features by
# monotone fixpoint.
#
# Stage 1 semantics (see PLAN.md): one unified feature set per package,
# resolver-v1-like. Build/normal deps share the feature space; host/target
# splitting (resolver v2) is a later milestone. This over-approximates: it
# can enable a feature or optional dep that cargo's v2 resolver would not,
# which is correctness-preserving in the same way cargo's own v1 was.
let
  inherit
    (builtins)
    attrNames
    concatLists
    concatStringsSep
    elem
    filter
    foldl'
    hasAttr
    head
    listToAttrs
    mapAttrs
    match
    sort
    ;
  cfgLib = import ./cfg.nix;
  lockLib = import ./lock.nix;
  indexLib = import ./index.nix;

  # Feature table entry classification:
  #   "dep:R"  -> enable optional dep R
  #   "R?/F"   -> if dep R is enabled (by other means), add feature F to it
  #   "R/F"    -> enable dep R (if optional) and add feature F to it
  #   "F"      -> plain feature, or (legacy) optional dep name
  classifyEntry = entry: let
    mDep = match "dep:(.*)" entry;
    mWeak = match "([^?/]+)\\?/(.*)" entry;
    mStrong = match "([^?/]+)/(.*)" entry;
  in
    if mDep != null
    then {
      t = "dep";
      dep = head mDep;
    }
    else if mWeak != null
    then {
      t = "weak";
      dep = head mWeak;
      feature = builtins.elemAt mWeak 1;
    }
    else if mStrong != null
    then {
      t = "strong";
      dep = head mStrong;
      feature = builtins.elemAt mStrong 1;
    }
    else {
      t = "plain";
      feature = entry;
    };

  # Effective feature table: declared features plus the implicit feature for
  # every optional dependency that is never referenced as "dep:name".
  effectiveFeatures = meta: let
    optionalNames = map (d: d.name) (filter (d: d.optional) meta.deps);
    entries = concatLists (map (f: meta.features.${f}) (attrNames meta.features));
    depReferenced =
      map (e: e.dep)
      (filter (e: e.t == "dep") (map classifyEntry entries));
    implicit =
      filter (
        n:
          !(elem n depReferenced) && !(hasAttr n meta.features)
      )
      optionalNames;
  in
    meta.features
    // listToAttrs (map (n: {
        name = n;
        value = ["dep:${n}"];
      })
      implicit);

  resolve = {
    lock, # lockLib.parseLock result
    indexDir,
    platform, # cfgLib platform attrset
    workspace, # manifestLib.loadWorkspace result (or compatible)
    roots, # list of workspace member package names
    rootFeatures ? [],
    noDefaultFeatures ? false,
    includeDev ? false,
  }: let
    # Metadata per lock package id (lazy; only forced for active nodes).
    metas =
      mapAttrs (
        id: pkg:
          if pkg.sourceInfo.type == "path"
          then workspace.byName.${pkg.name}
            or (throw "cargo resolve: ${pkg.name} is a path package but not a workspace member (path deps outside the workspace are not supported yet)")
          else if pkg.sourceInfo.type == "registry"
          then
            if !pkg.sourceInfo.cratesIo
            then throw "cargo resolve: alternative registries are not supported yet (${pkg.name})"
            else let
              e = indexLib.lookup indexDir pkg.name pkg.version;
            in
              if e.cksum != pkg.checksum
              then throw "cargo resolve: checksum mismatch for ${id}: lock ${pkg.checksum} vs index ${e.cksum} (stale index snapshot?)"
              else e
          else throw "cargo resolve: git dependencies are not supported yet (${pkg.name})"
      )
      lock.byId;

    effTables = mapAttrs (_id: effectiveFeatures) metas;

    isMember = id: lock.byId.${id}.sourceInfo.type == "path";
    rootIds =
      map (
        name: let
          m =
            workspace.byName.${name}
          or (throw "cargo resolve: root ${name} is not a workspace member");
        in "${name}-${m.version}"
      )
      roots;

    # Dependency records that apply on this platform (kind and cfg
    # filtering); optional-dep activation happens in the fixpoint.
    wantedOf =
      mapAttrs (
        id: _pkg:
          filter (
            d:
              (
                d.kind
                == "normal"
                || d.kind == "build"
                || (d.kind == "dev" && includeDev && elem id rootIds)
              )
              && (d.target == null || cfgLib.matchesTarget platform d.target)
              && d.registry == null
          )
          metas.${id}.deps
      )
      lock.byId;

    # Renames of platform-active deps: feature entries referencing a dep
    # that is filtered out for this platform must be ignored entirely
    # (resolver v2 semantics; e.g. tokio's "windows-sys/..." on linux).
    activeRenamesOf =
      mapAttrs (
        _id: deps:
          foldl' (acc: d: acc // {${d.name} = true;}) {} deps
      )
      wantedOf;

    # Active dependency records with resolved target ids. A dropped
    # optional dep that the lock does not even contain is fine; a missing
    # non-optional dep is an error.
    edgesOf =
      mapAttrs (
        id: _pkg:
          concatLists (map (
              d: let
                targetId = lockLib.findDep lock lock.resolvedDeps.${id} d.package d.req;
              in
                if targetId != null
                then [
                  {
                    dep = d;
                    inherit targetId;
                  }
                ]
                else if d.optional || d.kind == "dev"
                then []
                else throw "cargo resolve: ${id} dependency ${d.package} (req ${toString d.req}) not found in lock"
            )
            wantedOf.${id})
      )
      lock.byId;

    # Fixpoint state: id -> {
    #   features  = { name = true; }        enabled features of this package
    #   depOn     = { rename = true; }      activated optional deps
    #   depFeat   = { rename = { f = true; }; }   strong payloads
    #   depFeatWeak = same shape            weak payloads
    # }
    emptyNode = {
      features = {};
      depOn = {};
      depFeat = {};
      depFeatWeak = {};
    };

    mergeNode = a: b: {
      features = a.features // b.features;
      depOn = a.depOn // b.depOn;
      depFeat =
        a.depFeat
        // mapAttrs (n: v: (a.depFeat.${n} or {}) // v) b.depFeat;
      depFeatWeak =
        a.depFeatWeak
        // mapAttrs (n: v: (a.depFeatWeak.${n} or {}) // v) b.depFeatWeak;
    };

    mergeInto = state: id: contrib:
      state
      // {
        ${id} = mergeNode (state.${id} or emptyNode) contrib;
      };

    setOf = names:
      listToAttrs (map (n: {
          name = n;
          value = true;
        })
        names);

    # Filter a feature set against a node's table: unknown "default" is
    # dropped (a crate without a default feature is fine), anything else
    # unknown is an error, matching cargo.
    checkedFeatures = id: names: let
      t = effTables.${id};
      unknown = filter (f: !(hasAttr f t) && f != "default") names;
    in
      if unknown != []
      then throw "cargo resolve: package ${id} has no feature(s): ${concatStringsSep ", " unknown}"
      else setOf (filter (f: hasAttr f t) names);

    # Contributions of one node to the next state.
    nodeContribs = state: id: let
      st = state.${id};
      t = effTables.${id};
      isOptionalDep = n: elem n (map (d: d.name) (filter (d: d.optional) metas.${id}.deps));

      # Within-node: process entries of currently enabled features.
      entryContribs = concatLists (map (
        f: let
          entries = t.${f} or [];
        in
          map (
            entry: let
              c = classifyEntry entry;
              # Entries referencing a dep that is target-filtered on this
              # platform are ignored entirely (resolver v2 semantics).
              depActive = activeRenamesOf.${id} ? ${c.dep or ""};
            in
              if c.t == "dep"
              then
                (
                  if depActive
                  then {depOn = setOf [c.dep];}
                  else {}
                )
              else if c.t == "strong"
              then
                (
                  if depActive
                  then {
                    depOn = setOf [c.dep];
                    depFeat = {${c.dep} = setOf [c.feature];};
                    # "R/F" also enables the feature R itself when R is a
                    # feature (implicit or explicit); this is what weak
                    # "R?/F" deliberately does not do. Crates referenced
                    # only via dep:R have no such feature, so nothing is
                    # added.
                    features = setOf (filter (n: hasAttr n t) [c.dep]);
                  }
                  else {}
                )
              else if c.t == "weak"
              then
                (
                  if depActive
                  then {depFeatWeak = {${c.dep} = setOf [c.feature];};}
                  else {}
                )
              else if hasAttr c.feature t
              then {features = setOf [c.feature];}
              else if isOptionalDep c.feature
              then
                (
                  if activeRenamesOf.${id} ? ${c.feature}
                  then {depOn = setOf [c.feature];}
                  else {}
                )
              else throw "cargo resolve: package ${id} feature entry ${entry} names neither a feature nor an optional dependency"
          )
          entries
      ) (attrNames st.features));

      selfContrib =
        map (c: {
          inherit id;
          contrib = emptyNode // c;
        })
        entryContribs;

      # Edges: activate children and push feature payloads.
      childContribs = concatLists (map (
          e: let
            r = e.dep.name;
            enabled = !e.dep.optional || (st.depOn ? ${r});
          in
            if !enabled
            then []
            else let
              payload =
                (
                  if e.dep.defaultFeatures
                  then ["default"]
                  else []
                )
                ++ e.dep.features
                ++ attrNames (st.depFeat.${r} or {})
                ++ attrNames (st.depFeatWeak.${r} or {});
            in [
              {
                id = e.targetId;
                contrib =
                  emptyNode
                  // {
                    features = checkedFeatures e.targetId payload;
                  };
              }
            ]
        )
        edgesOf.${id});
    in
      selfContrib ++ childContribs;

    step = state: let
      contribs = concatLists (map (nodeContribs state) (attrNames state));
    in
      foldl' (s: c: mergeInto s c.id c.contrib) state contribs;

    fix = state: let
      next = step state;
    in
      if next == state
      then state
      else fix next;

    # Root seeds tolerate features that a given member does not define:
    # with several roots (cargo build --features at a virtual workspace
    # root), a feature may exist on only some members.
    seed =
      foldl' (
        s: id:
          mergeInto s id (emptyNode
            // {
              features = setOf (filter (f: hasAttr f effTables.${id}) (
                rootFeatures
                ++ (
                  if noDefaultFeatures
                  then []
                  else ["default"]
                )
              ));
            })
      ) {}
      rootIds;

    final = fix seed;

    # Assemble output nodes: only active packages, with sorted feature lists
    # and deduplicated active edges.
    nodes =
      mapAttrs (
        id: st: let
          activeEdges =
            filter (
              e:
                !e.dep.optional || (st.depOn ? ${e.dep.name})
            )
            edgesOf.${id};
        in {
          pkg = lock.byId.${id};
          meta = metas.${id};
          isWorkspaceMember = isMember id;
          features = sort (a: b: a < b) (attrNames st.features);
          edges =
            map (e: {
              name = e.dep.name;
              package = e.dep.package;
              kind = e.dep.kind;
              optional = e.dep.optional;
              inherit (e) targetId;
            })
            activeEdges;
        }
      )
      final;
  in {
    inherit nodes rootIds;
  };
in {
  inherit resolve classifyEntry effectiveFeatures;
}
