"""What the fixture-loader derives for a `raw_data` payload.

The rule is only observable three layers down — an empty payload yields an empty
field history, which yields no identity_inputs, which no metric asserts on. So it
is stated here instead, close enough to read.

NOT RUN IN CI: `meta/` is the rig's own framework suite and the workflow
deliberately leaves it out. The barrier these cases describe DOES run in CI, on
every fixture the metrics lane loads.
"""

from __future__ import annotations

import pytest
from lib import fixture_loader

SCHEMA = {"properties": {"id": {}, "workEmail": {}, "raw_data": {}}}


def _derive(record: dict) -> dict | None:
    return fixture_loader._with_derived_payload(record, SCHEMA).get("raw_data")


def test_a_record_stating_no_payload_gets_its_own_fields() -> None:
    assert _derive({"id": "e1", "workEmail": "a@b.test"}) == {"id": "e1", "workEmail": "a@b.test"}


@pytest.mark.parametrize(
    "framing", ["tenant_id", "source_id", "unique_key", "_airbyte_raw_id", "_airbyte_meta"]
)
def test_the_columns_the_warehouse_adds_are_not_part_of_the_payload(framing: str) -> None:
    payload = _derive({"id": "e1", framing: "v"})
    assert framing not in payload, f"should not carry framing: {framing!r}"


def test_a_stated_map_is_laid_over_the_derived_keys() -> None:
    """The connector's payload is a SUPERSET of the columns — it collects fields
    no column holds. Replacing rather than layering would make that the one edit
    that silently unpins the columns from the payload."""
    payload = _derive({"id": "e1", "raw_data": {"customField": "x"}})

    assert payload == {"id": "e1", "customField": "x"}


def test_a_stated_null_leaves_the_row_without_a_payload() -> None:
    """The shape a source emitted before it began collecting every field. It has
    to stay expressible, or the pre-rollout case cannot be tested at all."""
    assert _derive({"id": "e1", "raw_data": None}) is None


def test_a_record_whose_fields_are_all_absent_derives_nothing() -> None:
    """The state the loader's barrier exists to refuse: a payload that is present
    and empty reads downstream exactly like one that is missing."""
    assert _derive({"id": None, "workEmail": None}) == {}
