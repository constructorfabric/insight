from __future__ import annotations

import subprocess
from pathlib import Path

import pytest
import yaml

HERE = Path(__file__).resolve()
SUBCHART = HERE.parents[1]

SUBCHART_BASE = [
    "--set",
    "image.tag=0.0.0-test",
    "--set",
    "existingSecret=test-secret",
    "--set",
    "gateway.issuer=https://issuer.test",
]


def _render(*extra: str) -> tuple[int, str, str]:
    proc = subprocess.run(  # noqa: S603 — test-controlled argv
        ["helm", "template", "contract-test", str(SUBCHART), *SUBCHART_BASE, *extra],  # noqa: S607
        capture_output=True,
        text=True,
        timeout=120,
        check=False,
    )
    return proc.returncode, proc.stdout, proc.stderr


def _gear_config(*extra: str) -> dict:
    code, out, err = _render(*extra)
    assert code == 0, f"render failed: {err}"

    docs = [d for d in yaml.safe_load_all(out) if isinstance(d, dict)]
    configmaps = [d for d in docs if d.get("kind") == "ConfigMap"]
    assert len(configmaps) == 1, f"expected one ConfigMap, got {len(configmaps)}"

    host = yaml.safe_load(configmaps[0]["data"]["insight.yaml"])
    return host["gears"]["identity-resolution"]["config"]


def test_org_chart_knobs_default_into_the_gear_config() -> None:
    config = _gear_config()

    assert config["org_chart_source_type"] == "bamboohr"
    assert config["expand_subordinates"] is True
    assert config["max_depth"] == 16


@pytest.mark.parametrize(
    ("flag", "key", "expected"),
    [
        ("orgChartSourceType=github", "org_chart_source_type", "github"),
        ("expandSubordinates=false", "expand_subordinates", False),
        ("maxDepth=4", "max_depth", 4),
    ],
)
def test_a_top_level_override_reaches_the_gear_config(
    flag: str, key: str, expected: object
) -> None:
    config = _gear_config("--set", flag)

    assert config[key] == expected, f"should honour --set {flag}"

@pytest.mark.parametrize(
    ("flag", "expected"),
    [(None, "org_chart"), ("visibilityPolicy=flat", "flat"), ("visibilityPolicy=org_chart", "org_chart")],
)
def test_the_visibility_policy_reaches_the_gear_config(
    flag: str | None, expected: str
) -> None:
    config = _gear_config(*(("--set", flag) if flag else ()))

    assert config["visibility_policy"] == expected


def test_the_visibility_policy_is_never_rendered_empty() -> None:
    config = _gear_config("--set", "visibilityPolicy=")

    assert config["visibility_policy"] == "org_chart"
