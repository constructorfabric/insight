"""Quality-vector marker semantics, shared by the gate and its regression net.

The five vector NAMES live in one place — the marker declarations in
`tests/pyproject.toml` — and the exactly-one-vector rule is enforced in two:
the collection gate in `tests/stand/conftest.py` and the in-session meta test
`tests/stand/meta/test_vector_markers.py`. Both call these helpers so they
cannot disagree on which names are vectors, which items the rule governs, or
how markers are counted.

Framework-agnostic like the rest of the package: callers hand in the marker
declarations (`config.getini("markers")`), the item's path and its marker
names; nothing here imports pytest.
"""

from __future__ import annotations

from collections.abc import Iterable
from pathlib import PurePath

#: The tag marker declarations in tests/pyproject.toml carry so the vector set
#: can be derived rather than re-declared.
VECTOR_DECLARATION_TAG = "quality vector"


def quality_vectors(marker_declarations: Iterable[str]) -> frozenset[str]:
    """Vector names, parsed from the pyproject marker declarations."""
    return frozenset(
        declaration.split(":", 1)[0].strip()
        for declaration in marker_declarations
        if VECTOR_DECLARATION_TAG in declaration
    )


def governs_vector(path: PurePath) -> bool:
    """True for the files the exactly-one-vector rule applies to (api/ui).

    Compared in POSIX form so the predicate holds on a Windows checkout, where
    ``str(path)`` uses backslashes and a slash-substring test matches nothing.
    """
    posix = path.as_posix()
    return "/stand/api/" in posix or "/stand/ui/" in posix


def distinct_vectors(marker_names: Iterable[str], vectors: frozenset[str]) -> set[str]:
    """The DISTINCT vector names among an item's markers — the counted unit.

    A set on purpose: a module ``pytestmark`` plus a redundant per-test marker
    of the SAME vector still selects soundly under ``-m``, so only distinct
    vectors count toward the exactly-one rule.
    """
    return {name for name in marker_names if name in vectors}
