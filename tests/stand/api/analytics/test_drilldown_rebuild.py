"""`POST /v1/metric-drilldown` while its evidence table is rebuilt — the live half
of the snapshot guard.

    POST /v1/metric-drilldown   200 · 400 EVIDENCE_SNAPSHOT_EXPIRED once the build rotates

`test_drilldown.py` covers the routes' contract against a stand at rest. This
module is the one case that cannot live there: it CHANGES the stand mid-test,
by triggering the same rebuild a deployment runs — a scoped
`dbt run --select git_metric_evidence` through the stand's own seed image, the
exact mechanism `test-stand seed` uses. dbt's table materialization swaps the
relation atomically (a fresh table exchanged over the old name), so the
rebuild rotates the ClickHouse table UUID that every continuation token pins
as its `snapshot_id`, and a cursor issued before the swap must be refused
afterwards. That refusal-under-a-real-rebuild is what scenario 7 claims;
refusing a hand-tampered token is scenario 6's, in `test_drilldown.py`.

Why the paged read and not the export: both operations re-verify the snapshot
through the same `verify_evidence_snapshot` call, but an export does it inside
one request, and nothing outside the process can schedule a rebuild into that
window deterministically. A cursor is the same pinned snapshot made to SPAN
requests, so "rebuild, then present the pre-rebuild snapshot" becomes an exact
sequence rather than a race — and a refusal there is the refusal the export
path shares.

The lane is serialized by construction and opt-in by policy. The suite runs in
a single pytest process (no xdist in tests/uv.lock), so nothing in this run
holds a cursor while the rebuild happens; the `rebuild_lane` marker keeps the
test out of every run that did not pass `--rebuild-lane`, because a second run
sharing the stand could be mid-walk when the UUID rotates — and because the
trigger shells out to docker, which only exists beside the local compose
stand. The stand is left equivalent: the rebuild re-materializes the same
deterministic SQL over unchanged silver and identity data, and the final
reconciliation in the test is the proof.
"""

from __future__ import annotations

import os
import subprocess
from collections.abc import Mapping
from pathlib import Path
from typing import Any

import pytest
from insight_stand import ApiClient, Manifest, analytics_path
from insight_stand.api import JsonValue
from insight_stand.stand import CANDIDATE_ENV_FILES, ENV_FILE_ENV

from ..schemas import MetricResultsResponse, PeriodView, ProblemDocument
from ..schemas.analytics import MetricDrilldownResponse
from . import query_window

pytestmark = pytest.mark.reliability

DRILLDOWN = analytics_path("/v1/metric-drilldown")
METRIC_RESULTS = analytics_path("/v1/metric-results")

#: The metric under rebuild and the dbt model serving its evidence
#: (`registry.yaml`: source `git` → `evidence_ref: git_metric_evidence`).
#: `git.commits` because the seed guarantees it rows for `dev_lead`, and its
#: reconciliation rule is the simplest one there is: one evidence row per
#: counted commit.
GIT_COMMITS = "git.commits"
EVIDENCE_MODEL = "git_metric_evidence"

_REPO_ROOT = Path(__file__).resolve().parents[4]

#: The compose project `dev-compose.sh` runs every stand under unless an
#: `--instance` renamed it; an instance-named stand overrides this to
#: `insight-<instance>` so the rebuild lands on the stand under test.
_COMPOSE_PROJECT_ENV = "INSIGHT_STAND_COMPOSE_PROJECT"
_DEFAULT_COMPOSE_PROJECT = "insight"

#: Wall-clock ceiling for the rebuild container. Dominated by the one dbt
#: model build; generous because a cold run may build the seed image first.
_REBUILD_TIMEOUT_SECONDS = 900

_PAGE_LIMIT = 250
_PAGE_BUDGET = 40


def _stand_env_file() -> Path:
    """The env file of the stand this run is aimed at — same resolution as
    `insight_stand.stand`, so the rebuild targets the stand the requests hit."""
    override = (os.environ.get(ENV_FILE_ENV) or "").strip()
    candidates = (
        [Path(override)] if override else [_REPO_ROOT / name for name in CANDIDATE_ENV_FILES]
    )
    for candidate in candidates:
        if candidate.is_file():
            return candidate
    pytest.fail(
        "rebuild_lane: no compose env file found "
        f"(tried {', '.join(str(c) for c in candidates)}) — the rebuild can only target the "
        "local compose stand, brought up by ./dev-compose.sh test-stand up"
    )


def _rebuild_evidence_relation() -> None:
    """Rebuild ONLY the walked evidence relation, through the deployed path.

    `insight_seed.silver.apply_ch_migrations` is what `insight-seed gold` (and
    the k8s clickhouse-migrate Hook Job) runs; narrowing its dbt selection to
    the one model is the only difference from `test-stand seed gold`. Never
    the silver step — that regenerates rows — and never direct DDL, which
    would prove a hand-rolled swap rather than the deployment's.

    Mirrors `cmd_seed` in dev-compose.sh: same compose file, seed profile and
    uid mapping, minus `--build` (the stand's own seed built the image, and
    the ingestion tree is bind-mounted so the dbt project is current anyway).
    """
    code = (
        "from insight_seed.silver import apply_ch_migrations; "
        f"apply_ch_migrations(dbt_select={EVIDENCE_MODEL!r})"
    )
    project = os.environ.get(_COMPOSE_PROJECT_ENV, "").strip() or _DEFAULT_COMPOSE_PROJECT
    command = [
        "docker",
        "compose",
        "--project-name",
        project,
        "--env-file",
        str(_stand_env_file()),
        "-f",
        "docker-compose.yml",
        "--profile",
        "seed",
        "run",
        "--rm",
        "--no-deps",
        "--entrypoint",
        "python",
        "seed-sample",
        "-c",
        code,
    ]
    env = {**os.environ, "SEED_UID": str(os.getuid()), "SEED_GID": str(os.getgid())}
    try:
        completed = subprocess.run(
            command,
            cwd=_REPO_ROOT,
            env=env,
            capture_output=True,
            text=True,
            timeout=_REBUILD_TIMEOUT_SECONDS,
            check=False,
        )
    except subprocess.TimeoutExpired:
        # The timeout killed the docker CLI, not the container dockerd runs
        # for it — without this the rebuild keeps mutating the stand past the
        # ceiling the failure message claims to enforce.
        _remove_seed_containers(project)
        pytest.fail(
            f"rebuild of {EVIDENCE_MODEL} exceeded {_REBUILD_TIMEOUT_SECONDS}s and its "
            "container was force-removed — re-seed before trusting other results"
        )
    if completed.returncode != 0:
        pytest.fail(
            f"rebuild of {EVIDENCE_MODEL} failed (exit {completed.returncode}); the model "
            "either rebuilt or kept its previous build, never half of each — dbt swaps "
            f"atomically.\nstderr tail: {completed.stderr[-2000:]}"
        )


def _remove_seed_containers(project: str) -> None:
    """Force-remove any seed-sample container of the stand's compose project.

    Best effort: failing to remove must not mask the timeout failure that
    called this, so errors are reported by the caller's message alone.
    """
    listed = subprocess.run(
        [
            "docker",
            "ps",
            "--quiet",
            "--filter",
            f"label=com.docker.compose.project={project}",
            "--filter",
            "label=com.docker.compose.service=seed-sample",
        ],
        capture_output=True,
        text=True,
        timeout=30,
        check=False,
    )
    container_ids = listed.stdout.split()
    if container_ids:
        subprocess.run(
            ["docker", "rm", "--force", *container_ids],
            capture_output=True,
            timeout=30,
            check=False,
        )


def _request(
    manifest: Manifest, *, limit: int | None = None, cursor: str | None = None
) -> dict[str, JsonValue]:
    start, end = query_window(manifest)
    request: dict[str, JsonValue] = {
        "metric_key": GIT_COMMITS,
        "entity": {"type": "person", "id": manifest.fixture("dev_lead").uuid},
        "period": {"from": start, "to": end},
        "filters": [],
        "display_dimensions": [],
    }
    if limit is not None:
        request["limit"] = limit
    if cursor is not None:
        request["cursor"] = cursor
    return request


def _walk(api: ApiClient, manifest: Manifest) -> list[Mapping[str, Any]]:
    """Every evidence row of the selection, first page to last."""
    rows: list[Mapping[str, Any]] = []
    cursor: str | None = None
    for _ in range(_PAGE_BUDGET):
        response = api.post(
            DRILLDOWN, json_body=_request(manifest, limit=_PAGE_LIMIT, cursor=cursor)
        )
        assert response.status_code == 200, f"status={response.status_code} {response.text[:300]}"
        page = response.parse(MetricDrilldownResponse)
        rows.extend(row.values for row in page.rows)
        cursor = page.next_cursor
        if cursor is None:
            return rows
    pytest.fail(f"{GIT_COMMITS}: still paging after {_PAGE_BUDGET} pages of {_PAGE_LIMIT}")


def _period_value(api: ApiClient, manifest: Manifest) -> float | None:
    """The scalar the dashboard shows for the same selection — the walk's oracle."""
    start, end = query_window(manifest)
    person_id = manifest.fixture("dev_lead").uuid
    response = api.post(
        METRIC_RESULTS,
        json_body={
            "entity": {"type": "person", "ids": [person_id]},
            "period": {"from": start, "to": end},
            "metrics": [{"metric_key": GIT_COMMITS, "filters": [], "views": [{"view": "period"}]}],
        },
    )
    assert response.status_code == 200, f"status={response.status_code} {response.text[:300]}"
    result = response.parse(MetricResultsResponse)
    assert len(result.metrics) == 1
    views = result.metrics[0].root.views
    assert len(views) == 1
    assert isinstance(views[0].root, PeriodView)
    values = views[0].root.values
    assert len(values) == 1
    assert values[0].entity_id == person_id
    return values[0].value


@pytest.mark.rebuild_lane
@pytest.mark.requires_seed("dev_lead")
def test_a_rebuild_between_pages_expires_the_cursor_and_never_mixes_builds(
    api: ApiClient, stand_manifest: Manifest
) -> None:
    """#1603 scenario 7 — a walk interrupted by a real evidence rebuild.

    Sequence: take page one of `git.commits` at limit 1 (so a continuation
    token must exist), rebuild `git_metric_evidence` through the seed image,
    then present the pre-rebuild token. The service must refuse with the
    documented failed-precondition — 400 carrying `EVIDENCE_SNAPSHOT_EXPIRED`
    in a problem document — because honouring it would resume a row order the
    new build no longer defines: the mixed-builds page scenario 7 forbids.

    The refusal is also the proof the rebuild ROTATED the snapshot: had the
    swap kept the table UUID, the stale token would answer 200 and fail here.

    Then the recovery a caller is entitled to: a fresh walk (new first page,
    new tokens) completes and still reconciles row-for-row with the metric
    value, which pins two things at once — the rebuilt table serves one
    consistent build, and the stand is content-identical for every test that
    follows this one.
    """
    first = api.post(DRILLDOWN, json_body=_request(stand_manifest, limit=1))
    assert first.status_code == 200, f"status={first.status_code} {first.text[:300]}"
    page = first.parse(MetricDrilldownResponse)
    assert page.rows, f"{GIT_COMMITS}: the seed guarantees dev_lead commit evidence"
    assert page.next_cursor is not None, (
        f"{GIT_COMMITS}: a one-row page of a multi-row selection must continue — "
        "without a token there is no in-flight walk to interrupt"
    )
    pre_rebuild_row = page.rows[0].values

    _rebuild_evidence_relation()

    # The finally holds the equivalence proof: the rebuild has already mutated
    # the stand, so whether or not the refusal fires as documented, the run
    # must still establish that other tests face the same content — a refusal
    # failure alone would otherwise leave that unverified exactly when it
    # matters most. A failure inside finally supersedes the refusal failure,
    # which is the right precedence: changed content invalidates more.
    try:
        stale = api.post(
            DRILLDOWN, json_body=_request(stand_manifest, limit=1, cursor=page.next_cursor)
        )
        assert stale.status_code == 400, (
            f"a pre-rebuild cursor answered {stale.status_code}: {stale.text[:300]} — a 200 "
            "here is a page resumed across two builds"
        )
        assert stale.parse(ProblemDocument).status == 400
        assert "EVIDENCE_SNAPSHOT_EXPIRED" in stale.text, (
            f"refused, but not with the documented precondition: {stale.text[:300]}"
        )
    finally:
        rows = _walk(api, stand_manifest)
        assert pre_rebuild_row in rows, (
            "the pre-rebuild first row is gone from the rebuilt evidence — the rebuild "
            "changed content, so the stand is no longer the one other tests were written "
            "against"
        )
        period = _period_value(api, stand_manifest)
        assert period == len(rows), (
            f"{GIT_COMMITS}: {len(rows)} evidence rows against a metric value of {period} "
            "after the rebuild"
        )
