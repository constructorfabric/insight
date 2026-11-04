# Scheduling test harness

This harness supplies deterministic synthetic data through the real Airbyte →
ClickHouse bronze → dbt staging path. It makes scheduling, queueing, retries and
capacity observable without relying on an external source's availability or data.
All values and instances in this document are synthetic.

## Delivery slices

| Slice | Deliverable | Status |
|---|---|---|
| 1 | Controllable source, declarative connector, staging, local deployment, safe trigger errors | Implemented |
| 2 | Airbyte API fault proxy, including accepted requests whose responses disappear | Planned |
| 3 | Scenario controller and durable observer | Planned |
| 4 | Local capacity/fault profiles and state/transform fixtures | Planned |
| 5 | Required scheduling, contention, recovery and evidence scenarios | Planned |

Slice 1 has no API proxy, Kubernetes scenario runner, durable observer, business
silver tables, identity contributor or gold metrics. A successful HTTP read
cannot establish Airbyte admission, pod lifetime, destination commit or pipeline
completion.

## Components and callers

```mermaid
flowchart LR
    Operator -->|install synthetic Secrets| Kubernetes
    Kubernetes -->|discover Secrets| Reconcile
    Reconcile -->|manifest and instance config| Airbyte
    Schedule[Argo schedule or manual Workflow] -->|submit job| Airbyte
    Airbyte -->|probe and paginated GET| Source[Mock source]
    Airbyte -->|append dedup| Bronze[bronze_mock]
    Bronze -->|dbt FINAL reads| Staging[staging.mock__events and mock__updates]
    Controller[Operator; future scenario controller] -->|control API| Source
    Source -->|journal API| Observer[Operator; future durable observer]
```

The source is Python standard library code in `src/ingestion/tools/mock-source/`.
The existing toolbox `COPY . /ingestion` includes it and the connector in
`src/ingestion/connectors/testing/mock/`. No new image pipeline is needed.
The local-only Helm chart is `deploy/gitops/environments/local/mock-source/`.
It creates one Deployment/Service, initial scenarios, an authentication Secret
and two connector Secrets, `mock-a` and `mock-b`.

The connector has two data streams: full-refresh `mock_events` and incremental
`mock_updates`, with `last_updated_at` as its source-defined cursor. The empty
`mock_probe` helper is also advertised: declarative `CheckStream` needs a catalog
stream to check. Checks read `/probe`; discovery reads inline schemas without
HTTP. The helper emits no records and needs no staging relation. Reconcile
enables every advertised stream using the usual append-dedup catalog.

## Creating an instance

First install its scenario with the control API. Then create an Opaque Secret
in the application's namespace with:

```yaml
metadata:
  name: mock-a
  labels:
    app.kubernetes.io/part-of: insight
  annotations:
    insight.cyberfabric.com/connector: mock
    insight.cyberfabric.com/source-id: mock-a
stringData:
  base_url: "http://mock-source.example.com/instances/mock-a"
  api_token: "synthetic-mock-token"
```

Instance ids match `[a-z][a-z0-9-]{0,62}`. The URL's instance must match the
Secret's source id. Reconcile injects `insight_tenant_id` and
`insight_source_id`. It discovers the `name: mock`, `type: nocode` descriptor,
uses namespace `bronze_mock`, and selects `tag:mock+` for dbt. Runtime controls
stay outside the connector Secret, so changing a fault does not recreate an
Airbyte connection or lose its state. First-time creation may trigger the
ordinary initial sync; capture scenario baselines after configuration and
reconciliation. For controlled initialization, run reconcile with
`--connector mock --no-sync-trigger` and account for the periodic reconciler.

## Data API

Every data/probe request requires `Authorization: Bearer <api_token>`.

| Method and path | Request | Response |
|---|---|---|
| `GET /health` | No authentication | `200 {"status":"ok","generation":"opaque-process-id"}` |
| `GET /instances/{id}/probe` | Data bearer token | `200 {"records":[]}`; invalid credentials return 401 |
| `GET /instances/{id}/streams/mock_events` | Optional continuation `page_token` | Page below |
| `GET /instances/{id}/streams/mock_updates` | Optional UTC `cursor`; optional continuation `page_token` | Page below |

```json
{
  "records": [{
    "id": "mock_events-0",
    "value": 17,
    "checksum": "synthetic-checksum",
    "last_updated_at": "2026-01-01T00:00:00Z",
    "scenario_revision": 1
  }],
  "next_page_token": "opaque-read-and-position",
  "revision": 1,
  "read_id": "opaque-read-id",
  "generation": "opaque-process-id"
}
```

The example's value and checksum are illustrative. Actual checksum is lowercase
SHA-256 of UTF-8 `seed:instance:stream:index`; value is the integer represented by
its first seven hex digits. The id is `stream-index`, independent of seed, epoch
and revision. Timestamp is epoch plus index seconds. Changing the epoch changes
timestamps, while changing the seed changes payloads. Cursor filtering is
exclusive (`last_updated_at > cursor`). An exhausted window returns an empty
page. Use an epoch before the connector's current time.

The connector stamps `tenant_id`, `source_id` and
`unique_key = tenant-source-id` (where the final id includes the stream).
Both tagged staging models rebuild from bronze with `FINAL`, preserving value
and checksum. They introduce new relations, with no older mock schema to heal.
For independent assertions, compare per-source counts, `sum(value)` and sorted
`(id, checksum)` against the formula above; exclude revision and extraction
timestamps from deterministic comparisons. Read staging with `FINAL` too.
Append-dedup preserves ids from earlier, larger source populations: use a fresh
instance for an exact-count scenario that shrinks the population.

## Control API

Control and journal routes require `Authorization: Bearer <control_token>`.
This is separate from the connector's data token. No endpoint accepts tokens in
query strings. Errors contain only a generic error, never a request body or
credentials.

`PUT /control/instances/{id}` creates or replaces the current scenario. The
complete shape is required; omitted/unknown fields are rejected with 400:

```json
{
  "seed": 7,
  "epoch": "2026-01-01T00:00:00Z",
  "streams": {
    "mock_events": {
      "record_count": 20,
      "page_size": 5,
      "latency_seconds": 0,
      "duration_minutes": 0,
      "fail_after_pages": null,
      "fail_requests": 0,
      "failure_status": 503,
      "retry_after_seconds": 1,
      "auth_reject": false,
      "gate_page": null
    },
    "mock_updates": {
      "record_count": 20,
      "page_size": 5,
      "latency_seconds": 0,
      "duration_minutes": 0,
      "fail_after_pages": null,
      "fail_requests": 0,
      "failure_status": 503,
      "retry_after_seconds": 1,
      "auth_reject": false,
      "gate_page": null
    }
  }
}
```

Response: `200 {"instance":"mock-a","revision":1,"generation":"opaque-process-id"}`. Revisions increase across
instances and resets for this server process. A request captures its scenario;
continuations retain that revision, even after a replacement. Separate stream
reads can capture different revisions. Retrying an initial request without a
continuation starts a new read; read ids are correlation hints, never job ids.

| Control | Semantics |
|---|---|
| `seed` / `epoch` | Unsigned 32-bit seed; UTC epoch with whole seconds, year at most 9998 |
| `record_count` / `page_size` | 0–1,000,000 records; 1–1000 records per page |
| `latency_seconds` | Minimum delay per successful data page, 0–10 seconds |
| `duration_minutes` | 0–60 minutes distributed over the remaining cursor window's pages, starting with the first data request; delay is the greater of latency and duration/page count |
| `fail_after_pages` | Null disables; K rejects page K and later (zero-based), after K successful page positions |
| `fail_requests` | Fail the first N requests per revision and stream, then recover; independent for each instance |
| `failure_status` | Injected status 400–599; retry classification belongs to the connector |
| `retry_after_seconds` | 0–60; `Retry-After` on injected failures, including 429 |
| `auth_reject` | Reject data for that stream with 401, before consuming failures or entering gates |
| `gate_page` | Null disables; block the selected zero-based page until explicit release |

Duration settings requiring more than ten seconds per page are rejected. A
resumed incremental window must also have enough pages; increase record count
or lower page size. Gates and retries add time beyond duration. Probe requests
consume none of these controls. The probe tests the data token and the events
stream's auth rejection. The connector retries 429/500/502/503/504 at most three
times and fails terminal 400/401/403/404/410 responses.

`POST /control/instances/{id}/release` with
`{"revision":1,"stream":"mock_events"}` returns
`200 {"released":true,"revision":1}`. Releases are scoped to that revision and
stream, including old captured revisions; a release can precede gate entry.
Each blocked request waits at most twenty seconds, then returns 503 with
`Retry-After: 1`; the gate remains closed. Release promptly when testing a
successful gated job, before connector retries are exhausted.

`POST /control/instances/{id}/reset` removes the instance's scenario, sessions
and retained journal rows only while idle. It returns
`200 {"reset":true,"latest_sequence":42}`. Active requests and unfinished
paginated reads cause 409. Abandoned sessions expire after 300 seconds without
a request; active requests never expire. Recreate the instance with PUT after
reset. Resetting one instance preserves another's evidence. Sequences never
rewind. Server restart loses all runtime state and restores the initial file;
the future observer must key baselines by the explicit process `generation` id.

## Journal API and counting

`GET /observations?after=0&instance=mock-a` returns:

```json
{
  "generation": "opaque-process-id",
  "events": [{
    "generation": "opaque-process-id",
    "sequence": 2,
    "timestamp": "2026-01-01T00:00:00+00:00",
    "event": "request_finished",
    "instance": "mock-a",
    "stream": "mock_events",
    "read_id": "opaque-read-id",
    "revision": 1,
    "page": 0,
    "cursor": null,
    "traffic": "data",
    "status": 200
  }],
  "oldest_sequence": 1,
  "latest_sequence": 2,
  "truncated": false,
  "active_requests": []
}
```

`after` is required and exclusive; `instance` is optional. Every event and page
carries its process generation. Recheck generation even on empty journal polls;
on a change, restart from `after=0` and report lost evidence. Events are ordered by
the global sequence. Event kinds are `configured`, `probe`, `request_started`,
`request_finished`, `request_refused`, `gate_entered`, `gate_released`,
`gate_passed`, and `reset`. Finished status is the computed response status;
499 denotes request cancellation before a response. Active entries carry
`read_id`, `instance`, `stream`, `revision` and the active `requests` count.
Probe traffic has `traffic: probe`; data request events have `traffic: data`.
Discovery has no HTTP event. Malformed HTTP framing is rejected before data
dispatch and is not a source-read event.

Retention is bounded by `MOCK_JOURNAL_RETENTION` (1–10,000 rows). A baseline
older than retained evidence sets `truncated: true`; observers must surface the
gap. Instance reset intentionally removes its old rows and emits a reset event.
The source also bounds connections (64), simultaneous data requests (16),
instances (32), read sessions (256), request bodies (64 KiB), headers (8 KiB),
and header/body/write waits (5 seconds). Data saturation returns 503 while
control/health routes remain available within the connection bound. Completed
sessions remain available for replay until expiry; session exhaustion returns
503. Structured logs contain the journal fields, never headers or bodies.

**Count syncs by distinct Airbyte job id, never by HTTP requests or read ids.**
Checks, pagination, retries, attempts and checkpoint resumes can all multiply
requests. The durable observer must correlate request intent, Workflow UID and
node/retry, connection id, Airbyte job/attempt id, workload/pod lifetime,
source revision/read id and sweep tick. The source cannot supply Airbyte ids
that the declarative runtime does not send. Preserve independent Airbyte and
Workflow observations; do not infer one replication slot from non-overlapping
HTTP reads. Cancel surviving jobs and release gates during scenario cleanup.

## Local deployment

Use an already configured local GitOps inventory and cluster. Set `KIND_CLUSTER`
to your own local Kind cluster name if loading an image with Kind. Build once
and use the same toolbox image for mock-source, reconcile and dbt:

```bash
docker build -t insight-toolbox:mock-m1 \
  -f src/ingestion/tools/toolbox/Dockerfile src/ingestion
kind load docker-image insight-toolbox:mock-m1 --name "$KIND_CLUSTER"
make -C deploy/gitops deploy-app ENV=local CHART=../../charts/insight \
  HELM_UPGRADE_FLAGS='--set-string ingestion.toolboxImage=insight-toolbox:mock-m1'
make -C deploy/gitops local-mock-source-install ENV=local \
  MOCK_TOOLBOX_IMAGE=insight-toolbox:mock-m1
```

The chart constructs namespace-qualified Service URLs, so source pods in another
namespace can resolve them. No other environment or chart values are changed.
The initial file supplies both scenarios. Inspect/reconcile mock using the
normal ingestion commands; then run a real Airbyte sync and staging transform.
To access controls, use the application namespace from your local inventory:

```bash
kubectl -n "$NS_APP" port-forward service/mock-source 8080:8080
curl -H 'Authorization: Bearer synthetic-control-token' \
  'http://127.0.0.1:8080/observations?after=0&instance=mock-a'
make -C deploy/gitops local-mock-source-remove ENV=local
```

Removal uninstalls only this local release; the ordinary reconcile pass then
removes the connections for its deleted connector Secrets. It does not cancel
Airbyte jobs already in flight or remove bronze/staging data.

For source-only development without Kubernetes, set every runtime input:

```bash
MOCK_HOST=127.0.0.1 MOCK_PORT=8080 MOCK_JOURNAL_RETENTION=1000 \
MOCK_API_TOKEN=synthetic-mock-token MOCK_CONTROL_TOKEN=synthetic-control-token \
MOCK_SCENARIOS_FILE=deploy/gitops/environments/local/mock-source/scenarios.json \
python3.12 src/ingestion/tools/mock-source/server.py
```

## Verification

Use the existing connector harness environment described in
[`src/ingestion/tests/connectors/README.md`](../../src/ingestion/tests/connectors/README.md).
The source has no runtime dependencies; pytest runs its tests.

```bash
python3.12 -m pytest src/ingestion/tools/mock-source/tests -q
python3.12 -m pytest src/ingestion/reconcile-connectors/tests -q
python3.12 -m pytest deploy/gitops/environments/local/mock-source/tests -q
cd src/ingestion/tests/connectors
.venv/bin/pytest ../../connectors/testing/mock/tests
.venv/bin/pytest
cd ../../../..
helm lint deploy/gitops/environments/local/mock-source --set-string image=synthetic-toolbox:test
CLICKHOUSE_PASSWORD=synthetic dbt parse --no-partial-parse \
  --project-dir src/ingestion/dbt --profiles-dir src/ingestion/dbt
ruff check src/ingestion/tools/mock-source src/ingestion/connectors/testing/mock/tests \
  deploy/gitops/environments/local/mock-source/tests \
  src/ingestion/reconcile-connectors/python/schedule_trigger.py \
  src/ingestion/reconcile-connectors/tests/test_schedule_trigger.py
ruff format --check src/ingestion/tools/mock-source src/ingestion/connectors/testing/mock/tests \
  deploy/gitops/environments/local/mock-source/tests \
  src/ingestion/reconcile-connectors/python/schedule_trigger.py \
  src/ingestion/reconcile-connectors/tests/test_schedule_trigger.py
```

Use Ruff 0.15.21 for the hook's lint/format checks. Source unit tests and manifest
transport tests complement the final real Airbyte sync; neither proves capacity
or scheduling behavior by itself. Later slices add those scenarios here.
