"""Envelope: allowlisted property flattening, truncation, raw_data, unique_key."""

from __future__ import annotations

import json
import logging

from source_hubspot import envelope as envelope_mod
from source_hubspot.api import Hubspot
from source_hubspot.constants import ALLOWED_PROPERTIES_BY_OBJECT
from source_hubspot.envelope import _truncate, envelope, inject_envelope_properties


def wrap(record, allowed=frozenset({"amount"}), seen=None):
    return envelope(record, tenant_id="T", source_id="S", allowed_property_names=allowed, collision_seen=seen)


class TestEnvelope:
    def test_flattens_properties_and_adds_metadata(self):
        out = wrap({"id": "1", "updatedAt": "2024-06-01T00:00:00Z", "properties": {"amount": "10"}})
        assert out["id"] == "1"
        assert out["properties_amount"] == "10"
        assert "properties" not in out
        assert out["tenant_id"] == "T"
        assert out["source_id"] == "S"
        assert out["unique_key"] == "T-S-1"
        assert out["data_source"] == "hubspot"
        # collected_at is a UTC second-precision ISO timestamp.
        assert out["collected_at"].endswith("Z")

    def test_property_outside_allowlist_reaches_raw_data_only(self):
        out = wrap({"id": "1", "properties": {"amount": "10", "my_custom": "x", "uncurated_std": "y"}})
        assert out["properties_amount"] == "10"
        assert "properties_my_custom" not in out
        assert "properties_uncurated_std" not in out
        raw_properties = json.loads(out["raw_data"])["properties"]
        assert raw_properties["my_custom"] == "x"
        assert raw_properties["uncurated_std"] == "y"

    def test_allowlisted_property_keeps_its_column_when_empty(self):
        out = wrap({"id": "1", "properties": {"amount": None, "dealname": ""}}, allowed=frozenset({"amount", "dealname"}))
        assert out["properties_amount"] is None
        assert out["properties_dealname"] == ""

    def test_allowlisted_property_absent_from_record_gets_no_column(self):
        out = wrap({"id": "1", "properties": {"amount": "10"}}, allowed=frozenset({"amount", "dealname"}))
        assert "properties_dealname" not in out

    def test_emitted_property_keys_are_a_subset_of_the_declared_schema(self):
        hubspot = Hubspot("pat-test-token")
        for object_type, allowlist in ALLOWED_PROPERTIES_BY_OBJECT.items():
            declared = set(hubspot.generate_schema(object_type)["properties"])
            record = {"id": "1", "properties": {name: "v" for name in allowlist} | {"undeclared": "v"}}
            emitted = {k for k in wrap(record, allowed=allowlist) if k.startswith("properties_")}

            assert emitted == {f"properties_{name}" for name in allowlist}, f"object: {object_type}"
            assert emitted <= declared, f"object: {object_type}"

    def test_missing_properties_key_tolerated(self):
        out = wrap({"id": "1"})
        assert out["unique_key"] == "T-S-1"


class TestReservedNameCollision:
    def test_colliding_field_dropped_and_warned_once(self, caplog):
        seen: set = set()
        with caplog.at_level(logging.WARNING, logger="airbyte"):
            out1 = wrap({"id": "1", "tenant_id": "EVIL"}, seen=seen)
            out2 = wrap({"id": "2", "tenant_id": "EVIL"}, seen=seen)
        assert out1["tenant_id"] == "T"  # envelope value wins
        assert out2["tenant_id"] == "T"
        assert caplog.text.count("collides with Insight envelope field") == 1

    def test_warns_every_time_without_seen_set(self, caplog):
        with caplog.at_level(logging.WARNING, logger="airbyte"):
            wrap({"id": "1", "unique_key": "EVIL"}, seen=None)
            wrap({"id": "2", "unique_key": "EVIL"}, seen=None)
        assert caplog.text.count("collides with Insight envelope field") == 2


class TestUniqueKey:
    def test_missing_id_gets_content_hash(self, caplog):
        with caplog.at_level(logging.ERROR, logger="airbyte"):
            out = wrap({"properties": {"amount": "10"}})
        assert out["unique_key"].startswith("T-S-nohash:")
        assert "missing id" in caplog.text
        # Same content → same hash (stable across calls).
        out2 = wrap({"properties": {"amount": "10"}})
        assert out2["unique_key"] == out["unique_key"]

    def test_empty_string_id_gets_content_hash(self):
        out = wrap({"id": "", "properties": {}})
        assert "nohash:" in out["unique_key"]

    def test_zero_id_is_legitimate(self):
        out = wrap({"id": 0, "properties": {}})
        assert out["unique_key"] == "T-S-0"


class TestTruncation:
    def test_short_string_untouched(self):
        assert _truncate("short") == "short"

    def test_non_string_untouched(self):
        assert _truncate(12345) == 12345
        assert _truncate(None) is None

    def test_long_string_truncated_with_suffix(self):
        long = "x" * 5000
        out = _truncate(long)
        assert out.endswith("…[truncated]")
        assert len(out.encode("utf-8")) <= 2048

    def test_multibyte_boundary_stays_valid_utf8(self):
        long = "й" * 3000  # 2 bytes each — forces a mid-char cut
        out = _truncate(long)
        out.encode("utf-8")  # must not raise
        assert out.endswith("…[truncated]")

    def test_tiny_cap_returns_suffix_only(self, monkeypatch):
        monkeypatch.setattr(envelope_mod, "_VALUE_MAX_BYTES", 5)
        assert _truncate("x" * 100) == "…[truncated]"

    def test_applied_to_property_columns(self):
        long = "y" * 5000
        out = wrap({"id": "1", "properties": {"amount": long}})
        assert out["properties_amount"].endswith("…[truncated]")


class TestRawData:
    def test_keeps_record_shape_as_received(self):
        record = {
            "id": "1",
            "updatedAt": "2024-06-01T00:00:00Z",
            "properties": {"amount": "10", "uncurated_std": "kept", "my_custom": "x"},
            "associations_companies": ["7", "8"],
        }
        raw = json.loads(wrap(record)["raw_data"])
        assert raw["id"] == "1"
        assert raw["properties"] == {"amount": "10", "uncurated_std": "kept", "my_custom": "x"}
        assert raw["associations_companies"] == ["7", "8"]

    def test_present_without_properties_or_associations(self):
        assert json.loads(wrap({"id": "1"})["raw_data"]) == {"id": "1"}

    def test_truncates_values_not_the_blob(self):
        long = "y" * 5000
        out = wrap({"id": "1", "properties": {"amount": long}, "note": long})
        raw = json.loads(out["raw_data"])  # must stay parseable
        assert raw["properties"]["amount"].endswith("…[truncated]")
        assert raw["note"].endswith("…[truncated]")
        # Two capped values plus the record scaffolding exceed the per-value cap.
        assert len(out["raw_data"].encode("utf-8")) > 2048

    def test_source_record_left_unmodified(self):
        record = {"id": "1", "properties": {"amount": "y" * 5000}}
        wrap(record)
        assert record["properties"]["amount"] == "y" * 5000

    def test_colliding_source_field_still_captured(self, caplog):
        with caplog.at_level(logging.WARNING, logger="airbyte"):
            out = wrap({"id": "1", "raw_data": "SOURCE"})
        assert json.loads(out["raw_data"])["raw_data"] == "SOURCE"
        assert "collides with Insight envelope field" in caplog.text


class TestInjectEnvelopeProperties:
    def test_adds_envelope_fields(self):
        schema = {"type": "object", "properties": {"id": {"type": "string"}}}
        out = inject_envelope_properties(schema)
        assert out is schema  # mutates and returns the same mapping
        for field in (
            "tenant_id",
            "source_id",
            "unique_key",
            "data_source",
            "collected_at",
            "raw_data",
        ):
            assert field in schema["properties"], f"missing envelope field: {field}"
        assert "custom_fields" not in schema["properties"]
        assert schema["properties"]["id"] == {"type": "string"}

    def test_creates_properties_when_absent(self):
        out = inject_envelope_properties({"type": "object"})
        assert "unique_key" in out["properties"]
