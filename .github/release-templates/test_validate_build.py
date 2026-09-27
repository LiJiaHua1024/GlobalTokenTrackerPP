#!/usr/bin/env python3
"""Tests for validate_build.py (standard library only)."""

from __future__ import annotations

import contextlib
import io
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[2]
SCRIPT = REPO_ROOT / ".github" / "release-templates" / "validate_build.py"
CARGO = REPO_ROOT / "Cargo.toml"

FIXTURE = """\
[workspace]
members = ["crates/*"]

[workspace.package]
version = "1.4.2"
"""


def run_validate(*args: str) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        [sys.executable, str(SCRIPT), *args],
        check=False,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
        encoding="utf-8",
    )


class ValidateBuildTests(unittest.TestCase):
    def write_fixture(self) -> tuple[tempfile.TemporaryDirectory[str], Path]:
        tmp = tempfile.TemporaryDirectory()
        cargo = Path(tmp.name) / "Cargo.toml"
        cargo.write_text(FIXTURE, encoding="utf-8")
        return tmp, cargo

    def test_reads_real_workspace_version(self) -> None:
        result = run_validate(
            "--cargo-toml", str(CARGO),
            "--channel", "stable",
            "--release-tag", "v0.0.0",
        )
        # Fails tag-mismatch, not parse — proving the version was read.
        self.assertNotIn("workspace version not found", result.stderr)

    def test_stable_tag_must_match_workspace_version(self) -> None:
        with contextlib.ExitStack() as stack:
            tmp, cargo = self.write_fixture()
            stack.enter_context(tmp)
            ok = run_validate(
                "--cargo-toml", str(cargo),
                "--channel", "stable",
                "--release-tag", "v1.4.2",
            )
            self.assertEqual(ok.returncode, 0, ok.stderr)
            self.assertIn("release_tag=v1.4.2", ok.stdout)
            bad = run_validate(
                "--cargo-toml", str(cargo),
                "--channel", "stable",
                "--release-tag", "v1.4.3",
            )
            self.assertNotEqual(bad.returncode, 0)

    def test_stable_tag_rejects_prerelease_suffix(self) -> None:
        with contextlib.ExitStack() as stack:
            tmp, cargo = self.write_fixture()
            stack.enter_context(tmp)
            bad = run_validate(
                "--cargo-toml", str(cargo),
                "--channel", "stable",
                "--release-tag", "v1.4.2-alpha.1",
            )
            self.assertNotEqual(bad.returncode, 0)

    def test_alpha_tag_must_match_base_version(self) -> None:
        with contextlib.ExitStack() as stack:
            tmp, cargo = self.write_fixture()
            stack.enter_context(tmp)
            ok = run_validate(
                "--cargo-toml", str(cargo),
                "--channel", "alpha",
                "--release-tag", "v1.4.2-alpha.3",
            )
            self.assertEqual(ok.returncode, 0, ok.stderr)
            self.assertIn("GlobalTokenTracker-Setup-1.4.2-win-x64.exe", ok.stdout)
            bad = run_validate(
                "--cargo-toml", str(cargo),
                "--channel", "alpha",
                "--release-tag", "v1.5.0-alpha.1",
            )
            self.assertNotEqual(bad.returncode, 0)

    def test_alpha_rejects_stable_format(self) -> None:
        with contextlib.ExitStack() as stack:
            tmp, cargo = self.write_fixture()
            stack.enter_context(tmp)
            bad = run_validate(
                "--cargo-toml", str(cargo),
                "--channel", "alpha",
                "--release-tag", "v1.4.2",
            )
            self.assertNotEqual(bad.returncode, 0)

    def test_missing_tag_fails_for_stable(self) -> None:
        with contextlib.ExitStack() as stack:
            tmp, cargo = self.write_fixture()
            stack.enter_context(tmp)
            bad = run_validate(
                "--cargo-toml", str(cargo),
                "--channel", "stable",
            )
            self.assertNotEqual(bad.returncode, 0)

    def test_empty_alpha_tag_is_build_only(self) -> None:
        with contextlib.ExitStack() as stack:
            tmp, cargo = self.write_fixture()
            stack.enter_context(tmp)
            ok = run_validate(
                "--cargo-toml", str(cargo),
                "--channel", "alpha",
            )
            self.assertEqual(ok.returncode, 0, ok.stderr)
            self.assertIn("version_name=1.4.2", ok.stdout)
            self.assertNotIn("release_tag=", ok.stdout)


if __name__ == "__main__":
    unittest.main()
