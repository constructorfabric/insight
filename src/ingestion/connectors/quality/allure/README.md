# Allure TestOps Connector

Allure TestOps projects, launches and test results, read with an API token.

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
```

### Fields

| Field | Required | Description |
|-------|----------|-------------|
| `allure_url` | Yes | HTTPS base URL of the instance. The spec rejects `http://` and a trailing slash. |
| `allure_api_token` | Yes | API token (sensitive). Sent on every request as `Authorization: Api-Token <token>`. |
| `allure_project_ids` | Yes | JSON array of numeric project ids, at least one. Launches and test results are read for these projects only. |
| `allure_page_size` | No | Page size on every endpoint: default `100`, range `1`–`1000`. |

The first sync reads launches modified in the last 90 days. Later syncs resume from the saved cursor minus a 2-day lookback.

A project id added after the first sync does not start 90 days back. It starts from the newest launch already synced across all projects, minus the lookback. To backfill it, reset `launches` and `test_results`.

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

`test_results` reads its launches from an inline `_launches` parent, identical to `launches`, and persists that parent's cursor (`incremental_dependency`). A sync therefore requests results only for launches whose `lastModifiedDate` is inside the window. The child's own `lastModifiedDate` cursor filters nothing: every result of a selected launch is emitted.

Every endpoint pages with `page` and `size`, sorted `id,ASC`, until a response has `last: true`.

Every request retries `429`, `500`, `502`, `503` and `504` up to 5 times. It waits `Retry-After` when the response sends one, and backs off exponentially otherwise.

### Caveats

- A launch deleted mid-sync from a page already read moves the launches after it up one row, so the first launch of the next page is never returned. The 2-day lookback re-reads that launch only if its `lastModifiedDate` is within 2 days of the new cursor.
- `launches` and the `_launches` parent page separately, so such a skip can hit one stream and not the other. Two `connector_quality` checks report it: `assert_allure_launches_reach_test_results` and `assert_allure_test_results_name_a_synced_launch`.

## Silver Targets

None. `dbt/` holds two staging models, `allure__launches` and `allure__test_results`, tagged `allure` and nothing else. No silver class or gold model reads them.

No identity inputs: the connector syncs no user directory. `createdBy` and `lastModifiedBy` are Allure logins, and nothing resolves them to a person.

## Tests

```bash
cd src/ingestion/tests/connectors
.venv/bin/pytest ../../connectors/quality/allure/tests
```
