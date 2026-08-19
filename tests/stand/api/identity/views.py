"""Reading helpers over the generated identity models.

The models in `schemas/identity.py` are generated from the service's own OpenAPI
document, so they carry the contract's fields and nothing else. The behaviour the
suite used to reach for as methods — walking a subtree, collecting its emails,
asking whether a temporal row is still in force — lives here instead, as free
functions over those models.

Deliberately not subclasses. A wrapper model would have to re-declare the fields
it wants to keep, and a hand-maintained field list beside a generated one is the
drift the generator exists to prevent. These are the suite's conveniences, not
part of the contract, and the split says so.
"""

from __future__ import annotations

from typing import Protocol

from ..schemas import Subchart, SubchartForest, SubchartNode


def walk(node: SubchartNode) -> list[SubchartNode]:
    """This node and every descendant, at any depth."""
    found = [node]
    for child in node.subordinates:
        found += walk(child)
    return found


def node_emails(node: SubchartNode) -> set[str]:
    """Every email in this subtree — the shape scope assertions compare."""
    return {found.email for found in walk(node) if found.email}


def forest_emails(forest: SubchartForest) -> set[str]:
    return {email for root in forest.roots for email in node_emails(root)}


def subchart_emails(subchart: Subchart) -> set[str]:
    return node_emails(subchart.root)


class Temporal(Protocol):
    """A row closed by setting `valid_to` rather than by deletion — what role
    assignments and visibility grants have in common."""

    valid_to: str | None


def in_force(row: Temporal) -> bool:
    """Revocation is a `valid_to` stamp, so an open end is what "still applies"
    means for both journals."""
    return row.valid_to is None
