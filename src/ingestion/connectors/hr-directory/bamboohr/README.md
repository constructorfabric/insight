# BambooHR Connector

Employee directory, leave requests, Who's Out, and field metadata from BambooHR via API key
authentication. Python CDK source.

## Prerequisites

1. Log in to BambooHR as an admin
2. Go to **Account > API Keys** and generate a new API key
3. Note your BambooHR subdomain (e.g., `acme` from `acme.bamboohr.com`)

## K8s Secret

```yaml
apiVersion: v1
kind: Secret
metadata:
  name: insight-bamboohr-main
  labels:
    app.kubernetes.io/part-of: insight
  annotations:
    insight.cyberfabric.com/connector: bamboohr
    insight.cyberfabric.com/source-id: bamboohr-main
type: Opaque
stringData:
  bamboohr_api_key: ""    # BambooHR API key
  bamboohr_domain: ""     # Subdomain (e.g. "acme")
```

### Fields

| Field | Required | Description |
|-------|----------|-------------|
| `bamboohr_api_key` | Yes | BambooHR API key (Account > API Keys) |
| `bamboohr_domain` | Yes | BambooHR subdomain (e.g. `acme` from `acme.bamboohr.com`) |
| `bamboohr_start_date` | No | Leave requests and Who's Out history start date, ISO format (default: `2020-01-01`) |

### Automatically injected

Set by `reconcile-connectors` / `connect.sh`, must NOT be in the Secret:

| Field | Source |
|-------|--------|
| `insight_tenant_id` | `tenant_id` from tenant YAML |
| `insight_source_id` | `insight.cyberfabric.com/source-id` annotation |

### Local development

```bash
cp src/ingestion/secrets/connectors/bamboohr.yaml.example src/ingestion/secrets/connectors/bamboohr.yaml
# Fill in real values, then apply:
kubectl apply -f src/ingestion/secrets/connectors/bamboohr.yaml
```

Run the connector directly against a config file:

```bash
pip install -e src/ingestion/connectors/hr-directory/bamboohr
source-bamboohr spec
source-bamboohr check --config config.json
source-bamboohr discover --config config.json
```

## Streams

| Stream | Description | Sync Mode |
|--------|-------------|-----------|
| `employees` | Employee directory via the custom report API | Full refresh |
| `leave_requests` | Time-off requests from `bamboohr_start_date` to today | Full refresh |
| `meta_fields` | Field metadata (names, types, aliases) | Full refresh |
| `whos_out` | Employee time off and visible company holidays from `bamboohr_start_date` to today (UTC) | Full refresh |

### Who's Out

The stream calls [`time_off/whos_out`](https://documentation.bamboohr.com/reference/list-whos-out)
with `filter=off` to bypass the caller's saved employee calendar filter. Holiday
visibility still depends on BambooHR configuration and the caller's access;
a holiday entry does not establish which employees observe it.

Both `timeOff` and `holiday` entries retain their API payloads. Keys include the
tenant, source, entry type, ID, and start date to distinguish event types and
recurring holiday occurrences. Holidays need not contain an `employeeId`.

HTTP 403 skips only this stream with a warning, so an unavailable Who's Out
feature does not fail the other streams. Other API and transport failures remain
errors. Skipped or empty results do not establish that employees were available.

This stream collects Bronze data only. It does not feed Silver HR events,
metrics, comparisons, or attention panels. Full-refresh extraction uses the
existing append-only destination configuration; deleted or changed occurrences
are not reconciled here. Downstream availability semantics remain out of scope.

### Employee fields

Each sync reads `meta/fields` and requests every non-deprecated field it names in
the custom report, so customer-defined fields are collected without configuration.
The bronze columns declared by the `employees` stream are always requested, whatever
the field metadata returns; every field the report answers with — declared column or
not — is preserved in `raw_data`, which is what change detection and the field-level
history read.

BambooHR caps a custom report at 400 fields and answers a larger request with a
`400`, so an account defining more than that is read in several requests and the
rows merged on employee id.

Requested fields the API key cannot read are dropped from the report silently, with
the call still succeeding. An API key may hold access to only a subset of the
declared bronze columns; the sync proceeds and publishes the missing ones as null.
Bronze columns are named by alias and come back under it, so their absence is
detectable — the connector logs a warning naming them. The rest cannot be checked
that way — field metadata lists entries a custom report will not return, and a
field asked for by numeric id comes back under an indexed `<id>.N` key — so an
apparent gap there says more about BambooHR's naming than about access. A report
that declares no columns at all is treated as unverifiable and not warned about.

`SENSITIVE_FIELDS` in `source_bamboohr/streams/employees.py` is the exception:
government identifiers, protected demographics, personal contact details, street
address, photos, social profiles, and compensation amounts are never requested and
are dropped from `raw_data` if the report returns them anyway. The list is fixed in
the connector and covers standard BambooHR aliases — a customer-defined field
carries no classification in the field metadata and is collected in full.

## Silver Targets

- `class_people` — unified person registry (via `bamboohr__to_class_people`)
- `identity_inputs` — identity signals for the Identity Manager (via `bamboohr__identity_inputs`)
- `class_hr_events`, `class_hr_working_hours` — leave and schedule facts

## Build & deploy (CDK)

```bash
cd src/ingestion
./reconcile-connectors/main.sh          # discovers, builds, registers, connects
./run-sync.sh bamboohr <tenant>         # e2e: Airbyte sync → dbt (Bronze → Silver)
./logs.sh -f latest
```
