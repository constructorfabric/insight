"""Tests for reconcile's parse_descriptor.py.

parse_descriptor reads a dotted field from a connector descriptor.yaml. It
requires PyYAML -- every caller runs where it is present (the reconcile
toolbox image ships it via dbt-clickhouse; CI installs it explicitly). The
old hand-rolled fallback (for the retired host-python dev-up.sh path) was
removed, so these tests pin the dotted-path extraction, the shape of every
shipped descriptor, and the fail-fast when PyYAML is absent.
"""

from __future__ import annotations

import importlib.util
import os
import subprocess
import sys
from pathlib import Path

import pytest

pytestmark = pytest.mark.smoke

_REPO_ROOT = Path(__file__).resolve().parents[5]
_SCRIPT = (
    _REPO_ROOT / "src/ingestion/reconcile-connectors/python/parse_descriptor.py"
)
_CORPUS = sorted((_REPO_ROOT / "src/ingestion/connectors").glob("*/*/descriptor.yaml"))
_JIRA = _REPO_ROOT / "src/ingestion/connectors/task-tracking/jira/descriptor.yaml"


def _load():
    spec = importlib.util.spec_from_file_location(
        "parse_descriptor_under_test", _SCRIPT
    )
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


_MOD = _load()


@pytest.mark.parametrize(
    "path", _CORPUS, ids=lambda p: f"{p.parent.parent.name}/{p.parent.name}"
)
def test_every_descriptor_parses_to_a_dict(path: Path) -> None:
    """Every shipped descriptor must load to a non-empty dict -- catches a
    malformed/unparseable descriptor before reconcile consumes it."""
    doc = _MOD._read_yaml(str(path))
    assert isinstance(doc, dict) and doc


def test_corpus_glob_finds_descriptors() -> None:
    """Guard the corpus sweep against silently going empty if the connectors
    tree moves."""
    assert len(_CORPUS) >= 5


@pytest.mark.parametrize(
    ("field", "expected"),
    [
        ("name", "jira"),
        # The CONN-11 repro: the block-style list must resolve to the list,
        # not {} -- i.e. secret validation sees the real required fields.
        (
            "secret.required_fields",
            ["jira_instance_url", "jira_email", "jira_api_token"],
        ),
        ("does.not.exist", None),  # missing top-level path
        ("secret.nope", None),  # missing leaf under an existing dict
        ("name.deeper", None),  # walking past a scalar
    ],
)
def test_walk_dotted_paths(field, expected) -> None:
    assert _MOD._walk(_MOD._read_yaml(str(_JIRA)), field) == expected


def test_fail_fast_without_pyyaml(tmp_path: Path) -> None:
    """Without PyYAML the script must exit 2 with an actionable message --
    not crash with a raw ImportError and not silently mis-parse. PyYAML is a
    hard requirement now that the hand-rolled fallback is gone."""
    shadow = tmp_path / "shadow"
    shadow.mkdir()
    (shadow / "yaml.py").write_text(
        "raise ImportError('forced: no pyyaml')\n", encoding="utf-8"
    )
    env = {**os.environ, "PYTHONPATH": str(shadow)}
    proc = subprocess.run(
        [sys.executable, str(_SCRIPT), "--descriptor", str(_JIRA), "--field", "name"],
        env=env,
        capture_output=True,
        text=True,
    )
    assert proc.returncode == 2, proc.stderr
    assert "PyYAML" in proc.stderr
