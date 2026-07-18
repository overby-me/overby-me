# Build-time manifest planning: read a (published, normalized) Cargo.toml and
# emit the facts needed to invoke rustc. Runs inside the sandbox, so reading
# the manifest here is what keeps evaluation free of IFD (see PLAN.md).
#
# Only used for registry crates. Workspace members get their plan computed at
# eval time from the local manifest (which may use workspace inheritance that
# published crates never contain).
import json
import os
import sys
import tomllib


def snake(name: str) -> str:
    return name.replace("-", "_")


def plan_from_manifest(root: str) -> dict:
    with open(os.path.join(root, "Cargo.toml"), "rb") as f:
        m = tomllib.load(f)
    pkg = m["package"]

    lib = m.get("lib")
    has_auto_lib = os.path.exists(os.path.join(root, "src", "lib.rs"))
    if lib is None and not has_auto_lib:
        lib_plan = None
    else:
        lib = lib or {}
        proc_macro = lib.get("proc-macro", lib.get("proc_macro", False))
        crate_types = lib.get(
            "crate-type",
            lib.get("crate_type", ["proc-macro"] if proc_macro else ["lib"]),
        )
        lib_plan = {
            "name": lib.get("name", snake(pkg["name"])),
            "path": lib.get("path", "src/lib.rs"),
            "procMacro": proc_macro,
            "crateTypes": crate_types,
        }

    build = pkg.get("build")
    if build is None and os.path.exists(os.path.join(root, "build.rs")):
        build = "build.rs"
    if build is False:
        build = None

    authors = pkg.get("authors", [])
    return {
        "name": pkg["name"],
        "version": pkg["version"],
        "edition": str(pkg.get("edition", "2015")),
        "links": pkg.get("links"),
        "description": pkg.get("description") or "",
        "license": pkg.get("license") or "",
        "repository": pkg.get("repository") or "",
        "authors": authors,
        "rustVersion": str(pkg.get("rust-version", "")),
        "lib": lib_plan,
        "build": build,
        "bins": [],
    }


if __name__ == "__main__":
    json.dump(plan_from_manifest(sys.argv[1] if len(sys.argv) > 1 else "."), sys.stdout)
