"""The parsed dbt project as the graph a change travels through.

Built from `manifest.json` alone, so the leg selection can run before any stand
exists: `dbt parse` needs no warehouse.
"""

from __future__ import annotations

import posixpath
from collections.abc import Iterable, Mapping
from dataclasses import dataclass
from typing import Any

#: Where `dbt_project.yml` lives; every manifest path is relative to it.
PROJECT_DIR = "src/ingestion/dbt"

GOLD_TAG = "gold"
#: Unions every model carrying a tag with a positional `SELECT *`, so any member
#: decides the column layout every other member's rows land in.
UNION_MACRO = "union_by_tag"


@dataclass(frozen=True)
class DbtGraph:
    children: Mapping[str, frozenset[str]]
    parents: Mapping[str, frozenset[str]]
    #: Union model -> the models it unions.
    unions: Mapping[str, frozenset[str]]
    #: Repository path -> the nodes and sources a file defines or patches.
    by_path: Mapping[str, frozenset[str]]
    #: Repository path -> the project macros a file defines.
    macros_by_path: Mapping[str, frozenset[str]]
    #: Macro -> every node and hook that calls it, directly or through other macros.
    macro_callers: Mapping[str, frozenset[str]]
    #: Files dbt parses that build nothing: docs blocks and disabled nodes.
    inert_paths: frozenset[str]
    #: `(schema, identifier)` -> the source reading that relation.
    sources_by_relation: Mapping[tuple[str, str], str]
    #: Repository path of the yml that declares each source.
    source_paths: Mapping[str, str]
    gold: frozenset[str]
    hooks: frozenset[str]
    #: Nodes every spec reads whatever it seeds: dbt seeds, and models whose
    #: ancestry holds no source because they read raw relations.
    shared_roots: frozenset[str]


def repo_path(manifest_path: str) -> str:
    return posixpath.normpath(posixpath.join(PROJECT_DIR, manifest_path))


def _patch_path(node: Mapping[str, Any]) -> str | None:
    patch = node.get("patch_path")
    if not patch:
        return None
    return repo_path(patch.split("://", 1)[-1])


def _group(pairs: Iterable[tuple[str, str]]) -> dict[str, frozenset[str]]:
    grouped: dict[str, set[str]] = {}
    for key, value in pairs:
        grouped.setdefault(key, set()).add(value)
    return {key: frozenset(values) for key, values in grouped.items()}


def closure(start: Iterable[str], edges: Mapping[str, Iterable[str]]) -> frozenset[str]:
    seen = set(start)
    pending = list(seen)
    while pending:
        for neighbour in edges.get(pending.pop(), ()):
            if neighbour not in seen:
                seen.add(neighbour)
                pending.append(neighbour)
    return frozenset(seen)


def _macro_callers(manifest: Mapping[str, Any], project: str) -> dict[str, frozenset[str]]:
    called_by: dict[str, set[str]] = {}
    for macro_id, macro in manifest.get("macros", {}).items():
        for callee in macro.get("depends_on", {}).get("macros", []):
            called_by.setdefault(callee, set()).add(macro_id)
    direct: dict[str, set[str]] = {}
    for node_id, node in manifest.get("nodes", {}).items():
        for callee in node.get("depends_on", {}).get("macros", []):
            direct.setdefault(callee, set()).add(node_id)

    callers: dict[str, frozenset[str]] = {}
    for macro_id, macro in manifest.get("macros", {}).items():
        if macro.get("package_name") != project:
            continue
        macros = closure([macro_id], called_by)
        callers[macro_id] = frozenset(node for m in macros for node in direct.get(m, ()))
    return callers


def _shared_roots(
    manifest: Mapping[str, Any], parents: Mapping[str, Iterable[str]]
) -> frozenset[str]:
    nodes = manifest.get("nodes", {})
    seeds = {node_id for node_id, node in nodes.items() if node.get("resource_type") == "seed"}
    rootless = {
        node_id
        for node_id, node in nodes.items()
        if node.get("resource_type") == "model"
        and not any(ancestor.startswith("source.") for ancestor in closure([node_id], parents))
    }
    return frozenset(seeds | rootless)


def from_manifest(manifest: Mapping[str, Any]) -> DbtGraph:
    project = manifest["metadata"]["project_name"]
    nodes: Mapping[str, Any] = manifest.get("nodes", {})
    sources: Mapping[str, Any] = manifest.get("sources", {})

    children = {key: frozenset(value) for key, value in manifest.get("child_map", {}).items()}
    parents = {key: frozenset(value) for key, value in manifest.get("parent_map", {}).items()}

    defined = [
        (repo_path(entry["original_file_path"]), entry_id)
        for entry_id, entry in {**nodes, **sources}.items()
        if entry.get("package_name") == project
    ]
    patched = [
        (patch, node_id)
        for node_id, node in nodes.items()
        if (patch := _patch_path(node)) is not None
    ]
    macros = [
        (repo_path(macro["original_file_path"]), macro_id)
        for macro_id, macro in manifest.get("macros", {}).items()
        if macro.get("package_name") == project
    ]
    docs = {
        repo_path(doc["original_file_path"])
        for doc in manifest.get("docs", {}).values()
        if doc.get("package_name") == project
    }
    disabled = {
        repo_path(entry["original_file_path"])
        for entries in manifest.get("disabled", {}).values()
        for entry in entries
        if entry.get("package_name") == project
    }

    macro_callers = _macro_callers(manifest, project)
    unions = {
        union: frozenset(member for member in parents.get(union, ()) if member.startswith("model."))
        for union in macro_callers.get(f"macro.{project}.{UNION_MACRO}", frozenset())
        if union.startswith("model.")
    }

    return DbtGraph(
        children=children,
        parents=parents,
        unions=unions,
        by_path=_group(defined + patched),
        macros_by_path=_group(macros),
        macro_callers=macro_callers,
        inert_paths=frozenset(docs | disabled),
        sources_by_relation={
            (source["schema"], source.get("identifier") or source["name"]): source_id
            for source_id, source in sources.items()
        },
        source_paths={
            source_id: repo_path(source["original_file_path"])
            for source_id, source in sources.items()
        },
        gold=frozenset(
            node_id for node_id, node in nodes.items() if GOLD_TAG in node.get("tags", [])
        ),
        hooks=frozenset(
            node_id for node_id, node in nodes.items() if node.get("resource_type") == "operation"
        ),
        shared_roots=_shared_roots(manifest, parents),
    )
