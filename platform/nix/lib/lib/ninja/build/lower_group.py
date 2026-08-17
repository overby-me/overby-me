#!/usr/bin/env python3
"""Build-time grouped-lowering for nix-ninja (task #80).

Nix eval now computes only each group's { edge index list, external-group dirs,
source tree, config }. THIS tool does the per-edge work that used to run in Nix
eval (and cost ~15-40 min on the whole-Darling graph): it reads the shared
graph.json, filters to this group's edges, rewrites their commands (absolute
store-path roots -> $out-relative; host tool paths -> PATH-relative), computes
the $out-relative -I dirs for generated headers, topologically orders the edges,
stages the sources / external-group outputs / shared source-header base, and
runs each edge. Mirrors the logic that used to live in lower.nix `mkGroup`.

Invoked as:
  lower_group.py --graph GRAPH.json --edges 1,2,3 --out $out \
     --rewrite-root PATH [--rewrite-root PATH ...] \
     --src-tree PATH --src-headers PATH \
     --ext-dir PATH [--ext-dir PATH ...] \
     --toolsub FROM=TO [...]
"""
import argparse, json, os, re, shutil, subprocess, sys
from pathlib import Path


def edge_outputs(e):
    return list(e.get("outputs", [])) + list(e.get("implicit_outputs", []))


def edge_inputs(e):
    return (list(e.get("inputs", []))
            + list(e.get("implicit_inputs", []))
            + list(e.get("order_only_inputs", [])))


def is_noop(e):
    # phony or an empty/':'-style command: produces nothing itself.
    c = (e.get("command") or "").strip()
    return e.get("rule") == "phony" or c in ("", ":", "true")


CMAKE_HOUSEKEEPING_RE = re.compile(
    r"cmake_install\.cmake|--regenerate-during-build|--check-build-system"
    r"|cmake_check_build_system|-E cmake_echo_color|(^|\s)(ctest|cpack)(\s|$)")


def is_cmake_housekeeping(e):
    """CMake's built-in utility targets (rebuild_cache, edit_cache, install, test,
    package, ...) re-run cmake / run cmake_install.cmake / ctest / cpack against paths
    absent in the per-group sandbox and are not real build steps. componentGrouping can
    pull them into a real group; skip them. First-party custom targets (build-mig, the
    codegen) run real commands and are kept."""
    for o in edge_outputs(e):
        if re.search(r"CMakeFiles/(rebuild_cache|edit_cache)", o):
            return True
    return bool(CMAKE_HOUSEKEEPING_RE.search(e.get("command") or ""))


HDR_EXTS = {"h", "hpp", "hh", "hxx", "h++", "inc", "def", "defs", "modulemap",
            "apinotes", "tbd", "pch"}


def is_header(p):
    ext = p.rsplit(".", 1)[-1] if "." in p else ""
    return ext in HDR_EXTS


def module_key(p):
    cs = [c for c in p.split("/") if c]
    n = len(cs)
    if n >= 3 and cs[0] == "src" and cs[1] == "external":
        return "src/external/" + cs[2]
    if n >= 2 and cs[0] == "src":
        return "src/" + cs[1]
    return cs[0] if n >= 1 else ""


class Graph:
    def __init__(self, edges, roots):
        self.edges = edges
        self.roots = [r.rstrip("/") for r in roots]
        # output path -> producing edge index
        self.producer_of = {}
        for i, e in enumerate(edges):
            for o in edge_outputs(e):
                self.producer_of.setdefault(o, i)
        self._rp_memo = {}
        # module -> set of ancestor dirs of generated headers in that module
        self.gen_hdr_dirs = {}
        for i, e in enumerate(edges):
            if is_noop(e):
                continue
            for o in edge_outputs(e):
                if not is_header(o):
                    continue
                rel = self.rel_under(o) if self.under_root(o) else o
                mod = module_key(rel)
                self.gen_hdr_dirs.setdefault(mod, set()).update(self.ancestor_dirs(rel))

    # ---- path rewriting -------------------------------------------------
    def root_for(self, p):
        for r in self.roots:
            if p.startswith(r + "/"):
                return r
        return None

    def under_root(self, p):
        return self.root_for(p) is not None

    def rel_under(self, p):
        r = self.root_for(p)
        return p[len(r) + 1:] if r else p

    @staticmethod
    def ancestor_dirs(rel):
        dirs = [d for d in rel.split("/") if d][:-1]  # drop filename
        return set("/".join(dirs[:n]) for n in range(len(dirs) + 1))

    # ---- real producers (flatten no-op alias chains), memoised ----------
    def real_producers(self, p):
        if p not in self.producer_of:
            return []
        if p in self._rp_memo:
            return self._rp_memo[p]
        self._rp_memo[p] = []  # guard against pathological cycles
        i = self.producer_of[p]
        e = self.edges[i]
        if is_noop(e):
            out = []
            seen = set()
            for inp in edge_inputs(e):
                for j in self.real_producers(inp):
                    if j not in seen:
                        seen.add(j)
                        out.append(j)
        else:
            out = [i]
        self._rp_memo[p] = out
        return out


def topo(group_ids, g):
    """Kahn by layers over intra-group producer deps (producers first)."""
    idset = set(group_ids)
    intdeps = {}
    for i in group_ids:
        deps = set()
        for inp in edge_inputs(g.edges[i]):
            for j in g.real_producers(inp):
                if j in idset:
                    deps.add(j)
        intdeps[i] = deps
    # Undeclared generated-header ordering: a compile edge may #include a mig/codegen header
    # via -I with NO declared ninja dep (e.g. duct-tape's ipc_kobject.c includes the mig-
    # generated mach/notify.h). The declared-dep topo above would then run the compile before
    # the generator, so those symbols are undeclared at compile time. Add a precise ordering
    # edge j -> i whenever compile i has a -I dir that CONTAINS one of non-compile generator
    # j's header outputs. This cannot cycle the migcom chain (the consumer depends on the
    # header, never the reverse); any residual cycle is broken by Kahn's defensive dump below.
    dir_to_gens = {}
    for j in group_ids:
        ej = g.edges[j]
        if is_compile(ej) or is_noop(ej):
            continue
        for o in edge_outputs(ej):
            if not is_header(o):
                continue
            rel = g.rel_under(o) if g.under_root(o) else o
            d = os.path.dirname(rel)
            while d and d != ".":
                dir_to_gens.setdefault(d, set()).add(j)
                nd = os.path.dirname(d)
                if nd == d:
                    break
                d = nd
    if dir_to_gens:
        for i in group_ids:
            ei = g.edges[i]
            if not is_compile(ei):
                continue
            for tok in re.findall(r'-I\s*(\S+)', ei.get("command") or ""):
                d = g.rel_under(tok) if g.under_root(tok) else tok
                for j in dir_to_gens.get(d, ()):
                    if j != i:
                        intdeps[i].add(j)
    # Order producers before consumers. The group may be a real SCC (cycles), so condense the
    # intra-group dep graph into strongly-connected components with Tarjan and emit them in the
    # algorithm's natural post-order (dependencies first). Acyclic producer->consumer pairs that
    # merely RIDE ON a cycle stay correctly ordered; only a genuine cycle is emitted as an
    # (internally arbitrary) block. A plain Kahn with a list-order or min-dep fallback mis-orders
    # such pairs -- e.g. libc's dylib link (edge index 2054) precedes the notify_firstpass dylib
    # it links (edge index 4162) by index order while both ride the libSystem-umbrella SCC.
    deps_list = {i: list(intdeps[i]) for i in group_ids}
    idx = {}
    low = {}
    on_stack = set()
    tstack = []
    order = []
    ctr = 0
    for start in group_ids:
        if start in idx:
            continue
        work = [(start, 0)]
        while work:
            node, ci = work[-1]
            if ci == 0:
                idx[node] = low[node] = ctr
                ctr += 1
                tstack.append(node)
                on_stack.add(node)
            recurse = False
            children = deps_list[node]
            k = ci
            while k < len(children):
                w = children[k]
                if w not in idx:
                    work[-1] = (node, k + 1)
                    work.append((w, 0))
                    recurse = True
                    break
                if w in on_stack:
                    low[node] = min(low[node], idx[w])
                k += 1
            if recurse:
                continue
            if low[node] == idx[node]:
                while True:
                    w = tstack.pop()
                    on_stack.discard(w)
                    order.append(w)
                    if w == node:
                        break
            work.pop()
            if work:
                p = work[-1][0]
                low[p] = min(low[p], low[node])
    return order


def gen_incs_out(i, g):
    """$out-relative -I dirs for generated headers reachable from edge i."""
    e = g.edges[i]
    outs = edge_outputs(e)
    mod = ""
    if outs:
        o0 = outs[0]
        mod = module_key(g.rel_under(o0) if g.under_root(o0) else o0)
    dirs = set(g.gen_hdr_dirs.get(mod, set()))
    # plus header-output dirs of this compile's declared producers
    prod = set()
    for inp in edge_inputs(e):
        prod.update(g.real_producers(inp))
    for j in prod:
        for o in edge_outputs(g.edges[j]):
            if is_header(o):
                rel = g.rel_under(o) if g.under_root(o) else o
                dirs.update(g.ancestor_dirs(rel))
    flags = []
    for d in sorted(dirs):
        flags.append("-I$out" + ("/" + d if d and d != "." else ""))
    return flags


def is_compile(e):
    c = e.get("command") or ""
    # crude: a compile produces an object and runs the C/C++ frontend with -c
    return " -c " in c and any(o.endswith(".o") for o in edge_outputs(e))


def rewrite_command(cmd, g, toolsubs):
    # absolute rewrite-root paths -> $out-relative (longer form first)
    for r in g.roots:
        cmd = cmd.replace(r + "/", "$out/").replace(r, "$out")
    for frm, to in toolsubs:
        cmd = cmd.replace(frm, to)
    return cmd


def realize_writable(rel):
    """Make every path component of dir `rel` a real writable dir. Mirrors lower.nix
    realize_writable: a dir SYMLINK (into the read-only srcHeaders / ext-dir store base)
    is replaced by a real dir whose entries symlink to the target's DIRECT children, so
    writes land in a real tree without a deep copy. Idempotent."""
    cur = ""
    for comp in (c for c in rel.split("/") if c):
        cur = comp if not cur else cur + "/" + comp
        if os.path.islink(cur):
            tgt = os.path.realpath(cur)
            os.remove(cur)
            os.mkdir(cur)
            if os.path.isdir(tgt):
                for e in os.listdir(tgt):
                    dst = os.path.join(cur, e)
                    if not os.path.lexists(dst):
                        os.symlink(os.path.join(tgt, e), dst)
        elif not os.path.exists(cur):
            os.mkdir(cur)


SHEBANG_BASH_RE = re.compile(r"^#!\s*(/usr)?/bin/bash")
SHEBANG_ENV_BASH_RE = re.compile(r"^#!\s*/usr/bin/env\s+bash")
SHEBANG_ENV_RE = re.compile(r"^#!\s*/usr/bin/env\s+")

BASH_PATH = ""   # set from --bash-path
ENV_PATH = ""    # set from --env-path (a nix coreutils .../bin/env)
TOOLSUBS = []    # [(from,to)] host-tool-path -> PATH-relative subs, set from --toolsub


def fix_shebang(path):
    """Rewrite a script so it runs in the pure sandbox: fix a /usr/bin/env | /bin/bash
    shebang (line 1), AND rewrite hardcoded host tool paths (/bin/rmdir, /usr/bin/mkdir, ...)
    in the BODY -- generated scripts like mig's `build-mig` bake those into their body, not
    just the command line. Mirrors lower.nix shebangSedG + toolPathSubs, for script files."""
    try:
        with open(path, "rb") as f:
            if f.read(2) != b"#!":
                return
    except OSError:
        return
    with open(path, "r", errors="surrogateescape") as f:
        text = f.read()
    orig = text
    nl = text.find("\n")
    first, rest = (text[:nl], text[nl:]) if nl >= 0 else (text, "")
    if SHEBANG_BASH_RE.match(first):
        first = "#!" + BASH_PATH + first[SHEBANG_BASH_RE.match(first).end():]
    elif SHEBANG_ENV_BASH_RE.match(first):
        first = "#!" + BASH_PATH
    elif SHEBANG_ENV_RE.match(first):
        first = "#!" + ENV_PATH + " " + first[SHEBANG_ENV_RE.match(first).end():]
    # Toolsubs apply to the BODY only -- NOT the rewritten shebang, whose nix interpreter
    # path (/nix/store/...-coreutils/bin/env) contains "/bin/env"-like substrings a toolsub
    # would corrupt.
    for frm, to in TOOLSUBS:
        rest = rest.replace(frm, to)
    text = first + rest
    if text != orig:
        os.chmod(path, os.stat(path).st_mode | 0o200)
        with open(path, "w", errors="surrogateescape") as f:
            f.write(text)


def stage_file(srcpath, rel):
    """Copy srcpath -> rel (build-dir-relative), de-symlinking parents first and making
    the result writable (store sources are read-only). Fix script shebangs."""
    realize_writable(os.path.dirname(rel))
    if os.path.lexists(rel):
        os.remove(rel)
    shutil.copy(srcpath, rel)
    st = os.stat(srcpath)
    os.chmod(rel, (st.st_mode | 0o200) & 0o777)
    if st.st_mode & 0o111:
        fix_shebang(rel)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--graph", required=True)
    ap.add_argument("--edges", required=True)
    ap.add_argument("--out", required=True)
    ap.add_argument("--rewrite-root", action="append", default=[])
    ap.add_argument("--src-tree", default="")
    ap.add_argument("--src-headers", default="")
    ap.add_argument("--ext-dir", action="append", default=[])
    ap.add_argument("--toolsub", action="append", default=[])
    ap.add_argument("--bash-path", default="")
    ap.add_argument("--env-path", default="")
    a = ap.parse_args()
    global BASH_PATH, ENV_PATH, TOOLSUBS
    BASH_PATH, ENV_PATH = a.bash_path, a.env_path
    TOOLSUBS = [s.split("=", 1) for s in a.toolsub]

    graph = json.load(open(a.graph))
    g = Graph(graph["edges"], a.rewrite_root)
    ids = [int(x) for x in a.edges.split(",") if x != ""]
    ids = [i for i in ids if not is_cmake_housekeeping(g.edges[i])]
    toolsubs = [s.split("=", 1) for s in a.toolsub]

    out = a.out
    os.makedirs(out, exist_ok=True)
    os.chdir(out)

    # 1) shared source-header base: top-level directory symlinks (lazy tree)
    if a.src_headers:
        for entry in os.listdir(a.src_headers):
            if not os.path.lexists(entry):
                os.symlink(os.path.join(a.src_headers, entry), entry)

    # 2) external dependency groups: symlink their REAL produced files (dylibs / objects /
    #    generated headers) into $out. Skip their srcHeaders / shim SYMLINKS -- those are
    #    redundant here (we mount srcHeaders ourselves) and a blind cp -rsf of them collides
    #    with our realized dirs ("cannot overwrite directory with non-directory"). os.walk
    #    with followlinks=False won't descend the ext group's srcHeaders symlink dirs, so it
    #    yields exactly the paths that group actually wrote.
    for d in a.ext_dir:
        for root, _dirs, files in os.walk(d):
            rel = os.path.relpath(root, d)
            rel = "" if rel == "." else rel
            for name in files:
                src = os.path.join(root, name)
                if os.path.islink(src):
                    continue
                dst = os.path.join(rel, name) if rel else name
                realize_writable(os.path.dirname(dst))
                if os.path.lexists(dst):
                    if os.path.isdir(dst) and not os.path.islink(dst):
                        continue  # never clobber a real dir with a file
                    os.remove(dst)
                try:
                    os.symlink(src, dst)
                except OSError:
                    pass

    # 3) this group's under-root source inputs -- both declared edge inputs AND under-root
    #    files NAMED IN THE COMMAND (mig.awk, a linker alias list, a codegen template) that
    #    the graph does not list as inputs. Copied from the rewrite root that contains them
    #    (the group mounts those roots). Produced paths are built by an edge, not staged;
    #    non-symlink regular files only. Mirrors mkGroup relSrcs + rootSrcs.
    staged = set()

    def stage_candidate(p):
        if p in g.producer_of:
            return
        r = g.root_for(p)
        if r is None:
            return  # absolute store/tool path: Nix mounts it in place
        rel = g.rel_under(p)
        if rel in staged:
            return
        staged.add(rel)
        srcpath = os.path.join(r, rel)
        if os.path.isfile(srcpath) and not os.path.islink(srcpath):
            stage_file(srcpath, rel)

    for i in ids:
        e = g.edges[i]
        for inp in edge_inputs(e):
            stage_candidate(inp)
        for tok in re.split(r"[\s,]+", e.get("command") or ""):
            if tok:
                stage_candidate(tok)

    # 4) run each edge in topological order
    for i in topo(ids, g):
        e = g.edges[i]
        if is_noop(e):
            continue
        for o in edge_outputs(e):
            rel = g.rel_under(o) if g.under_root(o) else o
            realize_writable(os.path.dirname(rel))
            if os.path.islink(rel):
                os.remove(rel)
        # response file (link commands): write rewritten content, then remove after
        rsp = e.get("rspfile") or ""
        rsp_rel = None
        if rsp:
            rsp_rel = g.rel_under(rsp) if g.under_root(rsp) else rsp
            realize_writable(os.path.dirname(rsp_rel))
            with open(rsp_rel, "w", errors="surrogateescape") as f:
                f.write(rewrite_command(e.get("rspfile_content") or "", g, toolsubs))
        cmd = rewrite_command(e["command"], g, toolsubs)
        if is_compile(e):
            cmd = cmd + " " + " ".join(gen_incs_out(i, g))
        rc = subprocess.run(["bash", "-c", cmd], cwd=out).returncode
        if rsp_rel and os.path.lexists(rsp_rel):
            os.remove(rsp_rel)
        if rc != 0:
            print(f"lower_group: edge {i} failed (rc={rc}): "
                  f"{edge_outputs(e)}", file=sys.stderr)
            sys.exit(rc)
        # a GENERATED script output (e.g. mig's `build-mig`) run by a later edge needs its
        # /bin/bash | /usr/bin/env shebang rewritten too (mkGroup shebangSedG on outputs).
        for o in edge_outputs(e):
            orel = g.rel_under(o) if g.under_root(o) else o
            if os.path.isfile(orel) and not os.path.islink(orel):
                fix_shebang(orel)
        # Source-backed generated header: when a hand-written source header of the SAME rel path
        # exists in the source root, it is authoritative and must win. mach/notify.h is the case:
        # the source header defines MACH_NOTIFY_* and the notify message structs, while mig
        # re-emits a notify.h from notify.defs that has neither. The real build keeps the source
        # and build copies at distinct absolute roots so each includer resolves the right one
        # (kernel sources via source-first -I; the mig .c files via a same-dir quote include that
        # only needs the structs, which are in source too). Our single merged $out cannot hold
        # both, and mig's copy would shadow the source and break every <mach/notify.h> consumer.
        # So restore the SOURCE copy over any source-backed header this edge just produced; the
        # -I-containment ordering in topo() guarantees this runs before the consuming compiles.
        if g.roots:
            for o in edge_outputs(e):
                if not is_header(o):
                    continue
                orel = g.rel_under(o) if g.under_root(o) else o
                srcv = os.path.join(g.roots[0], orel)
                if os.path.isfile(srcv) and not os.path.islink(srcv):
                    realize_writable(os.path.dirname(orel))
                    if os.path.lexists(orel):
                        os.remove(orel)
                    shutil.copy(srcv, orel)


if __name__ == "__main__":
    main()
