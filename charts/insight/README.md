# Insight umbrella chart

Single canonical unit of delivery for the Insight platform.

See [MCP authentication](../../deploy/MCP_AUTHENTICATION.md) for connection lifetime,
revocation, and upgrade behavior.

For token-authenticated, read-only SQL execution, see the
[SQL query API guide](../../deploy/SQL_QUERY_API.md).

For raw-data ingestion and rotating its instance token, see the
[raw-data ingest guide](../../deploy/RAW_DATA_INGEST.md).

- **Chart**: `insight`
- **Version**: see `Chart.yaml` → `version`
- **App version**: see `Chart.yaml` → `appVersion` (matches image tags)

## What it contains

The umbrella bundles ONLY the first-party application services. Each is a
local `file://` subchart:

| Component            | Source                                              | Toggle                       | Default |
|----------------------|-----------------------------------------------------|------------------------------|---------|
| Gateway (edge)       | `src/backend/services/gateway/helm`                 | mandatory (no flag)          | on      |
| Authenticator        | `src/backend/services/authenticator/helm`           | mandatory (no flag)          | on      |
| Analytics            | `src/backend/services/analytics/helm`               | mandatory (no flag)          | on      |
| Insight v3 Core      | `src/backend/services/insight-v3-core/helm`         | `global.insightV3Core.deploy` | off     |
| Identity Resolution  | `src/backend/services/identity-resolution/helm`     | `identityResolution.deploy`  | on — the validator refuses `false` |
| Keycloak (broker)    | `src/backend/services/keycloak/helm`                | `keycloak.deploy`            | off     |
| Previews             | `src/backend/services/previews/helm`                | `global.previews.enabled`    | on      |
| Git CLI proxy        | `src/backend/services/git-cli-proxy/helm`           | `gitCliProxy.deploy`         | on      |
| Frontend (SPA)       | `src/frontend/helm`                                 | `frontend.deploy`            | on      |

> Identity Resolution is required: the authenticator's login-bootstrap resolve
> exists only there, so `insight.validate` fails the render when
> `identityResolution.deploy` is false. It is not an OIDC provider.

## What it does NOT contain

| Component                          | Why separate                                          | How to install                              |
|------------------------------------|-------------------------------------------------------|---------------------------------------------|
| ClickHouse / MariaDB / Redis / Redpanda (L2 infra) | Operated independently; shared lifecycle / managed services | Separate releases in `insight-infra` (gitops `make system-*`); the umbrella dials them via `<dep>.host` |
| Airbyte                            | Heavy (10+ pods), its own release cadence             | Separate helm release                       |
| Argo Workflows                     | Cluster-scoped infra, often shared across products    | Separate helm release                       |
| Plugins                            | Runtime-managed via UI (not Helm — see architecture)  | Through platform API                        |

See [`deploy/HELM_DEPLOY.md`](../../deploy/HELM_DEPLOY.md) for the full
external-consumer runbook and [`deploy/gitops/README.md`](../../deploy/gitops/README.md)
for the Makefile-driven deployment pipeline.

## Airbyte sync monitoring

The sync poller waits for Airbyte to report `succeeded`, `failed`, or `cancelled`.
Pending and queued jobs, retries (`incomplete`), and running jobs continue to be monitored
even when their record counters remain unchanged. Airbyte owns job liveness,
retry policy, and maximum duration; the poll pod has no independent job deadline.

`ingestion.airbyteSync.pollIntervalSeconds` controls polling frequency (default
30 seconds). `ingestion.airbyteSync.statusUnreadableThresholdSeconds` bounds
continuous failure to read a recognized job status (default 1800 seconds).
Every readable status resets that timer. On expiry, the poll step fails with
`sync.poll_failed`; the Airbyte job's outcome remains unknown and the job is not
cancelled. Downstream transforms do not run without confirmed sync success.

When upgrading, replace `ingestion.airbyteSync.idleThresholdSeconds` overrides
with `statusUnreadableThresholdSeconds` if an API outage timeout override is
needed. The old idle setting is no longer used. Deploy the chart with its matching
toolbox image because the poller's environment variable also changed.

## Release name convention

**This chart assumes release name = `insight`.**

Internal DNS references between app services (e.g. `http://insight-analytics:8081`, `http://insight-identity-resolution:8082`) are templated with the `insight-` prefix. Helm subcharts use `{{ .Release.Name }}-{chart-suffix}` for service naming, which produces these exact names when the release is `insight`. (External L2 infra is reached via the explicit `<dep>.host` wiring, not the release-name convention.)

If you install under a different name, override all cross-service URLs in your own values.yaml. Prefer sticking to the convention.

## Install

The published artifact is the normal install path:

```bash
helm upgrade --install insight oci://ghcr.io/constructorfabric/charts/insight \
  --version <x.y.z> \
  --namespace insight --create-namespace \
  -f my-values.yaml \
  --wait --timeout 10m
```

Omit `--version` for the latest published release. The full prerequisite list
(Gateway API, OIDC IdP, external L2 systems, required Secrets) is in
[`deploy/HELM_DEPLOY.md`](../../deploy/HELM_DEPLOY.md).

To iterate on the chart from a working copy:

```bash
helm dependency update charts/insight
helm template insight charts/insight --namespace insight   # dry-run render
helm upgrade --install insight charts/insight \
  --namespace insight --create-namespace -f my-values.yaml
```

## Install (production checklist)

Before going to prod:

- [ ] Decide on credentials strategy:
  - **Auto-gen (default):** `credentials.autoGenerate: true` — the umbrella creates `insight-db-creds` with random 24-char passwords on first install and reuses them via `lookup` on every upgrade.
  - **BYO / Constructor Platform:** pre-create `insight-db-creds` with all required keys (`clickhouse-password`, `mariadb-password`, `mariadb-root-password`, `redis-password`) before the first `helm install`. The umbrella picks them up. Missing/empty keys fail fast.
    - Works regardless of `credentials.autoGenerate`: the chart auto-detects BYO via absence of the `app.kubernetes.io/managed-by=Helm` label on the existing Secret and skips its own Secret-template emission, so Helm never tries to take ownership of the customer-managed Secret. No manual labeling required.
    - **Dry-run note**: `helm install --dry-run` (default, client-side) skips the `lookup` function, so the BYO preview will incorrectly show the chart emitting `insight-db-creds`. Use `helm install --dry-run=server` (Helm ≥3.13) for an accurate BYO sanity-check — it exercises `lookup` against the real cluster.
  - Rendering under a GitOps controller (`helm template`): set `deploymentMode: gitops` and `autoGenerate: false`, and supply the config Secrets out-of-band — the validator refuses `gitops` + `autoGenerate: true`.
- [ ] Configure OIDC under `authenticator.oidc.*`: `issuerUrl`, `clientId`, `redirectUri`, and the client secret via a Secret (never inline in a committed values file).
- [ ] Provide `insight-authenticator-signing-keys` (ES256 `current.pem`) — not auto-generated.
- [ ] With `global.insightV3Core.deploy` on, on every install path — GitOps or imperative — provide `insight-v3-core-token` with a `token` key of at least 32 bytes (`openssl rand -hex 32`). The v3 Deployment reads it by `secretKeyRef` (`insightV3Core.ingest.tokenSecret` / `.tokenKey`); without the Secret the pod does not start. Rotation: [raw-data ingest guide](../../deploy/RAW_DATA_INGEST.md).
- [ ] Attach routes to the shared Gateway: `gateway.route`, `frontend.route` (TLS terminates at the Gateway listener)
- [ ] Bump resources where needed (default `requests` are conservative)
- [ ] Provision the L2 infra (ClickHouse / MariaDB / Redis / Redpanda) out-of-chart and fill `<dep>.host` / `.port` / `.passwordSecret`. App-service URLs follow automatically (resolved by helpers).
- [ ] Set `global.imagePullSecrets` if pulling from a private registry

## Infra wiring

L2 infra (ClickHouse, MariaDB, Redis, Redpanda) is always **external** — deployed out-of-chart as separate releases in `insight-infra` (gitops `make system-*`), or pointed at managed services. The umbrella only carries the wiring it needs to dial them:

- `<dep>.host` is required (the validator / helpers fail fast otherwise). Redpanda uses `<dep>.brokers`.
- `<dep>.port` / `.database` / `.username` as applicable.
- `<dep>.passwordSecret` points at a Secret in the namespace (e.g. `insight-db-creds`) — auto-generated by the umbrella, mirrored by a platform operator, or pre-created (BYO).
- App-service URLs are computed by helpers from `<dep>.host` / `.port`, so no extra overrides are needed.

The umbrella validator (`templates/_helpers.tpl` → `insight.validate`) fails fast on the typical misconfigurations: missing `<dep>.host` / `.brokers`, missing `passwordSecret.{name,key}`, `identityResolution.deploy: false`, incomplete `authenticator.oidc` (e.g. `resolveBy: email` without `identityResolution.rosterSourceType`), and the `gitops` + `autoGenerate: true` combination.

## ClickHouse topology

A clustered ClickHouse needs `Replicated*` table engines and, depending on how
its databases were created, an `ON CLUSTER` clause; a single node needs
neither. Two chart values carry that one decision to every creator in the
release, and they are the one place an operator sets it. Under
`credentials.autoGenerate: false` the chart renders no service Secrets:
`deploy/gitops/scripts/compose-app-secrets.sh` builds them from the same two
`values.yaml` keys, so a flip reaches identity-resolution and v3-core only once
the composer has re-run and the Secrets are re-sealed; until then the two
services hold the old pair and refuse to start against a database whose engine
contradicts it.

- `clickhouse.clusterMode` (bool, default `false`) — created engines replicate.
- `clickhouse.clusterName` (string, default `""`) — the cluster the few
  statements that carry an `ON CLUSTER` clause name: the `CREATE DATABASE`s,
  the bronze heals and migrations, the destination's bronze DDL. Required
  whenever `clusterMode` is on, and `helm template` fails without it:
  replicated DDL that reaches only the node a creator connected to leaves
  every other replica without the relation. Every creator downstream reads
  the pair from this release; the chart check is the earliest, and
  `scripts/lib/ch-topology.sh`, `compose_destination_config.py` and the
  sweep's `ledger.py` refuse the flag without a name again at run time. The
  name is a bare ClickHouse identifier (`^[A-Za-z_][A-Za-z0-9_]*$`,
  `insight.clickhouse.requireNamedCluster`): every creator splices it into
  DDL unquoted. That check is separate from `insight.validate`.

Reading the pair is not the same as emitting a clause, and the table below says
which layer does what. The Rust services emit no `ON CLUSTER` and create no
database: they render a `Replicated*` engine on every table they create and
refuse to start work against a database whose engine contradicts the pair.

The same decision under a different name in each layer:

| Layer | Name | Where |
| --- | --- | --- |
| Chart values | `clickhouse.clusterMode` / `clickhouse.clusterName` | [`values.yaml`](./values.yaml) |
| Chart helpers | `insight.clickhouse.clusterMode` / `insight.clickhouse.clusterName` | [`templates/_helpers.tpl`](./templates/_helpers.tpl) |
| Toolbox jobs | `CLICKHOUSE_CLUSTER_MODE` / `CLICKHOUSE_CLUSTER_NAME` | the `<release>-platform` ConfigMap, the `clickhouse-databases` and `clickhouse-migrate` Jobs (one shared env block, `insight.clickhouse.ddlEnv`), `scripts/lib/ch-topology.sh`, which renders the database engine `create-databases.sh` creates with, expands the `/*@@REPLICATED@@*/` and `/*@@ON_CLUSTER@@*/` markers a numbered migration spells its engine and its cluster clause behind, and refuses a deploy the warehouse cannot serve; and `scripts/lib/ch-exec.sh`, whose every request in cluster mode carries the session settings a replicated warehouse needs (`distributed_ddl_output_mode=throw_only_active`, a 60 s `distributed_ddl_task_timeout`, quorum writes with sequential-consistency reads, `allow_nondeterministic_mutations`, `mutations_sync`/`alter_sync` 2) and a 900 s transport ceiling. The `clickhouse-field-value-map` Job carries the same pair to `scripts/load_field_value_map.py`, whose every request on a cluster carries the quorum writes and sequential-consistency reads its anti-join rests on |
| Reconcile | `RECONCILE_DEST_CLICKHOUSE_CLUSTER_MODE` / `RECONCILE_DEST_CLICKHOUSE_CLUSTER_NAME` | `reconcile-cron.yaml`, consumed by `compose_destination_config.py` as the destination's `use_replicated_engines` / `cluster_name`, and by the sweep's `sweep/ledger.py`, whose every ledger request on a cluster carries quorum writes and sequential-consistency reads, whose identity mutation waits for every replica (`mutations_sync=2`) and whose catalogue read goes through `clusterAllReplicas(<name>, system.tables)` |
| dbt | project var `cluster_mode`, read by the `insight_engine` macro every engine is declared through -- the models', the `on-run-start` hooks' raw DDL and the jira journal's stages; the project-level `+engine` default for a model that declares none; the profile's `cluster:`, `database_engine: Replicated` and `cluster_mode: true` keys and the quorum / mutation / CTE `custom_settings`, all written by the one profile writer. No dbt statement carries `ON CLUSTER`: `database_engine: Replicated` is what keeps the adapter from adding it, and the hooks never render it | `dbt_project.yml`, `dbt/macros/insight_engine.sql`, `scripts/dbt_profiles.py` |
| Rust | `insight_clickhouse::Topology` on `Config.topology`, from `clickhouse_cluster_mode` / `clickhouse_cluster_name` (the name is accepted for the shared contract and unused: a service emits no `ON CLUSTER`), decides the engine family every `insight_clickhouse::Ddl` create renders (`ReplicatedMergeTree`, argument-less, or `MergeTree`), the read-after-write settings a swap needs (only identity-resolution's staging count and watermark take them; v3-core's record inserts and preview reads never read their own write) and the wait every `Client::execute_ddl` makes for the active replicas (`throw_only_active`, 30 s). No service creates a database; `Client::assert_database_topology` refuses a missing one (naming `create-databases.sh`) or one whose engine contradicts the topology | `libs/insight-clickhouse` (`topology.rs`, `ddl.rs`, `consistency.rs`, `coordination.rs`), `insight-v3-core` and `identity-resolution` gear config |

Tests that must know the topology read `CLICKHOUSE_CLUSTER_MODE` /
`CLICKHOUSE_CLUSTER_NAME` from their harness, defaulting to a single node.

Every step that runs dbt — the Argo steps, the gold build, bootstrap, the
sample seeder, the jira transform rig, the data-path harness — takes its
profile body from `src/ingestion/scripts/dbt_profiles.py`, so a step cannot
reach a clustered warehouse with a standalone profile. The engine default is
the one thing that profile cannot carry: dbt-clickhouse resolves `engine` from
the model config alone, so it lives in `dbt_project.yml`.

One statement carries most of the fork: `src/ingestion/scripts/create-databases.sh`
is the only site that creates a ClickHouse database, so the `Replicated`
database engine has exactly one place to appear. Inside such a database every
later DDL is logged in Keeper and replayed on every replica, which is what lets
a creator state a table without naming a cluster. The
`clickhouse-databases` Hook (`pre-install,pre-upgrade`, weight 5) and the
`clickhouse-migrate` Hook (`post-install,post-upgrade`, weight 200) both run
that script; `clickhouse.createDatabases: false` turns the first one off for
a warehouse provisioned out of band. The
roles and grant-less users ride along in the same script, from both Hooks;
`clickhouse.provisionAccess: false` leaves those to the operator while the
databases are still created and checked.

Replaying the DDL is not replicating the rows, though. A table still declares
its own `Replicated*` engine, or it stands on every replica holding whatever
reached the node that answered — which is why the dbt models render a prefix
through `insight_engine` and the numbered migrations spell their engine behind
`/*@@REPLICATED@@*/` — a SQL comment, so the file stays executable unrendered
for the harnesses that apply it against a single node.

What a clustered install needs belongs to the server, not to this release:

- one shard with N replicas. Nothing in this repo creates a `Distributed`
  table; every creator emits `Replicated*` only, so a second shard holds
  nothing Insight writes;
- Keeper, the `{shard}` / `{replica}` macros, and the cluster `clusterName`
  names in every node's `<remote_servers>`, with `internal_replication` true
  and an interserver `<secret>` (the bronze sweep runs on the peers as the
  deploy user);
- SQL-created access stored in Keeper: a `<replicated>` entry in
  `<user_directories>` ahead of any writable `local_directory`. The guard
  counts `type = 'replicated'` rows only, so a local directory listed first
  passes it and still catches every `CREATE ROLE`. Or
  `clickhouse.provisionAccess: false`;
- `clickhouse.host` at a load balancer over the replicas;
- `clickhouse.runMigrations: true`: the post-install Hook is what checks and
  converges the databases on every upgrade;
- a deploy user holding `CREATE DATABASE`, `REMOTE ON *.*` and reads of
  `system.zookeeper`, `system.distributed_ddl_queue`, `system.user_directories`,
  `system.macros` and `system.clusters` (a denied `system.zookeeper` read is
  reported as "no Keeper");
- Airbyte's `destination-clickhouse` at 2.1.29 or later
  (`reconcile-connectors/python/check_destination_version.py`; reconcile
  refuses an older one).

The deploy checks what it can before it creates or grants anything, and each
refusal carries its own remedy:

| Guard | What it reads | Why |
| --- | --- | --- |
| `clusterName` is set and exists | `system.clusters` | every `ON CLUSTER` statement names it; the flag without a name would create a node-local `Replicated` database |
| Keeper is reachable | `system.zookeeper` | the `Replicated` database engine and the DDL queue coordinate through it |
| `{shard}` and `{replica}` are defined | `system.macros` | the database and table engines name them in their Keeper paths |
| access storage is replicated | `system.user_directories` | SQL-created roles and users are otherwise node-local, and authentication through the load balancer succeeds or fails by which replica answers. A warehouse that stores access locally takes no role from this release: create them on every replica yourself and set `clickhouse.provisionAccess: false`, which turns this guard into a note and skips every provisioning script |
| an existing database carries the right engine | `system.databases` | the flag never converts a database that already stands; a standalone deploy is checked in the opposite direction, since it would emit standalone DDL into a `Replicated` database |
| every `Replicated` database has joined every replica | `system.zookeeper` (`<database keeper path>/replicas`, against the host count of `clusterName` in `system.clusters`), `system.distributed_ddl_queue` | a replica that never ran the `CREATE DATABASE` answers empty; one whose `CREATE` is still queued because it was down is a note, not a refusal, and replays it on return |
| every `bronze_*` MergeTree table is `Replicated*` | `system.tables` on every replica (`clusterAllReplicas`) | the destination creates with `IF NOT EXISTS`, so a bronze table from before the flip keeps its plain engine on the one replica that created it; the guard lists each such table with its replica and names the remedy (drop the bronze database `ON CLUSTER` and re-sync, or convert the tables in place along the [conversion runbook](../../docs/operations/CLICKHOUSE-CLUSTER-CONVERSION.md)). The Jira rebuild asserts the same of its source before copying it. This is the one probe that needs every replica up: the one that is down may hold the table. It runs on the peers as the deploy user, which needs `REMOTE ON *.*` and `SHOW` on every `bronze_*` table (a full-access admin has both) and the cluster's `<remote_servers>` entry to carry an interserver `<secret>` |

Bronze is the exception to "no `ON CLUSTER` anywhere else": `bronze_*` is an
Atomic database the Airbyte destination creates, so nothing replays DDL issued
in it, and a heal or migration that creates, drops, renames or exchanges a
table there carries the clause.

With a replica down, a deploy does not start: the bronze-engine sweep needs
every replica to answer, since the absent one may hold the table it looks for.
Restore the replica, then deploy. A replica that drops out while a deploy
runs usually fails it, but not through the coordinated DDL: a `CREATE` or
`DROP` waits for the replicas Keeper still counts active
(`throw_only_active`, 60 s per statement from the hooks, 30 s from a service)
and fails only while the departed one is still counted; once its session has
expired, the statement completes on the others and the absent replica
replays its queue on return. The completeness guard notes a `CREATE DATABASE`
only while it is still queued for the inactive host; by the next deploy the
replica must be back anyway, and has usually replayed it. What does fail
outright is every mutation and ALTER, which run with `mutations_sync` /
`alter_sync` 2: ClickHouse waits for an inactive replica for
`replication_wait_for_inactive_replica_timeout` (120 s by default; nothing
here changes it) and then fails the statement as unfinished. So does every
quorum insert, which fails rather than leave one replica with the old rows,
and a heal whose
table probe the balancer could not answer: the hook stops there instead of
skipping the heal as if the table were absent. Nothing is retried around the
absent replica: restore it and re-run the deploy, which is idempotent. The
two hook Jobs carry an `activeDeadlineSeconds` sized from those ceilings.

A two-replica cluster loses its quorum writes during a replica restart:
`insert_quorum = auto` is a majority, and the majority of two is both, so
each connector sync's dbt build, the deploy hook's heals, the field-value-map
seed, the reconcile sweep's ledger writes and the persons sync fail until the
replica is back. The destination's bronze writes,
`insight-v3-core`'s dataset records and analytics' usage events carry no
quorum and keep landing. Three replicas tolerate one down.

Flipping the values converts nothing that already stands: an existing
standalone install moving to a cluster converts its databases and bronze
tables to the replicated engines first, along the
[conversion runbook](../../docs/operations/CLICKHOUSE-CLUSTER-CONVERSION.md)
(`src/ingestion/scripts/convert-database-to-cluster.sh`, one database at a
time, on the node that holds it), and the guards above refuse the deploy
until it has — in both directions, so the way back is in the runbook too.
[`cluster-mode.yml`](../../.github/workflows/cluster-mode.yml) runs four legs
in the merge queue (umbrella check "Cluster rig"): a standalone leg; a
replicated leg that runs the full deploy hook, a connector's bronze and its
dbt build twice on a two-replica warehouse and compares every node; a
warm-upgrade leg that upgrades a warm installation on that warehouse; and a
conversion leg that executes the runbook on a standalone install made on one
of the replicas. The standalone and replicated legs also deploy what the
warehouse must refuse: the flag the wrong way round, a plain bronze table from
before the flip, a replica down. Its rig
([`src/ingestion/tests/cluster-rig`](../../src/ingestion/tests/cluster-rig/README.md))
also spells out what a clustered ClickHouse must provide before Insight installs
onto it.

## Values reference

See comments in [`values.yaml`](./values.yaml) — every block is documented inline.

Key groups:

- `credentials.deploymentMode` / `credentials.autoGenerate` — who owns the generated Secrets (`helm` with lookup-based reuse, or `gitops` with out-of-band Secrets)
- `global.*` — cluster-wide defaults (pull secrets, storage class, `observability.logs.{level,format}`, `observability.otlp.endpoint`)
- `<dep>.host` / `<dep>.port` / `<dep>.passwordSecret` (Redpanda: `<dep>.brokers`) — external-infra wiring for ClickHouse, MariaDB, Redis, Redpanda
- `gateway` / `authenticator` / `analytics` — **mandatory** app services (no deploy flag; the gateway is the single entrance and the product is one unit)
- `global.insightV3Core.deploy` + `insightV3Core.*` — insight-v3-core, off by default (it needs `insightV3Core.ingest.tokenSecret` to exist first); the same flag drops the gateway's v3 routes and the `insight_v3_ro` ClickHouse role, which reads every table so that an install without the assistant does not carry it. Turning the flag off on a running install strips that role's grants; the role and its user are left standing, since the deploy did not create the identity
- `authenticator.oidc.*` — OIDC upstream and login-resolution mode (`resolveBy: external_id | email`)
- `identityResolution.*` — identity-resolution service (must stay deployed; `rosterSourceType` for email-mode logins)
- `keycloak.deploy` + `keycloakConfig.*` — the in-stack identity broker and its realms-as-code hook
- `clickhouse.clusterMode` / `clickhouse.clusterName` — warehouse topology, see [ClickHouse topology](#clickhouse-topology)
- `clickhouse.createDatabases` — whether the pre-install Hook creates the release's ClickHouse databases. `false` is for a warehouse provisioned out of band: the post-install Hook converges them only with `clickhouse.runMigrations: true` and `ingestion.toolboxImage` set, and on a fresh install with v3-core deployed its pre-install migrate Job (weight 10) asserts `insight_datasets` before that Hook runs, so the install fails unless the database already stands (see [ClickHouse topology](#clickhouse-topology))
- `clickhouse.provisionAccess` — whether the release provisions the ClickHouse roles and grant-less users beside them; `false` leaves access to the operator (a cluster without a replicated user directory)
- `clickhouse.fieldValueMap.*` — insert-only seed of operator-authored `config.field_value_map` / `config.field_value_defaults` rows from a TSV ConfigMap (post-upgrade hook, weight 300)
- `previews.*`, `gitCliProxy.*`, `frontend.*` — optional services, on by default
- `ingestion.templates.enabled` — whether to ship Argo WorkflowTemplates; requires Argo CRDs to be present in the cluster

## Operations

```bash
# Status
helm -n insight status insight
kubectl -n insight get pods -l app.kubernetes.io/part-of=insight

# Upgrade (new appVersion → update image tags via -f values.yaml)
helm upgrade insight charts/insight -n insight -f my-values.yaml

# Rollback
helm -n insight rollback insight <REVISION>

# Uninstall (does NOT delete PVCs for stateful components — cleanup manually)
helm -n insight uninstall insight
kubectl -n insight delete pvc -l app.kubernetes.io/part-of=insight
```

## Publishing

Publishing is automated: the `publish-chart` job in
[`.github/workflows/build-images.yml`](../../.github/workflows/build-images.yml)
packages the umbrella and pushes it to
`oci://ghcr.io/constructorfabric/charts/insight` on every push to `main` and
`release-*`, then commits the `version`/`appVersion` bump back to the branch.
Do not push chart packages by hand.
