from __future__ import annotations

import os
import shlex
import subprocess
import sys
from pathlib import Path

import pytest

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "python"))

from sweep.schedule import AppliedSchedule, read_schedule

ROOT = Path(__file__).resolve().parents[1]


@pytest.mark.parametrize("suspended", [True, False])
def test_recorded_suspension_crosses_into_the_ledger(suspended: bool) -> None:
    work = {
        "schedules": ["0 5 * * *"],
        "schedule_timezone": "UTC",
        "schedule_origin": "annotation",
        "schedule_suspended": suspended,
        "inherited_schedules": [],
        "inherited_schedule_origin": "",
    }
    assert read_schedule(work).row_fields() == work


@pytest.mark.parametrize("suspended", [None, "false", 0, 1, [], {}])
def test_unreadable_suspension_leaves_the_schedule_unknown(suspended: object) -> None:
    work = {"schedules": ["0 5 * * *"], "schedule_suspended": suspended}
    assert read_schedule(work) == AppliedSchedule(), f"should reject: {suspended!r}"


def test_unrecorded_suspension_defaults_to_false() -> None:
    assert read_schedule({"schedules": ["0 5 * * *"]}).suspended is False


@pytest.mark.parametrize(
    ("annotation", "descriptor", "origin", "schedule"),
    [
        ("0 1 * * *\n0 13 * * *", "schedule: '0 2 * * *'", "annotation", "0 1 * * *\n0 13 * * *"),
        ("", "schedule: '0 2 * * *'", "descriptor", "0 2 * * *"),
        ("", "schedule:\n  - '0 2 * * *'\n  - '0 3 * * *'", "descriptor", "0 2 * * *\n0 3 * * *"),
        ("", "", "default", "0 0 * * *"),
        ("", "schedule: ''", "default", "0 0 * * *"),
    ],
)
def test_schedule_and_origin_share_the_same_precedence(
    tmp_path: Path, annotation: str, descriptor: str, origin: str, schedule: str
) -> None:
    directory = tmp_path / "example-area" / "example-connector"
    directory.mkdir(parents=True)
    (directory / "descriptor.yaml").write_text(f"name: example-connector\n{descriptor}\n")
    lookup = [
        "-n",
        "example-namespace",
        "get",
        "secret",
        "example-secret",
        "-o",
        r"jsonpath={.metadata.annotations.insight\.cyberfabric\.com/schedule}",
    ]
    script = f"""
    set -euo pipefail
    source "{ROOT}/lib/connector-naming.sh"
    kubectl() {{
      [[ "$#" -eq 7 && "$*" == {shlex.quote(" ".join(lookup))} ]] || return 1
      printf '%s' "$EXAMPLE_SCHEDULE"
    }}
    reconcile_resolve_schedule example-connector example-secret
    printf '\\n'
    reconcile_compute_schedule example-connector example-secret
    """
    result = subprocess.run(
        ["bash", "-c", script],
        env={
            **os.environ,
            "INSIGHT_NAMESPACE": "example-namespace",
            "CONNECTORS_DIR": str(tmp_path),
            "EXAMPLE_SCHEDULE": annotation,
            "PATH": f"{Path(sys.executable).parent}{os.pathsep}{os.environ['PATH']}",
        },
        capture_output=True,
        text=True,
        check=False,
    )

    assert result.returncode == 0, result.stderr
    assert result.stdout == f"{origin}\n{schedule}\n{schedule}"
