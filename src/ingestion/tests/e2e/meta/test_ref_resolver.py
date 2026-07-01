"""Unit tests for ref_resolver — the 12 invariants of
`cpt-bronze-to-api-e2e-dod-yaml-ref-resolution`. Pure: no ClickHouse / dbt.
"""

from __future__ import annotations

from pathlib import Path

import pytest

from lib import ref_resolver
from lib.ref_resolver import RefError, resolve


pytestmark = pytest.mark.smoke


@pytest.fixture(autouse=True)
def _clear_cache():
    ref_resolver._FILE_CACHE.clear()
    yield
    ref_resolver._FILE_CACHE.clear()


def _write(path: Path, text: str) -> Path:
    path.write_text(text, encoding="utf-8")
    return path


# 1. local pointer
def test_local_pointer(tmp_path: Path) -> None:
    f = _write(tmp_path / "t.yaml", "templates:\n  base: {a: 1, b: 2}\n")
    out = resolve({"$ref": "#/templates/base"}, f)
    assert out == {"a": 1, "b": 2}


# 2. cross-file pointer
def test_cross_file_pointer(tmp_path: Path) -> None:
    _write(tmp_path / "people.yaml", "templates:\n  alice: {name: Alice}\n")
    t = _write(tmp_path / "t.yaml", "x: 1\n")
    out = resolve({"$ref": "people.yaml#/templates/alice"}, t)
    assert out == {"name": "Alice"}


# 3. sibling scalar overrides base
def test_sibling_overrides_scalar(tmp_path: Path) -> None:
    f = _write(tmp_path / "t.yaml", "templates:\n  base: {a: 1, b: 2}\n")
    out = resolve({"$ref": "#/templates/base", "b": 99}, f)
    assert out == {"a": 1, "b": 99}


# 4. sibling adds a new field
def test_sibling_adds_field(tmp_path: Path) -> None:
    f = _write(tmp_path / "t.yaml", "templates:\n  base: {a: 1}\n")
    out = resolve({"$ref": "#/templates/base", "c": 3}, f)
    assert out == {"a": 1, "c": 3}


# 5. sibling null overrides a non-null base value
def test_sibling_null_overrides(tmp_path: Path) -> None:
    f = _write(tmp_path / "t.yaml", "templates:\n  base: {a: 1}\n")
    out = resolve({"$ref": "#/templates/base", "a": None}, f)
    assert out == {"a": None}


# 6. chain A -> B -> C, closest wins
def test_chain_closest_wins(tmp_path: Path) -> None:
    f = _write(
        tmp_path / "t.yaml",
        "templates:\n"
        "  c: {v: 'C', only_c: 1}\n"
        "  b: {$ref: '#/templates/c', v: 'B'}\n"
        "  a: {$ref: '#/templates/b', v: 'A'}\n",
    )
    out = resolve({"$ref": "#/templates/a"}, f)
    assert out == {"v": "A", "only_c": 1}


# 7. nested $ref resolves relative to its OWN file
def test_nested_ref_relative_to_own_file(tmp_path: Path) -> None:
    _write(
        tmp_path / "people.yaml",
        "templates:\n"
        "  emp: {dept: Eng}\n"
        "  alice: {$ref: '#/templates/emp', name: Alice}\n",
    )
    t = _write(tmp_path / "t.yaml", "x: 1\n")
    # `alice` (in people.yaml) references `#/templates/emp` which exists ONLY in
    # people.yaml — must resolve there even though we call from t.yaml.
    out = resolve({"$ref": "people.yaml#/templates/alice"}, t)
    assert out == {"dept": "Eng", "name": "Alice"}


# 8. nested maps deep-merge
def test_deep_merge_nested_maps(tmp_path: Path) -> None:
    f = _write(tmp_path / "t.yaml", "templates:\n  base: {m: {a: 1, b: 2}}\n")
    out = resolve({"$ref": "#/templates/base", "m": {"b": 20, "c": 3}}, f)
    assert out == {"m": {"a": 1, "b": 20, "c": 3}}


# 9. missing file
def test_missing_file_raises(tmp_path: Path) -> None:
    t = _write(tmp_path / "t.yaml", "x: 1\n")
    with pytest.raises(RefError, match="not found"):
        resolve({"$ref": "nope.yaml#/templates/x"}, t)


# 10. missing pointer
def test_missing_pointer_raises(tmp_path: Path) -> None:
    f = _write(tmp_path / "t.yaml", "templates:\n  base: {a: 1}\n")
    with pytest.raises(RefError, match="pointer not found"):
        resolve({"$ref": "#/templates/ghost"}, f)


# 11. cycle A -> B -> A
def test_cycle_raises(tmp_path: Path) -> None:
    f = _write(
        tmp_path / "t.yaml",
        "templates:\n"
        "  a: {$ref: '#/templates/b'}\n"
        "  b: {$ref: '#/templates/a'}\n",
    )
    with pytest.raises(RefError, match="cycle"):
        resolve({"$ref": "#/templates/a"}, f)


# 12b. $ref inside a list value is resolved element-wise
def test_ref_inside_list(tmp_path: Path) -> None:
    f = _write(tmp_path / "t.yaml", "templates:\n  base: {x: 1}\n")
    out = resolve({"rows": [{"$ref": "#/templates/base"}, {"y": 2}]}, f)
    assert out == {"rows": [{"x": 1}, {"y": 2}]}


# 12c. a top-level list of refs resolves
def test_top_level_list_of_refs(tmp_path: Path) -> None:
    f = _write(tmp_path / "t.yaml", "templates:\n  base: {x: 1}\n")
    out = resolve([{"$ref": "#/templates/base", "x": 9}, {"z": 3}], f)
    assert out == [{"x": 9}, {"z": 3}]


# 12. two-layer override (ref to a record that itself has $ref + siblings)
def test_two_layer_override(tmp_path: Path) -> None:
    f = _write(
        tmp_path / "t.yaml",
        "templates:\n"
        "  base: {a: 1, b: 1, c: 1}\n"
        "  mid: {$ref: '#/templates/base', b: 2}\n",
    )
    out = resolve({"$ref": "#/templates/mid", "c": 3}, f)
    assert out == {"a": 1, "b": 2, "c": 3}
