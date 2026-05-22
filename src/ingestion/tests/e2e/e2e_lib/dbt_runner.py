"""dbt CLI wrapper: parse once per session, build per test.

We do NOT modify the existing `src/ingestion/dbt/profiles.yml` — instead we
generate a session-local profiles directory under `target/test-profiles/`
that points at our test ClickHouse. dbt's `--profiles-dir` flag picks it up.

Per-worker schema namespacing is wired via `--vars '{worker_id: N}'`.
Existing dbt models do NOT consume `worker_id` yet, so parallel runs against
the same database WILL collide today; the variable is plumbed in advance so
that switching dbt models to honor it is a one-file change in each model.
Tracked as risk in PRD §12.
"""

from __future__ import annotations

import json
import logging
import shutil
import subprocess
from pathlib import Path

import yaml

from e2e_lib.config import SessionConfig
from e2e_lib.worker import WorkerContext

LOG = logging.getLogger("e2e.dbt")


class DbtError(RuntimeError):
    pass


class DbtRunner:
    """Session-scoped wrapper around the `dbt` CLI."""

    def __init__(self, cfg: SessionConfig):
        self.cfg = cfg
        # Target dir is co-located with the dbt project so dbt's relative
        # paths (logs/, target/) keep working.
        self.dbt_project_dir = cfg.dbt_project_dir
        self.target_dir = cfg.repo_root / "src/ingestion/tests/e2e/target/dbt"
        self.profiles_dir = self.target_dir / "profiles"
        self._parsed = False

    def setup(self) -> None:
        """One-time per session: write test profiles.yml + `dbt parse`."""
        self._write_profiles()
        self._parse()
        self._parsed = True

    def build(
        self,
        selector: str,
        *,
        worker_ctx: WorkerContext,
        timeout_s: float = 120.0,
    ) -> None:
        """Run `dbt build --select <selector> --defer --state <target>`.

        Raises DbtError on non-zero exit, with the failing model + compiled SQL
        excerpt from `run_results.json` surfaced in the message.
        """
        if not self._parsed:
            raise DbtError("dbt_runner.setup() must be called before build()")
        worker_n = worker_ctx.worker_id.removeprefix("gw") if worker_ctx.worker_id != "master" else "0"
        cmd = [
            "dbt",
            "build",
            "--select",
            selector,
            "--profiles-dir",
            str(self.profiles_dir),
            "--target",
            "test",
            "--target-path",
            str(self.target_dir),
            "--defer",
            "--state",
            str(self.target_dir),
            "--vars",
            json.dumps({"worker_id": worker_n}),
        ]
        LOG.info("dbt build --select %s (worker=%s)", selector, worker_ctx.worker_id)
        result = subprocess.run(
            cmd,
            cwd=str(self.dbt_project_dir),
            capture_output=True,
            text=True,
            check=False,
            timeout=timeout_s,
        )
        if result.returncode != 0:
            failed = self._extract_failed_model_summary()
            raise DbtError(
                f"dbt build failed (exit={result.returncode}) for selector {selector!r}\n"
                f"failed models: {failed}\n"
                f"stdout tail:\n{result.stdout[-2000:]}\n"
                f"stderr tail:\n{result.stderr[-1000:]}"
            )

    # ----------------------------------------------------------------------
    # internals
    # ----------------------------------------------------------------------

    def _write_profiles(self) -> None:
        self.profiles_dir.mkdir(parents=True, exist_ok=True)
        profiles = {
            "ingestion": {
                "target": "test",
                "outputs": {
                    "test": {
                        "type": "clickhouse",
                        "host": "127.0.0.1",
                        "port": self.cfg.ch_http_port,
                        "schema": "default",
                        "user": self.cfg.ch_user,
                        "password": self.cfg.ch_password,
                        "secure": False,
                        # Match prod profile so models materialize identically
                        "engine": "ReplacingMergeTree(_version)",
                        "settings": {
                            "allow_nullable_key": 1,
                            "allow_experimental_refreshable_materialized_view": 1,
                        },
                    }
                },
            }
        }
        (self.profiles_dir / "profiles.yml").write_text(yaml.safe_dump(profiles))
        LOG.debug("wrote test profiles.yml to %s", self.profiles_dir)

    def _parse(self) -> None:
        """`dbt parse` produces target/manifest.json — reused by every per-test build via --defer."""
        cmd = [
            "dbt",
            "parse",
            "--profiles-dir",
            str(self.profiles_dir),
            "--target",
            "test",
            "--target-path",
            str(self.target_dir),
        ]
        LOG.info("dbt parse (one-time)")
        result = subprocess.run(
            cmd,
            cwd=str(self.dbt_project_dir),
            capture_output=True,
            text=True,
            check=False,
            timeout=120,
        )
        if result.returncode != 0:
            raise DbtError(
                f"dbt parse failed (exit={result.returncode}):\n"
                f"stdout:\n{result.stdout[-2000:]}\nstderr:\n{result.stderr[-1000:]}"
            )
        manifest = self.target_dir / "manifest.json"
        if not manifest.exists():
            raise DbtError(f"dbt parse did not produce {manifest}")

    def _extract_failed_model_summary(self) -> str:
        """Read target/run_results.json and return a one-liner per failed model."""
        run_results = self.target_dir / "run_results.json"
        if not run_results.exists():
            return "(no run_results.json)"
        try:
            data = json.loads(run_results.read_text(encoding="utf-8"))
        except Exception as e:
            return f"(failed to parse run_results.json: {e})"
        failed = [
            f"  - {r.get('unique_id', '?')}: {r.get('message') or r.get('status')}"
            for r in data.get("results", [])
            if r.get("status") not in (None, "success", "pass")
        ]
        return "\n" + "\n".join(failed) if failed else "(none)"

    def cleanup(self) -> None:
        """Remove generated profiles + target. Called by session teardown."""
        if self.target_dir.exists():
            shutil.rmtree(self.target_dir, ignore_errors=True)
