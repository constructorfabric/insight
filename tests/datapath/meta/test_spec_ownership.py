"""Every metric spec is claimed by exactly one module, and every claim names a spec.

A spec reaches a run only through a module's `SPEC`. An unclaimed one is silently inert:
its bronze is never seeded, its assertions never run, and no collection error says so.
"""

from __future__ import annotations

import re
from pathlib import Path

from insight_datapath.fixture_loader import discover_tests

METRICS_ROOT = Path(__file__).resolve().parents[1] / "metrics"

_DECLARATION = re.compile(r"^SPEC = [\"'](?P<name>[^\"']+)[\"']", re.MULTILINE)


def _claims() -> dict[Path, list[Path]]:
    """Spec path -> the modules naming it, whether or not that path exists."""
    claimed: dict[Path, list[Path]] = {}
    for module in sorted(METRICS_ROOT.rglob("test_*.py")):
        declaration = _DECLARATION.search(module.read_text(encoding="utf-8"))
        if declaration is None:
            continue
        spec = module.parent / f"{declaration['name']}.test.yaml"
        claimed.setdefault(spec, []).append(module)
    return claimed


def test_every_spec_is_claimed_by_a_module() -> None:
    claimed = _claims()
    unclaimed = [
        spec.relative_to(METRICS_ROOT)
        for spec in discover_tests(METRICS_ROOT)
        if spec not in claimed
    ]
    assert not unclaimed, f"specs no module names in SPEC, so nothing runs them: {unclaimed}"


def test_a_spec_is_claimed_by_no_more_than_one_module() -> None:
    shared = {
        str(spec.relative_to(METRICS_ROOT)): [str(m.relative_to(METRICS_ROOT)) for m in modules]
        for spec, modules in _claims().items()
        if len(modules) > 1
    }
    assert not shared, f"one spec claimed by several modules, which would seed it twice: {shared}"


def test_every_claim_names_a_spec_beside_its_module() -> None:
    dangling = {
        str(module.relative_to(METRICS_ROOT)): str(spec.relative_to(METRICS_ROOT))
        for spec, modules in _claims().items()
        for module in modules
        if not spec.exists()
    }
    assert not dangling, f"SPEC names a file that is not there: {dangling}"
