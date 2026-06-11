"""Unit tests for reconcile's parse_descriptor.py fallback YAML reader.

The script prefers PyYAML but ships a minimal hand-rolled parser for hosts
without it — dev-up.sh runs it with the system python3 and feeds
`images.enrich.image` straight to docker build, which rejects refs that
still carry the YAML quotes (`invalid reference format`). These tests pin
the fallback's quote handling to what yaml.safe_load produces.

The fallback is only defined when `import yaml` fails, so the module is
loaded with a poisoned sys.modules entry (None ⇒ ImportError) to force
that branch even though the e2e runner has PyYAML installed.
"""

from __future__ import annotations

import importlib.util
import sys
from pathlib import Path

import pytest
import yaml

pytestmark = pytest.mark.smoke

_REPO_ROOT = Path(__file__).resolve().parents[5]
_SCRIPT = (
    _REPO_ROOT / "src/ingestion/reconcile-connectors/python/parse_descriptor.py"
)

# Mirrors the shapes used by real descriptors (see the jira connector):
# double/single-quoted scalars, quoted scalars containing `:`/`*`, nested
# blocks, plain scalars, and flow-style lists with quoted items.
_DESCRIPTOR = """\
name: jira
version: "1.2.0"
schedule: "0 3 * * *"
images:
  enrich:
    name: insight-jira-enrich
    image: "ghcr.io/constructorfabric/insight-jira-enrich:2026.06.10.11.55-376321d"
single: 'quoted'
plain: unquoted
flow: ["a", 'b', c]
"""


def _load_fallback(monkeypatch: pytest.MonkeyPatch):
    """Import parse_descriptor.py with PyYAML unavailable."""
    monkeypatch.setitem(sys.modules, "yaml", None)
    spec = importlib.util.spec_from_file_location(
        "parse_descriptor_fallback_under_test", _SCRIPT
    )
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


@pytest.fixture()
def descriptor(tmp_path: Path) -> Path:
    path = tmp_path / "descriptor.yaml"
    path.write_text(_DESCRIPTOR, encoding="utf-8")
    return path


@pytest.mark.parametrize(
    ("field", "expected"),
    [
        (
            "images.enrich.image",
            "ghcr.io/constructorfabric/insight-jira-enrich:2026.06.10.11.55-376321d",
        ),
        ("version", "1.2.0"),
        ("schedule", "0 3 * * *"),
        ("single", "quoted"),
        ("plain", "unquoted"),
        ("flow", ["a", "b", "c"]),
    ],
)
def test_fallback_strips_yaml_quotes(
    monkeypatch: pytest.MonkeyPatch, descriptor: Path, field: str, expected
) -> None:
    mod = _load_fallback(monkeypatch)
    assert mod._walk(mod._read_yaml(str(descriptor)), field) == expected


def test_fallback_agrees_with_pyyaml(
    monkeypatch: pytest.MonkeyPatch, descriptor: Path
) -> None:
    mod = _load_fallback(monkeypatch)
    assert mod._read_yaml(str(descriptor)) == yaml.safe_load(_DESCRIPTOR)


@pytest.mark.parametrize(
    ("raw", "expected"),
    [
        ('"x"', "x"),
        ("'x'", "x"),
        ("x", "x"),
        ('"x', '"x'),  # unterminated — not a matching pair, leave as-is
        ("x'", "x'"),
        ("\"x'", "\"x'"),  # mismatched pair, leave as-is
        ('"', '"'),  # single character cannot be a pair
        ('""', ""),
    ],
)
def test_unquote_only_strips_matching_pairs(
    monkeypatch: pytest.MonkeyPatch, raw: str, expected: str
) -> None:
    mod = _load_fallback(monkeypatch)
    assert mod._unquote(raw) == expected
