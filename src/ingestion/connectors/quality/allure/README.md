# Allure TestOps Connector

Allure TestOps projects, launches, test results, test cases and project custom fields, read with an API token.

## Prerequisites

1. An Allure TestOps API token, created under your profile → API tokens. The token acts as its user, so that user needs access to every project in `allure_project_ids`.
2. The numeric id of each project to sync. It is the `<id>` in `https://<host>/project/<id>/...`, and the `id` column of the `projects` stream.

## K8s Secret

```yaml
apiVersion: v1
kind: Secret
metadata:
  name: insight-allure-main
  labels:
    app.kubernetes.io/part-of: insight
  annotations:
    insight.cyberfabric.com/connector: allure
    insight.cyberfabric.com/source-id: allure-main
type: Opaque
stringData:
  allure_url: "https://allure.example.com"
  allure_api_token: "CHANGE_ME"
  allure_project_ids: '[7, 12]'
  # allure_start_date: "2025-01-01"
```

### Fields

| Field | Required | Description |
|-------|----------|-------------|
| `allure_url` | Yes | HTTPS base URL of the instance. The spec rejects `http://` and a trailing slash. |
| `allure_api_token` | Yes | API token (sensitive). Sent on every request as `Authorization: Api-Token <token>`. |
| `allure_project_ids` | Yes | JSON array of numeric project ids, at least one. Launches, test results, test cases and custom fields are read for these projects only. |
| `allure_page_size` | No | Page size on every endpoint: default `100`, range `1`–`1000`. |
| `allure_start_date` | No | `YYYY-MM-DD` (UTC) the first sync of `launches` and `test_results` starts from. Empty means 90 days back. |

The first sync reads launches modified since `allure_start_date`, or in the last 90 days. Later syncs resume from the saved cursor minus a 2-day lookback, so changing `allure_start_date` after the first sync has no effect until `launches` and `test_results` are reset.

A project id added after the first sync does not start from `allure_start_date`. It starts from the newest launch already synced across all projects, minus the lookback. To backfill it, reset `launches` and `test_results`.

### Automatically injected

| Field | Source |
|-------|--------|
| `insight_tenant_id` | `tenant_id` from tenant YAML |
| `insight_source_id` | `insight.cyberfabric.com/source-id` annotation |

### Local development

```bash
cp src/ingestion/secrets/connectors/allure.yaml.example src/ingestion/secrets/connectors/allure.yaml
# Fill in real values, then apply:
kubectl apply -f src/ingestion/secrets/connectors/allure.yaml
```

### Multi-instance

Each Allure TestOps instance needs its own Secret with a different `insight.cyberfabric.com/source-id`. Several projects on one instance go into one `allure_project_ids` list.

## Streams

| Stream | Upstream | Sync Mode |
|--------|----------|-----------|
| `projects` | `GET /api/project`: every project the token can see, not only `allure_project_ids`. Also the connection check. | Full refresh |
| `launches` | `GET /api/launch/__search` per configured project, filtered by RQL `lastModifiedDate >= <cursor>` with no upper bound | Incremental (`lastModifiedDate`) |
| `test_results` | `GET /api/testresult?launchId=` per launch | Incremental (`lastModifiedDate`, through the launch cursor) |
| `test_cases` | `GET /api/testcase/{id}/overview` per test case that `GET /api/testcase/__search` finds per configured project. One record per test case, with its full `customFields` list | Incremental (`lastModifiedDate`, through the search cursor) |
| `custom_fields` | `GET /api/project/{id}/cf` per configured project: every custom field the project defines | Full refresh |

`test_results` reads its launches from an inline `_launches` parent, identical to `launches`, and persists that parent's cursor (`incremental_dependency`). A sync therefore requests results only for launches whose `lastModifiedDate` is inside the window. The child's own `lastModifiedDate` cursor filters nothing: every result of a selected launch is emitted.

`test_cases` works the same way over an inline `_test_cases` search parent, but its first sync starts at 2000-01-01, so it reads the whole catalog. Later syncs re-read only test cases modified since the saved cursor minus 2 days. Custom field names are not fixed anywhere: each record carries whatever fields its project defines.

Every endpoint pages with `page` and `size`, sorted `id,ASC`, until a response has `last: true`.

Every request retries `429`, `500`, `502`, `503` and `504` up to 5 times. It waits `Retry-After` when the response sends one, and backs off exponentially otherwise.

### Caveats

- A launch deleted mid-sync from a page already read moves the launches after it up one row, so the first launch of the next page is never returned. The 2-day lookback re-reads that launch only if its `lastModifiedDate` is within 2 days of the new cursor.
- `launches` and the `_launches` parent page separately, so such a skip can hit one stream and not the other. Two `connector_quality` checks report it: `assert_allure_launches_reach_test_results` and `assert_allure_test_results_name_a_synced_launch`.
- A custom field edit reaches `test_cases` only if Allure bumps the test case's `lastModifiedDate`. If it does not, the edit lands with the test case's next change.

## Silver Targets

None. `dbt/` holds five staging models tagged `allure` and nothing else. No silver class or gold model reads them.

| Model | Grain |
|-------|-------|
| `allure__launches` | One row per launch |
| `allure__test_results` | One row per test result |
| `allure__test_cases` | One row per test case; `custom_fields` maps each field name to its values |
| `allure__test_case_custom_fields` | One row per test case × custom field value, rebuilt each run so removed values drop out |
| `allure__custom_fields` | One row per custom field a project defines |

`allure__test_results.test_case_id` joins `allure__test_cases.test_case_id`.

No identity inputs: the connector syncs no user directory. `createdBy` and `lastModifiedBy` are Allure logins, and nothing resolves them to a person.

## Tests

```bash
cd src/ingestion/tests/connectors
.venv/bin/pytest ../../connectors/quality/allure/tests
```
