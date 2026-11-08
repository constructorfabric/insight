"""Registry decisions for the GHCR orphan sweep. The registry is a fake: no gh, no deletes."""

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

ORPHAN_TAG = "2026.01.02.03.04-abc1234.retired-topic"
LIVE_TAG = "2026.01.02.03.04-abc1234.topic"
RELEASE_TAG = "2026.01.02.03.04-abc1234.release-1"
TRUNK_TAG = "2026.01.02.03.04-abc1234"


def version(version_id: int, tags: list[str]) -> dict:
    return {"id": version_id, "metadata": {"container": {"tags": tags}}}


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
    """In-memory package versions. Deletes record intent and return a scripted response."""

    def __init__(self) -> None:
        self.versions: dict[tuple[str, int], dict] = {}
        self.readable: set[tuple[str, int]] = set()
        self.delete_error: dict[tuple[str, int], str | None] = {}
        self.calls: list[tuple[str, str]] = []
        self.branches = "main\ntopic\n"

    def add(self, package: str, version_id: int, tags: list[str], delete_error: str | None = None) -> None:
        key = (package, version_id)
        self.versions[key] = version(version_id, tags)
        self.readable.add(key)
        self.delete_error[key] = delete_error

    def hide(self, package: str, version_id: int) -> None:
        """The version stays in the list response and is gone on a later read."""
        self.readable.discard((package, version_id))

    def dispatch(self, path: str, args: tuple[str, ...]) -> str:
        kind = call_kind(path, args)
        package = path.split("/container/")[1].split("/versions")[0] if "/container/" in path else ""
        version_match = re.search(r"/versions/(\d+)$", path)
        version_id = int(version_match.group(1)) if version_match else None
        self.calls.append((kind, f"{package}:{version_id}" if version_id is not None else package or path))
        if kind == "branches":
            return self.branches
        if kind == "list":
            rows = [row for (name, _), row in self.versions.items() if name == package]
            return json.dumps(rows) + "\n"
        if kind == "get":
            assert version_id is not None
            if (package, version_id) not in self.readable:
                raise sweep.GhApiError(path, "gh: Package version not found. (HTTP 404)")
            return json.dumps(self.versions[(package, version_id)])
        assert version_id is not None
        key = (package, version_id)
        error = self.delete_error.get(key)
        if error is not None:
            if "Package version not found" in error:
                self.readable.discard(key)
            raise sweep.GhApiError(path, error)
        self.readable.discard(key)
        return ""


class OrphanSweepTests(unittest.TestCase):
    def setUp(self) -> None:
        self.registry = Registry()

        def gh_api(path: str, *args: str) -> str:
            return self.registry.dispatch(path, args)

        self._patch = mock.patch.object(sweep, "gh_api", gh_api)
        self._patch.start()
        self.addCleanup(self._patch.stop)

    def sweep(
        self,
        packages: str = "example-image",
        dry_run: str = "false",
        branches: str | None = None,
    ) -> tuple[int, str, str]:
        if branches is not None:
            self.registry.branches = branches
        with mock.patch.dict(sweep.os.environ, {"PACKAGES": packages, "DRY_RUN": dry_run}, clear=False):
            stdout, stderr = io.StringIO(), io.StringIO()
            with redirect_stdout(stdout), redirect_stderr(stderr):
                code = sweep.main()
        return code, stdout.getvalue(), stderr.getvalue()

    def deletes(self) -> list[str]:
        return [target for kind, target in self.registry.calls if kind == "delete"]

    def test_not_found_bodies_distinguish_a_missing_version_from_a_denied_package(self) -> None:
        missing = sweep.GhApiError("/versions/1", "gh: Package version not found. (HTTP 404)")
        denied = sweep.GhApiError("/versions/1", "gh: Package not found. (HTTP 404)")
        raw = sweep.GhApiError("/versions/1", '{"message":"Package not found.","status":"404"}')
        forbidden = sweep.GhApiError("/versions/1", "gh: Forbidden (HTTP 403)")

        self.assertEqual(sweep.not_found_kind(missing), "version", "should be a missing version")
        self.assertEqual(sweep.not_found_kind(denied), "package", "should be a package-level 404")
        self.assertEqual(sweep.not_found_kind(raw), "package", "should read the JSON status")
        self.assertIsNone(sweep.not_found_kind(forbidden), "should reject: 403")

    def test_version_already_gone_is_not_a_failed_delete(self) -> None:
        self.registry.add(
            "example-image",
            42,
            [ORPHAN_TAG],
            delete_error="gh: Package version not found. (HTTP 404)",
        )

        code, stdout, stderr = self.sweep()

        self.assertEqual(code, 0, stderr)
        self.assertIn("already absent", stdout)
        self.assertNotIn("could not be deleted", stderr)
        self.assertEqual(self.deletes(), ["example-image:42"])
        self.assertIn(("get", "example-image:42"), self.registry.calls)

    def test_readable_version_stays_a_failure_when_delete_says_package_not_found(self) -> None:
        self.registry.add(
            "example-image",
            42,
            [ORPHAN_TAG],
            delete_error="gh: Package not found. (HTTP 404)",
        )

        code, stdout, stderr = self.sweep()

        self.assertEqual(code, 1, stdout)
        self.assertIn("still present", stdout)
        self.assertIn("could not be deleted", stderr)
        self.assertEqual(self.deletes(), ["example-image:42"])
        self.assertIn(("example-image", 42), self.registry.readable)

    def test_package_not_found_is_success_only_after_the_version_read_misses(self) -> None:
        self.registry.add("example-image", 42, [ORPHAN_TAG], delete_error="gh: Package not found. (HTTP 404)")
        self.registry.hide("example-image", 42)

        code, stdout, stderr = self.sweep()

        self.assertEqual(code, 0, stderr)
        self.assertIn("already absent", stdout)

    def test_non_404_delete_errors_still_fail_without_treating_the_version_as_gone(self) -> None:
        cases = (
            "gh: Forbidden (HTTP 403)",
            "gh: Internal Server Error (HTTP 500)",
            "connection reset",
        )
        for error in cases:
            with self.subTest(error=error):
                self.registry = Registry()
                self.registry.add("example-image", 42, [ORPHAN_TAG], delete_error=error)

                code, stdout, stderr = self.sweep()

                self.assertEqual(code, 1, stdout)
                self.assertNotIn("already absent", stdout)
                self.assertIn("could not be deleted", stderr)
                self.assertNotIn(("get", "example-image:42"), self.registry.calls)
                self.assertIn(("example-image", 42), self.registry.readable)

    def test_a_later_package_is_still_swept_after_a_refused_delete(self) -> None:
        self.registry.add("example-image", 42, [ORPHAN_TAG], delete_error="gh: Package not found. (HTTP 404)")
        self.registry.add("other-image", 7, [ORPHAN_TAG], delete_error=None)

        code, _stdout, stderr = self.sweep(packages="example-image other-image")

        self.assertEqual(code, 1, stderr)
        self.assertEqual(self.deletes(), ["example-image:42", "other-image:7"])
        self.assertNotIn(("other-image", 7), self.registry.readable)
        self.assertIn(("example-image", 42), self.registry.readable)

    def test_active_and_protected_versions_are_not_deleted(self) -> None:
        protected = {
            "live branch": [LIVE_TAG],
            "release suffix": [RELEASE_TAG],
            "trunk tag": [TRUNK_TAG],
            "latest": ["latest"],
            "untagged": [],
            "mixed with a live tag": [ORPHAN_TAG, LIVE_TAG],
            "unrecognized": ["custom-pin"],
        }
        for name, tags in protected.items():
            with self.subTest(name=name):
                self.registry = Registry()
                self.registry.add("example-image", 42, tags)

                code, stdout, stderr = self.sweep()

                self.assertEqual(code, 0, stderr)
                self.assertEqual(self.deletes(), [], name)
                self.assertIn(("example-image", 42), self.registry.readable)
                if name == "unrecognized":
                    self.assertIn("unrecognized", stdout)

    def test_dry_run_reports_the_orphan_and_does_not_delete(self) -> None:
        self.registry.add("example-image", 42, [ORPHAN_TAG])

        code, stdout, stderr = self.sweep(dry_run="true")

        self.assertEqual(code, 0, stderr)
        self.assertIn("would delete", stdout)
        self.assertEqual(self.deletes(), [])
        self.assertIn(("example-image", 42), self.registry.readable)

    def test_too_few_live_branches_deletes_nothing(self) -> None:
        self.registry.add("example-image", 42, [ORPHAN_TAG])

        code, _stdout, stderr = self.sweep(branches="main\n")

        self.assertEqual(code, 1)
        self.assertIn("refusing", stderr)
        self.assertEqual(self.deletes(), [])

    def test_successful_delete_removes_only_the_orphan(self) -> None:
        self.registry.add("example-image", 42, [ORPHAN_TAG])
        self.registry.add("example-image", 43, [LIVE_TAG])

        code, stdout, stderr = self.sweep()

        self.assertEqual(code, 0, stderr)
        self.assertEqual(self.deletes(), ["example-image:42"])
        self.assertNotIn(("example-image", 42), self.registry.readable)
        self.assertIn(("example-image", 43), self.registry.readable)
        self.assertIn("orphaned=1", stdout)
        self.assertIn("failed=0", stdout)


if __name__ == "__main__":
    unittest.main()
