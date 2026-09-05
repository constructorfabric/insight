"""Record composition via `$ref` + sibling overrides.

Implements `cpt-bronze-to-api-e2e-algo-yaml-resolve-refs`. Pure (the only I/O is
reading referenced YAML files, which are cached). A record is a mapping that may
carry a `$ref: "<file>#/<json-pointer>"`; sibling keys override the resolved base
(closest layer wins). A `$ref` resolves relative to the file it is written in, and
the base resolves in *its own* file's context — so a `#/...` ref inside
`templates/people.yaml` stays local to that file even when referenced from a test.

See FEATURE `feature-yaml-rig` DoD `cpt-bronze-to-api-e2e-dod-yaml-ref-resolution`
for the 12 invariants exercised by `meta/test_ref_resolver.py`.
"""

from __future__ import annotations

import logging
from pathlib import Path
from typing import Any

import yaml

LOG = logging.getLogger("e2e.ref")


class RefError(ValueError):
    """Unresolvable `$ref`, missing pointer, or a reference cycle."""


_FILE_CACHE: dict[Path, Any] = {}


def _load_yaml(path: Path) -> Any:
    path = path.resolve()
    if path not in _FILE_CACHE:
        if not path.is_file():
            raise RefError(f"$ref target file not found: {path}")
        try:
            _FILE_CACHE[path] = yaml.safe_load(path.read_text(encoding="utf-8"))
        except yaml.YAMLError as e:  # pragma: no cover - defensive
            raise RefError(f"{path}: invalid YAML: {e}") from e
    return _FILE_CACHE[path]


def _json_pointer(doc: Any, pointer: str, *, ref: str) -> Any:
    """Navigate a `/a/b/c` JSON pointer. Empty pointer → whole doc."""
    node = doc
    for raw in [p for p in pointer.split("/") if p != ""]:
        token = raw.replace("~1", "/").replace("~0", "~")
        if isinstance(node, dict) and token in node:
            node = node[token]
        elif isinstance(node, list) and token.isdigit() and int(token) < len(node):
            node = node[int(token)]
        else:
            raise RefError(f"$ref pointer not found: '{ref}' (missing segment '{token}')")
    return node


def _deep_merge(base: Any, override: Any) -> Any:
    """map+map → recursive key-wise merge (override wins); otherwise override replaces."""
    if isinstance(base, dict) and isinstance(override, dict):
        out = dict(base)
        for k, v in override.items():
            out[k] = _deep_merge(out[k], v) if k in out else v
        return out
    return override


def resolve(node: Any, ctx_file: Path, _stack: tuple[tuple[Path, str], ...] = ()) -> Any:
    """Resolve `$ref`s in `node`, written in `ctx_file`. Returns a plain value."""
    if isinstance(node, list):
        return [resolve(v, ctx_file, _stack) for v in node]
    if not isinstance(node, dict):
        return node

    if "$ref" not in node:
        return {k: resolve(v, ctx_file, _stack) for k, v in node.items()}

    ref = node["$ref"]
    file_part, _, pointer = str(ref).partition("#")
    target_file = ctx_file if file_part == "" else (ctx_file.parent / file_part)
    target_file = target_file.resolve()

    key = (target_file, pointer)
    if key in _stack:
        chain = " → ".join(f"{f.name}#{p}" for f, p in (*_stack, key))
        raise RefError(f"$ref cycle: {chain}")

    target = _json_pointer(_load_yaml(target_file), pointer, ref=str(ref))
    base = resolve(target, target_file, (*_stack, key))
    overrides = {k: resolve(v, ctx_file, _stack) for k, v in node.items() if k != "$ref"}
    return _deep_merge(base, overrides)
