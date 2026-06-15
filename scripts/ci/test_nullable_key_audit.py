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
