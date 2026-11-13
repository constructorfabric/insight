from __future__ import annotations

import pytest
from conftest import FakeClient
from source_bamboohr.streams.employees import BUSINESS_FIELDS, SENSITIVE_FIELDS, report_fields
from source_bamboohr.streams.meta_fields import MetaField, read_meta_fields


def meta_field(field_id: int, alias: str = "", deprecated: bool = False) -> MetaField:
    return MetaField(id=str(field_id), alias=alias, deprecated=deprecated)


class TestRequestKeys:
    def test_a_field_is_requested_by_alias_when_it_has_one(self):
        assert "customTeam" in report_fields([meta_field(4001, alias="customTeam")])

    @pytest.mark.parametrize("alias", [None, "", "   "], ids=["absent", "empty", "blank"])
    def test_a_field_without_a_usable_alias_is_requested_by_id(self, alias):
        fields = report_fields([meta_field(4001, alias=alias)])
        assert "4001" in fields, f"should fall back to the id for alias {alias!r}"

    def test_a_deprecated_field_is_skipped(self):
        assert "oldTeam" not in report_fields([meta_field(4001, alias="oldTeam", deprecated=True)])

    def test_a_field_with_neither_alias_nor_id_is_skipped(self):
        assert report_fields([MetaField()]) == BUSINESS_FIELDS


class TestFieldMetadataParsing:
    @pytest.mark.parametrize("entry", ["nonsense", None, 7, ["id"]])
    def test_an_entry_that_is_not_an_object_is_skipped(self, entry):
        assert read_meta_fields(FakeClient({"meta/fields": [entry]})) == [], f"should skip: {entry!r}"

    @pytest.mark.parametrize(
        "entry", [{"id": 4021, "alias": ["ssn"]}, {"id": 4021, "alias": {"a": 1}}, {"id": 4021, "deprecated": "maybe"}]
    )
    def test_an_odd_typed_attribute_keeps_the_entry_and_its_id(self, entry):
        ((field, _),) = read_meta_fields(FakeClient({"meta/fields": [entry]}))
        assert field.id == "4021", f"should keep: {entry!r}"

    def test_a_numeric_id_reads_as_its_key(self):
        ((field, row),) = read_meta_fields(FakeClient({"meta/fields": [{"id": 4001, "name": "Team"}]}))
        assert field.id == "4001"
        assert row == {"id": 4001, "name": "Team"}, "the raw entry must survive unchanged"

    @pytest.mark.parametrize(
        ("body", "error_type"), [({"fields": []}, "list_type"), (b"<html>secret-page</html>", "json_invalid")]
    )
    def test_unreadable_field_metadata_is_an_error_that_does_not_quote_it(self, body, error_type):
        with pytest.raises(RuntimeError, match=error_type) as caught:
            read_meta_fields(FakeClient({"meta/fields": body}))

        assert "secret-page" not in str(caught.value)
        assert caught.value.__cause__ is None


class TestRequestedSet:
    def test_every_declared_bronze_column_is_always_requested(self):
        assert set(BUSINESS_FIELDS) <= set(report_fields([]))

    def test_a_field_is_requested_once_however_often_it_is_named(self):
        fields = report_fields(
            [meta_field(1, alias="jobTitle"), meta_field(2, alias="customTeam"), meta_field(3, alias="customTeam")]
        )
        assert fields.count("customTeam") == 1
        assert fields.count("jobTitle") == 1

    def test_custom_fields_follow_the_declared_columns(self):
        fields = report_fields([meta_field(4001, alias="customTeam")])
        assert fields == (*BUSINESS_FIELDS, "customTeam")


class TestSensitiveFields:
    @pytest.mark.parametrize("alias", sorted(SENSITIVE_FIELDS))
    def test_a_sensitive_field_is_never_requested(self, alias):
        fields = report_fields([meta_field(4001, alias=alias)])
        assert alias not in fields, f"should not request: {alias}"

    def test_no_bronze_column_is_treated_as_sensitive(self):
        assert SENSITIVE_FIELDS.isdisjoint(BUSINESS_FIELDS)

    def test_a_sensitive_alias_does_not_block_the_rest_of_discovery(self):
        fields = report_fields([meta_field(1, alias="ssn"), meta_field(2, alias="customTeam")])
        assert "customTeam" in fields
