from __future__ import annotations

import pytest
from conftest import SOURCE, TENANT, FakeClient, custom_report, meta_field
from source_bamboohr.streams.employees import MAX_REPORT_FIELDS, SCHEMA, EmployeesStream
from source_bamboohr.streams.leave_requests import LeaveRequestsStream
from source_bamboohr.streams.meta_fields import MetaFieldsStream

EMPLOYEE = {
    "id": "42",
    "displayName": "Jane Doe",
    "workEmail": "jane.doe@example.com",
    "jobTitle": "Engineer",
    "standardHoursPerWeek": "40",
    "customTeam": "Platform",
}


def employees_stream(rows, meta=(), omit=None, declare_columns=True):
    client = FakeClient(
        {
            "meta/fields": list(meta),
            "reports/custom": custom_report(rows, omit=omit, declare_columns=declare_columns),
        }
    )
    return EmployeesStream(client=client, tenant_id=TENANT, source_id=SOURCE), client


def read(stream):
    return list(stream.read_records(sync_mode="full_refresh"))


class TestEmployeeRecords:
    def test_the_record_carries_exactly_the_declared_columns(self):
        stream, _ = employees_stream([EMPLOYEE])
        (record,) = read(stream)
        assert set(record) == set(SCHEMA["properties"])

    def test_every_returned_field_is_preserved_in_the_raw_payload(self):
        stream, _ = employees_stream([EMPLOYEE])
        (record,) = read(stream)
        assert record["raw_data"] == EMPLOYEE

    def test_a_field_outside_the_declared_columns_stays_out_of_the_top_level(self):
        stream, _ = employees_stream([EMPLOYEE])
        (record,) = read(stream)
        assert "customTeam" not in record
        assert "standardHoursPerWeek" not in record

    def test_the_raw_payload_key_order_does_not_follow_the_api(self):
        stream, _ = employees_stream([{"id": "1", "z": "last", "a": "first"}])
        (record,) = read(stream)
        assert list(record["raw_data"]) == ["a", "id", "z"]

    def test_a_sensitive_field_the_report_returned_anyway_is_not_stored(self):
        stream, _ = employees_stream([{**EMPLOYEE, "ssn": "000-00-0000", "homePhone": "555"}])
        (record,) = read(stream)

        assert "ssn" not in record["raw_data"]
        assert "homePhone" not in record["raw_data"]
        assert record["raw_data"]["customTeam"] == "Platform"

    def test_a_sensitive_field_volunteered_under_its_id_is_not_stored(self):
        # The report returns columns that were never requested, and answers a
        # field asked for by numeric id under an indexed key.
        meta = [meta_field(17, alias="ssn"), meta_field(4001, alias="customTeam")]
        stream, _ = employees_stream([{**EMPLOYEE, "17.0": "000-00-0000", "17": "000-00-0000"}], meta=meta)
        (record,) = read(stream)

        assert "17.0" not in record["raw_data"]
        assert "17" not in record["raw_data"]
        assert record["raw_data"]["customTeam"] == "Platform"

    def test_a_declared_column_the_report_omitted_reads_as_null(self):
        stream, _ = employees_stream([{"id": "42"}])
        (record,) = read(stream)
        assert record["department"] is None

    def test_the_unique_key_is_tenant_source_and_employee_id(self):
        stream, _ = employees_stream([EMPLOYEE])
        (record,) = read(stream)
        assert record["unique_key"] == f"{TENANT}-{SOURCE}-42"

    @pytest.mark.parametrize("row", [{"id": None}, {"id": ""}, {}, "nonsense"])
    def test_a_row_without_a_usable_id_is_skipped(self, row):
        stream, _ = employees_stream([row])
        assert read(stream) == [], f"should skip: {row!r}"


class TestEmployeeReportRequest:
    def test_the_report_requests_the_discovered_fields(self):
        stream, client = employees_stream([EMPLOYEE], meta=[meta_field(4001, alias="customTeam")])
        read(stream)

        (_, _, body) = client.calls[-1]
        assert "customTeam" in body["fields"]
        assert body["fields"].count("jobTitle") == 1

    def test_a_field_list_within_the_limit_is_one_request(self):
        meta = [meta_field(4000 + n, alias=f"custom{n}") for n in range(50)]
        stream, client = employees_stream([EMPLOYEE], meta=meta)
        read(stream)

        assert len([c for c in client.calls if c[0] == "POST"]) == 1

    def test_a_field_list_over_the_limit_is_split_into_accepted_requests(self):
        meta = [meta_field(4000 + n, alias=f"custom{n}") for n in range(MAX_REPORT_FIELDS + 60)]
        stream, client = employees_stream([EMPLOYEE], meta=meta)
        read(stream)

        posts = [body["fields"] for verb, _, body in client.calls if verb == "POST"]
        assert len(posts) > 1
        for batch in posts:
            assert len(batch) <= MAX_REPORT_FIELDS, f"batch of {len(batch)} exceeds the API limit"
            assert batch[0] == "id", "every batch needs the id to merge on"

    def test_a_split_request_still_yields_one_record_per_employee(self):
        meta = [meta_field(4000 + n, alias=f"custom{n}") for n in range(MAX_REPORT_FIELDS + 60)]
        rows = [{"id": "1", "custom0": "a"}, {"id": "2", "custom0": "b"}]
        stream, _ = employees_stream(rows, meta=meta)

        assert [r["unique_key"] for r in read(stream)] == [f"{TENANT}-{SOURCE}-1", f"{TENANT}-{SOURCE}-2"]

    def test_a_split_request_merges_every_batch_into_the_payload(self):
        meta = [meta_field(4000 + n, alias=f"custom{n}") for n in range(MAX_REPORT_FIELDS + 60)]
        last = f"custom{MAX_REPORT_FIELDS + 59}"
        stream, _ = employees_stream([{"id": "1", "custom0": "first", last: "last"}], meta=meta)
        (record,) = read(stream)

        assert record["raw_data"]["custom0"] == "first"
        assert record["raw_data"][last] == "last", "a later batch's fields must survive the merge"

    def test_a_report_response_without_employees_is_an_error(self):
        client = FakeClient({"meta/fields": [], "reports/custom": {}})
        stream = EmployeesStream(client=client, tenant_id=TENANT, source_id=SOURCE)

        with pytest.raises(RuntimeError, match="employees"):
            read(stream)


class TestSilentlyOmittedFields:
    def test_a_withheld_custom_field_does_not_stop_the_sync(self):
        meta = [meta_field(4001, alias="customTeam")]
        stream, _ = employees_stream([EMPLOYEE], meta=meta, omit={"customTeam"})

        (record,) = read(stream)
        assert record["unique_key"] == f"{TENANT}-{SOURCE}-42"

    def test_a_field_answered_under_an_indexed_key_is_not_an_omission(self):
        # A field asked for by numeric id comes back as "<id>.0".
        meta = [meta_field(4463)]
        client = FakeClient(
            {
                "meta/fields": meta,
                "reports/custom": lambda body: {
                    "title": body["title"],
                    "fields": [
                        {"id": f"{name}.0" if name.isdigit() else name, "type": "text", "name": name}
                        for name in body["fields"]
                    ],
                    "employees": [{"id": "42", "4463.0": "checked"}],
                },
            }
        )
        stream = EmployeesStream(client=client, tenant_id=TENANT, source_id=SOURCE)

        (record,) = read(stream)
        assert record["raw_data"]["4463.0"] == "checked"

    def test_a_withheld_bronze_column_does_not_stop_the_sync(self):
        stream, _ = employees_stream([EMPLOYEE], omit={"workEmail"})

        (record,) = read(stream)
        assert record["workEmail"] is None

    def test_a_report_that_declares_no_columns_is_not_read_as_data_loss(self):
        meta = [meta_field(4001, alias="customTeam")]
        stream, _ = employees_stream([EMPLOYEE], meta=meta, declare_columns=False)

        assert len(read(stream)) == 1, "an unverifiable answer must not fail the stream"

    def test_a_column_no_employee_carries_is_not_an_omission(self):
        stream, _ = employees_stream([{"id": "42"}])
        assert len(read(stream)) == 1


class TestLeaveRequests:
    def test_the_window_starts_at_the_configured_date(self):
        client = FakeClient({"time_off/requests": []})
        stream = LeaveRequestsStream(
            client=client, tenant_id=TENANT, source_id=SOURCE, start_date="2024-01-01"
        )
        read(stream)

        (_, _, params) = client.calls[-1]
        assert params["start"] == "2024-01-01"
        assert params["end"] >= "2024-01-01"

    def test_the_record_keeps_the_api_payload_and_adds_the_framework_fields(self):
        client = FakeClient({"time_off/requests": [{"id": "7", "status": {"status": "approved"}}]})
        stream = LeaveRequestsStream(
            client=client, tenant_id=TENANT, source_id=SOURCE, start_date="2020-01-01"
        )
        (record,) = read(stream)

        assert record["status"] == {"status": "approved"}
        assert record["unique_key"] == f"{TENANT}-{SOURCE}-7"
        assert (record["tenant_id"], record["source_id"]) == (TENANT, SOURCE)


    @pytest.mark.parametrize("row", [{"id": None}, {"id": ""}, {}, "nonsense"])
    def test_a_request_without_a_usable_id_is_skipped(self, row):
        client = FakeClient({"time_off/requests": [row]})
        stream = LeaveRequestsStream(
            client=client, tenant_id=TENANT, source_id=SOURCE, start_date="2020-01-01"
        )
        assert read(stream) == [], f"should skip: {row!r}"

    def test_a_response_that_is_not_a_list_is_an_error(self):
        client = FakeClient({"time_off/requests": {"requests": []}})
        stream = LeaveRequestsStream(
            client=client, tenant_id=TENANT, source_id=SOURCE, start_date="2020-01-01"
        )
        with pytest.raises(RuntimeError, match="not a list"):
            read(stream)


class TestMetaFields:
    @pytest.mark.parametrize("row", [{"id": None}, {"id": ""}, {}, "nonsense"])
    def test_an_entry_without_a_usable_id_is_skipped(self, row):
        client = FakeClient({"meta/fields": [row]})
        stream = MetaFieldsStream(client=client, tenant_id=TENANT, source_id=SOURCE)
        assert read(stream) == [], f"should skip: {row!r}"

    def test_a_response_that_is_not_a_list_is_an_error(self):
        client = FakeClient({"meta/fields": {"fields": []}})
        stream = MetaFieldsStream(client=client, tenant_id=TENANT, source_id=SOURCE)
        with pytest.raises(RuntimeError, match="not a list"):
            read(stream)

    def test_an_active_field_keys_on_its_id(self):
        client = FakeClient({"meta/fields": [meta_field(9, alias="jobTitle")]})
        stream = MetaFieldsStream(client=client, tenant_id=TENANT, source_id=SOURCE)
        (record,) = read(stream)

        assert record["unique"] == "9"
        assert record["unique_key"] == f"{TENANT}-{SOURCE}-9"

    def test_a_deprecated_field_keys_apart_from_the_active_one(self):
        client = FakeClient(
            {"meta/fields": [meta_field(9, alias="jobTitle"), meta_field(9, deprecated=True)]}
        )
        stream = MetaFieldsStream(client=client, tenant_id=TENANT, source_id=SOURCE)
        active, deprecated = read(stream)

        assert deprecated["unique"] == "d9"
        assert active["unique_key"] != deprecated["unique_key"]
