"""Every connector descriptor's `dbt_select` must name work dbt can actually do.

The chart's post-sync step runs `dbt run --select <dbt_select>`. dbt exits non-zero
when a selector matches no node, so a descriptor whose only tagged model was deleted --
or whose tag was never carried by one -- turns every sync of that connector red (#2362)
while the repository itself builds and tests clean.

The graph is read off the committed model files rather than a compiled manifest, so
this costs a glob and needs no warehouse. Only the selector forms descriptors actually
use are understood; anything else is refused by name rather than passed silently.
"""

from __future__ import annotations

import re
from dataclasses import dataclass
from pathlib import Path

import pytest
import yaml

REPO_ROOT = Path(__file__).resolve().parents[3]
DBT_PROJECT = REPO_ROOT / "src/ingestion/dbt/dbt_project.yml"
CONNECTORS_DIR = REPO_ROOT / "src/ingestion/connectors"

_TAGS = re.compile(r"tags\s*=\s*\[([^\]]*)\]", re.DOTALL)
_QUOTED = re.compile(r"'([^']+)'")
_REF = re.compile(r"ref\(\s*'([^']+)'\s*\)")
_TAG_ATOM = re.compile(r"^tag:(?P<tag>[^\s,+]+)(?P<descendants>\+?)$")


class UnsupportedSelectorError(Exception):
    """A selector form this guard cannot resolve, and so must not vouch for."""


@dataclass(frozen=True)
class Model:
    name: str
    tags: frozenset[str]
    refs: frozenset[str]


@dataclass(frozen=True)
class Descriptor:
    connector: str
    selector: str


def _model_paths() -> list[Path]:
    """The trees dbt itself reads models from, as its project file declares them."""
    project = yaml.safe_load(DBT_PROJECT.read_text(encoding="utf-8"))
    return [(DBT_PROJECT.parent / declared).resolve() for declared in project["model-paths"]]


def _models() -> dict[str, Model]:
    found: dict[str, Model] = {}
    for root in _model_paths():
        for path in sorted(root.rglob("*.sql")):
            body = path.read_text(encoding="utf-8")
            tags = _TAGS.search(body)
            found[path.stem] = Model(
                name=path.stem,
                tags=frozenset(_QUOTED.findall(tags.group(1)) if tags else ()),
                refs=frozenset(_REF.findall(body)),
            )
    return found


def _children(models: dict[str, Model]) -> dict[str, set[str]]:
    child_map: dict[str, set[str]] = {name: set() for name in models}
    for model in models.values():
        for parent in model.refs & models.keys():
            child_map[parent].add(model.name)
    return child_map


def _descriptors() -> list[Descriptor]:
    """Every connector declaring a selector; one declaring none is nothing to check."""
    declared: list[Descriptor] = []
    for path in sorted(CONNECTORS_DIR.glob("*/*/descriptor.yaml")):
        document = yaml.safe_load(path.read_text(encoding="utf-8")) or {}
        selector = (document.get("dbt_select") or "").strip()
        if selector:
            declared.append(Descriptor(connector=path.parent.name, selector=selector))
    return declared


MODELS = _models()
CHILDREN = _children(MODELS)
DESCRIPTORS = _descriptors()


def _tagged(tag: str) -> set[str]:
    return {name for name, model in MODELS.items() if tag in model.tags}


def _descendants(seed: set[str]) -> set[str]:
    reached: set[str] = set()
    stack = list(seed)
    while stack:
        for child in CHILDREN.get(stack.pop(), ()):
            if child not in reached:
                reached.add(child)
                stack.append(child)
    return reached


def _atom_tag(atom: str) -> str:
    """The tag an atom selects on, refusing every form this guard cannot resolve."""
    match = _TAG_ATOM.match(atom)
    if not match:
        raise UnsupportedSelectorError(
            f"cannot resolve {atom!r}: this guard understands `tag:<name>` and "
            "`tag:<name>+` only, so teach it the new form rather than trusting it"
        )
    return match.group("tag")


def _atoms(selector: str) -> list[str]:
    return [atom for term in selector.split() for atom in term.split(",")]


def _selected(selector: str) -> set[str]:
    """`tag:X` atoms, `+` for descendants, comma for AND, space for OR -- dbt's rules."""
    union: set[str] = set()
    for term in selector.split():
        intersection: set[str] | None = None
        for atom in term.split(","):
            base = _tagged(_atom_tag(atom))
            reached = base | _descendants(base) if atom.endswith("+") else base
            intersection = reached if intersection is None else intersection & reached
        union |= intersection or set()
    return union


def _ids(descriptors: list[Descriptor]) -> list[str]:
    return [descriptor.connector for descriptor in descriptors]


def test_the_model_graph_parses() -> None:
    """A graph this file cannot read would make every rule below vacuous."""
    assert len(MODELS) > 100, f"only {len(MODELS)} dbt models parsed out of {_model_paths()}"
    assert any(model.tags for model in MODELS.values()), "no model carries any tag"
    assert any(CHILDREN.values()), "no model refs another, so `+` would reach nothing"
    assert len(DESCRIPTORS) > 10, f"only {len(DESCRIPTORS)} descriptors declare a selector"


@pytest.mark.parametrize("descriptor", DESCRIPTORS, ids=_ids(DESCRIPTORS))
def test_a_descriptor_selector_uses_a_form_this_guard_can_resolve(
    descriptor: Descriptor,
) -> None:
    """A form the guard silently mis-parses would let the rules below vouch for a
    selector they never evaluated."""
    for atom in _atoms(descriptor.selector):
        try:
            _atom_tag(atom)
        except UnsupportedSelectorError as unsupported:
            pytest.fail(f"{descriptor.connector}: {unsupported}")


@pytest.mark.parametrize("descriptor", DESCRIPTORS, ids=_ids(DESCRIPTORS))
def test_a_descriptor_selector_names_only_tags_a_model_carries(
    descriptor: Descriptor,
) -> None:
    """An atom no model answers to empties the whole term it sits in -- comma is AND --
    so the term is dead weight whether or not a sibling term rescues the run."""
    tags = {_atom_tag(atom) for atom in _atoms(descriptor.selector)}
    unknown = sorted(tag for tag in tags if not _tagged(tag))

    assert unknown == [], f"should reject {descriptor.selector!r}: no dbt model carries {unknown}"


@pytest.mark.parametrize("descriptor", DESCRIPTORS, ids=_ids(DESCRIPTORS))
def test_a_descriptor_selector_matches_at_least_one_model(descriptor: Descriptor) -> None:
    """dbt exits non-zero on a selector that matches nothing, so the chart's post-sync
    step fails every sync of the connector while the repository builds clean."""
    selected = _selected(descriptor.selector)
    assert selected, (
        f"should reject {descriptor.selector!r}: it selects no dbt model, so "
        f"`dbt run --select {descriptor.selector}` fails every {descriptor.connector} sync"
    )
