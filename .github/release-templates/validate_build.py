#!/usr/bin/env python3
"""Resolve and validate the source-controlled build identity for GTT releases.

The workspace version in the root Cargo.toml (`[workspace.package] version`)
is the single source of truth. Channel rules:

- stable: `--release-tag vX.Y.Z` must equal the workspace version exactly.
- alpha:  `--release-tag vX.Y.Z-alpha.N` where X.Y.Z is the workspace version
          (alphas ship the in-development base before it becomes stable).
          An empty tag is allowed for build-only verification runs and emits
          no `release_tag` output.
"""

from __future__ import annotations

import argparse
import re
import sys
from pathlib import Path

STABLE_TAG = re.compile(r"^v(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)$")
ALPHA_TAG = re.compile(r"^v(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)-alpha\.([1-9]\d*)$")


def workspace_version(cargo_toml: Path) -> str:
    text = cargo_toml.read_text(encoding="utf-8")
    match = re.search(
        r"(?ms)^\[workspace\.package\].*?^version\s*=\s*\"([^\"]+)\"",
        text,
    )
    if match is None:
        raise SystemExit(f"workspace version not found in {cargo_toml}")
    return match.group(1)


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--cargo-toml", required=True, type=Path)
    parser.add_argument("--channel", required=True, choices=("alpha", "stable"))
    parser.add_argument("--release-tag", default="")
    parser.add_argument("--github-output", type=Path)
    args = parser.parse_args()

    version = workspace_version(args.cargo_toml)
    tag = args.release_tag.strip()
    outputs: dict[str, str] = {
        "base_version": version,
        "version_name": version,
    }

    if args.channel == "stable":
        if not tag:
            raise SystemExit("stable builds require --release-tag")
        if STABLE_TAG.fullmatch(tag) is None:
            raise SystemExit(f"stable tag {tag!r} must look like vX.Y.Z")
        if tag != f"v{version}":
            raise SystemExit(
                f"stable tag {tag} does not match workspace version v{version} — "
                "bump Cargo.toml first, then release"
            )
        outputs["release_tag"] = tag
    else:
        if not tag:
            # Push-triggered verification builds have no publish intent; emit the
            # identity without a tag so the pipeline can still resolve outputs.
            outputs.pop("release_tag", None)
        else:
            match = ALPHA_TAG.fullmatch(tag)
            if match is None:
                raise SystemExit(f"alpha tag {tag!r} must look like vX.Y.Z-alpha.N")
            base = ".".join(match.groups()[:3])
            if base != version:
                raise SystemExit(
                    f"alpha tag base v{base} does not match workspace version v{version}"
                )
            outputs["release_tag"] = tag

    outputs["asset_filename"] = (
        f"GlobalTokenTracker-Setup-{version}-win-x64.exe"
    )
    for key, value in outputs.items():
        print(f"{key}={value}")
    if args.github_output is not None:
        with args.github_output.open("a", encoding="utf-8") as handle:
            for key, value in outputs.items():
                handle.write(f"{key}={value}\n")


if __name__ == "__main__":
    main()
