"""What each data-path suite seeds and which templates it builds rows from, read off its tree.

The leg planner trusts this reading, and `seed_audit` checks it against what a run
actually seeded.
"""

from __future__ import annotations

import ast
import re
from collections.abc import Iterable, Mapping
from dataclasses import dataclass
from pathlib import Path

import yaml

DATAPATH = "tests/datapath"
METRICS = f"{DATAPATH}/metrics"
IDENTITY_TREE = f"{DATAPATH}/identity"
TEMPLATES = f"{METRICS}/templates"
LIBRARY = "tests/lib/insight_datapath"

IDENTITY = "identity"
SHARED_METRICS_DIRS = frozenset({"schemas", "templates"})

TABLE_LITERAL = re.compile(r"""["']((?:bronze_[a-z0-9_]+|config)\.[a-z0-9_]+)["']""")
REF = re.compile(r"""\$ref:\s*["']?([^#"'\s}]+)""")
TEMPLATE_LITERAL = re.compile(r"""["'](?:templates/)?([a-z0-9_]+\.yaml)#""")
LIBRARY_IMPORT = re.compile(r"""insight_datapath(?:\.(\w+)|\s+import\s+([\w, ]+))""")
SEED_CALL = "seed_records"

Relation = tuple[str, str]


@dataclass(frozen=True)
class Suites:
    metric_classes: tuple[str, ...]
    #: Suite (a metric class or `identity`) -> the relations its tests seed.
    seeds: Mapping[str, frozenset[Relation]]
    #: Repository path of a template -> the suites that build rows from it.
    template_users: Mapping[str, frozenset[str]]
    #: Modules that seed a relation this reading cannot name.
    opaque: tuple[str, ...] = ()


def _relation(fqn: str) -> Relation:
    schema, _, table = fqn.partition(".")
    return schema, table


def _string_constants(module: ast.Module) -> dict[str, str]:
    return {
        target.id: node.value.value
        for node in module.body
        if isinstance(node, ast.Assign)
        and isinstance(node.value, ast.Constant)
        and isinstance(node.value.value, str)
        for target in node.targets
        if isinstance(target, ast.Name)
    }


def _called(call: ast.Call) -> str | None:
    if isinstance(call.func, ast.Attribute):
        return call.func.attr
    if isinstance(call.func, ast.Name):
        return call.func.id
    return None


def _string(node: ast.expr, constants: Mapping[str, str]) -> str | None:
    if isinstance(node, ast.Constant) and isinstance(node.value, str):
        return node.value
    if isinstance(node, ast.Name):
        return constants.get(node.id)
    return None


def seed_calls(source: str) -> tuple[frozenset[Relation], bool]:
    """The relations a module's `seed_records(schema, table, ...)` calls name, and
    whether every such call could be read."""
    module = ast.parse(source)
    constants = _string_constants(module)
    relations: set[Relation] = set()
    readable = True
    for node in ast.walk(module):
        if not isinstance(node, ast.Call) or _called(node) != SEED_CALL or len(node.args) < 2:
            continue
        schema = _string(node.args[0], constants)
        table = _string(node.args[1], constants)
        if schema is None or table is None:
            readable = False
            continue
        relations.add((schema, table))
    return frozenset(relations), readable


def _seeded_by_tree(tree: Path, opaque: list[str], repo_root: Path) -> frozenset[Relation]:
    tables: set[Relation] = set()
    for spec in sorted(tree.rglob("*.test.yaml")):
        document = yaml.safe_load(spec.read_text(encoding="utf-8")) or {}
        tables.update(_relation(fqn) for fqn in document.get("bronze") or {})
    for module in sorted(tree.rglob("*.py")):
        source = module.read_text(encoding="utf-8")
        tables.update(_relation(fqn) for fqn in TABLE_LITERAL.findall(source))
        called, readable = seed_calls(source)
        tables.update(called)
        if not readable:
            opaque.append(module.relative_to(repo_root).as_posix())
    return frozenset(tables)


def _imported_library_modules(source: str) -> set[str]:
    names: set[str] = set()
    for dotted, listed in LIBRARY_IMPORT.findall(source):
        names.update(name.strip() for name in (dotted, *listed.split(",")) if name.strip())
    return names


def _library_templates(repo_root: Path) -> dict[str, set[str]]:
    return {
        module.stem: set(TEMPLATE_LITERAL.findall(module.read_text(encoding="utf-8")))
        for module in (repo_root / LIBRARY).glob("*.py")
    }


def _template_users(repo_root: Path, trees: Mapping[str, Path]) -> dict[str, frozenset[str]]:
    library = _library_templates(repo_root)
    users: dict[str, set[str]] = {}

    def use(template: str, suite: str) -> None:
        users.setdefault(f"{TEMPLATES}/{template}", set()).add(suite)

    for suite, tree in trees.items():
        for spec in tree.rglob("*.test.yaml"):
            for ref in REF.findall(spec.read_text(encoding="utf-8")):
                target = (spec.parent / ref).resolve().relative_to(repo_root.resolve())
                users.setdefault(target.as_posix(), set()).add(suite)
        for module in tree.rglob("*.py"):
            source = module.read_text(encoding="utf-8")
            for template in TEMPLATE_LITERAL.findall(source):
                use(template, suite)
            for imported in _imported_library_modules(source):
                for template in library.get(imported, ()):
                    use(template, suite)
    return {path: frozenset(names) for path, names in users.items()}


def metric_classes(repo_root: Path) -> tuple[str, ...]:
    return tuple(
        sorted(
            entry.name
            for entry in (repo_root / METRICS).iterdir()
            if entry.is_dir()
            and entry.name not in SHARED_METRICS_DIRS
            and not entry.name.startswith(("_", "."))
        )
    )


def suite_of(test_path: Path, repo_root: Path) -> str | None:
    """The suite a test file belongs to: its metric class, `identity`, or none."""
    parts = test_path.resolve().relative_to(repo_root.resolve()).parts
    if parts[:3] == ("tests", "datapath", "identity"):
        return IDENTITY
    if parts[:3] == ("tests", "datapath", "metrics") and len(parts) > 4:
        return parts[3] if parts[3] not in SHARED_METRICS_DIRS else None
    return None


def scan_suites(repo_root: Path) -> Suites:
    classes = metric_classes(repo_root)
    trees = {name: repo_root / METRICS / name for name in classes}
    trees[IDENTITY] = repo_root / IDENTITY_TREE
    opaque: list[str] = []
    seeds = {suite: _seeded_by_tree(tree, opaque, repo_root) for suite, tree in trees.items()}
    return Suites(
        metric_classes=classes,
        seeds=seeds,
        template_users=_template_users(repo_root, trees),
        opaque=tuple(opaque),
    )


def unplanned(
    seeded: Mapping[str, Iterable[Relation]], planned: Mapping[str, frozenset[Relation]]
) -> dict[str, list[Relation]]:
    """Relations a suite seeded that its tree reading did not name."""
    missing = {
        suite: sorted(set(relations) - planned.get(suite, frozenset()))
        for suite, relations in seeded.items()
    }
    return {suite: relations for suite, relations in missing.items() if relations}
