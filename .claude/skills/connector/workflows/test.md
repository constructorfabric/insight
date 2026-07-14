---
name: connector-test
description: "Test an Insight Connector: check, discover, read"
---

# Test Connector

Runs the full test cycle for a connector.

## Prerequisites

- Connector package exists at `CONNECTOR_DIR`
- Tenant credentials configured in `connections/<tenant>.yaml`

## Phase 1: Resolve Tenant

If tenant not specified, auto-detect from connections/*.yaml:
```bash
# Find first tenant with credentials for this connector
ls src/ingestion/connections/*.yaml
```

## Phase 2: Validate Manifest

```bash
./tools/declarative-connector/source.sh validate CONNECTOR_PATH
```

Expected: `Manifest is valid`
If fails: show error, suggest fix, STOP.

## Phase 3: Run Mock-Server Tests (no credentials)

L1 of the test ladder — see `docs/domain/connector/specs/feature-connector-mock-tests/FEATURE.md`.

```bash
# one-time env: cd src/ingestion/tests/connectors && python3.12 -m venv .venv && .venv/bin/pip install -e '.[dev]'
src/ingestion/tests/connectors/.venv/bin/pytest src/ingestion/connectors/CONNECTOR_PATH/tests/
```

- If the suite fails: fix the manifest (or the tests, if the behavior change is intended), STOP before live testing.
- If `tests/` does not exist: the connector predates the mock-test spec. STOP — do NOT continue to Phase 4 silently. Report the gap and ask the user to choose: (a) author the suite now per create.md §5.7 (preferred), or (b) explicitly proceed to live testing without L1 (acceptable for legacy connectors only, with the gap recorded in the Phase 8 summary).

## Phase 4: Check Credentials

```bash
./tools/declarative-connector/source.sh check CONNECTOR_PATH <tenant>
```

Expected: `CONNECTION_STATUS: SUCCEEDED`
If fails: check credentials in tenant yaml, show error.

## Phase 5: Discover Streams

```bash
./tools/declarative-connector/source.sh discover CONNECTOR_PATH <tenant>
```

Parse output and show:
```
Discovered N streams:
  stream_name_1: M fields
  stream_name_2: M fields
```

## Phase 6: Read Data

```bash
# Generate catalog if missing
./airbyte-toolkit/generate-catalog.sh CONNECTOR_NAME <tenant>

# Read data
./tools/declarative-connector/source.sh read CONNECTOR_PATH <tenant>
```

Parse output and verify:
- [ ] Every RECORD has `tenant_id` field (not null, not empty)
- [ ] Every RECORD has `source_id` field (not null, not empty)
- [ ] Every RECORD has `unique_key` field (not null, not empty)
- [ ] `unique_key` contains `tenant_id` and `source_id`
- [ ] Expected streams are present in output
- [ ] Records have expected fields from API

If `read` emits nothing for minutes while the process stays CPU-busy, do NOT just wait — it is almost certainly a silent stall, not slowness. Diagnose:
1. Check the runner's SQLite requests-cache (`/tmp/tmp*/*.sqlite`): a parent-stream cache in the tens/hundreds of MB and no longer growing = heavy `SubstreamPartitionRouter` parent (e.g. `fields=*all`) choking the CDK cache → restructure with a lightweight key-enumeration parent (see create.md MUST NOT list).
2. Distinguish "stalled" from "slow fan-out": process CPU-time (`/proc/<pid>/stat`) steadily growing + records counter advancing = slow but healthy; counter frozen + a `:443` connection stuck in CLOSE_WAIT after a long idle = dead keepalive connection, re-run and watch continuously before concluding it hangs.

Show sample:
```
Stream: email_activity (N records)
  Sample: tenant_id=example_tenant, source_id=m365-main, unique_key=example_tenant-m365-main-...
  Fields: tenant_id, source_id, unique_key, userPrincipalName, sendCount, ...
```

## Phase 7: Verify Schema Completeness

After discover, verify that ALL cursor fields exist in the schema:
- For each stream with `incremental_sync`, check that `cursor_field` is in the stream's `json_schema.properties`
- If missing: the field is from raw API response but not in inline schema → add it to schema and re-test

```bash
# Generate schema from real data
./airbyte-toolkit/generate-schema.sh <name> <tenant>

# Compare generated schema with inline schema in manifest
# Every field in generated schema should be in manifest inline schema
# Especially cursor fields (e.g. end_time for meetings)
```

This prevents ClickHouse destination NPE on deploy.

## Phase 8: Summary

```
=== Test Results ===
  Validate:  PASS
  Mock tests: PASS (N tests) | MISSING (suite not written yet — see create.md §5.7)
  Check:     PASS
  Discover:  PASS (N streams)
  Read:      PASS (N records)
  tenant_id: PASS (present in all records)
  source_id: PASS (present in all records)
  unique_key: PASS (contains tenant_id + source_id)
  Schema:    PASS (all cursor fields present)

Next: /connector deploy <name>
```
