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
# blocks, plain scalars, flow-style lists with quoted items, block-style
# lists (secret.required_fields) with plain and quoted items, and trailing
# inline comments (github-copilot/claude-enterprise quote their schedule
# and follow it with `# daily at ...`).
_DESCRIPTOR = """\
name: jira
version: "1.2.0"
schedule: "0 3 * * *" # daily at 03:00 UTC
hash_inside: "a # b"   # trailing comment after a value containing '#'
plain_commented: unquoted # note
images:
  enrich:
    name: insight-jira-enrich
    image: "ghcr.io/constructorfabric/insight-jira-enrich:2026.06.10.11.55-376321d"
single: 'quoted'
plain: unquoted
flow: ["a", 'b', c]
secret:
  required_fields:
    - jira_instance_url
    - "jira_email"
    - 'jira_api_token'
    - jira_project_keys
after_block: still-parsed
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
        ("hash_inside", "a # b"),
        ("plain_commented", "unquoted"),
        ("single", "quoted"),
        ("plain", "unquoted"),
        ("flow", ["a", "b", "c"]),
        (
            "secret.required_fields",
            [
                "jira_instance_url",
                "jira_email",
                "jira_api_token",
                "jira_project_keys",
            ],
        ),
        ("after_block", "still-parsed"),
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


_CORPUS = sorted((_REPO_ROOT / "src/ingestion/connectors").glob("*/*/descriptor.yaml"))


@pytest.mark.parametrize(
    "path", _CORPUS, ids=lambda p: f"{p.parent.parent.name}/{p.parent.name}"
)
def test_fallback_agrees_with_pyyaml_on_real_corpus(
    monkeypatch: pytest.MonkeyPatch, path: Path
) -> None:
    """Full-corpus parity: every shipped descriptor must parse identically
    under the fallback and yaml.safe_load. github-copilot and
    claude-enterprise carry trailing inline comments after their quoted
    `schedule` values, which the fallback used to return verbatim."""
    mod = _load_fallback(monkeypatch)
    assert mod._read_yaml(str(path)) == yaml.safe_load(
        path.read_text(encoding="utf-8")
    )


def test_corpus_glob_finds_descriptors() -> None:
    """Guard the parity sweep against silently going empty if the
    connectors tree moves."""
    assert len(_CORPUS) >= 5


def test_fallback_parses_real_jira_descriptor(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    """The repro from the bug report: jira's secret.required_fields block
    list came back as {} from the fallback while PyYAML returned the list."""
    mod = _load_fallback(monkeypatch)
    path = _REPO_ROOT / "src/ingestion/connectors/task-tracking/jira/descriptor.yaml"
    parsed = mod._read_yaml(str(path))
    assert parsed == yaml.safe_load(path.read_text(encoding="utf-8"))
    assert parsed["secret"]["required_fields"] == [
        "jira_instance_url",
        "jira_email",
        "jira_api_token",
        "jira_project_keys",
    ]


@pytest.mark.parametrize(
    "doc",
    [
        "- orphan\n",  # list item with no key above it
        "key: scalar\n- orphan\n",  # list item under an already-valued key
    ],
)
def test_fallback_rejects_orphan_list_items(
    monkeypatch: pytest.MonkeyPatch, tmp_path: Path, doc: str
) -> None:
    """Fail-fast convention (cypilot/config/rules/code-conventions.md):
    constructs the fallback cannot represent must raise, not silently drop."""
    mod = _load_fallback(monkeypatch)
    path = tmp_path / "descriptor.yaml"
    path.write_text(doc, encoding="utf-8")
    with pytest.raises(ValueError, match="install PyYAML"):
        mod._read_yaml(str(path))


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
