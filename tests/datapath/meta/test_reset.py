"""What a per-spec reset clears, and the instance it refuses to run against."""

from __future__ import annotations

import ast
import re
from pathlib import Path

import pytest
from insight_datapath.reset import (
    SEED_OWNED,
    SERVICE_OWNED,
    SeededWarehouseError,
    refuse_a_seeded_warehouse,
)

REPO_ROOT = Path(__file__).resolve().parents[3]
SEEDER_INSERT = REPO_ROOT / "src/ingestion/tools/seed/insight_seed/generators/insert.py"


def _seeder_reset_targets() -> set[tuple[str, str]]:
    tree = ast.parse(SEEDER_INSERT.read_text(encoding="utf-8"))
    for node in ast.walk(tree):
        if not isinstance(node, ast.AnnAssign) or getattr(node.target, "id", "") != "RESET_TARGETS":
            continue
        if node.value is None:
            break
        return set(ast.literal_eval(node.value))
    raise AssertionError(
        f"{SEEDER_INSERT}: RESET_TARGETS is no longer a literal this test can read"
    )


def test_the_relations_the_stand_seed_owns_are_the_ones_it_declares() -> None:
    """Copied rather than imported — the seeder is not a dependency of this project —
    so the copy is checked against the original instead of trusted."""
    assert _seeder_reset_targets() == SEED_OWNED


def test_an_instance_seeded_with_a_roster_is_refused() -> None:
    """Its people live in the same silver classes a spec builds."""
    with pytest.raises(SeededWarehouseError, match="silver"):
        refuse_a_seeded_warehouse(["identity", "silver"])


def test_the_refusal_names_the_bring_up_that_works() -> None:
    with pytest.raises(SeededWarehouseError, match="test-stand minimal"):
        refuse_a_seeded_warehouse(["identity", "all"])


def test_an_instance_seeded_with_identity_alone_is_ours_to_write() -> None:
    refuse_a_seeded_warehouse(["identity"])


def test_identity_is_never_cleared_because_the_caller_resolves_through_it() -> None:
    assert ("identity", "identity_inputs") in SERVICE_OWNED
    assert ("identity", "identity_persons") in SERVICE_OWNED


def test_the_seeder_still_refuses_to_clear_a_relation_it_never_declared() -> None:
    """The copied list is only meaningful while the seeder keeps a single declared set."""
    source = SEEDER_INSERT.read_text(encoding="utf-8")
    assert re.search(r"if \(schema, table\) not in RESET_TARGETS", source), (
        f"{SEEDER_INSERT}: the seeder no longer gates its truncate on RESET_TARGETS"
    )
