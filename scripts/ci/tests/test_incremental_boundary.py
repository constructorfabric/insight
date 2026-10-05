from __future__ import annotations

import sys
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2].parent
sys.path.insert(0, str(ROOT / "scripts" / "ci"))

import incremental_boundary as ib

TABLE_WIDE = """
SELECT * FROM ({{ union_by_tag('silver:class_x') }})
{% if is_incremental() %}
WHERE _version > (SELECT max(_version) FROM {{ this }})
{% endif %}
"""

PER_SOURCE = """
SELECT candidate.* FROM ({{ union_by_tag('silver:class_x') }}) AS candidate
{{ silver_incremental_watermark(['tenant_id', "source_id"]) }}
"""

DDL = """
CREATE TABLE IF NOT EXISTS silver.class_x
(
    `tenant_id` Nullable(String),
    `source_id` String,
    `_version` UInt64
)
ENGINE = ReplacingMergeTree(_version)
ORDER BY unique_key;
"""

COLUMNS = ib.ddl_columns(DDL)
CLASS_X = ("silver", "class_x")


class TableWideBoundary(unittest.TestCase):
    def test_a_table_wide_boundary_is_rejected(self) -> None:
        cases = [
            TABLE_WIDE,
            "WHERE _version > (select MAX( _version ) from {{this}})",
            "WHERE c._version > (SELECT max(_version)\n FROM {{ this }})",
        ]
        for sql in cases:
            with self.subTest(sql=sql):
                errors, _ = ib.check_model("m.sql", CLASS_X, sql, COLUMNS)
                self.assertEqual(len(errors), 1, f"should reject: {sql!r}")

    def test_a_boundary_named_only_in_a_comment_is_allowed(self) -> None:
        cases = [
            PER_SOURCE + "\n-- was: WHERE _version > (SELECT max(_version) FROM {{ this }})\n",
            PER_SOURCE + "\n{# max(_version) FROM {{ this }} #}\n",
        ]
        for sql in cases:
            with self.subTest(sql=sql):
                errors, _ = ib.check_model("m.sql", CLASS_X, sql, COLUMNS)
                self.assertEqual(errors, [], f"should accept: {sql!r}")


class WatermarkKeys(unittest.TestCase):
    def test_keys_are_read_in_either_quote_style(self) -> None:
        self.assertEqual(ib.watermark_keys(PER_SOURCE), [["tenant_id", "source_id"]])

    def test_keys_that_are_columns_pass(self) -> None:
        self.assertEqual(ib.check_model("m.sql", CLASS_X, PER_SOURCE, COLUMNS), ([], []))

    def test_a_key_that_is_not_a_column_is_an_error(self) -> None:
        sql = "{{ silver_incremental_watermark(['tenant_id', 'source']) }}"
        errors, _ = ib.check_model("m.sql", CLASS_X, sql, COLUMNS)
        self.assertEqual(len(errors), 1)
        self.assertIn("source", errors[0].message)

    def test_a_relation_missing_from_the_snapshot_is_only_a_warning(self) -> None:
        errors, warnings = ib.check_model("m.sql", ("silver", "class_y"), PER_SOURCE, COLUMNS)
        self.assertEqual((len(errors), len(warnings)), (0, 1))


class Relation(unittest.TestCase):
    def test_the_relation_is_the_configured_schema_and_the_file_name(self) -> None:
        cases = [
            ("{{ config(schema='identity') }}", ("identity", "identity_inputs")),
            ("{{ config(materialized='incremental') }}", ("silver", "identity_inputs")),
        ]
        for sql, expected in cases:
            with self.subTest(sql=sql):
                self.assertEqual(ib.relation_of(Path("x/identity_inputs.sql"), sql), expected)


class Repository(unittest.TestCase):
    def test_the_repository_holds_no_table_wide_boundary(self) -> None:
        """The gate itself, run over this checkout: a model reintroducing the
        predicate, or a misspelt key, fails here as well as in CI."""
        ddl = "\n".join(p.read_text() for p in sorted((ROOT / ib.DDL_DIR).glob("*.sql")))
        columns = ib.ddl_columns(ddl)
        errors = []
        for path in ib.models(ROOT):
            sql = path.read_text()
            errors += ib.check_model(str(path), ib.relation_of(path, sql), sql, columns)[0]
        self.assertEqual(errors, [])


if __name__ == "__main__":
    unittest.main()
