"""The scanner installer must verify the tarball before it installs a binary."""

from __future__ import annotations

import hashlib
import os
import subprocess
import threading
import unittest
from collections.abc import Iterator
from contextlib import contextmanager
from http.server import BaseHTTPRequestHandler, HTTPServer
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
ACTION = Path(__file__).resolve().parent
SCRIPT = ACTION / "install-release.sh"
CACHE_PERMISSION = "actions: write # cache the sha256-verified scanner tarball"


class ReleaseServer(HTTPServer):
    hits: int
    failures: int
    body: bytes


class Handler(BaseHTTPRequestHandler):
    server: ReleaseServer

    def do_GET(self) -> None:
        self.server.hits += 1
        if self.server.hits <= self.server.failures:
            self.send_response(503)
            self.send_header("Content-Length", "0")
            self.end_headers()
            return
        body = self.server.body
        self.send_response(200)
        self.send_header("Content-Type", "application/octet-stream")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def log_message(self, fmt: str, *args: object) -> None:
        return


class Serving:
    def __init__(self, body: bytes, failures: int) -> None:
        self._server = ReleaseServer(("127.0.0.1", 0), Handler)
        self._server.hits = 0
        self._server.failures = failures
        self._server.body = body
        self._thread = threading.Thread(target=self._server.serve_forever, daemon=True)

    def __enter__(self) -> ReleaseServer:
        self._thread.start()
        return self._server

    def __exit__(self, *exc: object) -> None:
        self._server.shutdown()
        self._server.server_close()
        self._thread.join(timeout=5)


def sha256(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def make_tarball(directory: Path, name: str) -> bytes:
    member = directory / name
    member.write_text("#!/bin/sh\necho fixture version\n", encoding="utf-8")
    member.chmod(0o755)
    archive = directory / f"{name}.tar.gz"
    subprocess.run(["tar", "-czf", str(archive), "-C", str(directory), name], check=True)
    return archive.read_bytes()


def run_install(
    tmp: Path, *, url: str, digest: str, cache: Path, name: str = "trivy", env: dict[str, str] | None = None
) -> subprocess.CompletedProcess[str]:
    dest = tmp / "bin"
    child_env = os.environ.copy()
    child_env.update(env or {})
    return subprocess.run(
        [str(SCRIPT), name, url, digest, str(dest), str(cache)],
        check=False,
        capture_output=True,
        text=True,
        env=child_env,
    )


class InstallRelease(unittest.TestCase):
    def test_a_cached_tarball_with_the_pinned_digest_is_installed_without_a_download(self) -> None:
        with self._case() as case:
            body = make_tarball(case, "trivy")
            digest = sha256(body)
            cache = case / "cache" / "trivy.tar.gz"
            cache.parent.mkdir()
            cache.write_bytes(body)
            result = run_install(case, url="http://127.0.0.1:1/absent", digest=digest, cache=cache)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertTrue((case / "bin" / "trivy").stat().st_mode & 0o111)
            self.assertIn("fixture version", result.stdout)

    def test_a_cached_tarball_with_the_wrong_digest_is_replaced(self) -> None:
        with self._case() as case:
            good = make_tarball(case, "trivy")
            digest = sha256(good)
            cache = case / "cache" / "trivy.tar.gz"
            cache.parent.mkdir()
            cache.write_bytes(b"not-the-release")
            with Serving(good, failures=0) as server:
                port = server.server_address[1]
                result = run_install(
                    case,
                    url=f"http://127.0.0.1:{port}/trivy.tar.gz",
                    digest=digest,
                    cache=cache,
                    env={"INSTALL_RELEASE_RETRY_DELAY": "0"},
                )
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertEqual(server.hits, 1, "should download once after a bad cache")
            self.assertEqual(sha256(cache.read_bytes()), digest)

    def test_a_transient_download_failure_is_retried(self) -> None:
        with self._case() as case:
            body = make_tarball(case, "trivy")
            digest = sha256(body)
            cache = case / "cache" / "trivy.tar.gz"
            with Serving(body, failures=2) as server:
                port = server.server_address[1]
                result = run_install(
                    case,
                    url=f"http://127.0.0.1:{port}/trivy.tar.gz",
                    digest=digest,
                    cache=cache,
                    env={"INSTALL_RELEASE_RETRY_DELAY": "0", "INSTALL_RELEASE_MAX_ATTEMPTS": "4"},
                )
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertEqual(server.hits, 3, "should accept the third response")

    def test_a_checksum_mismatch_does_not_install_the_binary(self) -> None:
        with self._case() as case:
            body = make_tarball(case, "trivy")
            cache = case / "cache" / "trivy.tar.gz"
            with Serving(body, failures=0) as server:
                port = server.server_address[1]
                result = run_install(
                    case,
                    url=f"http://127.0.0.1:{port}/trivy.tar.gz",
                    digest="0" * 64,
                    cache=cache,
                    env={"INSTALL_RELEASE_MAX_ATTEMPTS": "5"},
                )
                self.assertNotEqual(result.returncode, 0)
                self.assertEqual(server.hits, 1, "should reject: checksum mismatch is not retried")
            self.assertFalse((case / "bin" / "trivy").exists())
            self.assertFalse(cache.exists())

    def test_a_download_that_keeps_failing_stops_at_the_attempt_budget(self) -> None:
        with self._case() as case:
            cache = case / "cache" / "trivy.tar.gz"
            with Serving(b"", failures=10) as server:
                port = server.server_address[1]
                result = run_install(
                    case,
                    url=f"http://127.0.0.1:{port}/trivy.tar.gz",
                    digest="ab" * 32,
                    cache=cache,
                    env={
                        "INSTALL_RELEASE_RETRY_DELAY": "0",
                        "INSTALL_RELEASE_MAX_ATTEMPTS": "3",
                        "INSTALL_RELEASE_DEADLINE_SECONDS": "120",
                    },
                )
                self.assertNotEqual(result.returncode, 0)
                self.assertEqual(server.hits, 3, "should stop at the attempt budget")
            self.assertFalse((case / "bin" / "trivy").exists())

    def test_a_download_stops_when_the_deadline_is_already_spent(self) -> None:
        with self._case() as case:
            cache = case / "cache" / "trivy.tar.gz"
            with Serving(b"", failures=10) as server:
                port = server.server_address[1]
                result = run_install(
                    case,
                    url=f"http://127.0.0.1:{port}/trivy.tar.gz",
                    digest="ab" * 32,
                    cache=cache,
                    env={
                        "INSTALL_RELEASE_RETRY_DELAY": "0",
                        "INSTALL_RELEASE_MAX_ATTEMPTS": "8",
                        "INSTALL_RELEASE_DEADLINE_SECONDS": "0",
                    },
                )
                self.assertNotEqual(result.returncode, 0)
                self.assertEqual(server.hits, 1, "should stop once the deadline has passed")

    def test_a_short_digest_is_refused(self) -> None:
        with self._case() as case:
            result = run_install(
                case, url="http://127.0.0.1:1/absent", digest="abc", cache=case / "cache" / "trivy.tar.gz"
            )
            self.assertEqual(result.returncode, 2, result.stderr)
            self.assertFalse((case / "bin" / "trivy").exists())

    @contextmanager
    def _case(self) -> Iterator[Path]:
        root = Path(os.environ.get("TMPDIR", "/tmp")) / "install-release-tests" / self.id().replace(".", "_")
        if root.exists():
            subprocess.run(["rm", "-rf", str(root)], check=True)
        root.mkdir(parents=True)
        try:
            yield root
        finally:
            subprocess.run(["rm", "-rf", str(root)], check=False)


class ScannerPinContract(unittest.TestCase):
    def test_the_cache_key_includes_the_pinned_digest(self) -> None:
        action = (ACTION / "action.yml").read_text(encoding="utf-8")
        pins = (ACTION / "pins.env").read_text(encoding="utf-8")
        self.assertIn("trivy-v1-${TRIVY_VERSION}-${TRIVY_SHA256}", action)
        self.assertIn("trufflehog-v1-${TRUFFLEHOG_VERSION}-${TRUFFLEHOG_SHA256}", action)
        self.assertIn("install-release.sh", action)
        self.assertIn("!cancelled()", action)
        self.assertIn('export TRIVY_CACHE_DIR="${RUNNER_TEMP}/trivy-db"', action)
        self.assertIn('echo "TRIVY_CACHE_DIR=${TRIVY_CACHE_DIR}" >> "$GITHUB_ENV"', action)
        for line in pins.splitlines():
            if line.startswith(("TRIVY_SHA256=", "TRUFFLEHOG_SHA256=")):
                digest = line.split("=", 1)[1]
                self.assertRegex(digest, r"^[0-9a-f]{64}$", f"should reject: {line!r}")

    def test_scans_do_not_skip_the_vulnerability_database_update(self) -> None:
        paths = [
            ROOT / ".github/workflows/security.yml",
            ROOT / ".github/workflows/trivy-images.yml",
            ROOT / ".github/actions/image-cve-gate/action.yml",
            ACTION / "action.yml",
        ]
        for path in paths:
            text = path.read_text(encoding="utf-8")
            self.assertNotIn("--skip-db-update", text, f"should reject: {path}")
            self.assertNotIn("--skip-java-db-update", text, f"should reject: {path}")

    def test_the_blocking_scan_still_fails_on_a_critical(self) -> None:
        security = (ROOT / ".github/workflows/security.yml").read_text(encoding="utf-8")
        images = (ROOT / ".github/workflows/trivy-images.yml").read_text(encoding="utf-8")
        self.assertIn("--severity CRITICAL \\\n", security)
        self.assertIn("--exit-code 1 \\\n", security)
        self.assertIn("trivy convert --severity CRITICAL --exit-code 1 --format table trivy-image.json", images)

    def test_scanner_workflows_can_write_the_actions_cache(self) -> None:
        counts = {
            ".github/workflows/security.yml": 3,
            ".github/workflows/trivy-images.yml": 1,
            ".github/workflows/build-images.yml": 2,
        }
        for path, expected in counts.items():
            text = (ROOT / path).read_text(encoding="utf-8")
            found = text.count(CACHE_PERMISSION)
            self.assertEqual(found, expected, f"should grant cache write {expected} times in {path}, found {found}")


if __name__ == "__main__":
    unittest.main()
