from __future__ import annotations

import pathlib
import re

import pytest

from insight_seed import silver
from insight_seed.generators.insert import RESET_TARGETS

_CONNECTORS = pathlib.Path(silver.__file__).resolve().parents[3] / "connectors"
_TAGS = re.compile(r"tags=\[([^\]]*)\]", re.S)


def _model_tags() -> dict[str, set[str]]:
    found: dict[str, set[str]] = {}
    for path in _CONNECTORS.rglob("dbt/*.sql"):
        match = _TAGS.search(path.read_text())
        if match:
            found[path.stem] = set(re.findall(r"'([^']+)'", match.group(1)))
    return found


def _directly_seeded_silver() -> set[str]:
    """Silver relations the generators write themselves."""
    return {table for schema, table in RESET_TARGETS if schema == "silver"}


@pytest.mark.parametrize("selector", silver.CI_CHAIN_SELECT)
def test_no_selected_model_feeds_a_directly_seeded_silver_relation(selector: str) -> None:
    """The on-run-start placeholder hook drops a silver placeholder once any
    staging model for its tag is materialised. Selecting a model that feeds a
    relation the generators write directly therefore destroys seeded rows on the
    next run — silently, and only on the second seed."""
    model = selector.rstrip("+")
    tags = _model_tags().get(model)
    assert tags is not None, f"{model} is not a connector dbt model"

    fed = {t.removeprefix("silver:") for t in tags if t.startswith("silver:")}
    seeded_directly = _directly_seeded_silver()
    collisions = fed & (
        seeded_directly
        - {
            "class_git_ci_runs",
            "class_git_deployments",
            "class_git_deployment_events",
            "class_git_repositories",
        }
    )
    assert not collisions, (
        f"{model} feeds {sorted(collisions)}, which the generators write directly; "
        "building it from staging would replace the seeded rows with an empty union"
    )


def test_the_selector_names_only_models_that_exist() -> None:
    known = _model_tags()
    missing = [s.rstrip("+") for s in silver.CI_CHAIN_SELECT if s.rstrip("+") not in known]
    assert not missing, f"selector names models that do not exist: {missing}"
