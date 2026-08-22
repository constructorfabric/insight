"""The manifest this package produces is the one the stand suite can parse.

The shape is stated twice: `insight_seed.manifest.Manifest` here, and the
frozen dataclasses in `tests/lib/insight_stand/manifest.py` the stand suite
parses with. Nothing else makes the two meet, so a field added or retyped on
one side shows up as a `ManifestError` in a stand run rather than at build
time. These tests are where they meet.

Read the other tree rather than depend on it: the stand library is a separate
project and the seed image ships none of it — which is a skip, not an error.

Run against the installed package (see the README's develop section):

    uv run --extra dev pytest tests
"""

from __future__ import annotations

import importlib.util
import json
import pathlib
import sys
import types
from typing import Any

import pytest

from insight_seed import config, manifest

_EMAIL = "dev@company.nonpresent"
_TENANT = "00000000-df51-5b42-9538-d2b56b7ee953"


def _find_upwards(*parts: str) -> pathlib.Path | None:
    for parent in pathlib.Path(__file__).resolve().parents:
        candidate = parent.joinpath(*parts)
        if candidate.is_file():
            return candidate
        if (parent / ".git").exists():
            break
    return None


def _load_stand_reader() -> Any:
    """`insight_stand.manifest`, loaded without running its package `__init__`.

    WORKAROUND: importing the package executes `api.py`, which needs httpx and
    pydantic — dependencies of that project, not of this one.
    """
    reader = _find_upwards("tests", "lib", "insight_stand", "manifest.py")
    if reader is None:
        pytest.skip("tests/lib/insight_stand/manifest.py is not in this tree")

    package = "_insight_stand_contract"
    if package not in sys.modules:
        shim = types.ModuleType(package)
        shim.__path__ = [str(reader.parent)]
        sys.modules[package] = shim
        try:
            for name in ("errors", "manifest"):
                spec = importlib.util.spec_from_file_location(
                    f"{package}.{name}", reader.parent / f"{name}.py"
                )
                if spec is None or spec.loader is None:
                    raise ImportError(f"cannot load {name}.py from the stand library")
                module = importlib.util.module_from_spec(spec)
                sys.modules[spec.name] = module
                spec.loader.exec_module(module)
        except Exception:
            # SAFETY: leave nothing half-registered, or the next caller gets a
            # KeyError from this shim instead of the failure that caused it.
            for key in (package, f"{package}.errors", f"{package}.manifest"):
                sys.modules.pop(key, None)
            raise
    return sys.modules[f"{package}.manifest"]


def _env(headcount: str | None = None) -> dict[str, str]:
    env = {
        "DEV_USER_EMAIL": _EMAIL,
        "TENANT_DEFAULT_ID": _TENANT,
        "SEED_ANCHOR_DATE": "2026-06-30",
        "SEED_DAYS": "60",
        config.CROSS_TENANT_FIXTURE_ENV: "1",
    }
    if headcount is not None:
        env[config.ORG_HEADCOUNT_ENV] = headcount
    return env


def test_the_declared_model_is_the_document_that_is_built() -> None:
    """A TypedDict mypy accepts can still not be the dict the code returns."""
    doc = manifest.build_manifest(_env())

    assert set(doc) == set(manifest.Manifest.__annotations__)
    persona_fields = set(manifest.PersonaEntry.__annotations__)
    assert set(doc["personas"][0]) == persona_fields, "persona drifted from PersonaEntry"
    assert set(next(iter(doc["fixtures"].values()))) == persona_fields, "fixture ref drifted"
    # Spelled out rather than looped: a TypedDict is only indexable by literal.
    assert set(doc["capabilities"]) == set(manifest.Capabilities.__annotations__)
    assert set(doc["realm"]) == set(manifest.RealmRef.__annotations__)
    assert set(doc["tenants"]) == set(manifest.TenantRefs.__annotations__)


@pytest.mark.parametrize("headcount", [None, "250"])
def test_the_stand_suite_parses_what_this_package_produces(headcount: str | None) -> None:
    """Through JSON, because that is how the document reaches the suite."""
    reader = _load_stand_reader()
    doc = json.loads(json.dumps(manifest.build_manifest(_env(headcount))))

    parsed = reader.Manifest.parse(doc, source_path=pathlib.Path("contract-test"))

    assert parsed.manifest_version == manifest.MANIFEST_VERSION
    assert len(parsed.personas) == len(doc["personas"])
    assert parsed.fixtures.keys() == doc["fixtures"].keys()


def test_both_sides_agree_on_the_supported_version() -> None:
    reader = _load_stand_reader()

    assert manifest.MANIFEST_VERSION == reader.SUPPORTED_MANIFEST_VERSION


#: The published field set. A change here is a wire change: bump
#: MANIFEST_VERSION and SUPPORTED_MANIFEST_VERSION together, or readers built
#: against the old set parse a document that no longer matches it.
_PUBLISHED_FIELDS = (
    "manifest_version",
    "tenant",
    "tenants",
    "realm",
    "personas",
    "service_urls",
    "fixtures",
    "capabilities",
    "seed_revision",
    "data_window",
    "anchor_date",
    "seeded",
)


def test_the_published_field_set_has_not_changed_without_a_version_bump() -> None:
    """The version gate is coarse: it fires on a bump, not on a silent change."""
    assert tuple(manifest.Manifest.__annotations__) == _PUBLISHED_FIELDS
