# Airbyte deep dive: architecture, replication, and multitenancy

> Scope: Airbyte's **data-replication platform**. Airbyte also offers a separate
> context layer for AI agents; that product is not covered here. This document
> is project-independent. It starts with the compact mental model, then goes
> into protocol, state, persistence, scaling, security, and multitenancy. It was
> checked against the official Airbyte documentation on 2026-08-13.

## What Airbyte is

Airbyte is an open-source data-integration platform. It moves data from a
**source**—such as an API, database, or file store—to a **destination**—such as
a warehouse, lake, database, or operational application.

Airbyte is not read-only as a whole. In a conventional replication connection,
the source connector reads source records and the destination connector writes
them to the destination; Airbyte does not write the copied records back to the
source. Give source connectors read-only, least-privilege credentials whenever
the connector permits it. CDC can additionally require database-specific
replication privileges and access to replication slots, publications, or logs.
Destination connectors—and reverse-ETL connections—need write permission in
their target system. Always check the permissions documented for the exact
connector and sync mode.

Moving data into analytical storage is commonly called ELT. Moving warehouse
data back into an operational application is data activation, or reverse ETL.

Its usual job is ELT:

```text
source -> extract -> load into destination -> transform downstream
```

Airbyte provides connectors, configuration, scheduling, retries, state,
logging, schema handling, and monitoring. It is not the source system, the
destination storage, or a complete analytics/transformation system.

## The mental model

| Term | Meaning |
|---|---|
| **Connector** | Versioned code that knows how to talk to one kind of system. A connector is normally packaged as a container image. |
| **Source** | A configured source connector: connector + credentials + settings. |
| **Destination** | A configured destination connector: connector + credentials + settings. |
| **Connection** | The pipeline joining one configured source to one configured destination. It holds stream selection, sync modes, schedule, namespace, and schema-change policy. |
| **Stream** | A set of related records: for example, a database table or an API resource. |
| **Catalog** | The streams and JSON Schemas discovered by a source. A configured catalog adds the user's stream, field, cursor, primary-key, and sync-mode choices. |
| **Sync job** | One run of a connection. A failed job can have multiple retry **attempts**. |
| **State** | An opaque checkpoint produced and understood by the source connector, such as a timestamp, page token, or database-log position. |
| **Workspace** | A group of sources, destinations, connections, and related configuration. |

The most important distinction is:

```text
connector = reusable program
source/destination = configured instance of that program
connection = rule for moving data between two configured instances
```

## What the platform contains

Airbyte is conceptually split into the **platform** and **connectors**.

```text
                       CONTROL AND ORCHESTRATION
 UI / API -> config server -> Temporal -> worker -> workload API -> launcher
                    |                                      |
              config/job database                    starts a workload
                                                           |
 DATA MOVEMENT                                             v
                  +-----------------------------------------------+
                  | source connector -> orchestrator -> destination|
                  +-----------------------------------------------+
                              records + checkpoints
```

In a typical self-managed Kubernetes deployment, the important components are:

[Open the self-contained Airbyte component and relationship map](./AIRBYTE_ARCHITECTURE.html)
for a visual control-plane, workload, protocol, persistence, and external-system
view. The page also contains a complete relationship table for the major
components described below.

- **UI and API/server**: create and manage sources, destinations, connections,
  schedules, and jobs.
- **PostgreSQL configuration/job database**: stores platform configuration and
  job history. Secrets can be integrated with an external secret manager.
- **Temporal**: persists and coordinates workflow execution.
- **Worker**: applies connection scheduling and sequencing logic.
- **Workload API and launcher**: queue work, apply back pressure, and start
  Kubernetes workload pods.
- **Workload pods**: temporary runtime containers for `spec`, `check`,
  `discover`, or a full source-to-destination replication.
- **Container orchestrator/sidecar**: interprets connector messages, records
  results, collects statistics, scrubs sensitive values, handles checkpoints,
  and manages other platform bookkeeping.
- **Cron and bootloader**: perform maintenance, connector-definition updates,
  database migrations, and startup checks.
- **Source and destination connectors**: independent programs that implement
  the Airbyte Protocol.
- **Connector catalog/registry**: metadata and versioned images for available
  connectors, alongside workspace-specific custom connectors.

Names and packaging can change between Airbyte versions and managed plans, but
the responsibilities remain similar. Airbyte is available as managed cloud,
hybrid, and self-managed software. Kubernetes with Helm is the documented
self-managed deployment model.

Users can control a running platform through its UI, HTTP API, language SDKs,
or Terraform provider. PyAirbyte is different: it is a Python library for
running connectors in Python workflows without operating the full platform; it
is not an administration SDK for an Airbyte server.

### What can be verified from a terminal

The names are easy to confuse:

| Tool | Useful verification | Important limit |
|---|---|---|
| **PyAirbyte**—the Python package is installed as `airbyte` | Validate connector configuration, run `check`, discover streams and schemas, inspect samples, and perform a bounded local read. | It runs connectors from Python; it does not inspect the configuration database of an existing Airbyte server. A read can consume source API quota and return sensitive data. |
| **`abctl`** | Install or upgrade local Airbyte Core and report the local installation, chart, and application status with `abctl local status`. | It is a deployment-management CLI, not a general connector or remote-server validation CLI. |
| **Airbyte API or SDK** | List configured resources, inspect connection/job status, trigger a sync, and retrieve its result from an existing platform. | A successful job proves protocol completion, not destination correctness or business-level completeness. |
| **Connector protocol commands** | Run `spec`, `check`, `discover`, or `read` directly against a connector executable/container. | This is low-level connector development and debugging; the platform normally invokes these commands. |

A safe synthetic PyAirbyte smoke test can run without an Airbyte server:

```bash
python -m pip install airbyte
```

```python
import airbyte as ab

source = ab.get_source("source-faker", config={})
source.check()
print(source.get_available_streams())
```

For a real connector, keep its configuration outside shell history and source
control. Start with `check` and discovery. Run `read`, `get_samples`, or
`print_samples` only in an approved environment because those operations access
records. To verify an existing Airbyte deployment, use its API/SDK and add an
independent destination-side check; PyAirbyte is not a substitute for that.

## What Airbyte persists

The data being copied is only one kind of data in an Airbyte system. An operator
must account for four persistence classes:

| Persistence class | Typical contents | Why it matters |
|---|---|---|
| **Configuration database** | Workspaces, connector definitions and versions, source/destination configuration, connections, schedules, catalog selections, job records, and other control metadata. | Losing it loses the configured platform. It is also a security boundary. |
| **Replication state** | Source-defined checkpoints such as a cursor, page token, or database-log position. | Losing or corrupting it can force a re-read, create duplicates, or create a data gap if replaced incorrectly. |
| **Logs and workload output** | Platform logs, connector logs, traces, and operation results. | Logs and state can contain sensitive values even when record payloads are transient. |
| **Destination data** | The copied records, Airbyte metadata columns, temporary tables, and—depending on destination generation—raw or final tables. | The destination remains the durable data system and needs its own access, retention, and backup policy. |

Self-managed Airbyte can use object storage for state, logs, and workload
output. A durable external PostgreSQL database and external object storage avoid
making the lifecycle of platform state depend on one Kubernetes installation.

Connector credentials need special attention. The documented default for
self-managed Airbyte stores connector secrets as unencrypted plain text in the
configured database. Airbyte recommends an external secret manager and supports
AWS Secrets Manager, Google Secret Manager, Azure Key Vault, and HashiCorp
Vault. Database encryption alone is not a replacement for least-privilege
secret access and rotation.

This guide intentionally does not claim which secret mode the Insight project—or
any particular deployment—currently uses. Repository configuration describes
intent, not necessarily the rendered or running installation. For a
self-managed installation, verify it without exposing secret values:

1. Record the Airbyte application and Helm chart versions; configuration keys
   and supported integrations can change between releases.
2. Inspect the rendered Helm values and workload environment for the configured
   secret-persistence backend. Do not print Kubernetes `Secret` contents.
3. If an external manager is configured, verify only the provider, secret
   references, service-account identity, and access policy—not the credential
   payloads.
4. Confirm that new connector credentials are written to the expected backend
   and that Airbyte stores references rather than plain values. Perform any
   database inspection only in an isolated synthetic environment because the
   default database may expose every connector secret.
5. Verify the running deployment separately from the repository. If no external
   secret manager is explicitly configured, treat the documented plain-text
   database default as the security assumption until proven otherwise.

Airbyte describes copied record payloads as transient and purged after transfer,
but this does **not** mean the whole platform is free of sensitive data at rest.
Configuration metadata includes stream and field names; state can contain
cursor values; and logs may contain state or connector diagnostics. Treat the
configuration database, state storage, logs, and secrets manager as sensitive.

## What happens before a sync

Each connector implements a small command interface:

1. `spec` returns a JSON Schema describing its configuration form and
   capabilities.
2. `check` validates the supplied configuration and access.
3. A source's `discover` returns its catalog: streams, fields, schemas, supported
   sync modes, possible cursors, and primary keys.
4. The user creates a connection and chooses streams, fields, source and
   destination sync modes, schedule, destination namespace, and schema policy.

This separation lets the platform run connectors written in different
languages, as long as they follow the same protocol.

## What happens during a sync

1. A schedule, API call, or UI action creates a sync job.
2. The platform queues the workload. The launcher starts a replication pod when
   capacity is available.
3. The source receives its configuration, the configured catalog, and the last
   committed state.
4. The source runs `read` and emits newline-delimited JSON messages on standard
   output. The important runtime messages are:

   - `RECORD`: one data record and its stream;
   - `STATE`: a checkpoint;
   - `LOG` and `TRACE`: diagnostics and structured failure information.

5. The container orchestrator validates and relays records and state messages to
   the destination's `write` command over standard input.
6. The destination loads records. It emits a received state message only after
   every earlier record has been successfully written. The platform can then save
   that state as a safe checkpoint.
7. On success, the job is completed. On failure, Airbyte creates another
   attempt and resumes from the last safe checkpoint when the connector supports
   it.

This is why the protocol is more than “source prints JSON, destination reads
JSON”: checkpoints pass through the destination before becoming committed.

### The checkpoint handshake

The ordering rule is the core of Airbyte's resumability:

```text
source          orchestrator          destination          platform state
  | RECORD A -------->|------------------->|                    |
  | RECORD B -------->|------------------->|                    |
  | STATE S1 -------->|------------------->|                    |
  |                   |                    | write A and B      |
  |                   |<--------- STATE S1 |                    |
  |                   |---------------------------------------> | persist S1
```

The destination must return state messages in the order received. Returning
`S1` means all records before `S1` were written successfully. The platform and
destination treat the contents of state as a black box; only the source should
interpret or modify it.

If the attempt fails after the destination writes A and B but before `S1` is
persisted, the retry starts from the previous state and may send A and B again.
This is the practical reason Airbyte provides **at-least-once**, not
exactly-once, delivery.

### State types

The Airbyte Protocol has three state shapes:

| State type | Meaning | Operational consequence |
|---|---|---|
| **`STREAM`** | Each stream emits and owns its state independently. | A stream can be reset or checkpointed without replacing unrelated stream state. It also permits stream-level replication isolation and potential parallelism. |
| **`GLOBAL`** | One state message represents the source as a whole, with optional shared state plus per-stream parts. | Use when streams share a cursor, such as a common CDC log position. Independent stream parallelism is constrained because progress is coupled. |
| **`LEGACY`** | Older opaque state for the whole source. | Kept for compatibility and lacks the granularity of current state types. New connectors should not use it. |

State is scoped to a connection, not merely to a connector type. Two configured
connections that use the same connector image must not share state unless a
custom integration intentionally implements that behavior outside Airbyte.
A refresh/reset, deleting a connection, and retrying a failed attempt have
different effects; do not treat them as interchangeable recovery actions.

## How records are selected and written

A sync mode combines a **source read mode** with a **destination write mode**.

| Mode | Behavior | Main trade-off |
|---|---|---|
| **Full Refresh + Overwrite** | Read the whole stream and replace destination data. | Simple, but expensive for large streams. |
| **Full Refresh + Append** | Read the whole stream and append every copy. | Keeps snapshots/history but deliberately creates repeats. |
| **Full Refresh + Overwrite + Deduped** | Replace all data and deduplicate by primary key. | Requires connector/destination support and a primary key. |
| **Incremental + Append** | Read new or changed records after the saved cursor and append them. | Efficient, but repeated keys and retry duplicates can remain. |
| **Incremental + Append + Deduped** | Read changes, keep append history internally, and expose the latest row per primary key. | Requires a reliable cursor and primary key. |

The first incremental sync is effectively a full read. A normal cursor-based
incremental sync sees records exposed by the cursor; it does not automatically
detect hard deletes.

### CDC

For supported databases, Change Data Capture reads `INSERT`, `UPDATE`, and
`DELETE` changes from the database transaction log. The first run takes a
snapshot; later scheduled runs continue from a stored log position.

Airbyte CDC is usually scheduled micro-batch replication, not an infinite
streaming process. Log retention must be long enough for Airbyte to consume the
changes. Operations such as `TRUNCATE` or `ALTER` may not appear as row changes,
and support varies by source connector.

## How data appears in the destination

For structured records, the exact table layout is a destination-connector
contract:

- **Direct Load** casts top-level fields and writes them directly to the final
  table. Temporary tables can still be used for deduplication or overwrite.
- **Typing and Deduping**, which Airbyte is phasing out in favor of Direct Load,
  first writes JSON to persistent raw tables and then builds typed final tables.
- `_airbyte_meta` can record row-level typing or serialization problems without
  necessarily failing the entire sync.
- Some file connectors can instead copy raw file bytes and deliver structured
  file metadata.

Do not assume every destination uses the same layout or supports every mode.
Check the specific destination connector and version. Airbyte may recreate or
swap tables during refreshes or schema changes, so adding unmanaged constraints
or permissions directly to Airbyte-owned tables can be unsafe.

## Schemas and schema changes

The source describes every stream with JSON Schema. A connection can select only
some streams or fields and can choose whether detected source changes are
propagated automatically, held for approval, ignored, or used to stop syncs.

Adding a field is usually non-breaking. Removing a configured cursor or primary
key is breaking because incremental progress or deduplication becomes ambiguous;
Airbyte pauses the connection for review. Type changes can also require a
refresh or manual cleanup, especially with Direct Load.

Schema discovery is not schema governance. Downstream consumers still need
contracts, tests, and a response plan for source drift.

## Multitenancy with Airbyte

### The short answer

Airbyte does not have a universal `tenant_id` primitive that automatically
isolates every record, credential, job, and destination table. The application
using Airbyte must choose an isolation model.

### What a workspace means in practice

A workspace is Airbyte's logical container for configured sources,
destinations, connections, connection state, jobs, and—on editions with
RBAC—workspace membership. It is similar to a project or folder with an access
boundary. It does **not** create another Airbyte installation, Kubernetes
cluster, configuration database, connector image, or secrets manager.

The distinction between a connector and a configured source matters for
multitenancy. Two tenants can use the same versioned GitHub **connector
definition/image**, but each tenant should have a different configured GitHub
**source** with its own credentials and settings. Each tenant should also have
its own connection and destination boundary, so its jobs and connection state
remain separate.

Consider the synthetic case of one Airbyte instance and two clients that both
replicate GitHub data:

```text
one Airbyte platform
  shared connector definition: GitHub <version>

  client-a resource set
    configured GitHub source A -> connection A -> destination A/schema A
    source credential A          state A       destination credential A

  client-b resource set
    configured GitHub source B -> connection B -> destination B/schema B
    source credential B          state B       destination credential B
```

- With **self-managed Core**, both resource sets must be in its single
  workspace. Do not give either client direct Airbyte UI/API access. A trusted
  application service must map each client to its Airbyte UUIDs and enforce that
  mapping on every operation. Use separate source credentials and preferably
  separate destination credentials and schemas/databases.
- With an edition that supports **multiple workspaces**, put client A and client
  B in different workspaces. This adds workspace grouping and supported RBAC,
  but destination permissions, secret access, compute, and shared platform
  failure domains still need separate controls.
- If the configuration database, secrets, cluster, or control-plane failure
  domain must not be shared, use a separate Airbyte deployment for each client.

The usual models are:

1. **Resources per tenant in one workspace**: works with Core, but isolation is
   enforced by the application, destination, secret manager, and network—not by
   Airbyte workspace RBAC.
2. **Workspace per tenant**: the normal Airbyte Embedded pattern when the plan
   supports multiple workspaces. It gives a useful administrative and RBAC
   boundary while sharing the larger platform.
3. **Airbyte deployment per tenant**: the strongest control-plane, state,
   secret, compute, and failure-domain isolation, with the highest operational
   cost.
4. **Workspace or deployment per security/residency group**: tenants share a
   boundary only when policy permits it; heavy or sensitive tenants can be moved
   to a stronger boundary.

`tenant` is an application concept. Airbyte's native hierarchy is:

```text
organization
  +-- workspace
        +-- sources
        +-- destinations
        +-- connections
        +-- jobs and connection state
```

### Edition boundaries

Multitenancy advice is edition-specific:

| Airbyte shape | Native structure documented by Airbyte | Consequence |
|---|---|---|
| **Self-managed Core** | One organization and one workspace. Core does not provide workspace-level user management/RBAC. | A shared Core instance is one Airbyte administrative boundary. Tenant users must not receive direct UI/API access unless seeing all resources is acceptable. |
| **Cloud Standard** | One workspace per organization. | Workspace-per-tenant inside one organization is unavailable. |
| **Cloud Pro / Enterprise Flex** | Multiple workspaces and workspace-scoped RBAC. | Workspace-per-tenant or workspace-per-security-domain is supported. |
| **Legacy Self-Managed Enterprise** | Capabilities depend on the licensed version. Airbyte no longer sells this edition and preserves its documentation as an archive. | Existing users must follow the documentation for their exact licensed version; it is not a current option for a new deployment. |

### If you self-manage Airbyte and want different workspaces

First identify which product is running; “self-managed” describes deployment
ownership, not a single feature set:

- **Current Airbyte Core:** one deployment has exactly one organization and one
  workspace. You cannot create `client-a` and `client-b` workspaces inside that
  Core instance. Either keep both clients as separately credentialed resource
  sets in the single workspace—with no direct client access—or run two Core
  deployments.
- **Existing legacy Self-Managed Enterprise:** some licensed versions provide
  multi-user/multi-workspace features. Airbyte no longer sells this product, so
  confirm behavior in the archived documentation for the installed version and
  license. Its workspaces are still logical boundaries that share the deployed
  control plane unless that version's documentation explicitly says otherwise.
- **Enterprise Flex:** supports multiple workspaces and self-managed data
  planes, but its control plane is managed by Airbyte. It is hybrid rather than
  a fully self-managed control plane.

For two clients on current Core, separate deployments are the only way to get
two fully separate Airbyte workspaces/control planes:

```text
self-managed Core deployment A        self-managed Core deployment B
  one organization                      one organization
  one workspace for client-a            one workspace for client-b
  config DB / state / secrets A          config DB / state / secrets B
```

This costs more to operate, but it removes the shared Airbyte API, configuration
database, state store, scheduler, and upgrade failure domain. If those may be
shared, the single-workspace Core pattern is simpler.

Do not design from screenshots or an old API response. Confirm the target
Airbyte edition, connector versions, API surface, and workspace limits first.

### What a workspace isolates—and what it does not

A workspace groups sources, destinations, connections, connector settings, and
members. Airbyte describes organizations and workspaces as its primary way to
segregate data and connections. With RBAC, workspace roles can limit who may
read, run, edit, or administer that workspace.

A workspace is therefore a useful **logical and administrative boundary**. It
is not automatically all of the following:

- a separate Airbyte deployment or configuration database;
- a separate encryption key or secrets-manager account;
- a separate Kubernetes cluster or namespace;
- a tenant-specific compute quota or fairness guarantee;
- a separate failure domain for the shared API, Temporal, database, launcher,
  or data plane;
- row-level isolation inside a shared destination table.

Organization roles need special care. An organization role applies across the
organization and all its workspaces. Airbyte allows a higher workspace role but
does not allow a workspace role to be more restrictive than the user's
organization role. An organization admin is effectively an admin of every
workspace. Assign low organization roles and grant additional access only in
specific workspaces.

### Choosing an isolation pattern

| Pattern | Access/config isolation | Data-path isolation | Shared failure domain | Cost/operations | Good fit |
|---|---|---|---|---|---|
| **One Core workspace, resources per tenant** | Application-enforced only | Workload containers are separate per job, but use a shared platform and cluster | Large | Lowest | An internal service where tenants never access Airbyte directly and logical isolation is sufficient. |
| **Workspace per tenant** | Native workspace grouping and, on supported plans, RBAC | Jobs are isolated as workloads, but may share a data plane and capacity | Medium | Medium | Embedded data integration and SaaS control planes. |
| **Workspace per security/residency group** | Native grouping at group level; tenant isolation still partly application-enforced | One region/data plane per workspace | Medium | Medium | Many small tenants governed by the same policy and destination boundary. |
| **Deployment or dedicated data plane per tenant** | A deployment isolates the control plane; a Flex data plane still shares its managed control plane | Strong data-path/network separation | A deployment has the smallest domain; a data plane retains shared control-plane dependencies | Highest | Strict contractual, regulatory, network, or blast-radius requirements. |

The decision should follow the required boundary, not tenant count alone. Ask:

- May one tenant's operator ever list another tenant's connection metadata?
- May jobs share nodes, a secrets manager, logs, state storage, or a platform
  database?
- May cursor and primary-key values reach a shared control plane?
- Can one tenant's workload delay another tenant's sync?
- What is the acceptable blast radius of a platform outage or bad upgrade?
- Must deletion be independently provable?

If the answer to any shared-boundary question is “no,” use a boundary that
removes that specific sharing rather than a naming convention. A separate data
plane isolates the data path; a separate deployment is required when the control
plane, configuration database, or Airbyte organization cannot be shared.

### Recommended workspace-per-tenant shape

When multiple workspaces are supported, a common embedded architecture is:

```text
                           application control service
                      authn + authz + tenant resource map
                                      |
                         service-owned Airbyte token
                                      |
                             Airbyte API/control plane
                                      |
              +-----------------------+-----------------------+
              |                                               |
       workspace tenant-a                              workspace tenant-b
       source(s) tenant-a                              source(s) tenant-b
       destination tenant-a                           destination tenant-b
       connection/state/jobs                          connection/state/jobs
              |                                               |
      schema/bucket tenant-a                          schema/bucket tenant-b
```

The application—not the browser—owns the privileged Airbyte API token. Its own
database stores the authoritative mapping:

```text
tenant_id -> organization_id
          -> workspace_id
          -> source_id(s)
          -> destination_id(s)
          -> connection_id(s)
          -> desired configuration version/hash
          -> provisioning status
```

Airbyte UUIDs identify resources; they are not proof that the caller is allowed
to use the resource. Every operation should first resolve the resource through
the application's tenant-scoped mapping. Never accept an arbitrary Airbyte
resource ID from a tenant-facing request and forward it without that check.

Some Airbyte list APIs, including the source-list API, can return resources from
every workspace accessible to the API principal when no workspace filter is
supplied. Always scope list calls and filter results by the expected workspace,
even when the application expects its token to have narrow access.

### Tenant provisioning as a reconciliation workflow

Provisioning crosses several systems and is not one atomic transaction. Model it
as a retryable state machine or reconciler:

1. Create the application tenant record in `provisioning` state.
2. Create or resolve its Airbyte workspace when the edition supports it.
3. Create a tenant-specific destination configuration with least-privilege
   credentials and an explicit database, dataset, schema, bucket, or path.
4. Complete source authentication for that tenant. For OAuth, bind the callback
   state to the authenticated tenant and provisioning operation.
5. Run `check` and `discover`; save the discovered catalog version used to make
   stream choices.
6. Create the connection with an explicit stream catalog, sync mode, namespace,
   schema-change policy, and initially safe schedule/status.
7. Persist every returned Airbyte ID before moving to the next step.
8. Trigger a bounded validation sync and verify the expected destination
   boundary, not merely a successful Airbyte job status.
9. Mark the tenant active only after validation. Reconciliation can then repair
   missing or drifted resources from the desired configuration.

Creation APIs should not be assumed to be idempotent. A timeout can occur after
Airbyte created a resource but before the caller stored its ID. On retry, search
within the expected workspace using an application-generated stable label/name,
validate the full configuration, and adopt exactly one match. Names help
recovery but should not replace stored UUIDs or authorization checks.

Use a saga-style cleanup policy for partial provisioning. Do not automatically
delete a destination or workspace merely because a later step failed; it may
contain data from a completed attempt. Record the partial state and reconcile it
deliberately.

### Shared Core instance: the safe minimum

With Core, all tenants live in one Airbyte workspace unless each gets a separate
deployment. A safe shared design requires:

- no direct tenant access to the Airbyte UI or API;
- a source, destination, and connection per tenant or per intentionally shared
  upstream account;
- a service-owned tenant-to-resource mapping checked on every operation;
- tenant-specific upstream credentials;
- tenant-specific destination credentials and/or schemas/paths;
- an external secret manager with narrow access;
- schedule and concurrency control in the surrounding application;
- logs and metrics that can be filtered by connection and tenant without
  putting sensitive values in labels;
- tests proving that one tenant's refresh, clear, schema change, or deletion
  cannot select another tenant's resources.

Prefixes such as `tenant-a__` make resources easier to operate. They are not an
authorization boundary. If tenant users need direct Airbyte access, move to a
workspace-per-tenant plan or separate deployments.

### Destination isolation

Airbyte resource separation does not prevent data collision in the destination.
Choose a destination boundary explicitly, strongest first:

1. Separate destination account/project/database/bucket and credentials per
   tenant.
2. Separate schema/dataset/container/path with tenant-limited credentials.
3. Separate prefixed tables in a shared schema.
4. Shared tables carrying a mandatory `tenant_id` column.

The last two are logical conventions, not strong storage isolation. Airbyte does
not universally inject an application tenant ID into every emitted record.
Shared-table tenancy therefore needs a connector-provided tenant key, a mapping
or transformation that cannot be bypassed, or a downstream merge with strong
tests and constraints.

Connections support destination namespaces and stream prefixes, but a prefix is
not a permission system. Destination connectors may sanitize, truncate, or
case-fold identifiers. Generate collision-resistant destination names, maintain
an authoritative registry, and test the exact destination connector's naming
rules.

If multiple workspaces intentionally point at the same warehouse, a destination
configuration is still workspace-owned. Create and govern each configuration
explicitly. Prefer separate least-privilege service identities so a connector
for `tenant-a` cannot write into `tenant-b` even if a namespace is misconfigured.

### Source credentials and upstream tenancy

Use one configured source per independent credential/security context. If an
upstream API token can read several tenants, Airbyte cannot provide stronger
source-side isolation than that token and the connector's configuration allow.

A custom “fan-out” connector that loops over many tenant credentials can reduce
resource count, but it couples state, retries, rate limits, logs, schema changes,
and blast radius. Prefer separate configured sources unless the upstream system
natively exposes a multi-account feed with an unambiguous tenant identifier and
the coupled lifecycle is intentional.

For OAuth:

- the authorization request must be initiated in an authenticated tenant
  context;
- `state` must be one-time, short-lived, and bound to that tenant and connector
  request;
- the callback must not trust a tenant ID supplied by the browser;
- refresh tokens belong in the configured secrets manager;
- reconnecting one tenant must update only its configured source;
- offboarding must revoke the upstream grant as well as deleting the Airbyte
  reference.

### Secrets, logs, and metadata isolation

Use distinct secret entries and least-privilege policies per tenant or isolation
group. A single broad warehouse credential defeats schema separation because a
misconfigured connection can write elsewhere.

Remember that isolation must cover more than records:

- connector configuration and OAuth tokens;
- source catalogs, stream names, and field names;
- connection state and cursor values;
- job logs and trace messages;
- temporary staging locations;
- destination temporary/raw tables;
- metrics, alerts, and audit records.

Do not put e-mail addresses, account names, or other sensitive values in
workspace names, connection names, Kubernetes labels, metric labels, or log
fields. Use opaque internal identifiers and resolve them in an authorized
operator tool.

### Compute isolation and noisy neighbors

Workspaces do not by themselves guarantee compute fairness. Sync workloads may
share the launcher, cluster, nodes, network egress, object storage, Temporal,
configuration database, and global concurrency limits.

For a shared deployment:

- stagger schedules instead of starting every tenant at the same boundary;
- enforce per-tenant and global concurrency budgets before triggering jobs;
- use connection-specific resource requests/limits for exceptional workloads;
- limit retries and backfills so one failing tenant cannot continuously consume
  capacity;
- measure queue delay separately from sync duration;
- isolate heavy connectors or tenants into dedicated node pools, data planes,
  or deployments when policy and product capabilities allow it;
- apply source API and destination warehouse quotas independently of Airbyte's
  own worker limits.

Airbyte supports instance-wide, connector-definition, and connection-specific
resource settings, with the narrower setting taking precedence. This helps tune
memory and CPU; it is not a complete per-tenant scheduler. If strict fairness is
required, the tenant-aware control service must decide when to trigger jobs or
use stronger compute boundaries.

### Enterprise Flex and data residency

Enterprise Flex separates a managed control plane from managed or self-managed
data planes. A workspace is assigned one region/data plane, and its connections
run there. This is useful for workspace-per-region or workspace-per-security-
domain designs.

It is not a “no metadata leaves the data plane” guarantee:

- cursor and primary-key values are processed/stored through the control plane;
- Connector Builder development/testing sends data through the control plane,
  although published connector syncs use the workspace's region;
- the data plane must make outbound requests to the control plane;
- multiple data planes in one region must use the same secrets manager, and the
  control and data planes must agree on secret management.

Therefore, classify cursor and primary-key fields as part of the control-plane
data model. If policy forbids those values leaving a tenant-controlled boundary,
choose non-sensitive fields where technically correct or use a fully
self-managed deployment whose control plane is inside that boundary.

### Tenant offboarding

Offboarding is a data-retention workflow, not just `DELETE workspace`:

1. Stop new sync triggers and cancel or allow active jobs according to policy.
2. Record the last successful job and committed state needed for audit or
   recovery.
3. Decide whether destination data is retained, exported, or removed.
4. Revoke upstream OAuth grants/API keys and destination credentials.
5. Delete Airbyte connections, configured sources/destinations, and external
   secret entries in the intended order.
6. Delete the workspace only after verifying that it contains no unrelated
   resources. Airbyte documents workspace deletion as irreversible and as
   deleting its sources, destinations, and connections.
7. Tombstone the application mapping so delayed callbacks or retry messages
   cannot recreate or trigger the old resources.
8. Verify logs, state storage, backups, and audit records against the applicable
   retention policy.

### Multitenancy failure modes

| Failure | Result | Guardrail |
|---|---|---|
| Arbitrary `connection_id` accepted from a tenant request | Cross-tenant job control | Resolve IDs only through the authenticated tenant mapping. |
| Unscoped list API call | Metadata from other accessible workspaces enters application memory or a response | Always pass/verify the expected workspace and filter server-side. |
| Shared destination credential is too broad | Namespace error becomes cross-tenant write | Separate credentials and storage permissions. |
| Sanitized namespace names collide | Two connections target the same table/path | Collision-resistant names plus destination-side verification. |
| Organization role is too powerful | User gains access to every workspace | Minimal organization role; workspace-specific elevation. |
| OAuth callback is not tenant-bound | Credential attached to the wrong source | Signed one-time state bound to tenant and provisioning operation. |
| One tenant launches many backfills | Queueing and resource starvation | Per-tenant budgets, schedule staggering, and isolation for heavy workloads. |
| Shared custom connector upgrade breaks all tenants | Wide sync failure | Pin versions, canary on a synthetic workspace, and roll out in stages. |
| Workspace deleted during partial cleanup | Irrecoverable Airbyte configuration loss | Inventory and retention gate before destructive deletion. |
| Sensitive cursor appears in logs/control metadata | Data-boundary violation | Choose non-sensitive cursors when valid and secure logs/state as sensitive data. |

### Multitenancy invariants to test

A multitenant Airbyte integration should have automated tests for these
invariants:

- every Airbyte resource belongs to exactly one application tenant or an
  explicitly declared shared scope;
- every tenant-facing operation checks that ownership before calling Airbyte;
- destination credentials cannot write outside the assigned boundary;
- source credentials cannot read outside the assigned upstream boundary;
- connection state is never copied between tenants;
- refresh, clear, schema approval, cancellation, and deletion affect only the
  selected tenant;
- list operations cannot return resources from another tenant;
- logs, alerts, and metric labels contain opaque IDs rather than sensitive data;
- a large or repeatedly failing sync cannot consume unbounded shared capacity;
- backup/restore preserves tenant-resource mappings and connection state;
- offboarding revokes credentials and prevents delayed retries or callbacks.

Names and UI grouping are useful for humans. The real isolation proof is the
combination of authorization, resource ownership, credentials, destination
permissions, compute boundaries, and tests.

## Scaling and operating Airbyte

The control plane schedules and records work; replication workloads consume most
of the CPU, memory, network, and destination compute. Scaling therefore starts
with the number and shape of concurrently running syncs, not simply the number
of configured connections.

The workload API and launcher separate “job should run” from “the cluster can
start its pod now.” This queue provides back pressure. A scheduled job may be
healthy but waiting for capacity, so monitor queue time and execution time as
different signals.

### Capacity model

For each connection, estimate:

- source read rate, page/partition size, and API/database limits;
- average and worst-case record size, including nested objects;
- destination batch size, commit latency, staging space, and warehouse limits;
- connector and orchestrator container memory;
- network bandwidth and cross-region transfer;
- log volume and temporary storage;
- checkpoint frequency and cost of replay after failure;
- full-refresh/backfill demand, not only normal incremental demand.

A source can buffer records faster than a destination commits them. Airbyte
applies back pressure, but memory still depends on connector implementation and
record size. One unusually large record or page can matter more than a high
count of small records.

Use the resource hierarchy deliberately:

```text
instance default < connector-definition override < connection override
```

Set sane defaults, override a connector type when its implementation has a
consistent need, and reserve connection overrides for exceptional pipelines.
Resource limits that are too low cause retries and replay; limits that are too
high reduce the number of pods the cluster can schedule.

### Durable dependencies

For a recoverable self-managed installation, treat these as first-class stateful
dependencies:

- configuration PostgreSQL and its backups;
- Temporal persistence and connectivity;
- object storage for state, logs, and workload output;
- the external secrets manager;
- container registry access for pinned connector images;
- destination staging locations;
- Kubernetes capacity, DNS, and network paths to every source and destination.

Restoring only the Airbyte database is not necessarily a complete restore if
state, secrets, or workload output lives elsewhere. Test the restore procedure
with synthetic connections and verify both control metadata and incremental
continuity.

### What to monitor

At minimum, monitor by connection and isolation group:

- time since last successful sync;
- queued, running, failed, cancelled, and retried jobs;
- queue delay, run duration, and checkpoint progress;
- records/bytes read and written, with sudden zero-output detection;
- source rate-limit and authentication failures;
- destination commit, typing, serialization, and capacity errors;
- schema-change and connection-paused events;
- workload pod scheduling failures, evictions, and out-of-memory exits;
- configuration database, Temporal, object storage, and secret-manager health;
- destination freshness independently of Airbyte's reported job success.

A green job means the protocol run completed. It does not prove that downstream
queries see the expected tenant, schema, row count, freshness, or business
semantics. Add destination-side validation.

### Upgrade discipline

The platform, protocol, connector images, CDKs, and destination writing behavior
have separate version lifecycles. For upgrades:

1. Back up control metadata and state stores.
2. Read platform and connector breaking-change notes.
3. Test with synthetic sources and destinations.
4. Canary connector upgrades on selected connections/workspaces.
5. Compare discovered catalogs before approving schema changes.
6. Validate checkpoint continuation and destination table behavior.
7. Keep a rollback plan that accounts for database migrations and connector
   state compatibility, not only container image tags.

## Reliability guarantees and caveats

- **Delivery is at least once, not exactly once.** A retry can resend records
  after the last checkpoint. Use destination deduplication or idempotent
  downstream models when uniqueness matters.
- **State is connector-defined.** Its correctness and checkpoint frequency
  depend on the source connector.
- **Incremental is only as good as its cursor.** A mutable, non-monotonic, or
  low-resolution cursor can miss or repeat data.
- **Deduplication needs a stable primary key.** It does not repair a bad key or
  define business meaning.
- **Full refresh can load the source heavily.** Estimate API limits, database
  load, transfer volume, and destination cost before choosing it.
- **CDC has operational prerequisites.** Database permissions, log retention,
  table keys, and schema-change procedures must be configured correctly.
- **Connectors evolve independently.** Pin and test connector versions; upgrades
  can include schema or behavior changes.
- **Retries do not make every failure harmless.** External rate limits,
  destination capacity, invalid records, expired credentials, and schema drift
  still need monitoring.
- **Airbyte is usually not low-latency event streaming.** It targets reliable
  bulk and incremental replication. Use a streaming system when per-event
  latency is the primary requirement.
- **Airbyte does limited shaping, not all business transformation.** Complex
  models and business logic normally belong in a downstream tool such as dbt or
  a processing engine. Product-plan support for mappings and transformations
  varies.

## Building a connector

For an HTTP API source, Airbyte recommends starting with Connector Builder.
Other choices are the declarative low-code YAML CDK, the Python CDK, or any custom
program that implements the Airbyte Protocol. Builder and the low-code CDK are
for sources, not destination connectors.

A source connector must mainly define configuration, access checking, discovery,
pagination, authentication, rate-limit behavior, record extraction, incremental
state, and error reporting. The platform supplies the scheduling and execution
machinery around it.

## A small synthetic example

Suppose a connection copies an `orders` stream from an example API into a data
warehouse every hour:

1. `discover` reports `orders`, its fields, `updated_at` as a cursor, and `id` as
   a primary key.
2. The first incremental job reads all orders and commits a state such as the
   latest processed cursor.
3. The next job receives that state and requests only later pages or records.
4. If the job fails after a checkpoint, the retry starts from that checkpoint.
   A few records may be emitted twice.
5. With Incremental + Append, both copies can remain. With Incremental + Append
   + Deduped, the final table keeps the winning row for each `id` according to
   the connector's cursor rules.

## Official references

- [Data replication platform](https://docs.airbyte.com/platform/)
- [Core concepts](https://docs.airbyte.com/platform/using-airbyte/core-concepts)
- [Architecture overview](https://docs.airbyte.com/platform/understanding-airbyte/high-level-view)
- [Airbyte Protocol](https://docs.airbyte.com/platform/understanding-airbyte/airbyte-protocol)
- [Workloads and jobs](https://docs.airbyte.com/platform/understanding-airbyte/jobs)
- [Sync modes](https://docs.airbyte.com/platform/using-airbyte/core-concepts/sync-modes)
- [Change Data Capture](https://docs.airbyte.com/platform/understanding-airbyte/cdc)
- [Schema change management](https://docs.airbyte.com/platform/using-airbyte/schema-change-management)
- [Direct loading](https://docs.airbyte.com/platform/using-airbyte/core-concepts/direct-load-tables)
- [Connector development](https://docs.airbyte.com/platform/connector-development)
- [PyAirbyte API](https://airbytehq.github.io/PyAirbyte/airbyte.html)
- [Self-managed deployment](https://docs.airbyte.com/platform/deploying-airbyte)
- [`abctl`](https://docs.airbyte.com/platform/deploying-airbyte/abctl)
- [Self-Managed Enterprise status and archived documentation](https://docs.airbyte.com/platform/enterprise-setup)
- [Organizations and workspaces](https://docs.airbyte.com/platform/organizations-workspaces)
- [Workspace management](https://docs.airbyte.com/platform/using-airbyte/workspaces)
- [Role-based access control](https://docs.airbyte.com/platform/access-management/rbac)
- [Airbyte Embedded](https://reference.airbyte.com/reference/powered-by-airbyte)
- [Enterprise Flex](https://docs.airbyte.com/platform/enterprise-flex)
- [Data residency](https://docs.airbyte.com/platform/cloud/managing-airbyte-cloud/manage-data-residency)
- [Secret management](https://docs.airbyte.com/platform/deploying-airbyte/integrations/secrets)
- [State and log storage](https://docs.airbyte.com/platform/deploying-airbyte/integrations/storage)
- [External configuration database](https://docs.airbyte.com/platform/deploying-airbyte/integrations/database)
- [Scaling Airbyte](https://docs.airbyte.com/platform/operator-guides/scaling-airbyte)
- [Connector resource configuration](https://docs.airbyte.com/platform/operator-guides/configuring-connector-resources)
- [Security](https://docs.airbyte.com/platform/operating-airbyte/security)
- [List sources API](https://reference.airbyte.com/reference/listsources)
