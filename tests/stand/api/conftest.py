"""Fixtures for the endpoint contract tests.

Every resource a case needs is created through the same client the test uses
and removed afterwards, so tests stay one-case and order-independent. The rules
those fixtures obey — and why a stand needs a stricter version of them than the
in-process rig does — are in `scratch.py`.

The teardown deletes are deliberately unchecked. A delete-case test has already
removed its row, so a 404 in teardown is the expected outcome, not a failure to
report.
"""

from __future__ import annotations

import json
from collections.abc import Callable, Iterator
from pathlib import Path

import pytest
from insight_stand import (
    ADMIN_OPERATOR_FIXTURE,
    ApiClient,
    PersonaSession,
    analytics_path,
    artifact_dir,
)

from . import scratch
from .operations import ALL_OPERATIONS
from .schemas import CustomMetric, SavedQuery

#: Written beside the coverage ledger at session end (see the root conftest for
#: the ledger itself). The gate compares the two, so it needs no import from
#: this suite at run time: it is a stdlib script over two JSON files, runnable
#: on a machine with no stand, no uv and no browser.
_ARTIFACTS = artifact_dir(Path(__file__).resolve().parents[3] / ".artifacts")
CATALOGUE_NAME = "stand_operations.json"


def pytest_sessionfinish(session: pytest.Session, exitstatus: int) -> None:
    """Export the operation catalogue, whatever the run's verdict was.

    Unconditionally, for the same reason the ledger is: a failing run's
    catalogue is still the right denominator, and making the gate's input depend
    on the suite's result is backwards.
    """
    del session, exitstatus

    _ARTIFACTS.mkdir(parents=True, exist_ok=True)
    (_ARTIFACTS / CATALOGUE_NAME).write_text(
        json.dumps(
            [
                # Both forms: the gate groups observed calls by `template`, and
                # `path` is what the sweep actually requested. See `Operation`.
                {"method": op.method, "path": op.path, "template": op.template}
                for op in ALL_OPERATIONS
            ],
            indent=2,
        )
        + "\n",
        encoding="utf-8",
    )


@pytest.fixture
def api(lead_session: PersonaSession) -> ApiClient:
    """An authenticated client, for cases that are not about who is calling.

    A lead rather than an admin on purpose: analytics has no admin gate at all
    (`require_admin` appears nowhere in it, and `/v1/admin/*` is admin by name
    only), so an ordinary persona is what the endpoints actually face.
    """
    return lead_session.client


@pytest.fixture
def scratch_saved_query(api: ApiClient) -> Iterator[SavedQuery]:
    """A scratch saved query, hard-deleted afterwards."""
    query = scratch.create_saved_query(api, "query")
    yield query
    api.delete(analytics_path(f"/v1/queries/{query.id}"))


@pytest.fixture
def scratch_custom_metric(api: ApiClient) -> Iterator[CustomMetric]:
    """A scratch custom metric, hard-deleted afterwards."""
    metric = scratch.create_custom_metric(api, "metric")
    yield metric
    api.delete(analytics_path(f"/v1/metrics/{metric.metric_key}"))


@pytest.fixture(scope="session", autouse=True)
def no_scratch_rows_survive(
    session_for: Callable[[str], PersonaSession],
) -> Iterator[None]:
    """Fail the session if any row this run created is still on the stand.

    Without this the mutation policy is a comment. A leaked row does not break
    the run that leaked it — it changes what the NEXT run sees, on a stand only
    reset by `test-stand down`. That is exactly the kind of failure that gets
    diagnosed as flakiness.

    Costs nothing when no scratch resource was created: the registries are empty,
    so it returns before asking `session_for` for anything and the browser
    journey and the 401 sweep never pay for a login they do not need.
    """
    yield

    if not scratch.issued_names() and not scratch.tracked_ids():
        return

    leaked = scratch.surviving_scratch_rows(
        analytics=session_for("dev_lead").client,
        identity=session_for(ADMIN_OPERATOR_FIXTURE).client,
    )
    assert not leaked, (
        "scratch resources survived the run — the stand is now dirty and the "
        "next run against it will see them:\n  " + "\n  ".join(leaked)
    )
