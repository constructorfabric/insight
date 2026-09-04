"""What a per-spec reset clears, and what it refuses to touch."""

from __future__ import annotations

import ast
import re
from pathlib import Path

import pytest
from insight_datapath.reset import PROTECTED, SEED_OWNED, ProtectedRelationError, refuse_protected

REPO_ROOT = Path(__file__).resolve().parents[3]
SEEDER_INSERT = REPO_ROOT / "src/ingestion/tools/seed/insight_seed/generators/insert.py"


def _seeder_reset_targets() -> set[tuple[str, str]]:
    source = SEEDER_INSERT.read_text(encoding="utf-8")
    tree = ast.parse(source)
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


def test_a_spec_that_would_write_the_stand_s_own_rows_is_refused() -> None:
    with pytest.raises(ProtectedRelationError, match=r"silver\.class_people"):
        refuse_protected([("bronze_jira", "issues"), ("silver", "class_people")])


def test_the_refusal_says_how_to_bring_up_an_instance_that_works() -> None:
    with pytest.raises(ProtectedRelationError, match="test-stand minimal"):
        refuse_protected([("silver", "class_people")])


def test_relations_no_seed_owns_pass_the_check() -> None:
    refuse_protected(
        [("bronze_jira", "issues"), ("staging", "jira__issues"), ("silver", "class_tasks")]
    )


def test_identity_is_protected_because_the_caller_resolves_through_it() -> None:
    assert ("identity", "identity_inputs") in PROTECTED
    assert ("identity", "identity_persons") in PROTECTED


def test_the_seeder_still_refuses_to_clear_a_relation_it_never_declared() -> None:
    """The copied list is only meaningful while the seeder keeps a single declared set."""
    source = SEEDER_INSERT.read_text(encoding="utf-8")
    assert re.search(r"if \(schema, table\) not in RESET_TARGETS", source), (
        f"{SEEDER_INSERT}: the seeder no longer gates its truncate on RESET_TARGETS"
    )
