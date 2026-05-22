"""Sanity tests for the GitHub Actions workflow.

We don't run actions in pytest, but we DO check that the YAML parses, that
the trigger paths align with PRD §4.1, and that the workflow uses the
same runner image as local development (./e2e.sh build) so dev and CI
never drift.
"""

from __future__ import annotations

from pathlib import Path

import pytest
import yaml


pytestmark = pytest.mark.smoke


_REPO_ROOT = Path(__file__).resolve().parents[5]
_WORKFLOW = _REPO_ROOT / ".github/workflows/e2e-bronze-to-api.yml"


@pytest.fixture(scope="module")
def workflow() -> dict:
    assert _WORKFLOW.is_file(), f"missing {_WORKFLOW}"
    return yaml.safe_load(_WORKFLOW.read_text(encoding="utf-8"))


def test_yaml_parses(workflow: dict) -> None:
    assert isinstance(workflow, dict)
    assert "jobs" in workflow


def test_required_paths_in_filter(workflow: dict) -> None:
    """PR-touch path filter MUST cover ingestion, analytics-api, insight-clickhouse lib."""
    # PyYAML coerces `on:` (a YAML truthy key) to the boolean True. Accept either.
    on = workflow.get("on") or workflow.get(True)
    assert on, "workflow has no `on:` triggers"
    pr_paths = set(on.get("pull_request", {}).get("paths", []))
    for required in (
        "src/ingestion/**",
        "src/backend/services/analytics-api/**",
        "src/backend/libs/insight-clickhouse/**",
    ):
        assert required in pr_paths, f"PR path filter missing {required!r}"


def test_uses_local_runner_image(workflow: dict) -> None:
    """CI must build via ./e2e.sh so dev + CI use the same runner image."""
    job = next(iter(workflow["jobs"].values()))
    runs = [s.get("run", "") for s in job["steps"]]
    assert any("./e2e.sh build" in r for r in runs), "no `./e2e.sh build` step found"
    assert any("./e2e.sh test" in r for r in runs), "no `./e2e.sh test` step found"


def test_ci_env_set_to_true(workflow: dict) -> None:
    """CI=true env enforces the --update-snapshots guard in fixtures/test_fixtures.py."""
    env = workflow.get("env") or {}
    assert env.get("CI") == "true", "workflow must export CI=true to enforce snapshot guard"


def test_pytest_runs_with_xdist(workflow: dict) -> None:
    """The pytest invocation MUST use -n auto so the suite parallelizes."""
    job = next(iter(workflow["jobs"].values()))
    test_step = next(s for s in job["steps"] if s.get("name") == "Run E2E suite")
    assert "-n auto" in test_step["run"]


def test_compose_logs_dumped_on_failure(workflow: dict) -> None:
    job = next(iter(workflow["jobs"].values()))
    has_dump = any(
        s.get("name") == "Dump compose logs on failure" and s.get("if") == "failure()"
        for s in job["steps"]
    )
    assert has_dump


def test_cargo_target_cached(workflow: dict) -> None:
    """Cargo target/ MUST be cached so the typical PR build takes seconds, not minutes."""
    job = next(iter(workflow["jobs"].values()))
    cache_step = next(
        (s for s in job["steps"] if isinstance(s.get("uses"), str) and "actions/cache" in s["uses"]),
        None,
    )
    assert cache_step is not None, "no actions/cache step in workflow"
    paths = cache_step["with"]["path"]
    assert "src/backend/target" in paths
    assert "~/.cargo/registry" in paths
