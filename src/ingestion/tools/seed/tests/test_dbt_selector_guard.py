from __future__ import annotations

import pathlib
import re

from insight_seed import silver
from insight_seed.generators.insert import RESET_TARGETS

_CONNECTORS = pathlib.Path(silver.__file__).resolve().parents[3] / "connectors"
GENERATORS = pathlib.Path(silver.__file__).resolve().parent / "generators"
_TAGS = re.compile(r"tags=\[([^\]]*)\]", re.S)


def _models_for(selector: str) -> list[str]:
    """The connector models a selector names: one, or every model with a tag."""
    body = selector.strip("+")
    if not body.startswith("tag:"):
        return [body]
    tag = body.removeprefix("tag:")
    return sorted(name for name, tags in _model_tags().items() if tag in tags)


def _model_tags() -> dict[str, set[str]]:
    found: dict[str, set[str]] = {}
    for path in _CONNECTORS.rglob("dbt/*.sql"):
        match = _TAGS.search(path.read_text())
        if match:
            found[path.stem] = set(re.findall(r"'([^']+)'", match.group(1)))
    return found


def test_no_generator_writes_a_silver_relation() -> None:
    """Every silver relation is dbt's to build, so a selector can never
    destroy seeded rows — which is what this guard used to check one model at
    a time. A generator writing silver again brings that hazard back: dbt
    would rebuild the relation from staging and replace what it wrote.

    """
    writers = {
        path.name: path.read_text(encoding="utf-8").count('bulk_insert(client, "silver"')
        for path in sorted(GENERATORS.glob("*.py"))
    }

    offenders = {name: count for name, count in writers.items() if count}
    assert not offenders, (
        f"these generators write silver directly: {offenders}. Every silver relation is "
        f"built by dbt from bronze; a direct write is replaced by the next build."
    )


def test_the_reset_registry_still_covers_silver() -> None:
    """The relations are cleared, not written: a re-seed at another anchor
    would otherwise leave the previous window behind an incremental watermark."""
    cleared = {table for schema, table in RESET_TARGETS if schema == "silver"}

    assert len(cleared) >= 30, f"only {len(cleared)} silver relations are cleared"


def test_the_selector_names_only_models_that_exist() -> None:
    known = _model_tags()
    selectors = [
        *silver.CI_CHAIN_SELECT,
        *silver.SUPPORT_CHAIN_SELECT,
        *silver.WIKI_CHAIN_SELECT,
        *silver.CRM_CHAIN_SELECT,
        *silver.COLLAB_CHAIN_SELECT,
        *silver.HR_CHAIN_SELECT,
        *silver.AI_CHAIN_SELECT,
        *silver.GIT_CHAIN_SELECT,
        *silver.TASK_CHAIN_SELECT,
        silver.AI_INVOICE_SELECT,
    ]
    missing = [name for s in selectors for name in _models_for(s) if name not in known]
    assert not missing, f"selector names models that do not exist: {missing}"
