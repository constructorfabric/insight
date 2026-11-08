"""Fake-registry checks for orphan deletion. No gh process and no real deletes."""

from __future__ import annotations

import importlib.util
import io
import json
import re
import unittest
from contextlib import redirect_stderr, redirect_stdout
from pathlib import Path
from unittest import mock

SCRIPT = Path(__file__).resolve().parents[1] / "ghcr-orphan-sweep.py"
SPEC = importlib.util.spec_from_file_location("ghcr_orphan_sweep", SCRIPT)
assert SPEC is not None and SPEC.loader is not None
sweep = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(sweep)

ORPHAN = "2026.01.02.03.04-abc1234.retired-topic"
LIVE = "2026.01.02.03.04-abc1234.topic"
GONE = "gh: Package version not found. (HTTP 404)"
DENIED = "gh: Package not found. (HTTP 404)"


def call_kind(path: str, args: tuple[str, ...]) -> str:
    if path.endswith("/branches"):
        return "branches"
    if "-X" in args:
        return "delete"
    if re.search(r"/versions/\d+$", path):
        return "get"
    if path.endswith("/versions"):
        return "list"
    raise AssertionError((path, args))


class Registry:
    def __init__(self) -> None:
        self.rows: dict[tuple[str, int], list[str]] = {}
        self.readable: set[tuple[str, int]] = set()
        self.errors: dict[tuple[str, int], str] = {}
        self.calls: list[tuple[str, str]] = []
        self.branches = "main\ntopic\n"

    def add(self, package: str, version_id: int, tags: list[str], error: str | None = None) -> None:
        key = (package, version_id)
        self.rows[key] = tags
        self.readable.add(key)
        if error is not None:
            self.errors[key] = error

    def dispatch(self, path: str, args: tuple[str, ...]) -> str:
        kind = call_kind(path, args)
        package = path.split("/container/")[1].split("/versions")[0] if "/container/" in path else ""
        found = re.search(r"/versions/(\d+)$", path)
        version_id = int(found.group(1)) if found else None
        label = f"{package}:{version_id}" if version_id is not None else package or path
        self.calls.append((kind, label))
        if kind == "branches":
            return self.branches
        if kind == "list":
            rows = [
                {"id": vid, "metadata": {"container": {"tags": tags}}}
                for (name, vid), tags in self.rows.items()
                if name == package
            ]
            return json.dumps(rows) + "\n"
        key = (package, version_id)
        assert version_id is not None
        if kind == "get":
            if key not in self.readable:
                raise sweep.GhApiError(path, GONE)
            return json.dumps({"id": version_id})
        error = self.errors.get(key)
        if error is not None:
            if error == GONE:
                self.readable.discard(key)
            raise sweep.GhApiError(path, error)
        self.readable.discard(key)
        return ""


class OrphanSweepTests(unittest.TestCase):
    def setUp(self) -> None:
        self.registry = Registry()

        def gh_api(path: str, *args: str) -> str:
            return self.registry.dispatch(path, args)

        patcher = mock.patch.object(sweep, "gh_api", gh_api)
        patcher.start()
        self.addCleanup(patcher.stop)

    def run_sweep(
        self,
        packages: str = "example-image",
        dry_run: str = "false",
        branches: str | None = None,
    ) -> tuple[int, str, str]:
        if branches is not None:
            self.registry.branches = branches
        env = {"PACKAGES": packages, "DRY_RUN": dry_run}
        with mock.patch.dict(sweep.os.environ, env, clear=False):
            stdout, stderr = io.StringIO(), io.StringIO()
            with redirect_stdout(stdout), redirect_stderr(stderr):
                code = sweep.main()
        return code, stdout.getvalue(), stderr.getvalue()

    def deletes(self) -> list[str]:
        return [target for kind, target in self.registry.calls if kind == "delete"]

    def test_not_found_bodies_distinguish_a_missing_version_from_a_package_404(self) -> None:
        missing = sweep.GhApiError("/versions/1", GONE)
        denied = sweep.GhApiError("/versions/1", DENIED)
        raw = sweep.GhApiError("/versions/1", '{"message":"Package not found.","status":"404"}')
        self.assertEqual(sweep.not_found_kind(missing), "version", "should be a missing version")
        self.assertEqual(sweep.not_found_kind(denied), "package", "should be a package-level 404")
        self.assertEqual(sweep.not_found_kind(raw), "package", "should read the JSON status")
        self.assertIsNone(sweep.not_found_kind(sweep.GhApiError("/versions/1", "gh: Forbidden (HTTP 403)")))

    def test_a_readable_version_stays_a_failure(self) -> None:
        self.registry.add("example-image", 42, [ORPHAN], DENIED)
        code, stdout, stderr = self.run_sweep()
        self.assertEqual(code, 1, stdout)
        self.assertIn("still present", stdout)
        self.assertIn("could not be deleted", stderr)
        self.assertIn(("example-image", 42), self.registry.readable)
        self.assertEqual(self.deletes(), ["example-image:42"])

    def test_a_missing_version_is_not_a_failed_delete(self) -> None:
        cases = ("delete says the version is gone", "package 404 and the follow-up read misses")
        for name in cases:
            with self.subTest(name=name):
                self.registry = Registry()
                error = GONE if name.startswith("delete") else DENIED
                self.registry.add("example-image", 42, [ORPHAN], error)
                if error == DENIED:
                    self.registry.readable.discard(("example-image", 42))
                code, stdout, stderr = self.run_sweep()
                self.assertEqual(code, 0, stderr)
                self.assertIn("already absent", stdout)
                self.assertNotIn("could not be deleted", stderr)

    def test_other_delete_errors_still_fail(self) -> None:
        for error in ("gh: Forbidden (HTTP 403)", "gh: Internal Server Error (HTTP 500)", "connection reset"):
            with self.subTest(error=error):
                self.registry = Registry()
                self.registry.add("example-image", 42, [ORPHAN], error)
                code, stdout, stderr = self.run_sweep()
                self.assertEqual(code, 1, stdout)
                self.assertNotIn("already absent", stdout)
                self.assertNotIn(("get", "example-image:42"), self.registry.calls)
                self.assertIn(("example-image", 42), self.registry.readable)

    def test_a_later_package_is_still_swept_after_a_refused_delete(self) -> None:
        self.registry.add("example-image", 42, [ORPHAN], DENIED)
        self.registry.add("other-image", 7, [ORPHAN])
        code, _stdout, stderr = self.run_sweep(packages="example-image other-image")
        self.assertEqual(code, 1, stderr)
        self.assertEqual(self.deletes(), ["example-image:42", "other-image:7"])
        self.assertNotIn(("other-image", 7), self.registry.readable)

    def test_protected_versions_are_not_deleted(self) -> None:
        protected = {
            "live branch": [LIVE],
            "release suffix": ["2026.01.02.03.04-abc1234.release-1"],
            "trunk tag": ["2026.01.02.03.04-abc1234"],
            "latest": ["latest"],
            "untagged": [],
            "mixed with a live tag": [ORPHAN, LIVE],
            "unrecognized": ["custom-pin"],
        }
        for name, tags in protected.items():
            with self.subTest(name=name):
                self.registry = Registry()
                self.registry.add("example-image", 42, tags)
                code, stdout, stderr = self.run_sweep()
                self.assertEqual(code, 0, stderr)
                self.assertEqual(self.deletes(), [], name)
                self.assertIn(("example-image", 42), self.registry.readable)
                if name == "unrecognized":
                    self.assertIn("unrecognized", stdout)

    def test_dry_run_and_a_short_branch_list_delete_nothing(self) -> None:
        self.registry.add("example-image", 42, [ORPHAN])
        code, stdout, stderr = self.run_sweep(dry_run="true")
        self.assertEqual(code, 0, stderr)
        self.assertIn("would delete", stdout)
        self.assertEqual(self.deletes(), [])
        code, _stdout, stderr = self.run_sweep(branches="main\n")
        self.assertEqual(code, 1)
        self.assertIn("refusing", stderr)
        self.assertEqual(self.deletes(), [])

    def test_a_successful_delete_removes_only_the_orphan(self) -> None:
        self.registry.add("example-image", 42, [ORPHAN])
        self.registry.add("example-image", 43, [LIVE])
        code, stdout, stderr = self.run_sweep()
        self.assertEqual(code, 0, stderr)
        self.assertEqual(self.deletes(), ["example-image:42"])
        self.assertNotIn(("example-image", 42), self.registry.readable)
        self.assertIn(("example-image", 43), self.registry.readable)
        self.assertIn("failed=0", stdout)


if __name__ == "__main__":
    unittest.main()
