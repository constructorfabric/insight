from __future__ import annotations

import logging
from collections.abc import Iterable, Mapping, Sequence
from typing import Any

from airbyte_cdk.models import SyncMode
from airbyte_cdk.sources.streams import Stream

from source_bamboohr.client import BambooClient

logger = logging.getLogger("airbyte")

REPORT_TITLE = "Insight Employee Sync"

EMPLOYEE_ID_FIELD = "id"

# BambooHR rejects a custom report asking for more than 400 fields with a 400,
# which no retry recovers: an account defining more fields than that would lose
# the whole stream rather than the overflow.
MAX_REPORT_FIELDS = 400

NULLABLE_STR: Mapping[str, Any] = {"type": ["string", "null"]}

SCHEMA: Mapping[str, Any] = {
    "$schema": "http://json-schema.org/schema#",
    "type": "object",
    "additionalProperties": True,
    "required": ["unique_key"],
    "properties": {
        "id": NULLABLE_STR,
        "city": NULLABLE_STR,
        "status": NULLABLE_STR,
        "country": NULLABLE_STR,
        "division": NULLABLE_STR,
        "hireDate": NULLABLE_STR,
        "jobTitle": NULLABLE_STR,
        "lastName": NULLABLE_STR,
        "location": NULLABLE_STR,
        "raw_data": {"type": ["object", "null"], "additionalProperties": True},
        "firstName": NULLABLE_STR,
        "source_id": NULLABLE_STR,
        "tenant_id": NULLABLE_STR,
        "workEmail": NULLABLE_STR,
        "department": NULLABLE_STR,
        "supervisor": NULLABLE_STR,
        "unique_key": {"type": "string"},
        "displayName": NULLABLE_STR,
        "lastChanged": NULLABLE_STR,
        "supervisorEId": NULLABLE_STR,
        "employeeNumber": NULLABLE_STR,
        "supervisorEmail": NULLABLE_STR,
        "terminationDate": NULLABLE_STR,
        "originalHireDate": NULLABLE_STR,
        "employmentHistoryStatus": NULLABLE_STR,
    },
}

FRAMEWORK_FIELDS = frozenset({"raw_data", "tenant_id", "source_id", "unique_key"})

# Bronze columns projected out of the report row. Derived from the schema so the
# projection and the declared column set cannot drift apart.
BUSINESS_FIELDS = tuple(name for name in SCHEMA["properties"] if name not in FRAMEWORK_FIELDS)

# Standard BambooHR fields never requested and never stored, whatever permission
# the API key holds. Discovery collects every field an account defines, and an
# HR record carries far more than an analytics warehouse has any reason to hold;
# these are the categories the PRD puts out of scope, plus the identifiers and
# pay amounts whose exposure is the worst outcome of getting this wrong. No
# analytics surface reads any of them. An alias listed here that the account
# does not define costs nothing; one missing from here is collected.
SENSITIVE_FIELDS = frozenset(
    {
        # National, tax and government identifiers
        "ssn", "sin", "nin", "nationalId",
        # Protected demographics
        "dateOfBirth", "gender", "ethnicity", "maritalStatus",
        "veteranStatus", "disabilityStatus",
        # Personal contact details
        "homeEmail", "homePhone", "mobilePhone",
        "workPhone", "workPhoneExtension", "workPhonePlusExtension",
        # Street address (the work city and country stay — they are bronze columns)
        "address1", "address2", "zipcode", "state", "stateCode",
        # Photos and social profiles
        "photoUrl", "photoUploaded",
        "linkedIn", "twitterFeed", "facebook", "instagram", "pinterest",
        # Compensation amounts
        "payRate", "payRateEffectiveDate", "commissionRate", "bonusAmount",
    }
)


def report_fields(meta_fields: Any) -> tuple[str, ...]:
    """Field keys to request from the custom report: every field BambooHR knows
    except SENSITIVE_FIELDS, with the declared bronze columns as a floor so a gap
    in the field metadata can never empty a column."""
    if not isinstance(meta_fields, list):
        raise RuntimeError(f"BambooHR field metadata is not a list: {type(meta_fields).__name__}")

    keys = list(BUSINESS_FIELDS)
    seen = set(keys)

    for field in meta_fields:
        if not isinstance(field, Mapping) or field.get("deprecated"):
            continue

        key = _request_key(field)
        if key is None or key in seen or key in SENSITIVE_FIELDS:
            continue

        seen.add(key)
        keys.append(key)

    return tuple(keys)


def field_batches(fields: Sequence[str]) -> tuple[tuple[str, ...], ...]:
    """Split a field list into requests BambooHR will accept. Every batch carries
    the employee id so the rows can be merged back into one record."""
    if len(fields) <= MAX_REPORT_FIELDS:
        return (tuple(fields),)

    rest = [name for name in fields if name != EMPLOYEE_ID_FIELD]
    per_batch = MAX_REPORT_FIELDS - 1

    return tuple(
        (EMPLOYEE_ID_FIELD, *rest[start : start + per_batch])
        for start in range(0, len(rest), per_batch)
    )


def returned_fields(payload: Mapping[str, Any], rows: Sequence[Any]) -> set[str] | None:
    """Field keys the report answered with, or None when it did not declare its
    columns. None is not "nothing came back" — it means the completeness of this
    batch cannot be judged, and guessing would turn an unreadable answer into a
    reported data loss."""
    columns = payload.get("fields")
    if not isinstance(columns, list):
        return None

    answered = {str(column["id"]) for column in columns if isinstance(column, Mapping) and "id" in column}

    # A column the report declares but no row carries is still answered; a key a
    # row carries that the declaration missed is answered too.
    for row in rows:
        if isinstance(row, Mapping):
            answered.update(row.keys())

    return answered


def _report_omissions(requested: Sequence[str], answered: set[str]) -> None:
    """BambooHR silently drops requested fields the credential cannot read and
    still answers 200. The API key may legitimately hold access to only a subset
    of the declared bronze columns, so an omission is not an error — the sync
    proceeds and the columns are published as null. The warning names them so a
    key that lost access it used to have is still diagnosable from the logs.
    """
    missing = [name for name in requested if name in BUSINESS_FIELDS and name not in answered]
    if missing:
        logger.warning(
            "BambooHR omitted %d declared employee column(s) from the report: %s. "
            "The API key has no access to them; they are published as null.",
            len(missing),
            ", ".join(missing),
        )


def sensitive_keys(meta_fields: Sequence[Any]) -> frozenset[str]:
    """Every key a sensitive field could arrive under: its alias, its numeric id,
    and the indexed `<id>.N` form the report answers with. The report volunteers
    columns that were never requested, so excluding these from the request is not
    on its own enough to keep them out of the payload."""
    keys = set(SENSITIVE_FIELDS)

    for field in meta_fields:
        if not isinstance(field, Mapping):
            continue

        alias = str(field.get("alias") or "").strip()
        field_id = field.get("id")
        if alias in SENSITIVE_FIELDS and field_id is not None:
            keys.add(str(field_id))

    return frozenset(keys)


def _is_sensitive(key: str, sensitive: frozenset[str]) -> bool:
    return key in sensitive or key.split(".", 1)[0] in sensitive


def _request_key(field: Mapping[str, Any]) -> str | None:
    alias = str(field.get("alias") or "").strip()
    if alias:
        return alias

    field_id = field.get("id")
    if field_id is None or str(field_id).strip() == "":
        return None

    return str(field_id)


class EmployeesStream(Stream):
    name = "employees"
    primary_key = "unique_key"

    def __init__(self, client: BambooClient, tenant_id: str, source_id: str) -> None:
        self._client = client
        self._tenant_id = tenant_id
        self._source_id = source_id

    def read_records(
        self,
        sync_mode: SyncMode,
        cursor_field: list[str] | None = None,
        stream_slice: Mapping[str, Any] | None = None,
        stream_state: Mapping[str, Any] | None = None,
    ) -> Iterable[Mapping[str, Any]]:
        meta = self._client.get("meta/fields")
        fields = report_fields(meta)
        sensitive = sensitive_keys(meta)
        batches = field_batches(fields)
        logger.info(
            "BambooHR employee report requests %d fields in %d request(s)", len(fields), len(batches)
        )

        merged: dict[str, dict[str, Any]] = {}
        answered: set[str] = set()
        verifiable: set[str] = set()

        for batch in batches:
            payload = self._client.post("reports/custom", {"title": REPORT_TITLE, "fields": list(batch)})
            rows = payload.get("employees") if isinstance(payload, Mapping) else None
            if not isinstance(rows, list):
                raise RuntimeError("BambooHR custom report response carries no 'employees' collection")

            batch_fields = returned_fields(payload, rows)
            if batch_fields is not None:
                answered |= batch_fields
                verifiable.update(batch)

            self._merge(merged, rows)

        _report_omissions([name for name in fields if name in verifiable], answered)

        for row in merged.values():
            yield self._to_record(row, sensitive)

        logger.info("BambooHR employees stream emitted %d records", len(merged))

    @staticmethod
    def _merge(merged: dict[str, dict[str, Any]], rows: Sequence[Any]) -> None:
        for row in rows:
            if not isinstance(row, Mapping):
                logger.warning("Skipping BambooHR employee row that is not an object")
                continue

            employee_id = row.get("id")
            if employee_id is None or str(employee_id).strip() == "":
                logger.warning("Skipping BambooHR employee row without an id")
                continue

            merged.setdefault(str(employee_id), {}).update(row)

    def _to_record(self, row: Mapping[str, Any], sensitive: frozenset[str]) -> Mapping[str, Any]:
        employee_id = row["id"]
        payload = {
            key: value for key, value in sorted(row.items()) if not _is_sensitive(key, sensitive)
        }

        record: dict[str, Any] = {name: row.get(name) for name in BUSINESS_FIELDS}
        record["raw_data"] = payload
        record["tenant_id"] = self._tenant_id
        record["source_id"] = self._source_id
        record["unique_key"] = f"{self._tenant_id}-{self._source_id}-{employee_id}"
        return record

    def get_json_schema(self) -> Mapping[str, Any]:
        return SCHEMA
