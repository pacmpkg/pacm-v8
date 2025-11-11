#!/usr/bin/env python3
from __future__ import annotations

import os
import sys
import subprocess
import shlex
import shutil
import tarfile
import platform
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
DEPS = ROOT / "third_party"
DEPOT_TOOLS = DEPS / "depot_tools"
V8_DIR = DEPS / "v8"

def run(cmd, cwd=None, env=None):
    print(">", " ".join(cmd))
    exec_cmd = cmd
    if os.name == "nt" and isinstance(cmd, (list, tuple)) and cmd:
        # Resolve batch files on Windows because CreateProcess cannot execute them directly.
        search_path = env.get("PATH") if env else None
        resolved = shutil.which(cmd[0], path=search_path)
        if resolved is None:
            raise FileNotFoundError(f"Command {cmd[0]!r} not found in PATH")
        exec_cmd = [resolved, *cmd[1:]]
    subprocess.check_call(exec_cmd, cwd=cwd, env=env)

def ensure_submodules():
    if not DEPOT_TOOLS.exists() or not V8_DIR.exists():
        print("Please initialize submodules: git submodule update --init --recursive")
        sys.exit(1)


def infer_default_target_triple(gn_target: str) -> str:
    host = platform.system().lower()
    target_lower = gn_target.lower()

    def has_any(*needles):
        return any(n in target_lower for n in needles)

    if host == "windows":
        if has_any("arm64", "arm"):
            return "aarch64-pc-windows-msvc"
        if has_any("x86", "ia32") and not has_any("64"):
            return "i686-pc-windows-msvc"
        return "x86_64-pc-windows-msvc"
    if host == "darwin":
        if has_any("arm64", "m1", "m2", "m3", "mac-arm"):
            return "aarch64-apple-darwin"
        return "x86_64-apple-darwin"
    # Default to Linux triples for other hosts.
    if has_any("arm64", "arm"):
        return "aarch64-unknown-linux-gnu"
    if has_any("x86", "ia32") and not has_any("64"):
        return "i686-unknown-linux-gnu"
    return "x86_64-unknown-linux-gnu"

def find_librarian(env: dict) -> Path | None:
    for candidate in ("lib.exe", "llvm-lib.exe"):
        path = shutil.which(candidate, path=env.get("PATH"))
        if path:
            return Path(path)

    fallback = V8_DIR / "third_party" / "llvm-build" / "Release+Asserts" / "bin" / "llvm-lib.exe"
    if fallback.exists():
        return fallback
    return None

def bundle_libcxx(outdir: Path, env: dict) -> Path | None:
    obj_dir = outdir / "obj" / "buildtools" / "third_party" / "libc++" / "libc++"
    if not obj_dir.exists():
        return None

    objects = sorted(obj_dir.glob("*.obj"))
    if not objects:
        return None

    librarian = find_librarian(env)
    if librarian is None:
        print("Warning: Could not find lib.exe or llvm-lib.exe; libc++ will not be bundled.")
        return None

    target = obj_dir.parent / "v8_libcxx.lib"
    rsp_path = obj_dir.parent / "libcxx.rsp"

    with open(rsp_path, "w", encoding="utf-8", newline="") as rsp:
        rsp.write("/nologo\n")
        rsp.write(f"/OUT:\"{target}\"\n")
        for obj in objects:
            rsp.write(f"\"{obj}\"\n")

    run([str(librarian), f"@{rsp_path}"], cwd=outdir, env=env)

    try:
        rsp_path.unlink()
    except FileNotFoundError:
        pass

    return target if target.exists() else None

def build_v8(target="x64.release", revision: str | None = None):
    env = os.environ.copy()
    # Make depot_tools available in PATH
    env["PATH"] = str(DEPOT_TOOLS) + os.pathsep + env.get("PATH", "")
    # Avoid authenticated toolchain downloads; rely on locally installed VS toolchain instead.
    env.setdefault("DEPOT_TOOLS_WIN_TOOLCHAIN", "0")
    vpython = shutil.which("vpython3", path=env.get("PATH")) or shutil.which("vpython", path=env.get("PATH"))
    python_exe = vpython or sys.executable
    # sync (in case)
    sync_cmd = ["gclient", "sync"]
    if revision:
        run(["git", "fetch", "origin"], cwd=V8_DIR, env=env)
        sync_cmd.append(f"--revision=src/v8@{revision}")
    run(sync_cmd, cwd=V8_DIR, env=env)
    # generate build (ensure v8_monolithic is available)
    extra_args_raw = env.get("GN_ARGS", "")
    extra_args_list = shlex.split(extra_args_raw) if extra_args_raw else []

    def ensure_arg(prefix, value):
        key = f"{prefix}="
        if not any(arg.startswith(key) for arg in extra_args_list):
            extra_args_list.append(f"{key}{value}")

    ensure_arg("v8_monolithic", "true")
    ensure_arg("is_component_build", "false")
    ensure_arg("v8_use_external_startup_data", "false")
    ensure_arg("v8_enable_temporal_support", "false")
    ensure_arg("use_custom_libcxx", "false")
    ensure_arg("treat_warnings_as_errors", "false")
    ensure_arg("use_remoteexec", "false")
    ensure_arg("use_siso", "false")
    ensure_arg("use_goma", "false")
    ensure_arg("use_clang_modules", "false")
    ensure_arg(
        "extra_cflags_cc",
        "[\"-D_SILENCE_CXX20_OLD_SHARED_PTR_ATOMIC_SUPPORT_DEPRECATION_WARNING\"]"
    )

    cpu_prefix = target.split('.', 1)[0].lower()
    cpu_map = {
        "x64": "x64",
        "x86": "x86",
        "ia32": "x86",
        "arm64": "arm64",
    }
    cpu_value = cpu_map.get(cpu_prefix)
    if cpu_value:
        ensure_arg("target_cpu", f"\"{cpu_value}\"")
        ensure_arg("v8_target_cpu", f"\"{cpu_value}\"")

    if platform.system() == "Darwin":
        ensure_arg("use_system_xcode", "true")
        ensure_arg("mac_sdk_min", "\"14.0\"")
        ensure_arg("mac_deployment_target", "\"14.0\"")
        ensure_arg("mac_min_system_version", "\"14.0\"")

    env["GN_ARGS"] = " ".join(extra_args_list)

    builder = target
    if cpu_value == "x86":
        builder = "x64.release"

    v8gen_cmd = [python_exe, "tools/dev/v8gen.py", "gen", "-vv", "-b", builder, target]
    if extra_args_list:
        v8gen_cmd.append("--")
        v8gen_cmd.extend(extra_args_list)
    run(v8gen_cmd, cwd=V8_DIR, env=env)
    # ninja build
    outdir = V8_DIR / f"out.gn/{target}"
    run(["ninja", "-C", str(outdir), "v8_monolith"], cwd=V8_DIR, env=env)
    return outdir, env

def package(outdir, target_triple, env):
    # expected artifacts:
    # outdir/obj/libv8_monolith.a  (linux)
    lib_candidates = [
        outdir / "obj" / "libv8_monolith.a",
        outdir / "obj" / "v8_monolith.lib",
        outdir / "libv8_monolith.a"
    ]
    lib_path = None
    for p in lib_candidates:
        if p.exists():
            lib_path = p
            break
    if lib_path is None:
        raise RuntimeError("libv8_monolith not found in build output")
    artifacts = ROOT / "artifacts"
    artifacts.mkdir(exist_ok=True)
    pkg_dir = artifacts / f"v8-{target_triple}"
    if pkg_dir.exists():
        shutil.rmtree(pkg_dir)
    pkg_dir.mkdir(parents=True)
    # copy lib
    shutil.copy(lib_path, pkg_dir / lib_path.name)
    libcxx_lib = bundle_libcxx(outdir, env)
    extra_libs = [lib_path]
    if libcxx_lib is not None:
        extra_libs.append(libcxx_lib)

    lib_subdir = pkg_dir / "lib"
    lib_subdir.mkdir(exist_ok=True)
    for lib in extra_libs:
        shutil.copy(lib, lib_subdir / lib.name)
    icu_src = outdir / "icudtl.dat"
    if icu_src.exists():
        shutil.copy(icu_src, pkg_dir / "icudtl.dat")
    else:
        print("Warning: icudtl.dat was not found in the build output directory.")
    config_src = outdir / "v8_build_config.json"
    if config_src.exists():
        shutil.copy(config_src, pkg_dir / "v8_build_config.json")
    # copy include tree
    inc_src = V8_DIR / "include"
    if inc_src.exists():
        shutil.copytree(inc_src, pkg_dir / "include")
    else:
        raise RuntimeError("include/ not found in v8 checkout")
    # tar.gz
    tar_name = artifacts / f"v8-{target_triple}.tar.gz"
    with tarfile.open(tar_name, "w:gz") as tar:
        tar.add(pkg_dir, arcname=".")
    print("Created:", tar_name)
    return tar_name

def main():
    ensure_submodules()
    # target mapping: choose a GN build target name and target triple for packaging
    # For example: x64.release -> x86_64-unknown-linux-gnu
    gn_target = os.environ.get("GN_TARGET", "x64.release")
    target_triple = os.environ.get("TARGET_TRIPLE", infer_default_target_triple(gn_target))
    revision = os.environ.get("V8_GIT_REVISION")
    outdir, env = build_v8(gn_target, revision)
    pkg = package(outdir, target_triple, env)
    print("Package:", pkg)

if __name__ == "__main__":
    main()