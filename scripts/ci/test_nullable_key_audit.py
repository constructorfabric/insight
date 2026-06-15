"""Parser self-test for nullable_key_audit — proves the gate catches a nullable
dedup key, ignores commas inside MD5(concat(...)), and is not tainted by a
sibling column's CAST(NULL ...)."""
import pathlib
import sys

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))
import nullable_key_audit as a  # noqa: E402


def _nullable(sql: str, col: str = "unique_key") -> bool:
    return any(
        a.NULLABLE_RE.search(p) or a.CAST_NULL_RE.search(p)
        for p in a.projections_for(sql, col)
    )


def test_flags_nullable_cast_key():
    assert _nullable("SELECT CAST(x AS Nullable(String)) AS unique_key, v FROM t")


def test_flags_cast_null_key():
    assert _nullable("SELECT CAST(NULL AS String) AS unique_key FROM t")


def test_md5_concat_key_is_clean_despite_inner_commas():
    sql = "SELECT MD5(concat(a, '-', b, '-', c)) AS unique_key, x FROM t"
    assert a.projections_for(sql, "unique_key")  # found
    assert not _nullable(sql)


def test_sibling_cast_null_does_not_taint_the_key():
    # The real false positive: a neighbour is nullable, the key (empty string) is not.
    sql = (
        "SELECT CAST(NULL AS Nullable(String)) AS tenant_id, "
        "CAST('' AS String) AS unique_key FROM t"
    )
    assert a.projections_for(sql, "unique_key")
    assert not _nullable(sql)


def test_passthrough_and_union_keys_are_inherited():
    # key passed through from upstream (audited there) → not a local gap
    assert a.inherits_key("SELECT c.unique_key, v FROM up", "unique_key")
    assert a.inherits_key("SELECT\n    unique_key,\n    v\nFROM up", "unique_key")
    assert a.inherits_key("SELECT * FROM up", "unique_key")
    assert a.inherits_key("SELECT * FROM ( {{ union_by_tag('x') }} )", "unique_key")


def test_config_literals_are_not_a_passthrough():
    # only config mentions the key (quoted), nothing selects it → unresolvable
    cfg = "config(unique_key='unique_key', order_by=['unique_key'])\nSELECT v FROM t"
    assert not a.inherits_key(cfg, "unique_key")


def test_fails_closed_on_unresolvable_key(tmp_path):
    # order_by names a key that is neither projected nor passed through → ERROR
    m = tmp_path / "weird.sql"
    m.write_text("{{ config(order_by=['unique_key']) }}\nSELECT v, w FROM t")
    findings = a.audit_model(m)
    assert any(s == "ERROR" for s, _, _ in findings), findings


def test_macro_generated_key_is_inherited(tmp_path):
    # {{ snapshot(unique_key_col='unique_key') }} owns the projection — not a gap,
    # and config()'s own order_by=['unique_key'] must NOT count as the macro.
    m = tmp_path / "snap.sql"
    m.write_text(
        "{{ config(order_by=['unique_key'], engine='MergeTree') }}\n"
        "{{ snapshot(source_ref=source('s','t'), unique_key_col='unique_key') }}"
    )
    assert a.audit_model(m) == []
