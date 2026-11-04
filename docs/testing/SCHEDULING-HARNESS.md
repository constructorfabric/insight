# Scheduling test harness

This harness supplies deterministic synthetic data through the real Airbyte →
ClickHouse bronze → dbt staging path. It makes scheduling, queueing, retries and
capacity observable without relying on an external source's availability or data.
All values and instances in this document are synthetic.

## Delivery slices

| Slice | Deliverable | Status |
|---|---|---|
| 1 | Controllable source, declarative connector, staging, local deployment, safe trigger errors | Implemented |
| 2 | Airbyte API fault proxy, local routing and declarative image seeding | Implemented |
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
    Reconcile -->|manifest and instance config| Proxy[Optional local Airbyte API proxy]
    Proxy --> Airbyte
    Schedule[Argo schedule or manual Workflow] -->|submit job| Proxy
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
  KUBE_CTX="$KUBE_CTX" NS_APP="$NS_APP" MOCK_TOOLBOX_IMAGE=insight-toolbox:mock-m1
```

The chart constructs namespace-qualified Service URLs, so source pods in another
namespace can resolve them. No other environment or chart values are changed.
The initial file supplies both scenarios. Inspect/reconcile mock using the
normal ingestion commands; then run a real Airbyte sync and staging transform.
To access controls, use the application namespace from your local inventory:

```bash
kubectl --context "$KUBE_CTX" -n "$NS_APP" port-forward service/mock-source 8080:8080
curl -H 'Authorization: Bearer synthetic-control-token' \
  'http://127.0.0.1:8080/observations?after=0&instance=mock-a'
make -C deploy/gitops local-mock-source-remove ENV=local KUBE_CTX="$KUBE_CTX" NS_APP="$NS_APP"
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

## Airbyte API fault proxy

`src/ingestion/tools/mock-airbyte-api/` is a standard-library Python HTTP proxy.
Its sibling local chart is `deploy/gitops/environments/local/mock-airbyte-api/`.
Without a rule it forwards the method, path, query, duplicate request headers
(including authorization and Host) and body to the configured upstream origin.
Chunked request framing, extensions and trailers are preserved. Responses stream
in chunks of at most 64 KiB; the proxy closes each client connection and removes
response hop-by-hop headers, decoding upstream chunk framing. End-to-end headers
and response bytes pass through, including compressed responses. It does not
follow redirects, retry requests or mint credentials.

Only `/health`, `/control/*` and `/observations` are reserved locally. Health is
unauthenticated. Every control/journal call needs
`Authorization: Bearer <control_token>`; upstream API calls carry their own
ordinary Airbyte auth unchanged. Configure rules only against synthetic instances.

### Scenario and matching

`PUT /control/scenario` atomically replaces the rules and returns
`{"revision":1,"generation":"opaque-process-id"}`. The complete scenario is
`{"rules":[...]}`. Each rule has all seven fields below; null disables an id or
call filter. Unknown fields, duplicate names and invalid controls return 400.

```json
{
  "rules": [{
    "name": "second-submit-conflict",
    "method": "POST",
    "path": "/api/v1/connections/sync",
    "connection_id": "00000000-0000-0000-0000-000000000001",
    "job_id": null,
    "call": 2,
    "action": {"type":"status","status":409,"body":"{\"error\":\"synthetic conflict\"}"}
  }]
}
```

Methods are uppercase or `*`. Path patterns use Python `fnmatchcase` glob
semantics (`*`, `?`, brackets), match the complete path and exclude the query.
Connection/job filters match UUIDs or decimal ids under `/connections/{id}` or
`/jobs/{id}`, or recursive JSON body keys `connectionId`, `connection_id`, `jobId`,
`job_id` and `job.id`. Query parameters are forwarded but are not id match inputs.
Malformed/non-JSON bodies still forward and can match method/path rules.

Every matching method/path/id candidate increments that rule's counter, even
when its `call` does not select this request or an earlier rule wins. `call` is
one-based within the revision, across all connections matching that rule. First
eligible rule wins; one action applies. Use separate rules/call numbers to script
successive responses or public listing pages. Counters reset on replacement.
A request captures its rule and revision before waiting or forwarding.

### Actions

| Action object | Behavior |
|---|---|
| `{"type":"status","status":503,"body":"optional text"}` | Return 200–599 without forwarding; supports 409/500/502/503 and synthetic JSON listing fixtures. Body is a literal UTF-8 string, empty if omitted. |
| `{"type":"malformed","body":"{invalid"}` | Return HTTP 200 with these literal bytes and a correct length. |
| `{"type":"truncated","body":"{partial"}` | Return HTTP 200, advertise one more byte than sent, then close. |
| `{"type":"delay","seconds":2}` | Wait before submitting upstream. |
| `{"type":"barrier","barrier":"submit"}` | Hold before submitting until explicitly released. |
| `{"type":"drop_response"}` | Submit once, receive upstream headers, capture up to 64 KiB of response evidence, then close without sending any response to the caller. |
| `{"type":"delay_response","seconds":2}` | Submit once, receive upstream headers, wait, then stream the response. |
| `{"type":"rewrite","field":["job","status"],"value":"cancelled"}` | Replace an existing JSON field in the upstream response; numeric path components index arrays. For example, replace `job.streamStats` with `[]`. |

Delays are finite, 0–60 seconds. A barrier waits at most 60 seconds per request,
then returns 504 without forwarding; it stays closed for subsequent requests.
`POST /control/release` with `{"revision":1,"barrier":"submit"}` opens that
revision's barrier, including a previously captured revision. Release before
entry is supported. A nonexistent revision/barrier returns 404. Wait for
`barrier_entered` in the journal before releasing when a precise boundary matters.

JSON rewrites buffer at most 4 MiB, preserve upstream status, repair Content-Length
and remove stale validators. Missing fields, invalid JSON, compressed responses
or oversized input/output return 502. Use identity encoding for rewrite scenarios.
A string `body` may hold any synthetic JSON, including an entire public listing;
use successive `call` rules to model page movement, missing stats or new status
vocabulary. A `status:200` fixture bypasses the real API and creates no real jobs.

`POST /control/reset` clears rules, counters, retained revisions/barriers and
journal rows only when no API request is active; otherwise 409. Revisions and
journal sequences never rewind. The next PUT gets a new revision. At most 64
revisions (including the initial empty revision) are retained until reset; further
PUTs return 503. Clean up jobs through the independent upstream route, release
barriers, wait for idle and reset between scenarios. Restart loses runtime state
and changes `generation`; an observer must report the evidence gap.

### Proxy evidence and bounds

`GET /observations?after=0` requires an exclusive nonnegative sequence baseline:

```json
{
  "generation":"opaque-process-id",
  "events":[{
    "generation":"opaque-process-id",
    "sequence":2,
    "timestamp":"2026-01-01T00:00:00+00:00",
    "event":"request_started",
    "method":"POST",
    "endpoint":"/api/v1/connections/sync",
    "revision":1,
    "rule":"second-submit-conflict",
    "connection_ids":["00000000-0000-0000-0000-000000000001"],
    "job_ids":[]
  }],
  "oldest_sequence":1,
  "latest_sequence":2,
  "truncated":false,
  "active_requests":[]
}
```

Request sequence ids correlate `request_started`, `upstream_response`, `response`,
`transport_failure`, `request_finished`, `barrier_entered`, `barrier_passed`,
`drop_response` and `delay_response`. `drop_capture_unavailable` records a lost
response whose optional body evidence timed out or was unreadable; that failure
still sends no response to the caller. Saturation emits `request_refused` with
503 without consuming rule counters. Configuration, release and reset also emit
events. Upstream status, delivered status and a dropped response are distinct;
`request_finished.outcome` describes proxy execution, never job success. IDs are
extracted from requests and complete JSON responses no larger than 64 KiB
(rewrite responses use their already-buffered JSON). No response body is retained
in the journal. Status fixtures don't parse their body for ids. Control, health,
malformed HTTP framing and rejected saturated connections are outside the API
request journal. A failure after response headers closes the connection; the
client may see an incomplete response. No proxy retry occurs.

Retention is `MOCK_JOURNAL_RETENTION`, 1–10,000 events. `truncated` reports a
baseline older than retained evidence, including rows removed by reset. Inspect
`generation` on every poll. Logs contain only the same structured fields; no
request/response headers, queries, bodies, credentials or exception strings.
Literal route words starting with a lowercase letter and containing lowercase
letters, digits, `_` or `-` are logged. UUIDs, numeric segments and other path
values are redacted; the result is capped at 512 characters. UUID/decimal ids are
retained at most 16 per kind per event/active entry; matching still examines the full body.
This is correlation evidence for synthetic tests, not a packet capture.

Other bounds: 64 simultaneous connections, 16 active API requests, 64 rules per
revision, 1 MiB decoded request/control body, 2 MiB chunk wire framing, 8 KiB
request headers/trailers, and ten-second client socket inactivity and upstream
connect timeouts. Upstream response inactivity uses `MOCK_UPSTREAM_TIMEOUT`
(seconds), exposed as the proxy chart's `upstreamTimeout` value, defaulting to
300 seconds. Data saturation returns 503 while reserved controls retain connection
capacity. Header parsing additionally uses the standard library's request-line/header-count limits.
Large ordinary responses are streamed without a total byte cap. A peer sending
continuously can keep a connection active; the scenario controller must use its
own overall watchdog and cancel surviving jobs.

## Local proxy routing and acceptance

Source and proxy releases install/remove independently. Their four Make targets
work without `environments/local/inventory.yaml` when `KUBE_CTX` and `NS_APP` are
explicit. The proxy profile and seed targets are also local-only. No profile is
loaded automatically by ordinary application deploys.

Set `KUBE_CTX`, `NS_APP`, `NS_INFRA`, `AIRBYTE_RELEASE` and `APP_RELEASE` to the
local test installation's coordinates. Build a toolbox image containing this
slice and make it available to the local Kubernetes runtime (load into Kind or
use the image store shared with your local cluster):

```bash
export MOCK_TOOLBOX_IMAGE=insight-toolbox:mock-m2
docker build -t "$MOCK_TOOLBOX_IMAGE" \
  -f src/ingestion/tools/toolbox/Dockerfile src/ingestion
export MOCK_AIRBYTE_UPSTREAM="http://${AIRBYTE_RELEASE}-airbyte-server-svc.${NS_INFRA}.svc.cluster.local:8001"
make -C deploy/gitops local-airbyte-manifest-seed ENV=local \
  KUBE_CTX="$KUBE_CTX" NS_INFRA="$NS_INFRA"
make -C deploy/gitops local-mock-source-install local-mock-airbyte-api-install ENV=local \
  KUBE_CTX="$KUBE_CTX" NS_APP="$NS_APP" \
  MOCK_TOOLBOX_IMAGE="$MOCK_TOOLBOX_IMAGE" MOCK_AIRBYTE_UPSTREAM="$MOCK_AIRBYTE_UPSTREAM"
make -C deploy/gitops local-mock-airbyte-api-profile ENV=local \
  KUBE_CTX="$KUBE_CTX" NS_APP="$NS_APP"
helm upgrade "$APP_RELEASE" charts/insight --kube-context "$KUBE_CTX" -n "$NS_APP" \
  --reuse-values -f deploy/gitops/.deploy/local-airbyte-proxy.yaml \
  --set-string ingestion.toolboxImage="$MOCK_TOOLBOX_IMAGE" --wait --timeout 30m
```

With an inventory, the usual `make deploy-app ENV=local CHART=../../charts/insight`
can instead receive
`HELM_UPGRADE_FLAGS='-f .deploy/local-airbyte-proxy.yaml --set-string ingestion.toolboxImage=insight-toolbox:mock-m2'`.
The generated overlay changes only `airbyte.apiUrl`:

| Caller | Route source |
|---|---|
| `airbyte-sync` trigger/poll/record steps and `ingestion-pipeline` defaults | `insight.airbyte.url` helper, passed as `airbyte_url`/`AIRBYTE_URL` |
| Reconcile CronWorkflow, its token minting and private API calls | Same helper → `AIRBYTE_URL` |
| Sweep public `/api/public/v1/jobs` reader and its token minting | Inherits reconcile's `AIRBYTE_URL` |
| Platform ConfigMap consumers | Same helper → `AIRBYTE_API_URL` |

Reconcile's generated connector CronWorkflows and schedule-trigger Workflows
refer to `ingestion-pipeline`; they inherit its chart-rendered URL. Existing
running Workflows retain their captured parameters; resubmit them after enabling
the profile. Explicit `airbyte_url` overrides, host-side CLI `AIRBYTE_URL` values
and arbitrary external callers must be aimed separately. The Airbyte profile
cannot route ClickHouse/Kubernetes faults. Schedule reads
(`CONNECTOR_SCHEDULES_URL`) and service-token minting (`SERVICE_TOKEN_URL`) are
hardcoded separately by `insight.connectorSchedulesEnv`, with no URL values
knob. They do not traverse Airbyte; case 44 is therefore not enabled by this
profile. The same chart can run additional releases with analytics/authenticator
as their direct `upstream` origins (resource names follow the release name).
For that case the local scenario driver must override those two environment
variables in its reconcile Workflow; schedule-trigger has the same two separate
URLs. This slice does not mutate those CronWorkflow environments or add an
analytics routing profile.

Keep `MOCK_AIRBYTE_UPSTREAM` as the independent observer URL inside the cluster.
For host-side direct observation, use a second terminal:

```bash
kubectl --context "$KUBE_CTX" -n "$NS_INFRA" port-forward \
  "service/${AIRBYTE_RELEASE}-airbyte-server-svc" 8001:8001
```

In another terminal expose the proxy control and data port:

```bash
kubectl --context "$KUBE_CTX" -n "$NS_APP" port-forward service/mock-airbyte-api 8081:8080
```

Use the chart's synthetic control token (or the token you explicitly installed):

```bash
curl --fail-with-body -X PUT http://127.0.0.1:8081/control/scenario \
  -H 'Authorization: Bearer synthetic-control-token' -H 'Content-Type: application/json' \
  --data '{"rules":[]}'
kubectl --context "$KUBE_CTX" -n "$NS_APP" get cronworkflows
```

Set `RECONCILE_CRON` to the reconcile CronWorkflow name returned by the listing;
the Helm release may prefix it. Read the controller instance id from that
CronWorkflow's `workflows.argoproj.io/controller-instanceid` label and pass it to
`argo submit` so the controller selects the submitted Workflow. Submit that pass,
then list its generated mock CronWorkflows:

```bash
ARGO_INSTANCE_ID=$(kubectl --context "$KUBE_CTX" -n "$NS_APP" get cronworkflow "$RECONCILE_CRON" \
  -o jsonpath='{.metadata.labels.workflows\.argoproj\.io/controller-instanceid}')
argo submit --context "$KUBE_CTX" -n "$NS_APP" --instanceid "$ARGO_INSTANCE_ID" \
  --from "cronwf/$RECONCILE_CRON" --watch
kubectl --context "$KUBE_CTX" -n "$NS_APP" get cronworkflows
```

Set `MOCK_CRON` to one of the generated mock CronWorkflow names and submit it:

```bash
ARGO_INSTANCE_ID=$(kubectl --context "$KUBE_CTX" -n "$NS_APP" get cronworkflow "$MOCK_CRON" \
  -o jsonpath='{.metadata.labels.workflows\.argoproj\.io/controller-instanceid}')
argo submit --context "$KUBE_CTX" -n "$NS_APP" --instanceid "$ARGO_INSTANCE_ID" \
  --from "cronwf/$MOCK_CRON" --watch
curl --fail-with-body -H 'Authorization: Bearer synthetic-control-token' \
  'http://127.0.0.1:8081/observations?after=0'
```

Require trigger, poll, reconcile and public-listing events in the proxy journal;
observe the corresponding job via port 8001, then verify the workflow's transform
and identity steps and the synthetic staging checks from slice 1. For a controlled
fault, PUT the example rule with the actual synthetic connection id, capture its
revision, and submit two runs. Check the selected call and injected status in the
journal. For accepted-but-lost, change the action to `drop_response`, use `call:1`,
and confirm one upstream job independently even though the submitter loses its
response. Record outcomes; this slice supplies fault controls, not the slice 3
observer or a claim that all scheduling cases pass.

Before removal, restore the direct route by upgrading the same application chart
with `--reuse-values --set-string airbyte.apiUrl=`, restoring the chart's empty
default so it derives the in-cluster Airbyte URL. Then allow reconcile to refresh
schedules and finish/cancel captured proxy workflows.
Release barriers and clean up real synthetic jobs through the independent route.
Then uninstall only the harness releases:

```bash
make -C deploy/gitops local-mock-airbyte-api-remove local-mock-source-remove ENV=local \
  KUBE_CTX="$KUBE_CTX" NS_APP="$NS_APP"
```

### Declarative image version seeding

`make system-airbyte ENV=local` runs a local-only post-setup seed after the normal
Airbyte wizard. On an existing local installation use the standalone seed target
above. It executes
`deploy/gitops/environments/local/airbyte-manifest-versions.sql` in the pinned
chart's embedded `statefulset/airbyte-db`, using that pod's required
`POSTGRES_USER`/`POSTGRES_DB` and `psql -X -v ON_ERROR_STOP=1`. External PostgreSQL
is outside this local profile. A missing pod, missing credentials, rejected SQL
or connection failure stops the target.

The one SQL file pins missing majors 5, 6 and 7 with image tags and multi-platform
index SHAs. `ON CONFLICT (major_version) DO NOTHING` preserves all existing values
and timestamps, including updater-owned rows. This allows local nocode publishing
without requiring the image updater's registry lookup to succeed. It does not
change any non-local deployment.

Pin provenance is public image metadata: [5.17.0 base-image reference](https://hub.docker.com/layers/airbyte/source-square/1.7.3/images/sha256-6867c9d7a307044ea63caf39a7cc424c4d1c00e5404fcebb2ce837295981244b),
[6.61.6 upstream metadata](https://github.com/airbytehq/airbyte/blob/master/airbyte-integrations/connectors/source-plausible/metadata.yaml),
and [7.27.0 index digest](https://hub.docker.com/layers/airbyte/source-declarative-manifest/7.27.0/images/sha256-71b1ef326346f68a77ea84e3b053eb30b2c3c696a5d8f7d5a51836c4ce856f1e).
To refresh, choose a released tag for each supported major and inspect the public
registry with `docker buildx imagetools inspect airbyte/source-declarative-manifest:<tag>`.
Use the top-level multi-platform digest, confirm both required architectures,
update only the SQL pins and provenance here, run the seed tests, and publish a
synthetic nocode connector on a fresh local database. Never derive pins from a
deployed database. Rerunning the seed intentionally does not upgrade existing rows.

### Coverage enabled by this slice

These are fault-injection capabilities for the coverage review's numbered cases;
completion still requires a scenario driver, independent observation and outcome
assertions. An injected symptom does not establish its original cause.

| Cases | Proxy contribution | Additional evidence required |
|---|---|---|
| 2–5 | Concurrent submits, deterministic 409, cancellation/status rewriting, missed-poll barriers | Real jobs, cancellation propagation, workflow intent accounting |
| 16–22 | Token/trigger/poll failures, malformed/truncated JSON, transient status sequences, accepted-but-lost and delays | Caller retries, exact job counts and independent private job details |
| 24 | API unavailability during admission/polling | Actual server restart/OOM still needs Kubernetes faults |
| 29, 33 | Distinguish public job listings from internal listings; fail source updates after hash mutation | Endpoint assertions and hash/credential convergence |
| 36–37, 39 | Discovery/catalog, state export/import/recreate and destination API failpoints/barriers | Incremental state and reconcile convergence assertions |
| 44 | Same proxy code can inject schedule/token faults against separate upstreams; the Airbyte profile alone does not route them | Separate proxies and explicit schedule/auth URL overrides described in the routing section |
| 51 | Hold connection lookup response before returning a stale UUID | Rename/recreate driver and independent job/intent accounting |
| 63–69 | Barriers around listings; listing/status/time/stat fixtures by call; public API failures and pagination movement | Sweep ticks, sync history and watermark assertions, persistent Workflow evidence; Kubernetes/ClickHouse faults remain separate |

## Planned redeploy scenario

For slice 5, save a request for a synthetic connector instance and capture its
revision and CronWorkflow schedule. A release upgrade that restarts analytics,
the trigger and reconcile must leave the request, revision and schedule
unchanged. Run one pass with an older reconcile image without the API read:
it must write the annotation schedule, and the next current pass must restore
the request without changing its revision. Change the Secret schedule annotation
under the active request: the pass must warn once with the instance and both
deployment schedules, keep the requested CronWorkflow schedule, and record the
new inherited schedule. The following unchanged pass must produce no warning.

## Verification

Use the existing connector harness environment described in
[`src/ingestion/tests/connectors/README.md`](../../src/ingestion/tests/connectors/README.md).
The source has no runtime dependencies; pytest runs its tests.

```bash
python3.12 -m pytest src/ingestion/tools/mock-source/tests src/ingestion/tools/mock-airbyte-api/tests -q
python3.12 -m pytest deploy/gitops/scripts/tests/test_local_airbyte_harness.py \
  deploy/gitops/environments/local/mock-airbyte-api/tests -q
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

Also lint/format `src/ingestion/tools/mock-airbyte-api`,
`deploy/gitops/scripts/tests/test_local_airbyte_harness.py` and
`deploy/gitops/environments/local/mock-airbyte-api/tests`, and run
`helm lint deploy/gitops/environments/local/mock-airbyte-api --set-string image=synthetic-toolbox:test --set-string upstream=http://synthetic.invalid:8001`.
Proxy transport tests use connected socket pairs to exercise real HTTP parsers,
streaming and concurrency without opening a TCP listener. The seed test executes
the idempotent INSERT with SQLite's equivalent conflict handling and separately
checks the kubectl/psql invocation; the PostgreSQL and nocode publication acceptance
checks require the local stand. Mutation evidence names the deliberately broken
implementation each new test rejected in the slice result report.

Use Ruff 0.15.21 for the hook's lint/format checks. Source unit tests and manifest
transport tests complement the final real Airbyte sync; neither proves capacity
or scheduling behavior by itself. Later slices add those scenarios here.
