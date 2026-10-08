# Allure TestOps Connector

Allure TestOps projects, launches with their environment and errors, test results, test cases, project custom fields, failure categories and defects, read with an API token.

## Prerequisites

1. An Allure TestOps API token, created under your profile → API tokens. The token acts as its user: the connector syncs every project that user can see.
2. Only to sync a subset: the numeric id of each project to keep. It is the `<id>` in `https://<host>/project/<id>/...`, and the `id` column of the `projects` stream.

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
  # allure_project_ids: '[7, 12]'
  # allure_start_date: "2025-01-01"
```

### Fields

| Field | Required | Description |
|-------|----------|-------------|
| `allure_url` | Yes | HTTPS base URL of the instance. The spec rejects `http://` and a trailing slash. |
| `allure_api_token` | Yes | API token (sensitive). Sent on every request as `Authorization: Api-Token <token>`. |
| `allure_project_ids` | No | JSON array of numeric project ids, e.g. `'[7, 12]'`. Empty or unset: every project the token can see, discovered through `GET /api/project` on each sync, so projects created later are picked up. Set: only these projects. |
| `allure_page_size` | No | Page size on every endpoint: default `100`, range `1`–`1000`. |
| `allure_start_date` | No | `YYYY-MM-DD` (UTC) the first sync of `launches` and `test_results` starts from. Empty means 90 days back. |

The first sync reads launches modified since `allure_start_date`, or in the last 90 days. Later syncs resume from the saved cursor minus a 2-day lookback, so changing `allure_start_date` after the first sync has no effect until `launches` and `test_results` are reset.

A project that joins after the first sync (a new project in Allure, or a new id in the list) does not start from `allure_start_date`. It starts from the newest launch already synced across all projects, minus the lookback. To backfill it, reset `launches` and `test_results`.

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

Each Allure TestOps instance needs its own Secret with a different `insight.cyberfabric.com/source-id`. All projects of one instance come through one Secret.

## Streams

| Stream | Upstream | Sync Mode |
|--------|----------|-----------|
| `projects` | `GET /api/project`: every project the token can see, whatever `allure_project_ids` says. Also the connection check. Each record carries the configured `allure_url`. | Full refresh |
| `launches` | `GET /api/launch/__search` per synced project, filtered by RQL `lastModifiedDate >= <cursor>` with no upper bound | Incremental (`lastModifiedDate`) |
| `test_results` | `GET /api/testresult?launchId=` per launch | Incremental (`lastModifiedDate`, through the launch cursor) |
| `launch_environment` | `GET /api/launch/{id}/env` per launch: one record per environment variable value | Incremental (the launch's `lastModifiedDate`, through the launch cursor) |
| `launch_errors` | `GET /api/launch/error?launchId=` per launch: errors recorded against the launch itself, outside any result | Incremental (the launch's `lastModifiedDate`, through the launch cursor) |
| `test_cases` | `GET /api/testcase/{id}/overview` per test case that `GET /api/testcase/__search` finds per synced project. One record per test case, with its full `customFields` list | Incremental (`lastModifiedDate`, through the search cursor) |
| `custom_fields` | `GET /api/project/{id}/cf` per synced project: every custom field the project defines | Full refresh |
| `categories` | `GET /api/project/{id}/category` per synced project: the failure categories it uses | Full refresh |
| `category_matchers` | `GET /api/project/{id}/categorymatcher` per synced project: the message and trace regexes that assign a category | Full refresh |
| `defects` | `GET /api/defect/{id}` per defect that `GET /api/defect?projectId=` lists per synced project | Full refresh |
| `defect_test_results` | `GET /api/defect/{id}/testresult` per listed defect: the results linked to it | Full refresh |

The synced projects come from an inline `_projects` parent: `GET /api/project` on every sync, kept whole when `allure_project_ids` is empty or unset, filtered to those ids otherwise.

`test_results` reads its launches from an inline `_launches` parent, identical to `launches`, and persists that parent's cursor (`incremental_dependency`). A sync therefore requests results only for launches whose `lastModifiedDate` is inside the window. The child's own `lastModifiedDate` cursor filters nothing: every result of a selected launch is emitted.

`launch_environment` and `launch_errors` read the same `_launches` parent the same way. The parent passes each launch's `projectId` and `lastModifiedDate` down, and every record carries them as `project_id` and `lastModifiedDate`, plus the launch as `launch_id`.

`test_cases` works the same way over an inline `_test_cases` search parent, but its first sync starts at 2000-01-01, so it reads the whole catalog. Later syncs re-read only test cases modified since the saved cursor minus 2 days. Custom field names are not fixed anywhere: each record carries whatever fields its project defines.

`defects` and `defect_test_results` share an inline `_defects` parent: `GET /api/defect?projectId=` per synced project, which stamps each listed defect with its project. Both re-read every listed defect on each sync.

`categories` and `category_matchers` are global objects a project opts into, so one id can appear under several projects. Their records carry the project they were read for as `project_id`, and `unique_key` includes it.

Every list endpoint pages with `page` and `size`, sorted `id,ASC`, until a response has `last: true`. `GET /api/launch/{id}/env` and `GET /api/defect/{id}` answer in one response and do not page.

Every request retries `429`, `500`, `502`, `503` and `504` up to 5 times. It waits `Retry-After` when the response sends one, and backs off exponentially otherwise.

`categories`, `category_matchers` and the `_defects` parent skip a project that answers `403`. `launch_environment` and `launch_errors` skip a launch that answers `404`, and `defects` and `defect_test_results` a defect, as one deleted between the list and the read does.

### Caveats

- A launch deleted mid-sync from a page already read moves the launches after it up one row, so the first launch of the next page is never returned. The 2-day lookback re-reads that launch only if its `lastModifiedDate` is within 2 days of the new cursor.
- `launches` and the `_launches` parent page separately, so such a skip can hit one stream and not the other. Two `connector_quality` checks report it: `assert_allure_launches_reach_test_results` and `assert_allure_test_results_name_a_synced_launch`.
- A custom field edit reaches `test_cases` only if Allure bumps the test case's `lastModifiedDate`. If it does not, the edit lands with the test case's next change.
- `allure__test_results.launch_env` is built when the results are staged. A result staged before its launch's environment landed — `launch_environment` failed in a sync where `test_results` succeeded — keeps `{}` until the launch is modified again or the model is fully refreshed. `assert_allure_launch_environment_names_a_synced_launch` reports environment rows whose launch `launches` never synced.
- The Allure links on `allure__test_results` are built when the results are staged. A result staged while its project record carries no `allure_url` keeps empty links until the model is fully refreshed.
- Full-refresh streams land in an `append_dedup` destination, so a sync adds to bronze and never replaces it. A custom field, category, matcher, defect or defect link deleted in Allure stays in bronze and staging. Its `_airbyte_extracted_at` stops advancing with each sync, which tells it apart.

## Silver Targets

None. `dbt/` holds eleven staging models tagged `allure` and nothing else. No silver class or gold model reads them.

| Model | Grain |
|-------|-------|
| `allure__launches` | One row per launch |
| `allure__test_results` | One row per test result |
| `allure__test_cases` | One row per test case; `custom_fields` maps each field name to its values |
| `allure__test_case_custom_fields` | One row per test case × custom field value, rebuilt each run so removed values drop out |
| `allure__custom_fields` | One row per custom field a project defines |
| `allure__launch_environment` | One row per launch × environment variable value |
| `allure__launch_errors` | One row per launch error |
| `allure__categories` | One row per project × failure category |
| `allure__category_matchers` | One row per project × category matcher |
| `allure__defects` | One row per defect |
| `allure__defect_test_results` | One row per defect × linked test result |

`allure__test_results.test_case_id` joins `allure__test_cases.test_case_id`; `category_id` joins `allure__categories.category_id`. `allure__test_results.launch_env` holds the launch's environment as a JSON object, variable name → value. `test_result_url`, `launch_url` and `test_case_url` open the result, its launch and its test case in Allure, built from the `allure_url` its project record carries. `allure__defect_test_results.test_result_id` joins `allure__test_results.test_result_id`.

No identity inputs: the connector syncs no user directory. `createdBy` and `lastModifiedBy` are Allure logins, and nothing resolves them to a person.

## Tests

```bash
cd src/ingestion/tests/connectors
.venv/bin/pytest ../../connectors/quality/allure/tests
```
