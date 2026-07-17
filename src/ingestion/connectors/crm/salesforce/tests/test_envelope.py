"""Tests for source_salesforce.envelope: record envelope + schema injection."""

from __future__ import annotations

import json

from source_salesforce.envelope import ENVELOPE_FIELDS_SCHEMA, envelope, inject_envelope_properties

TENANT = "T"
SOURCE = "S"


def _wrap(record, custom=frozenset(), collision_seen=None):
    return envelope(
        record, tenant_id=TENANT, source_id=SOURCE, custom_field_names=custom, collision_seen=collision_seen
    )


class TestEnvelope:
    def test_basic_fields_injected(self):
        out = _wrap({"Id": "001", "Name": "Acme"})
        assert out["tenant_id"] == TENANT
        assert out["source_id"] == SOURCE
        assert out["unique_key"] == "T-S-001"
        assert out["data_source"] == "salesforce"
        assert out["collected_at"].endswith("Z")
        assert out["Name"] == "Acme"

    def test_attributes_metadata_dropped(self):
        out = _wrap({"Id": "001", "attributes": {"type": "Account"}})
        assert "attributes" not in out

    def test_custom_fields_packed_into_json_blob(self):
        out = _wrap(
            {"Id": "001", "Name": "Acme", "Custom__c": "x", "Other__c": 5}, custom=frozenset({"Custom__c", "Other__c"})
        )
        assert "Custom__c" not in out and "Other__c" not in out
        assert json.loads(out["custom_fields"]) == {"Custom__c": "x", "Other__c": 5}

    def test_no_custom_fields_yields_empty_blob(self):
        assert _wrap({"Id": "001"})["custom_fields"] == "{}"

    def test_reserved_field_collision_dropped_and_warned_once(self, caplog):
        seen: set = set()
        with caplog.at_level("WARNING", logger="airbyte"):
            first = _wrap({"Id": "001", "tenant_id": "EVIL"}, collision_seen=seen)
            second = _wrap({"Id": "002", "tenant_id": "EVIL"}, collision_seen=seen)
        assert first["tenant_id"] == TENANT
        assert second["tenant_id"] == TENANT
        assert seen == {"tenant_id"}
        # Warned exactly once across both records.
        warnings = [r for r in caplog.records if "collides" in r.message]
        assert len(warnings) == 1

    def test_collision_without_seen_set_warns_every_time(self, caplog):
        with caplog.at_level("WARNING", logger="airbyte"):
            _wrap({"Id": "001", "unique_key": "EVIL"})
            _wrap({"Id": "002", "unique_key": "EVIL"})
        warnings = [r for r in caplog.records if "collides" in r.getMessage()]
        assert len(warnings) == 2

    def test_lowercase_id_fallback(self):
        out = _wrap({"id": "abc"})
        assert out["unique_key"] == "T-S-abc"

    def test_missing_id_derives_content_hash(self, caplog):
        with caplog.at_level("ERROR", logger="airbyte"):
            out = _wrap({"Name": "NoId"})
        assert out["unique_key"].startswith("T-S-nohash:")
        assert "missing Id" in caplog.text
        # Deterministic: same content, same hash.
        again = _wrap({"Name": "NoId"})
        assert again["unique_key"] == out["unique_key"]
        # Different content, different hash.
        other = _wrap({"Name": "Other"})
        assert other["unique_key"] != out["unique_key"]


class TestInjectEnvelopeProperties:
    def test_adds_all_envelope_fields(self):
        schema = {"properties": {"Id": {"type": ["string", "null"]}}}
        result = inject_envelope_properties(schema)
        for name in ENVELOPE_FIELDS_SCHEMA:
            assert name in result["properties"]
        assert result["properties"]["Id"] == {"type": ["string", "null"]}

    def test_creates_properties_when_absent(self):
        result = inject_envelope_properties({})
        assert set(result["properties"]) == set(ENVELOPE_FIELDS_SCHEMA)
