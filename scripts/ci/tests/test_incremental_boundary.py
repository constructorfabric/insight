from __future__ import annotations

import sys
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2].parent
sys.path.insert(0, str(ROOT / "scripts" / "ci"))

import incremental_boundary as ib  # noqa: E402 — importable only after the sys.path insert above

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


class TableWideBoundary(unittest.TestCase):
    def test_a_table_wide_boundary_is_rejected(self) -> None:
        cases = [
            TABLE_WIDE,
            "WHERE _version > (select MAX( _version ) from {{this}})",
            "WHERE c._version > (SELECT max(_version)\n FROM {{ this }})",
        ]
        for sql in cases:
            with self.subTest(sql=sql):
                errors = ib.check_model("m.sql", sql)
                self.assertEqual(len(errors), 1, f"should reject: {sql!r}")

    def test_a_boundary_named_only_in_a_comment_is_allowed(self) -> None:
        cases = [
            PER_SOURCE + "\n-- was: WHERE _version > (SELECT max(_version) FROM {{ this }})\n",
            PER_SOURCE + "\n{# max(_version) FROM {{ this }} #}\n",
        ]
        for sql in cases:
            with self.subTest(sql=sql):
                errors = ib.check_model("m.sql", sql)
                self.assertEqual(errors, [], f"should accept: {sql!r}")


class Repository(unittest.TestCase):
    def test_the_repository_holds_no_table_wide_boundary(self) -> None:
        """The gate itself, run over this checkout: a model reintroducing the
        predicate fails here as well as in CI."""
        errors = []
        for path in ib.models(ROOT):
            errors += ib.check_model(str(path), path.read_text())
        self.assertEqual(errors, [])

    def test_the_gate_reads_a_model_set_worth_checking(self) -> None:
        """An empty model list would make the assertion above vacuous."""
        self.assertGreater(len(ib.models(ROOT)), 100)


if __name__ == "__main__":
    unittest.main()
