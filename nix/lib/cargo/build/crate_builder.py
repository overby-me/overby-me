# Crate build driver: compiles one crate (lib and/or bins) with rustc
# directly, no cargo. Implements the cargo build-script protocol: compile
# build.rs with host deps, run it with the documented environment, parse
# cargo: directives, apply them to the library compile, and persist links
# metadata plus native link flags for dependents via $out/nix-support/.
#
# Config (JSON, path in argv[1]): see buildCrate.nix.
import importlib.util
import json
import os
import shutil
import subprocess
import sys
from typing import Any, Callable, NoReturn

Json = dict[str, Any]


def fail(msg: str) -> NoReturn:
    print(msg, file=sys.stderr)
    sys.exit(1)


def snake(name: str) -> str:
    return name.replace("-", "_")


def upper(name: str) -> str:
    return name.upper().replace("-", "_")


def require_env(name: str) -> str:
    value = os.environ.get(name)
    if not value:
        fail(f"required environment variable {name} is not set")
    return value


def load_plan_fn() -> Callable[[str], Json]:
    spec = importlib.util.spec_from_file_location(
        "manifest_plan", require_env("MANIFEST_PLAN_PY")
    )
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod.plan_from_manifest


def run(args: list[str], env: dict[str, str]) -> None:
    p = subprocess.run(args, env=env)
    if p.returncode != 0:
        fail(f"command failed ({p.returncode}): {' '.join(args)}")


def pkg_env(plan: Json, features: list[str]) -> dict[str, str]:
    version = plan["version"]
    core = version.split("+")[0]
    nums, _, pre = core.partition("-")
    parts = nums.split(".")
    env = {
        "CARGO_MANIFEST_DIR": os.getcwd(),
        "CARGO_MANIFEST_PATH": os.path.join(os.getcwd(), "Cargo.toml"),
        "CARGO_PKG_NAME": plan["name"],
        "CARGO_PKG_VERSION": version,
        "CARGO_PKG_VERSION_MAJOR": parts[0],
        "CARGO_PKG_VERSION_MINOR": parts[1] if len(parts) > 1 else "0",
        "CARGO_PKG_VERSION_PATCH": parts[2] if len(parts) > 2 else "0",
        "CARGO_PKG_VERSION_PRE": pre,
        "CARGO_PKG_AUTHORS": ":".join(plan.get("authors", [])),
        "CARGO_PKG_DESCRIPTION": plan.get("description", ""),
        "CARGO_PKG_HOMEPAGE": "",
        "CARGO_PKG_REPOSITORY": plan.get("repository", ""),
        "CARGO_PKG_LICENSE": plan.get("license", ""),
        "CARGO_PKG_LICENSE_FILE": "",
        "CARGO_PKG_README": "",
        "CARGO_PKG_RUST_VERSION": plan.get("rustVersion", ""),
    }
    for f in features:
        env[f"CARGO_FEATURE_{upper(f)}"] = "1"
    if plan.get("links"):
        env["CARGO_MANIFEST_LINKS"] = plan["links"]
    return env


def rustc_cfg_env(rustc: str) -> dict[str, str]:
    out = subprocess.run(
        [rustc, "--print", "cfg"], capture_output=True, text=True, check=True
    ).stdout
    cfgs: dict[str, list[str]] = {}
    for line in out.splitlines():
        if "=" in line:
            k, _, v = line.partition("=")
            cfgs.setdefault(k, []).append(v.strip('"'))
        elif line:
            cfgs.setdefault(line, [])
    return {f"CARGO_CFG_{upper(k)}": ",".join(v) for k, v in cfgs.items()}


def extern_args(externs: list[Json]) -> list[str]:
    args: list[str] = []
    for e in externs:
        marker = os.path.join(e["out"], "nix-support", "extern")
        if not os.path.exists(marker):
            fail(f"dependency {e['name']} ({e['out']}) has no lib artifact")
        with open(marker) as f:
            path = f.read().strip()
        args += ["--extern", f"{snake(e['name'])}={path}"]
    return args


def dep_dir_args(dep_outs: list[str]) -> list[str]:
    return [arg for o in dep_outs for arg in ("-L", f"dependency={o}/lib")]


def feature_cfg_args(features: list[str]) -> list[str]:
    return [arg for f in features for arg in ("--cfg", f'feature="{f}"')]


class BuildScriptOutput:
    def __init__(self) -> None:
        self.cfgs: list[str] = []
        self.envs: list[tuple[str, str]] = []
        self.link_libs: list[str] = []
        self.link_search: list[str] = []
        self.metadata: list[tuple[str, str]] = []


def parse_rustc_flags(val: str, out: BuildScriptOutput) -> None:
    toks = val.split()
    i = 0
    while i < len(toks):
        if toks[i] in ("-l", "-L") and i + 1 < len(toks):
            dest = out.link_libs if toks[i] == "-l" else out.link_search
            dest.append(toks[i + 1])
            i += 2
        elif toks[i].startswith(("-l", "-L")):
            dest = out.link_libs if toks[i].startswith("-l") else out.link_search
            dest.append(toks[i][2:])
            i += 1
        else:
            fail(f"unsupported rustc-flags token: {toks[i]}")


def parse_directives(stdout: str) -> BuildScriptOutput:
    out = BuildScriptOutput()
    for line in stdout.splitlines():
        if line.startswith("cargo::"):
            content, modern = line[len("cargo::") :], True
        elif line.startswith("cargo:"):
            content, modern = line[len("cargo:") :], False
        else:
            continue
        key, _, val = content.partition("=")
        if key == "rustc-cfg":
            out.cfgs.append(val)
        elif key == "rustc-env":
            k, _, v = val.partition("=")
            out.envs.append((k, v))
        elif key == "rustc-link-lib":
            out.link_libs.append(val)
        elif key == "rustc-link-search":
            out.link_search.append(val)
        elif key == "rustc-flags":
            parse_rustc_flags(val, out)
        elif key == "metadata":
            k, _, v = val.partition("=")
            out.metadata.append((k, v))
        elif key == "warning":
            print(f"build script warning: {val}", file=sys.stderr)
        elif key == "error":
            fail(f"build script error: {val}")
        elif key.startswith("rerun-if") or key in (
            "rustc-check-cfg",
            "rustc-cdylib-link-arg",
        ):
            pass
        elif not modern:
            # Legacy links metadata: cargo:KEY=VALUE
            out.metadata.append((key, val))
        else:
            fail(f"unknown cargo:: directive: {line}")
    return out


def dep_links_env(links_deps: list[str]) -> dict[str, str]:
    env: dict[str, str] = {}
    for dep in links_deps:
        path = os.path.join(dep, "nix-support", "cargo-links")
        if not os.path.exists(path):
            continue
        with open(path) as fh:
            links = fh.readline().strip()
            for kv in fh:
                k, _, v = kv.rstrip("\n").partition("=")
                if k:
                    env[f"DEP_{upper(links)}_{upper(k)}"] = v
    return env


def run_build_script(
    cfg: Json, plan: Json, rustc: str, base_env: dict[str, str]
) -> tuple[BuildScriptOutput, str]:
    bs_dir = os.path.abspath(".build-script")
    out_dir = os.path.abspath(".build-out")
    os.makedirs(bs_dir, exist_ok=True)
    os.makedirs(out_dir, exist_ok=True)

    compile_args = (
        [
            rustc,
            plan["build"],
            "--crate-name",
            "build_script_build",
            "--crate-type",
            "bin",
            "--edition",
            plan["edition"],
            "-C",
            "opt-level=0",
            "--out-dir",
            bs_dir,
        ]
        + (["--cap-lints", "allow"] if cfg["capLints"] else [])
        + feature_cfg_args(cfg["features"])
        + extern_args(cfg["buildExterns"])
        + dep_dir_args(cfg["buildDepOuts"])
    )
    run(compile_args, env={**os.environ, **base_env})

    env = {
        **os.environ,
        **base_env,
        **rustc_cfg_env(rustc),
        **dep_links_env(cfg["linksDeps"]),
        "OUT_DIR": out_dir,
        "TARGET": cfg["target"],
        "HOST": cfg["target"],
        "NUM_JOBS": str(os.cpu_count() or 1),
        "OPT_LEVEL": cfg["profile"]["optLevel"],
        "PROFILE": "debug" if cfg["profile"]["debug"] else "release",
        "DEBUG": "true" if cfg["profile"]["debug"] else "false",
        "RUSTC": rustc,
        "RUSTDOC": "rustdoc",
        "CARGO": shutil.which("cargo") or "cargo",
        "CARGO_ENCODED_RUSTFLAGS": "",
    }
    p = subprocess.run(
        [os.path.join(bs_dir, "build_script_build")],
        env=env,
        stdout=subprocess.PIPE,
        text=True,
    )
    if p.returncode != 0:
        sys.stderr.write(p.stdout)
        fail(f"build script failed ({p.returncode})")
    return parse_directives(p.stdout), out_dir


def rewrite_out_dir(path: str, out_dir: str | None, out: str) -> str:
    if out_dir is not None and path.startswith(out_dir):
        return f"{out}/out" + path[len(out_dir) :]
    return path


def native_link_args(bs: BuildScriptOutput | None) -> list[str]:
    if bs is None:
        return []
    args = [arg for s in bs.link_search for arg in ("-L", s)]
    args += [arg for lib in bs.link_libs for arg in ("-l", lib)]
    return args


def collect_transitive_native(dep_outs: list[str]) -> list[str]:
    args: list[str] = []
    for o in dep_outs:
        path = os.path.join(o, "nix-support", "rustc-link")
        if not os.path.exists(path):
            continue
        with open(path) as fh:
            for raw in fh:
                line = raw.strip()
                if line:
                    flag, _, val = line.partition(" ")
                    args += [flag, val]
    return args


def profile_args(profile: Json) -> list[str]:
    args = ["-C", f"opt-level={profile['optLevel']}"]
    if profile["debug"]:
        args += ["-C", "debuginfo=2"]
    return args


def compile_lib(
    plan: Json,
    rustc: str,
    base_env: dict[str, str],
    common_args: list[str],
    bs: BuildScriptOutput | None,
    crate_hash: str,
    out: str,
) -> str:
    lib = plan["lib"]
    types = [t for t in lib["crateTypes"] if t in ("lib", "rlib", "proc-macro")] or [
        "lib"
    ]
    os.makedirs(f"{out}/lib", exist_ok=True)
    args = (
        [
            rustc,
            lib["path"],
            "--crate-name",
            lib["name"],
            "--crate-type",
            ",".join(types),
            "--edition",
            plan["edition"],
            "-C",
            f"metadata={crate_hash}",
            "-C",
            f"extra-filename=-{crate_hash}",
            "--out-dir",
            f"{out}/lib",
        ]
        + common_args
        + native_link_args(bs)
    )
    run(args, env={**os.environ, **base_env, "CARGO_CRATE_NAME": lib["name"]})
    produced = sorted(os.listdir(f"{out}/lib"))
    rlibs = [f for f in produced if f.endswith(".rlib")]
    dylibs = [f for f in produced if f.endswith((".so", ".dylib"))]
    artifact = f"{out}/lib/{(rlibs + dylibs)[0]}"
    with open(f"{out}/nix-support/extern", "w") as f:
        f.write(artifact)
    return artifact


def compile_bins(
    cfg: Json,
    plan: Json,
    rustc: str,
    base_env: dict[str, str],
    common_args: list[str],
    bs: BuildScriptOutput | None,
    lib_artifact: str | None,
    out: str,
) -> None:
    bins = cfg.get("bins") or plan.get("bins") or []
    os.makedirs(f"{out}/bin", exist_ok=True)
    bin_common = (
        common_args
        + (
            ["--extern", f"{plan['lib']['name']}={lib_artifact}"]
            if lib_artifact
            else []
        )
        + native_link_args(bs)
        + collect_transitive_native(cfg["depOuts"])
    )
    bin_env = {**os.environ, **base_env}
    for b in bins:
        crate_name = snake(b["name"])
        bin_env["CARGO_CRATE_NAME"] = crate_name
        run(
            [
                rustc,
                b["path"],
                "--crate-name",
                crate_name,
                "--crate-type",
                "bin",
                "--edition",
                plan["edition"],
                "-o",
                os.path.join(out, "bin", b["name"]),
            ]
            + bin_common,
            env=bin_env,
        )


def persist_build_script(
    plan: Json, bs: BuildScriptOutput, bs_out_dir: str, out: str
) -> None:
    search = [rewrite_out_dir(s, bs_out_dir, out) for s in bs.link_search]
    needs_out = any(s.startswith(f"{out}/out") for s in search) or any(
        v.startswith(bs_out_dir) for _, v in bs.metadata
    )
    if needs_out or os.listdir(bs_out_dir):
        shutil.copytree(bs_out_dir, f"{out}/out", dirs_exist_ok=True)
    if plan.get("links"):
        lines = [plan["links"]] + [
            f"{k}={rewrite_out_dir(v, bs_out_dir, out)}" for k, v in bs.metadata
        ]
        with open(f"{out}/nix-support/cargo-links", "w") as f:
            f.write("\n".join(lines) + "\n")
    link_lines = [f"-L {s}" for s in search] + [f"-l {lib}" for lib in bs.link_libs]
    if link_lines:
        with open(f"{out}/nix-support/rustc-link", "w") as f:
            f.write("\n".join(link_lines) + "\n")


def main() -> None:
    with open(sys.argv[1]) as f:
        cfg: Json = json.load(f)
    out = require_env("out")
    rustc = shutil.which("rustc")
    if rustc is None:
        fail("rustc not found in PATH")
    plan = cfg["plan"] or load_plan_fn()(".")
    base_env = pkg_env(plan, cfg["features"])

    os.makedirs(f"{out}/nix-support", exist_ok=True)

    bs: BuildScriptOutput | None = None
    bs_out_dir: str | None = None
    if plan.get("build"):
        bs, bs_out_dir = run_build_script(cfg, plan, rustc, base_env)
        base_env["OUT_DIR"] = bs_out_dir
        base_env.update(bs.envs)

    common_args = (
        profile_args(cfg["profile"])
        + (["--cap-lints", "allow"] if cfg["capLints"] else [])
        + feature_cfg_args(cfg["features"])
        + [arg for c in (bs.cfgs if bs else []) for arg in ("--cfg", c)]
        + extern_args(cfg["externs"])
        + dep_dir_args(cfg["depOuts"])
    )

    lib_artifact = None
    if plan.get("lib"):
        lib_artifact = compile_lib(
            plan, rustc, base_env, common_args, bs, cfg["crateHash"], out
        )

    if cfg["buildBins"]:
        compile_bins(cfg, plan, rustc, base_env, common_args, bs, lib_artifact, out)

    if bs is not None and bs_out_dir is not None:
        persist_build_script(plan, bs, bs_out_dir, out)


if __name__ == "__main__":
    main()
