#!/usr/bin/env python3
from __future__ import annotations

import argparse
import re
import subprocess
import sys
from pathlib import Path


VERSION_TAG_RE = re.compile(r"^\d+(?:\.\d+){1,3}$")


def to_crate_version(version: str) -> str:
    parts = version.split(".")
    if len(parts) <= 3:
        return version

    base = ".".join(parts[:3])
    suffix = ".".join(parts[3:])
    return f"{base}-patch.{suffix}"


def git_show(repo: Path, ref: str, file_path: str) -> str:
    return subprocess.check_output(
        ["git", "-C", str(repo), "show", f"{ref}:{file_path}"],
        text=True,
        stderr=subprocess.STDOUT,
    )


def git_rev_parse(repo: Path, ref: str) -> str:
    return (
        subprocess.check_output(["git", "-C", str(repo), "rev-parse", ref], text=True)
        .strip()
    )


def parse_version(header: str) -> tuple[int, int, int, int]:
    patterns = {
        "major": re.compile(r"^#define\\s+V8_MAJOR_VERSION\\s+(\\d+)", re.MULTILINE),
        "minor": re.compile(r"^#define\\s+V8_MINOR_VERSION\\s+(\\d+)", re.MULTILINE),
        "build": re.compile(r"^#define\\s+V8_BUILD_NUMBER\\s+(\\d+)", re.MULTILINE),
        "patch": re.compile(r"^#define\\s+V8_PATCH_LEVEL\\s+(\\d+)", re.MULTILINE),
    }

    try:
        major = int(patterns["major"].search(header).group(1))
        minor = int(patterns["minor"].search(header).group(1))
        build = int(patterns["build"].search(header).group(1))
        patch = int(patterns["patch"].search(header).group(1))
    except (AttributeError, ValueError) as exc:
        raise RuntimeError("Unable to parse V8 version macros from header") from exc

    return major, minor, build, patch


def determine_latest_tag(repo: Path, pattern: str | None) -> tuple[str, str]:
    cmd = ["git", "-C", str(repo), "tag", "--list", "--sort=-v:refname"]
    if pattern:
        cmd.append(pattern)

    output = subprocess.check_output(cmd, text=True)
    for raw_tag in output.splitlines():
        tag = raw_tag.strip()
        if not tag:
            continue
        if pattern is None and not VERSION_TAG_RE.fullmatch(tag):
            continue
        commit = git_rev_parse(repo, tag)
        return tag, commit

    raise RuntimeError("Unable to determine latest V8 tag")


def main() -> None:
    parser = argparse.ArgumentParser(description="Extract V8 version information")
    parser.add_argument("--repo", required=True, type=Path, help="Path to local V8 repository")
    parser.add_argument("--ref", default="HEAD", help="Git ref to inspect when reading headers")
    parser.add_argument(
        "--mode",
        choices=("header", "latest-tag"),
        default="header",
        help="How to determine the V8 version",
    )
    parser.add_argument(
        "--tag-pattern",
        help="Optional glob passed to `git tag --list` when --mode latest-tag is used",
    )
    parser.add_argument(
        "--output",
        type=Path,
        help="Optional path (e.g. $GITHUB_OUTPUT) to append GitHub Actions outputs",
    )
    args = parser.parse_args()

    if args.mode == "header":
        header = git_show(args.repo, args.ref, "include/v8-version.h")
        major, minor, build, patch = parse_version(header)
        version = f"{major}.{minor}.{build}.{patch}"
        commit = git_rev_parse(args.repo, args.ref)
    else:
        version, commit = determine_latest_tag(args.repo, args.tag_pattern)

    print(version)

    crate_version = to_crate_version(version)

    if args.output is not None:
        with args.output.open("a", encoding="utf-8") as handle:
            handle.write(f"version={version}\n")
            handle.write(f"commit={commit}\n")
            handle.write(f"crate_version={crate_version}\n")


if __name__ == "__main__":
    try:
        main()
    except subprocess.CalledProcessError as exc:
        sys.stderr.write(exc.output if exc.output else str(exc))
        sys.exit(exc.returncode)
    except Exception as exc:  # pragma: no cover - best effort logging
        sys.stderr.write(f"Error: {exc}\n")
        sys.exit(1)
