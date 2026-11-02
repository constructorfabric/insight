from __future__ import annotations

import os
import subprocess
import sys
from pathlib import Path

import pytest

ROOT = Path(__file__).resolve().parents[1]


@pytest.mark.parametrize(
    ("annotation", "descriptor", "origin", "schedule"),
    [
        ("0 1 * * *", "schedule: '0 2 * * *'", "annotation", "0 1 * * *"),
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
    script = f"""
    set -euo pipefail
    source "{ROOT}/lib/connector-naming.sh"
    kubectl() {{ printf '%s' "$EXAMPLE_SCHEDULE"; }}
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
