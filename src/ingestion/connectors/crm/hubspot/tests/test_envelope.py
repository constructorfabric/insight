"""Envelope: property flattening, custom-field routing, truncation, unique_key."""

from __future__ import annotations

import json
import logging

from source_hubspot import envelope as envelope_mod
from source_hubspot.envelope import _truncate, envelope, inject_envelope_properties


def wrap(record, custom=frozenset(), seen=None):
    return envelope(record, tenant_id="T", source_id="S", custom_property_names=custom, collision_seen=seen)


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

    def test_custom_properties_go_to_json_blob(self):
        out = wrap({"id": "1", "properties": {"amount": "10", "my_custom": "x"}}, custom=frozenset({"my_custom"}))
        assert "properties_my_custom" not in out
        assert json.loads(out["custom_fields"]) == {"my_custom": "x"}

    def test_empty_custom_values_dropped(self):
        out = wrap({"id": "1", "properties": {"a": None, "b": "", "c": "kept"}}, custom=frozenset({"a", "b", "c"}))
        assert json.loads(out["custom_fields"]) == {"c": "kept"}

    def test_no_customs_serializes_empty_object(self):
        out = wrap({"id": "1", "properties": {}})
        assert out["custom_fields"] == "{}"

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

    def test_applied_to_flat_and_custom_properties(self):
        long = "y" * 5000
        out = wrap({"id": "1", "properties": {"amount": long, "my_custom": long}}, custom=frozenset({"my_custom"}))
        assert out["properties_amount"].endswith("…[truncated]")
        assert json.loads(out["custom_fields"])["my_custom"].endswith("…[truncated]")


class TestInjectEnvelopeProperties:
    def test_adds_envelope_fields(self):
        schema = {"type": "object", "properties": {"id": {"type": "string"}}}
        out = inject_envelope_properties(schema)
        assert out is schema  # mutates and returns the same mapping
        for field in ("tenant_id", "source_id", "unique_key", "data_source", "collected_at", "custom_fields"):
            assert field in schema["properties"]
        assert schema["properties"]["id"] == {"type": "string"}

    def test_creates_properties_when_absent(self):
        out = inject_envelope_properties({"type": "object"})
        assert "unique_key" in out["properties"]
