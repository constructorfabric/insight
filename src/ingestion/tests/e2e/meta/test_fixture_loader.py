"""Unit tests for the fixture loader — no compose / no CH / no analytics-api.

These tests use synthetic fixtures written to tmp_path. They run fast and
do not depend on the data plane being up.
"""

from __future__ import annotations

from pathlib import Path

import pytest

from e2e_lib.fixture_loader import (
    Fixture,
    FixtureError,
    SpecYaml,
    discover_all,
    load,
)


pytestmark = pytest.mark.smoke


# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------


def _write_fixture(
    root: Path,
    *,
    spec_yaml: str | None = None,
    bronze_files: dict[str, str] | None = None,
    expected_csv: str | None = None,
) -> Path:
    """Write a synthetic fixture folder to root and return its path."""
    fixture = root / "test_fixture"
    fixture.mkdir(parents=True)
    if spec_yaml is not None:
        (fixture / "spec.yaml").write_text(spec_yaml)
    if bronze_files is not None:
        (fixture / "bronze").mkdir()
        for name, body in bronze_files.items():
            (fixture / "bronze" / name).write_text(body)
    if expected_csv is not None:
        (fixture / "expected").mkdir()
        (fixture / "expected" / "response.csv").write_text(expected_csv)
    return fixture


_VALID_SPEC = """\
spec_version: 1
description: smoke
endpoint: /v1/metrics/{metric_id}/query
method: POST
metric_id: 00000000-0000-0000-0000-000000000001
request_body:
  $top: 25
dbt_selector: +silver_people+
key_columns: [person_id]
float_tolerance: 0.001
"""

_VALID_EXPECTED = "person_id,display_name,job_title\nalice,Alice,Eng\nbob,Bob,PM\n"


# ---------------------------------------------------------------------------
# Happy path
# ---------------------------------------------------------------------------


def test_loads_valid_fixture(tmp_path: Path) -> None:
    p = _write_fixture(
        tmp_path,
        spec_yaml=_VALID_SPEC,
        bronze_files={"bronze_bamboohr.employees.csv": "id,name\n1,Alice\n"},
        expected_csv=_VALID_EXPECTED,
    )
    fx = load(p)

    assert isinstance(fx, Fixture)
    assert fx.name == "test_fixture"
    assert fx.spec.spec_version == 1
    assert fx.spec.dbt_selector == "+silver_people+"
    assert fx.spec.key_columns == ["person_id"]
    assert fx.spec.float_tolerance == 0.001
    assert len(fx.bronze_csvs) == 1
    assert fx.bronze_csvs[0].schema == "bronze_bamboohr"
    assert fx.bronze_csvs[0].table == "employees"
    assert list(fx.expected_df.columns) == ["person_id", "display_name", "job_title"]
    assert len(fx.expected_df) == 2


def test_resolved_endpoint_interpolates_metric_id(tmp_path: Path) -> None:
    p = _write_fixture(
        tmp_path,
        spec_yaml=_VALID_SPEC,
        bronze_files={"bronze_bamboohr.employees.csv": "id\n1\n"},
        expected_csv=_VALID_EXPECTED,
    )
    fx = load(p)
    assert fx.spec.resolved_endpoint() == "/v1/metrics/00000000-0000-0000-0000-000000000001/query"


def test_touched_schemas(tmp_path: Path) -> None:
    p = _write_fixture(
        tmp_path,
        spec_yaml=_VALID_SPEC,
        bronze_files={
            "bronze_bamboohr.employees.csv": "id\n1\n",
            "bronze_jira.issues.csv": "id\n2\n",
        },
        expected_csv=_VALID_EXPECTED,
    )
    fx = load(p)
    assert fx.touched_schemas == {"bronze_bamboohr", "bronze_jira"}


# ---------------------------------------------------------------------------
# Error paths — every misshape MUST raise FixtureError at load time
# ---------------------------------------------------------------------------


def test_missing_directory(tmp_path: Path) -> None:
    with pytest.raises(FixtureError, match="not found"):
        load(tmp_path / "nope")


def test_missing_spec_yaml(tmp_path: Path) -> None:
    p = _write_fixture(
        tmp_path,
        spec_yaml=None,
        bronze_files={"bronze_x.y.csv": "id\n1\n"},
        expected_csv="id\n1\n",
    )
    with pytest.raises(FixtureError, match="missing spec.yaml"):
        load(p)


def test_invalid_yaml(tmp_path: Path) -> None:
    p = _write_fixture(
        tmp_path,
        spec_yaml="spec_version: [unclosed",
        bronze_files={"bronze_x.y.csv": "id\n1\n"},
        expected_csv="id\n1\n",
    )
    with pytest.raises(FixtureError, match="invalid YAML"):
        load(p)


def test_unknown_spec_version(tmp_path: Path) -> None:
    bad = _VALID_SPEC.replace("spec_version: 1", "spec_version: 99")
    p = _write_fixture(
        tmp_path,
        spec_yaml=bad,
        bronze_files={"bronze_x.y.csv": "id\n1\n"},
        expected_csv=_VALID_EXPECTED,
    )
    with pytest.raises(FixtureError, match="schema"):
        load(p)


def test_missing_required_key(tmp_path: Path) -> None:
    bad = _VALID_SPEC.replace("endpoint: /v1/metrics/{metric_id}/query\n", "")
    p = _write_fixture(
        tmp_path,
        spec_yaml=bad,
        bronze_files={"bronze_x.y.csv": "id\n1\n"},
        expected_csv=_VALID_EXPECTED,
    )
    with pytest.raises(FixtureError, match="endpoint"):
        load(p)


def test_dbt_selector_is_optional(tmp_path: Path) -> None:
    """View-only metrics (e.g. insight.people) don't need dbt; selector may be absent."""
    no_selector = _VALID_SPEC.replace("dbt_selector: +silver_people+\n", "")
    p = _write_fixture(
        tmp_path,
        spec_yaml=no_selector,
        bronze_files={"bronze_x.y.csv": "id\n1\n"},
        expected_csv=_VALID_EXPECTED,
    )
    fx = load(p)
    assert fx.spec.dbt_selector is None


def test_bronze_csv_naming(tmp_path: Path) -> None:
    p = _write_fixture(
        tmp_path,
        spec_yaml=_VALID_SPEC,
        bronze_files={"badname.csv": "id\n1\n"},
        expected_csv=_VALID_EXPECTED,
    )
    with pytest.raises(FixtureError, match="must match"):
        load(p)


def test_empty_bronze_dir(tmp_path: Path) -> None:
    p = _write_fixture(
        tmp_path,
        spec_yaml=_VALID_SPEC,
        bronze_files={},
        expected_csv=_VALID_EXPECTED,
    )
    with pytest.raises(FixtureError, match="no bronze CSV"):
        load(p)


def test_missing_expected_csv(tmp_path: Path) -> None:
    p = _write_fixture(
        tmp_path,
        spec_yaml=_VALID_SPEC,
        bronze_files={"bronze_x.y.csv": "id\n1\n"},
        expected_csv=None,
    )
    with pytest.raises(FixtureError, match="missing expected"):
        load(p)


def test_key_column_missing_in_expected(tmp_path: Path) -> None:
    p = _write_fixture(
        tmp_path,
        spec_yaml=_VALID_SPEC,
        bronze_files={"bronze_x.y.csv": "id\n1\n"},
        expected_csv="display_name,job_title\nAlice,Eng\n",  # no person_id
    )
    with pytest.raises(FixtureError, match="key_columns missing"):
        load(p)


# ---------------------------------------------------------------------------
# discover_all
# ---------------------------------------------------------------------------


def test_discover_all_skips_dot_and_underscore(tmp_path: Path) -> None:
    (tmp_path / "alpha").mkdir()
    (tmp_path / "beta").mkdir()
    (tmp_path / "_helpers").mkdir()
    (tmp_path / ".hidden").mkdir()
    (tmp_path / "README.md").touch()

    found = discover_all(tmp_path)
    names = [p.name for p in found]
    assert names == ["alpha", "beta"]


def test_discover_all_missing_returns_empty(tmp_path: Path) -> None:
    assert discover_all(tmp_path / "nope") == []
