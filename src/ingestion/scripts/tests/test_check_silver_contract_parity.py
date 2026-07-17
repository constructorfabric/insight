#!/usr/bin/env python3
"""Tests for check_silver_contract_parity.py.

Dependency-free: runs under pytest OR standalone (`python3 <thisfile>`), so CI
can exercise it without a pytest install. Covers the comparison logic
(check_connector) and the opt-in / exit-code behaviour (end-to-end via the CLI).
"""

from __future__ import annotations

import json
import subprocess
import sys
import tempfile
from pathlib import Path

_SCRIPT_DIR = Path(__file__).resolve().parents[1]
_SCRIPT = _SCRIPT_DIR / "check_silver_contract_parity.py"
sys.path.insert(0, str(_SCRIPT_DIR))

import check_silver_contract_parity as gate  # noqa: E402


# --- canonical 3-column reference (name, data_type), order significant --------
REF = [("tenant_id", "String"), ("unique_key", "String"), ("files_changed", "Int64")]


def _connector(columns, *, enforced=True):
    return {"columns": list(columns), "enforced": enforced}


# --- check_connector: the comparison core ------------------------------------

def test_in_sync_passes():
    assert gate.check_connector(REF, "gh", _connector(REF)) == []


def test_missing_column_flagged():
    cols = [c for c in REF if c[0] != "files_changed"]
    problems = gate.check_connector(REF, "bb", _connector(cols))
    assert any("MISSING" in p and "files_changed" in p for p in problems)


def test_extra_column_flagged():
    cols = REF + [("co_authors", "String")]
    problems = gate.check_connector(REF, "gh", _connector(cols))
    assert any("UNEXPECTED" in p and "co_authors" in p for p in problems)


def test_type_mismatch_flagged():
    cols = [("tenant_id", "String"), ("unique_key", "String"), ("files_changed", "UInt64")]
    problems = gate.check_connector(REF, "gh", _connector(cols))
    assert any("files_changed" in p and "UInt64" in p and "Int64" in p for p in problems)


def test_order_mismatch_flagged():
    # Same set + same types, different order — must still fail (UNION ALL is positional).
    cols = [("unique_key", "String"), ("tenant_id", "String"), ("files_changed", "Int64")]
    problems = gate.check_connector(REF, "gh", _connector(cols))
    assert any("ORDER" in p for p in problems)
    # ...and a pure reorder must NOT be reported as missing/extra.
    assert not any("MISSING" in p or "UNEXPECTED" in p for p in problems)


def test_unenforced_connector_flagged():
    problems = gate.check_connector(REF, "gh", _connector(REF, enforced=False))
    assert any("not enforced" in p for p in problems)


def test_no_columns_flagged():
    problems = gate.check_connector(REF, "gh", _connector([]))
    assert any("no typed columns" in p for p in problems)


# --- end-to-end via the CLI: opt-in + exit codes -----------------------------

def _node(name, tags, columns, enforced):
    return {
        "resource_type": "model",
        "name": name,
        "tags": tags,
        "contract": {"enforced": enforced},
        "columns": {n: {"data_type": t} for n, t in columns},
    }


def _run(nodes):
    with tempfile.NamedTemporaryFile("w", suffix=".json", delete=False) as f:
        json.dump({"nodes": nodes}, f)
        path = f.name
    try:
        return subprocess.run(
            [sys.executable, str(_SCRIPT), "--manifest", path],
            capture_output=True, text=True,
        )
    finally:
        Path(path).unlink(missing_ok=True)


def test_e2e_enforced_consistent_exit0():
    r = _run({
        "s": _node("class_git_commits", ["silver"], REF, True),
        "gh": _node("github__commits", ["github", "silver:class_git_commits"], REF, True),
        "bb": _node("bitbucket_cloud__commits", ["bitbucket-cloud", "silver:class_git_commits"], REF, True),
    })
    assert r.returncode == 0, r.stdout + r.stderr
    assert "✓ silver:class_git_commits" in r.stdout


def test_e2e_drift_exit1():
    bb_missing = [c for c in REF if c[0] != "files_changed"]
    r = _run({
        "s": _node("class_git_commits", ["silver"], REF, True),
        "gh": _node("github__commits", ["github", "silver:class_git_commits"], REF, True),
        "bb": _node("bitbucket_cloud__commits", ["bitbucket-cloud", "silver:class_git_commits"], bb_missing, True),
    })
    assert r.returncode == 1
    assert "MISSING" in r.stdout


def test_e2e_unenforced_silver_skipped_exit0():
    # Silver model without an enforced contract -> class is skipped, not failed,
    # even though the connectors blatantly disagree.
    r = _run({
        "s": _node("class_git_commits", ["silver"], REF, False),
        "gh": _node("github__commits", ["github", "silver:class_git_commits"], REF, True),
        "bb": _node("bitbucket_cloud__commits", ["bitbucket-cloud", "silver:class_git_commits"], [("x", "String")], True),
    })
    assert r.returncode == 0, r.stdout + r.stderr
    assert "skipped" in r.stdout


def _main() -> int:
    tests = [v for k, v in sorted(globals().items()) if k.startswith("test_") and callable(v)]
    failed = 0
    for t in tests:
        try:
            t()
            print(f"ok   {t.__name__}")
        except AssertionError as e:
            failed += 1
            print(f"FAIL {t.__name__}: {e}")
    print(f"\n{len(tests) - failed}/{len(tests)} passed")
    return 1 if failed else 0


if __name__ == "__main__":
    raise SystemExit(_main())
