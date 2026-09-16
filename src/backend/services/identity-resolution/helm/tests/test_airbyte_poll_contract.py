from __future__ import annotations

import pytest
from conftest import TENANT, UMBRELLA, UMBRELLA_BASE, render


@pytest.mark.parametrize("threshold", [None, 3600])
def test_poller_bounds_unreadability_without_a_job_deadline(umbrella_deps, threshold: int | None) -> None:
    overrides = (
        [] if threshold is None else ["--set", f"ingestion.airbyteSync.statusUnreadableThresholdSeconds={threshold}"]
    )
    code, out, err = render(
        UMBRELLA,
        *UMBRELLA_BASE,
        "--set",
        f"global.tenantDefaultId={TENANT}",
        "--set",
        "ingestion.templates.enabled=true",
        *overrides,
        "--show-only",
        "templates/ingestion/airbyte-sync.yaml",
    )
    assert code == 0, err
    assert "activeDeadlineSeconds:" not in out
    assert "IDLE_THRESHOLD_SECONDS" not in out
    assert f'- name: STATUS_UNREADABLE_THRESHOLD_SECONDS\n            value: "{threshold or 1800}"' in out
