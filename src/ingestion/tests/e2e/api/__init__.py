"""Rig smoke tests + API endpoint contract tests.

`test_session_smoke.py` pokes each session fixture; the other `test_*` modules
are the endpoint contract suite: together they exercise EVERY operation in the
committed OpenAPI spec (docs/components/backend/analytics/openapi.json) through
the recording client, so the endpoint-coverage gate needs no SKIP_LIST. One
module per path group, one test per (path, method, status-code) case:

  test_catalog.py            POST /v1/catalog/get_metrics
  test_metrics.py            GET+POST /v1/metrics · GET+PUT+DELETE /v1/metrics/{id}
                             POST /v1/metrics/{id}/query · POST /v1/metrics/queries
  test_metric_thresholds.py  GET+POST /v1/metrics/{id}/thresholds
                             PUT+DELETE /v1/metrics/{id}/thresholds/{tid}
  test_admin_thresholds.py   GET+POST /v1/admin/metric-thresholds
                             GET+PUT+DELETE /v1/admin/metric-thresholds/{id}
  test_columns.py            GET /v1/columns · GET /v1/columns/{table}
  test_persons.py            GET /v1/persons/{email}

Resources come from fixtures (`api/conftest.py`): `scratch_metric` /
`scratch_threshold` / `admin_threshold_row` create the row a case needs and
delete it afterwards, so the metric catalog (`metric_catalog`, the
metric-coverage gate's universe) is never touched, soft-deleted scratch metrics
stay invisible to `GET /v1/metrics`, and the yaml rig's batch-query path is
untouched.

Status codes: success codes plus every error code reachable in the rig are
pinned (400 validation, 404 unknown/soft-deleted, 500 identity-unconfigured).
The remaining declared codes (401/403/409/429 and generic 500s) are
unreachable by design here — auth is disabled and nothing rate-limits.

Authz notes pinned by the tests: the rig runs auth-disabled with
`X-Insight-Tenant-Id: TEST_TENANT_ID` on every request; the admin gate
(`is_tenant_admin`) is a documented stub returning true, but per-row tenant
ownership is enforced (`tenant_id == Some(caller)`), so the admin lifecycle
operates only on its own tenant-scope row — the seeded product-default rows
(tenant_id NULL) are deliberately not readable per-id.
"""
