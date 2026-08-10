# `test-stand` — what has to exist beneath the umbrella

[`README.md`](README.md) describes the one thing this directory deploys: the
umbrella Helm release `insight`, from [`values.yaml`](values.yaml). This file
describes everything the release lands *on top of* — the cluster, the eight
infrastructure components under it, and the settings among them that are not
obvious and not recoverable by guesswork.

It exists because the release cannot tell you any of it. `helm upgrade --wait`
reports Ready pods and a `deployed` release; it does not report a Redis the
authenticator's client cannot address, an Argo controller that will never look
at the CronWorkflows it was just given, or a database account four services
are about to exhaust. Most of the failures collected below share one signature:
**every pod Ready, the release `deployed`, and the thing that matters silently
not happening.** That sentence is the most useful one in this document.

> **Scope.** This is the L0/L2 half of the ownership boundary in
> [`README.md`](README.md#the-ownership-boundary). Nothing here is deployed by
> anything in this repository, and nothing here is deployed by CI. It is
> written so that a stand of this shape can be rebuilt, and so that a reviewer
> of a change to `values.yaml` can see what that value is coupled to.

## What this file is, and what it is not

**This file is the specification. The deployment repository is the executable
form.** That repository holds one `deploy-<svc>.sh` and one `validate-<svc>.sh`
per component, a `deploy-all.sh` that runs them in a fixed order, per-stand
overlays under `stands/`, and the manifests and values files they apply. Running
it is how a stand actually gets built. Reading this file is how you find out
what it is doing and why, without a checkout of it and without cluster access.

That split is deliberate, and it is the same split
[`inventory.yaml`](inventory.yaml) already makes for the L0/L2 layers: this
environment is a values overlay plus a cluster address, and the layers beneath
it are brought up and owned elsewhere. What was missing until now is any record
*in this repository* of what those layers are. A CI job that deploys a chart
into a cluster it did not build, and gates a merge on the result, needs that
record — otherwise the first unexplained red run is an archaeology exercise
against a repository the reader may not have.

**Two consequences follow, and both need stating out loud.**

*This file can drift from the executable form.* There is no generation step and
no test that compares the two. A change made in the deployment repository — a
version bump, a new required setting, a namespace rename — reaches the stand
without touching anything here. Treat every version and every name below as
"true when it was verified against a live stand", not as "true now".

*Drift is cheap to detect, and the checks are read-only.* Against an admin
kubeconfig:

```bash
helm list -A                                   # against the inventory table below
kubectl get sc                                 # `cinder`, and it must be the default
kubectl get clusterissuer                      # insight-selfsigned + insight-ca, both Ready
kubectl -n mariadb get user insight -o jsonpath='{.spec.maxUserConnections}{"\n"}'
kubectl -n envoy-gateway-system get gateway insight \
  -o jsonpath='{range .spec.listeners[*]}{.name}={.protocol}/{.port}{"\n"}{end}'
kubectl -n insight get sa argo-workflow
kubectl -n airbyte get role insight-airbyte-auth-reader
```

None of those is available to CI. The deploy credential is a namespace-scoped
ServiceAccount in `insight` that is granted nothing cluster-scoped and nothing
in another namespace beyond one Role/RoleBinding pair in `airbyte` (see
[`../../scripts/provision-ci-deployer.sh`](../../scripts/provision-ci-deployer.sh)).
It cannot `helm list -A`, cannot read `mariadb/`, cannot see the Gateway it
attaches routes to. That is the right shape for a deploy credential, and it is
also why this file has to be maintained by hand rather than generated from the
cluster on every run.

### Why there is no `infra-versions.yaml`

A machine-readable companion pinning the versions was considered and rejected.
Nothing in this repository would read it: the Makefile reads
`inventory.yaml` and `values.yaml` and nothing else from this directory, the
workflow reads neither, and the credential that runs the workflow cannot
enumerate the releases it would be compared against. It would be a second copy
of the numbers in the table below, kept in step by nobody — the exact failure
mode this tree spends a page of `values.yaml` warning about for a single tenant
UUID. If a checker is ever written, it belongs next to `doctor.sh` and it should
parse the table here rather than a duplicate of it.

## The layer model

| Layer | What | Deployed by | This repository's relationship to it |
|---|---|---|---|
| **L-1** | The cluster itself: nodes, a CNI, the OpenStack Cinder CSI driver registered as `csidriver/cinder.csi.openstack.org`, and a public DNS record for the hostname | the cloud platform, outside every repository | assumed; five deploy scripts hard-fail without the CSI driver, and the Insight step is gated on the DNS record resolving |
| **L0** | Edge and PKI: Envoy Gateway (`GatewayClass`, `Gateway`, `EnvoyProxy`), cert-manager and the two-step `ClusterIssuer` chain, the `cinder` StorageClass | the deployment repository | named in `values.yaml` (`gateway.route.parentRef`, `authenticator.tlsDiscovery.issuerRef`); never created here |
| **L2** | Datastores under their own operators — ClickHouse, MariaDB, Redis, Redpanda — plus Airbyte and Argo Workflows, each in its own namespace | the deployment repository | addressed in `values.yaml` by in-cluster Service DNS; never created here |
| **L2** | The generate-once Secrets and the realm ConfigMap the release consumes by name | the deployment repository | referenced by name in `values.yaml`; listed with `enabled: false` in `inventory.yaml` |
| **L3** | **The umbrella release `insight` in namespace `insight` — every value in `values.yaml`** | **this directory** | **owned, and changed by CI on every merge to `main`** |

The single most important structural fact: **the release renders its own edge
routes but not the Gateway they attach to.** Since chart 0.5.107 the umbrella
carries native Gateway API templates (`gateway.route`, `keycloak.route`,
`frontend.route`), so `HTTPRoute/insight-gateway` and `HTTPRoute/insight-keycloak`
arrive with the release and carry its Helm ownership metadata. What they attach
*to* — `Gateway/insight` in `envoy-gateway-system`, its `EnvoyProxy`, its
certificate — is L0 and stays L0. An HTTPRoute whose `parentRef` names a Gateway
that does not exist is created successfully and serves nothing.

## The deploy order

Nine steps, and they are not interchangeable. `deploy-all.sh` runs each
`deploy-<svc>.sh` followed immediately by its `validate-<svc>.sh`, stopping at
the first failure:

```text
envoy-gateway → cert-manager → clickhouse → mariadb → redis → redpanda
  → airbyte → argo-workflows → insight        ( → seed, deliberately separate )
```

Two edges make that an order rather than a list, and both are worth knowing
before an attempt in the wrong order wastes an afternoon:

* **envoy-gateway must be first**, because its chart installs the Gateway API
  CRDs. cert-manager is installed with `config.enableGatewayAPI=true`, and its
  gateway-shim controller refuses to start without
  `gateways.gateway.networking.k8s.io`. The cert-manager deploy script fails
  fast on the missing CRD rather than letting the controller crash-loop, which
  is the difference between a five-second error and a twenty-minute one.
* **cert-manager back-fills what envoy-gateway deliberately left undone.** On a
  fresh cluster there is no `ClusterIssuer` yet, so the Gateway's `https`
  listener comes up *unprogrammed* — it has no certificate. That is expected,
  not a failure. The cert-manager step completes the handshake by waiting for
  `Certificate/insight-origin-tls` in the gateway namespace to go Ready.
  Neither script is complete on its own, and reading either one's output in
  isolation is misleading.

The seed step is **not** in this list, and its absence is a decision: a seeded
stand is deployed and seeded by two different actions, because seeding is what
gives the stand its people and the deploy is what gives it its services. See
[Between deploy and seed](#between-deploy-and-seed).

## Component inventory

Versions below were cross-checked against `helm list -A` on a live stand of
this shape. They are pinned, not floating — every deploy script passes an
explicit `--version`.

| # | Layer | Component | Namespace | Release | Chart @ version (source) | What a later step takes from it |
|---|---|---|---|---|---|---|
| — | L-1 | CNI | `kube-system` | `cilium` | `cilium` 1.16.3 | pod networking; its pod CIDR has to fall inside the ClickHouse user's IP allow-list (see §20) |
| — | L-1 | Block storage | — | — | `csidriver/cinder.csi.openstack.org` | every PVC below; five deploy scripts hard-fail if it is absent |
| — | L-1 | StorageClass | — | — | `cinder`, provisioner `cinder.csi.openstack.org`, `WaitForFirstConsumer`, `allowVolumeExpansion`, **annotated as the cluster default** | named explicitly by every datastore; taken implicitly by Airbyte's bundled PostgreSQL and MinIO (see §21) |
| 1 | L0 | Envoy Gateway | `envoy-gateway-system` | `eg` | `gateway-helm` v1.8.3 (`oci://docker.io/envoyproxy/gateway-helm`) | the Gateway API CRDs; `GatewayClass/envoy`; `Gateway/insight`; the LoadBalancer Service holding the stand's floating address |
| 2 | L0 | cert-manager | `cert-manager` | `cert-manager` | `cert-manager` v1.21.1 (`https://charts.jetstack.io`) | `ClusterIssuer/insight-ca` — the issuer `authenticator.tlsDiscovery.issuerRef` names — and, via the gateway-shim, the Gateway's origin certificate |
| 3 | L2 | ClickHouse operator | `clickhouse-operator` | `clickhouse-operator` | `altinity-clickhouse-operator` 0.27.1 (`https://helm.altinity.com`) | watches `clickhouse` only |
| 3 | L2 | ClickHouse + Keeper | `clickhouse` | — (CRs) | `ClickHouseKeeperInstallation/clickhouse-keeper`, `ClickHouseInstallation/clickhouse` | Secrets `clickhouse-default-credentials` / `clickhouse-insight-credentials`; the `insight` user; the **per-pod** Service `chi-clickhouse-clickhouse-0-0` |
| 4 | L2 | MariaDB operator | `mariadb-operator` | `mariadb-operator-crds`, `mariadb-operator` | both 26.6.0 (`oci://ghcr.io/mariadb-operator/charts/…`) | the `MariaDB`/`Database`/`User`/`Grant` CRDs |
| 4 | L2 | MariaDB (Galera ×3) | `mariadb` | — (CRs) | `MariaDB/mariadb`, image `mariadb:11.8.8` | Secrets `mariadb-root` / `mariadb-insight-credentials`; the `insight` database, user and grant; Service `mariadb-primary` |
| 5 | L2 | Redis operator | `redis-operator` | `redis-operator` | `redis-operator` 0.25.0 (`https://ot-container-kit.github.io/helm-charts`) | watches `redis` only; cluster-scoped RBAC |
| 5 | L2 | Redis (replication ×3) | `redis` | `redis` | `redis-replication` 0.17.0 (same repo) | Secret `redis-auth`; Service `redis-master`, label-selected on the operator's `redis-role=master` |
| 6 | L2 | Redpanda | `redpanda` | `redpanda` | `redpanda` 26.1.9 (`https://charts.redpanda.com`) | Service `redpanda` with the **internal** Kafka listener on **9093** |
| 7 | L2 | Airbyte | `airbyte` | `airbyte` | `airbyte` 1.8.5 (`https://airbytehq.github.io/helm-charts`) | the `airbyte` namespace itself (a deploy preflight); Secret `airbyte-auth-secrets`; the server the reconcile loop calls |
| 8 | L2 | Argo Workflows | `argo` | `argo-workflows` | `argo-workflows` 1.0.22 (`https://argoproj.github.io/argo-helm`) | the `workflowtemplates` / `cronworkflows` CRDs — **without which the umbrella cannot render at all** — and the cluster-scoped controller RBAC |
| 9 | L3 | **Insight umbrella** | `insight` | `insight` | `insight` 0.5.111 (`oci://ghcr.io/constructorfabric/charts/insight`) | the stand |

Persistent volumes, for capacity planning: ClickHouse data 100Gi, Keeper 10Gi,
MariaDB 100Gi data plus a separate 1Gi Galera-config volume per member, Redis
10Gi per member, Redpanda 50Gi per broker, Airbyte 10Gi each for the
pre-created PostgreSQL and MinIO claims. Retention is deliberately
conservative: ClickHouse and Keeper use `reclaimPolicy: Retain`, MariaDB
retains on both `whenDeleted` and `whenScaled`, and Redis does the same.

Node shape is a hard requirement, not a recommendation. Redpanda and MariaDB
Galera both use **hard** pod anti-affinity on `kubernetes.io/hostname` for their
three members, so a cluster with fewer than three schedulable workers leaves
both StatefulSets permanently unconverged. Redis uses **soft** anti-affinity on
purpose: a required spread across three workers deadlocks against the
operator's own master/replica anti-affinity. The reference profile is three
control-plane nodes plus three workers on Kubernetes 1.31, with workers sized
to hold the datastore requests above with headroom.

## The load-bearing settings

Each entry is the same three-part contract: **the setting**, **what breaks
without it**, and **how the breakage presents**. The third part is the one that
earns the section — almost every item here fails in a way that names something
other than itself.

Ordered by how badly the failure lies to you.

### 1. `maxUserConnections: 100` on the MariaDB `User/insight`

**The setting.** A field on the `k8s.mariadb.com/v1alpha1` `User` object in the
`mariadb` namespace — not a chart value, and not something `values.yaml` can
influence. The operator's default is **10**.

**What breaks without it.** That one account backs **four** connection pools:
Analytics, Identity Resolution, the chart's `migrate`/`init` Jobs, and — because
[`values.yaml`](values.yaml) points `keycloak.database.username` at the same
user rather than provisioning a second one — the bundled Keycloak, whose own
pool defaults to 100 on its own. Ten is not enough for that, and the server's
global ceiling is 151, so 100 leaves headroom for `root` and the operator's
agent.

**How it presents.** Not as a connection-limit error anywhere you would look
first. Identity Resolution and the Analytics `migrate` initContainer crash-loop
at startup with

```text
1226 (42000): User 'insight' has exceeded the max_user_connections resource
```

while every other service stays Ready and the MariaDB cluster itself reports
healthy. The deploy does not surface that message at all: `helm upgrade --wait`
sits until its timeout and then dies with

```text
client rate limiter Wait returned an error: context deadline exceeded
```

which reads like an API-server or throttling problem and is not one. If a
deploy of this stack times out and the only error mentions a rate limiter,
read the pod logs of `insight-identity-resolution` before reading anything
else.

### 2. `clickhouse.host` is the **per-pod** Service, never the shared one

**The setting.** `chi-clickhouse-clickhouse-0-0.clickhouse.svc.cluster.local` —
the Altinity operator's `chi-<installation>-<cluster>-<shard>-<replica>` Service,
which always resolves to exactly one server. Not `clickhouse-clickhouse`, the
all-pod Service one word away.

**What breaks without it.** The chart is not cluster-aware: no template emits
`ON CLUSTER`, there is no cluster-name value, and every table it creates is
`ReplacingMergeTree` / `MergeTree` / `View` rather than `Replicated*`. Nothing
synchronises between servers, so DDL sent through a fan-out Service is applied
to whichever server answered that connection.

**How it presents.** The post-install migration scatters its statements across
independent servers that then disagree about which tables exist, and fails part
way through with ClickHouse's `UNKNOWN_TABLE` surfaced as `HTTP Error 404` —
i.e. the server that answered did not have the table another server had just
been given. Airbyte writes and Analytics reads scatter the same way afterwards.
The per-pod name is used **even at one replica** so that raising the replica
count cannot silently reintroduce round-robin DDL; see the comment on the key
in [`values.yaml`](values.yaml).

### 3. `redis.host` is a replication primary, not a Redis Cluster endpoint

**The setting.** `redis-master.redis.svc.cluster.local`, backed by a
three-member `redis-replication` release. Redis **Cluster** is the wrong
topology here, and Sentinel is deliberately not deployed either.

**What breaks without it.** The authenticator's Rust Redis client is compiled
with only `tokio-comp` and `connection-manager`; the crate feature-gates cluster
support behind `cluster`, so `ClusterClient` is not in the binary and it opens a
plain standalone connection. A clustered Redis answers `MOVED` for any key whose
slot the contacted node does not own, and that client cannot follow the
redirect.

**How it presents.** `/auth/login` returns HTTP 500 for every visitor, while
every pod stays Ready and the release reports `deployed`. Sessions are not
optional, so the symptom is "nobody can log in" with nothing in the deploy
output to suggest why. `-master` selects on the operator's `redis-role=master`
label, so it follows a failover with no client change — which is the reason the
Service name and not a pod name is the right address.

### 4. Argo `singleNamespace: false` **and** a matching `instanceID`

**The setting.** Two values that only work as a pair.
`singleNamespace: false` in the Argo release, and
`controller.instanceID.explicitID` equal to the umbrella's
`ingestion.reconcile.argoInstanceId` — both `argo-workflows-argo`, i.e.
`<argo release name>-<argo namespace>`.

**What breaks without either.** With `singleNamespace: true` the chart passes
`--namespaced` and the controller watches only the `argo` namespace, while every
CronWorkflow and WorkflowTemplate the umbrella installs lives in `insight`. And
the umbrella stamps each of those objects with
`workflows.argoproj.io/controller-instanceid: <id>`; Argo's matching rule is
exact, and a controller with no instanceID processes **only** objects carrying
no such label — so a labelled CronWorkflow is invisible to it.

**How it presents.** Identically, and silently, for both causes: the
CronWorkflows exist, are unsuspended, carry a valid schedule, and their
`status` stays `{}` forever with zero `Workflow` objects ever created. It reads
as "nothing has been triggered yet". The controller logs `instanceID=""` at
startup and then never mentions the objects again.

### 5. Argo `controller.workflowNamespaces` stays `[argo]` — do not add `insight`

**The setting.** A list that looks like it should name the namespace the
workflows run in, and must not.

**What breaks if you extend it.** That list controls only where the *chart*
creates the workflow ServiceAccount and its Role/RoleBinding. `insight` already
has an `argo-workflow` SA + Role + RoleBinding applied with `kubectl` (see
[Bootstrap §5](#5-the-argo-workflow-serviceaccount-in-insight)), and Helm cannot
adopt objects it did not create.

**How it presents.** The Argo upgrade fails outright with `invalid ownership
metadata`. What actually lets the controller *watch* other namespaces is the
ClusterRole/ClusterRoleBinding that `singleNamespace: false` renders — not this
list.

### 6. `airbyte.namespace: airbyte`

**The setting.** A chart value that does two things, only one of which is the
API URL.

**What breaks without it.** It also decides which namespace the chart renders
`Role`/`RoleBinding` `insight-airbyte-auth-reader` into — the RBAC that lets the
ingestion reconcile loop read Airbyte's own `airbyte-auth-secrets`. Left empty
it defaults to the **release** namespace.

**How it presents.** The RBAC lands in `insight`, where that Secret does not
exist, and connector provisioning fails at run time while every pod looks
healthy and the deploy reports success. Verify with
`kubectl -n airbyte get role insight-airbyte-auth-reader` — in the *airbyte*
namespace, not in `insight`.

### 7. Airbyte `global.auth.enabled: true`

**The setting.** An Airbyte chart value, and it is not about protecting the UI.

**What breaks without it.** `charts/server` only sets
`API_AUTHORIZATION_ENABLED` when the edition is pro/enterprise **or**
(community **and** `global.auth.enabled`). Without that variable the server
never mounts the authorization endpoints, so `POST /api/v1/applications/token`
— the token mint the ingestion reconcile loop uses — answers 404.

**How it presents.** `reconcile` logs `applications/token failed: … 404 /
Object not found` for every connector and exits non-zero, while
`/api/v1/health` returns 200 and the unauthenticated public API at `/v1/*`
still answers — so the server looks healthy and only half of it is. The
consequence of `true` is the one-time instance setup in
[Bootstrap §7](#7-the-airbyte-instance-setup-call).

### 8. `mariadb.host` is the operator's `-primary` Service

**The setting.** `mariadb-primary.mariadb.svc.cluster.local`, not the
round-robin `mariadb` Service that exists beside it.

**What breaks without it.** MariaDB runs as Galera, which certifies every write
cluster-wide. Concurrent writers arriving on different members through a
round-robin Service produce certification conflicts.

**How it presents.** The Analytics `migrate` Job — many statements, one
transaction each, no retry — fails with `1213` deadlocks, and the upgrade fails
with it. `-primary` is kept pointed at a single member by the operator and moves
on failover, so it gives a single writer without pinning a pod name. The same
applies to `keycloak.database.host`, which must be the same address for the same
reason.

### 9. `redpanda.brokers` is one `host:PORT` string, and the port is 9093

**The setting.** A single comma-separated bootstrap string —
`redpanda.redpanda.svc.cluster.local:9093` — not the host/port pair every other
datastore in `values.yaml` takes. 9093 is the redpanda chart's *internal* Kafka
listener; there is no 9092 listener on the Service at all.

**What breaks without the port.** The chart hands the value to a Kafka client
verbatim, so a bare hostname silently becomes `host:9092`.

**How it presents.** It never connects, and every pod stays healthy. The
deployment repository's preflight parses this value and refuses any entry
without a colon-port precisely because nothing downstream will tell you. Confirm
the listener on a stand with
`kubectl -n redpanda get svc redpanda -o jsonpath='{range .spec.ports[*]}{.name}={.port}{"\n"}{end}'`.

### 10. `authenticator.tlsDiscovery.enabled: true` with a Ready `ClusterIssuer`

**The setting.** `issuerRef: {name: insight-ca, kind: ClusterIssuer}`. This is
not optional hardening.

**What breaks without it.** The chart hardcodes
`gateway_issuer: https://insight-authenticator.insight.svc.cluster.local:8443`
in its secrets template and mounts `insight-authenticator-authn-tls-cert` as a
**non-optional** Secret volume in both Analytics and Identity Resolution.
Turning `tlsDiscovery` off does not remove the dependency; it removes the thing
that satisfies it. An issuer that exists but cannot sign has the same effect.

**How it presents.** `helm --wait` does not wait on `Certificate` objects, so
Analytics and Identity Resolution sit in `ContainerCreating` until the entire
20-minute timeout expires, and the failure message is a timeout rather than a
missing certificate. Check the issuer's `Ready` condition, not just its
existence.

### 11. `authenticator.oidc.externalIdClaim: idp_sub` — **not** `sub`

**The setting.** The claim the login bootstrap reads as the external id. On a
seeded stand it is `idp_sub`, a custom claim, even though the realm generator
sets each realm user's `id` to that person's roster UUID and its own comments
say `sub` carries it.

**What breaks with `sub`.** That would hold for a Keycloak *realm import*. The
chart applies realms with keycloak-config-cli, which creates users one at a
time through the admin REST API — and Keycloak assigns its own id on
`POST /users`, silently discarding the document's. So `sub` is not the roster
UUID. The deployment repository post-processes the generated realm to copy the
roster UUID into an `idp_sub` user attribute and adds an
`oidc-usermodel-attribute-mapper` on every client that emits it.

**How it presents.** Late, and as a data problem. Every user is created, the
password login succeeds, and the callback answers `login denied: no matching
person in Identity` with `event="login_denied_unknown_person"` against a fully
populated `identity.persons`.

### 12. The generated realm's declarative user profile — `unmanagedAttributePolicy: ENABLED`

**The setting.** A property of the realm document the deployment repository
generates and packs, not of anything in this directory.

**What breaks without it.** Keycloak 26 always runs the declarative user
profile, and its default policy **discards** every attribute the profile does
not declare. The realm sets three per user (`tenant_id`, `org_unit`,
`idp_sub`).

**How it presents.** None of the three survives import: `USER_ATTRIBUTE` comes
back empty for every user while the protocol mappers are created correctly, so
the mappers emit nothing and login fails at the callback with `id_token carries
no non-empty idp_sub claim`. Nothing warns, and the realm looks right in the
admin console — the attributes were never rejected, only dropped.

### 13. `keycloakConfig.filesLocations: "/config/*.json"`

**The setting.** In [`values.yaml`](values.yaml). The chart's default pattern is
`"/config/*.yaml"` only.

**What breaks without it.** The generated roster realm is JSON, and in seeded
mode the broker realm's YAML is not packed at all — so the default pattern
matches zero files, and keycloak-config-cli **errors** on a glob that matches
nothing rather than treating it as a no-op.

**How it presents.** The post-install/post-upgrade hook Job fails, which fails
the whole upgrade. Loud and correctly attributed, which is why it is this far
down the list — but it costs a deploy to find out if the ConfigMap's one key is
ever renamed.

### 14. `keycloak.hostname` must end in `/kc`

**The setting.** The advertised issuer base, in [`values.yaml`](values.yaml).

**What breaks without the suffix.** Keycloak 26's hostname-v2 does not fold
`--http-relative-path` into the advertised issuer, so the subchart passes both
and the path has to be part of the hostname.

**How it presents.** The server advertises an issuer the browser cannot reach.
Discovery fails in a way that reads like a TLS or routing problem. The matching
in-cluster value is `keycloakConfig.url: http://insight-keycloak:8085/kc` with
`allowInsecureUrl: true` — required for a non-https URL and acceptable only
because that hop never leaves the cluster.

### 15. `authenticator.oidc.sourceType` must equal what the seeder writes

**The setting.** `keycloak` on a seeded stand. The seeder writes
`(insight_source_type='keycloak', value_type='id', value_id=<roster uuid>)`, and
`seed-stand.sh` reads the source type back out of `insight-authenticator-config`
rather than taking it as a flag — so there is one writer, not two copies. The
dev-lead persona address is read back the same way, out of the realm ConfigMap;
see [One roster, two projections](#one-roster-two-projections), which is the
general form of this argument.

**What breaks if they disagree.** Nothing, visibly. Both halves succeed.

**How it presents.** A login that authenticates and then resolves to nobody,
against a projection that is fully seeded. Identical symptom to §11, different
cause — which is why both are worth knowing separately.

### 16. `frontend.route.enabled: false`, and the creation-timestamp tie-break

**The setting.** The gateway owns the only `/` route and proxies to the frontend
Service via its `frontUrl`. The frontend must not publish itself.

**What breaks with a second route.** Two HTTPRoutes claiming the same path on
the same host are resolved by Gateway API using **creation timestamp**, oldest
wins — and both report `Accepted`.

**How it presents.** Depending on deploy order, the edge either bypasses the
auth gateway entirely or the frontend route does nothing, with no error
anywhere. The same rule makes the deployment repository's optional maintenance
page a trap: it claims `PathPrefix: /` on the same hostname with no `hostnames:`
of its own, so being older it keeps serving after Insight is deployed. Removing
it is a required step, not a cleanup.

### 17. Application routes attach with `sectionName: https`

**The setting.** `gateway.route.parentRef` and `keycloak.route.parentRef` name
`{name: insight, namespace: envoy-gateway-system, sectionName: https}` — the
**named** listener.

**What breaks with the wrong section.** The Gateway also carries a
hostname-less HTTPS listener on 443 reserved for bare-address and unknown-Host
visitors. A route can narrow a listener's hostname but never widen it, which is
why the maintenance route names no hostname and every application route does.
Attaching an application route to the catch-all mixes the two populations.

**How it presents.** As a routing oddity rather than an error — both listeners
are legal on one port because an absent hostname means `*` and envoy picks by
SNI. The deployment repository's preflight additionally asserts that
`gateway.route.parentRef` names *this* Gateway, because a values file
re-pointed at the chart's upstream default would render routes no Gateway here
ever serves, with everything reporting healthy.

### 18. `insight-db-creds` must exist **without** Helm's ownership label

**The setting.** A plain Secret in `insight`, created outside any release, with
keys `clickhouse-password`, `mariadb-password`, `mariadb-root-password`,
`redis-password`.

**What breaks otherwise.** The chart detects "bring your own" by the *absence*
of `app.kubernetes.io/managed-by=Helm` and then skips emitting its own copy;
`credentials.autoGenerate: true` composes the per-service config Secrets from
it.

**How it presents.** Anything that has claimed the name for Helm makes the
install abort with `invalid ownership metadata`. Note the dry-run artefact:
Helm skips `lookup` on a dry run, so rendered output shows the chart emitting
its own copy — a real install finds the pre-created one. Do not "fix" that.

### 19. A DSN charset constraint on all four datastore passwords

**The setting.** The chart composes every DSN by string interpolation, and its
helpers **reject** any of `@ : / ? # %` in *every* key of `insight-db-creds`,
including `mariadb-root-password`, failing the install rather than warning.

**What breaks without the discipline.** The deployment repository's generators
emit hex for exactly this reason, and the MariaDB root password is
repository-owned (`generate: false`) rather than operator-generated because the
operator's mixed-character value fails often enough to matter.

**How it presents.** A hard install failure naming the key — correctly
attributed, but only after a datastore has already been provisioned with the
offending value. One caveat is recorded in the manifest and worth repeating:
`generate: false` only decides the **bootstrap** password. On a cluster that has
already run, editing the Secret does not change the live server — rotate with
`ALTER USER` on both `root@%` and `root@localhost` in the same breath, or the
operator loses its own SQL access.

### 20. MariaDB `log_bin_trust_function_creators=1`

**The setting.** In the `MariaDB` CR's `myCnf`.

**What breaks without it.** The Insight analytics migration creates stored
functions as the unprivileged `insight` user, and MariaDB refuses with
`1419 (HY000): You do not have the SUPER privilege and binary logging is
enabled`. Galera raises this even with binary logging off, because wsrep
replicates DDL by statement.

**How it presents.** The Analytics Deployment sits in `Init:CrashLoopBackOff`
until `helm --wait` gives up with "Progress deadline exceeded". The alternative
— granting `SUPER` to the application user — is worse on every axis.

### 21. The ClickHouse `insight` user carries a network allow-list

**The setting.** `insight/networks/ip` on the `ClickHouseInstallation` — a
single private-range block, written in the deployment repository's
`clickhouse/clickhouse.yaml`. The literal is not repeated here; read it from
that manifest, or from `kubectl -n clickhouse get chi clickhouse -o yaml`.

**What breaks without a matching pod network.** The allow-list silently assumes
the cluster's pod CIDR falls inside that one block. A cluster built with a pod
network outside it authenticates nothing from any pod.

**How it presents.** ClickHouse itself is perfectly healthy and every
application connection is refused. Nothing in the manifest checks the
assumption — a cluster template whose pod CIDRs happen to fall inside the range
makes it invisible. Compare the allow-list against
`kubectl get nodes -o jsonpath='{range .items[*]}{.spec.podCIDR}{"\n"}{end}'`
before reusing the manifest on a differently-provisioned cluster.

### 22. `cinder` must be the cluster **default** StorageClass

**The setting.** The `storageclass.kubernetes.io/is-default-class: "true"`
annotation, not just the class's existence.

**What breaks without it.** Every other datastore names `storageClassName:
cinder` explicitly, but the Airbyte chart's bundled PostgreSQL and MinIO take
the **default**.

**How it presents.** The Airbyte deploy script reads the annotation and
hard-fails unless the default is exactly `cinder` — so this one is caught. Left
unchecked it would surface as PVCs that never bind, long after the rest of the
stack is up.

### 23. Non-default security contexts on three workloads

**The setting.** MariaDB runs with `runAsNonRoot: false` / `runAsUser: 0` /
`fsGroup: 999` / `fsGroupChangePolicy: Always`; Redis runs with
`runAsUser`/`runAsGroup`/`fsGroup` all 0; ClickHouse and Keeper set
`fsGroup: 101`; and Airbyte's bundled PostgreSQL and MinIO PVCs are
**pre-created** and ownership-initialised by two throwaway Jobs before the chart
is installed.

**What breaks without them.** This Cinder deployment does not apply `fsGroup`
ownership to new volumes, so each workload's entrypoint has to chown its own
data directory before dropping privileges.

**How it presents.** Permission-denied crash loops on first start of a
freshly-provisioned volume. These look like sloppiness if copied without the
reason, which is exactly why the reason is written here — they are not
candidates for tightening without first fixing the volume-ownership behaviour.

### 24. The edge Service: pinned address plus `keep-floatingip`, and `externalTrafficPolicy: Cluster`

**The setting.** `EnvoyProxy/insight` sets a pinned `loadBalancerIP` **and** the
annotation `loadbalancer.openstack.org/keep-floatingip: "true"`, with
`externalTrafficPolicy: Cluster`.

**What breaks without each.** Without `keep-floatingip`, a teardown releases the
address back to the pool and the DNS record dangles — that annotation is the
entire reason a rebuild keeps the same public name. With an address pinned that
is still attached to something else, the cloud controller refuses to steal it.
And `externalTrafficPolicy: Local` — the Envoy Gateway default — only routes
through nodes actually running an envoy pod, depending on the load balancer's
health monitors to notice which those are.

**How it presents.** A contended pinned address means the Gateway never gets an
address and the deploy times out after ten minutes rather than failing fast.
`Local` costs reachability and buys nothing here: client IPs are lost either
way, because Cloudflare fronts the origin and the real client is in
`CF-Connecting-IP`, not the TCP peer. **For a brand-new stand the correct
setting is `auto`**, which drops the `loadBalancerIP` line entirely and lets the
cloud controller allocate; the deploy script then prints the allocated address
to pin.

### 25. After every upgrade, Analytics, Authenticator and Identity Resolution must be restarted

**The setting.** Not a value — a step. See
[`README.md` step 4](README.md#deploying-by-hand).

**What breaks without it.** The chart injects `insight-<svc>-config` with
`envFrom.secretRef`, and environment variables are read once at container start.
Each subchart's `checksum/config` annotation covers its **gears ConfigMap**, not
that Secret.

**How it presents.** `helm upgrade` rewrites datastore hosts and passwords, the
pod spec is identical so nothing restarts, and the running pods keep serving the
old configuration. Moving Redis from one endpoint shape to another is the
canonical case: the release reported `deployed`, every pod stayed Ready, and the
authenticator went on dialling a Service that no longer existed. Note a
disagreement worth resolving: the deployment repository rolls two Deployments
(authenticator, analytics) while this repository's README rolls three —
`identity-resolution` also consumes its config Secret with `envFrom` and is
subject to the identical staleness, so three is the correct list. `gateway` and
`frontend` are excluded deliberately: neither reads that Secret, and restarting
them drops live traffic for nothing.

### 26. `helm --wait` does not wait on HTTPRoute status

**The setting.** Not a value — the post-condition the deploy has to add itself.

**What breaks without the check.** A route the Gateway rejects leaves the
release `deployed` and the site dark. Both conditions matter: `Accepted` alone
still 503s when the `backendRef` names a Service that does not exist.

**How it presents.** A green deploy and an unreachable stand. This is why the
deploy step in [`README.md`](README.md) and the workflow both check `Accepted`
after the upgrade even though the release now owns the routes — the check's
meaning changed from "somebody else's object survived" to "the object this
upgrade just wrote was accepted", and it earns its place either way.

### 27. Helm 4 apply-mode coupling

**The setting.** `--server-side=true --force-conflicts`, named explicitly rather
than inherited. Helm 3 takes neither flag.

**What breaks without naming it.** Helm 4.0/4.1 defaulted to server-side apply,
so `--force-conflicts` alone sufficed. Helm 4.2+ made `--server-side` default to
`auto` — meaning "whatever the previous release used" — so a release whose last
revision went client-side resolves to client-side.

**How it presents.** `invalid client update option(s): forceConflicts enabled
when serverSideApply disabled`, inherited rather than chosen: a stand starts
failing because of how somebody ran the *previous* upgrade.

### 28. Sizing values that must move together

**The setting.** Three couplings, each of which is silent when broken.
ClickHouse `replicasCount` and Keeper `replicasCount` must be raised in lockstep
with the replica counts the deploy and validate scripts size their waits from —
and ClickHouse must not be scaled at all until the chart issues cluster-wide DDL
(§2). `MARIADB_BUFFER_POOL` must move **with** the MariaDB memory request,
because InnoDB never uses a byte more than the buffer pool, so raising container
memory alone reserves capacity the database cannot touch. And the anti-affinity
asymmetry described under [Component inventory](#component-inventory) — hard for
Redpanda and Galera, soft for Redis — is a decision, not an inconsistency.

**How it presents.** A raised replica count that the deploy script never waits
for, a database that ignores the memory it was given, or a StatefulSet that
never converges with no event explaining why.

### 29. Template rendering discipline

**The setting.** Several manifests in the deployment repository are
**templates** carrying `__INSIGHT_HOST__` / `__ENVOY_GATEWAY_LB_IP__` /
`__CLICKHOUSE_*__` / `__KEEPER_*__` / `__MARIADB_*__` placeholders. They must
never be `kubectl apply -f`'d directly, only through their deploy script.

**What breaks otherwise.** A literal `__FOO__` is valid YAML.

**How it presents.** It reaches the API server as a hostname or a quantity and
fails at admission with a message naming neither the stand nor the file. The
repository asserts in both directions — it fails a deploy on any leftover
placeholder, and it fails if a literal value creeps back **into** a template,
which would silently re-pin every stand to one host or address.

## Bootstrap steps that are not a `helm install`

These are the steps that create state the release consumes by name. They are
invisible in `helm get manifest`, so anyone reconstructing the stand from the
rendered release alone will miss all of them.

### 1. Databases and grants — created by two different layers

The deployment repository creates only the MariaDB `insight`
Database + User + Grant (operator CRs) and the ClickHouse `default` and
`insight` users. **The rest are created in-server by the umbrella's own
pre-install/pre-upgrade hook Jobs:**

* `insight-mariadb-init-svcdbs` (hook-weight 5) connects as MariaDB **root**,
  using `insight-db-creds`/`mariadb-root-password`, and runs
  `CREATE DATABASE IF NOT EXISTS` for `identity` and `keycloak` (utf8mb4 /
  utf8mb4_unicode_ci) plus `GRANT ALL` to `insight@%`.
* `insight-clickhouse-init-svcdbs` (hook-weight 5) creates the ClickHouse
  databases `insight` and `presentation` as the `insight` user over the HTTP
  API.

That is **why the root password has to be in `insight-db-creds`** even though no
DSN interpolates it, and why `identity` and `keycloak` do not appear as MariaDB
`Database` CRs. The other two hooks are
`insight-keycloak-config` (post-install/post-upgrade, weight 100 — the
keycloak-config-cli Job, drift-reverting, so admin-console edits do not survive
a deploy) and `insight-clickhouse-migrate` (weight 200 — the ClickHouse schema
migration, whose failure fails the whole upgrade). Every hook Job carries
`ttlSecondsAfterFinished: 600`, so Kubernetes deletes it and its logs ten
minutes after it finishes: **capture hook logs immediately on a failure.**

### 2. The Secrets the release expects to already exist

In `insight`, all created outside the release:

| Secret | Keys | Semantics |
|---|---|---|
| `insight-db-creds` | `clickhouse-password`, `mariadb-password`, `mariadb-root-password`, `redis-password` | **Recomposed on every run** by reading the four operator-owned Secrets below. Applied without a Helm label (§18). |
| `insight-authenticator-signing-keys` | `current.pem` | **Generate once, reuse forever.** See §3 below. |
| `insight-keycloak-admin` | `username`, `password` | Bootstrap admin for the bundled Keycloak. Generated once; regenerating strands the server's stored admin user. |
| `insight-oidc` | `client-secret` | The confidential client secret. Generated once; the keycloak-config Job pushes the same value into the realm's client on every deploy so the two cannot drift. Rotation is "delete the Secret, re-run the deploy". |
| `insight-keycloak-config` | config-cli login + client secret | **Recomposed on every run** from the two above. On a seeded stand it carries nothing else — the external-IdP OAuth passthrough values exist only in the GitHub login mode. |

Required in **other** namespaces before the Insight step will run:
`clickhouse/clickhouse-insight-credentials`, `mariadb/mariadb-insight-credentials`,
`mariadb/mariadb-root`, `redis/redis-auth`. Separately,
`airbyte/airbyte-auth-secrets` is created by Airbyte on first boot and is warned
about rather than failed on — the reconcile loop cannot authenticate without it.

A seeded stand needs **no GitHub OAuth App**; requiring one would be asking for
a registration nothing ever reads.

### 3. The ES256 signing key

```text
openssl ecparam -name prime256v1 -genkey -noout | openssl pkcs8 -topk8 -nocrypt
```

under `umask 077`, stored as key `current.pem`. The chart mounts it
non-optionally and never generates it. Generate-once semantics here are
absolute: a new key silently invalidates every issued gateway JWT and every
token the gateway has cached, so "re-run the bootstrap to be safe" logs
everybody out.

### 4. The realm ConfigMap pack

`<release>-keycloak-config-realms` in `insight`, holding **exactly one key** —
`realm-insight.json` on a seeded stand. It is written with
`create configmap --dry-run=client | apply`, which rewrites the whole data map,
so switching login modes *prunes* the other mode's realm file rather than
leaving two for the config Job's glob to find. The key name is load-bearing
because `keycloakConfig.filesLocations` globs by extension (§13).

On a seeded stand the realm is **generated, not checked in**, from the seeder's
own module, so the realm and `identity.persons` stay two projections of one
roster. Two guards run before generation: the tenant must be non-empty (a realm
minting users with no tenant claim would be invisible to every login), and the
generator's password constant must match the copy the validator signs in with,
or the deploy refuses. The generated document is then post-processed to add the
`idp_sub` attribute and mapper (§11) and the declarative user profile (§12).

This ConfigMap is now **read back by the seed**, not only applied by the config
Job: `seed-stand.sh` takes the dev-lead persona address from the user whose `id`
is the roster's dev-lead UUID, so the realm is the source of truth for who
exists and the seed follows it. Two consequences for anyone changing how it is
packed. The post-processing must keep `.users[].id` and `.users[].email` intact
— the discovery depends on those two fields and on nothing else about the
document's shape, deliberately, because the applied realm is a derivative this
repository does not produce. And a stand that packs a *broker* realm here, or
packs several files naming different people, offers no unambiguous answer; the
seed then stops and names `--email` rather than guessing, which is the intended
disposition. See [One roster, two projections](#one-roster-two-projections).

### 5. The `argo-workflow` ServiceAccount in `insight`

A ServiceAccount, a Role granting `create` and `patch` on
`workflowtaskresults.argoproj.io`, and a RoleBinding — applied with `kubectl`,
**outside** the Helm release so `helm uninstall` cannot take the account away
from workflows that are still queued.

The umbrella pins every WorkflowTemplate and CronWorkflow it ships to that
account and lists it as a subject of its own `insight-airbyte-auth-reader`
RoleBinding, but does not create it; the Argo release's copy lives in the `argo`
namespace, and a workflow pod runs in the namespace of its Workflow. Without it
every dbt transform and data-quality check fails with
`serviceaccount "argo-workflow" not found` while all app pods stay healthy —
nothing surfaces until a scheduled run. The `workflowtaskresults` Role is
separately load-bearing: Argo 3.4+ has each step report its outcome through the
pod's own ServiceAccount, so a step without those two verbs dies with exit 64
and `workflowtaskresults.argoproj.io is forbidden` before the user container
starts.

### 6. The Gateway and the ClusterIssuer chain

Applied as manifests, not as chart values:

* `GatewayClass/envoy` (controller
  `gateway.envoyproxy.io/gatewayclass-controller`), `Gateway/insight` in
  `envoy-gateway-system` with `infrastructure.parametersRef` →
  `EnvoyProxy/insight` and the annotation
  `cert-manager.io/cluster-issuer: insight-ca`, plus
  `HTTPRoute/redirect-http-to-https` issuing a 301 from the `http` listener.
* A **two-step** self-signed chain, not one issuer:
  `ClusterIssuer/insight-selfsigned` (bootstrap only) signs
  `Certificate/insight-ca` in the `cert-manager` namespace (isCA, ECDSA-256,
  long-lived, secret `insight-ca-key-pair`), and `ClusterIssuer/insight-ca`
  built on that key pair is the one the umbrella names. A bare selfSigned
  issuer would also publish a `ca.crt`, but every leaf would be its own
  unrelated root, so downstream trust could not be pinned to one CA. The
  Certificate's namespace is fixed: a `ca` ClusterIssuer only ever reads its key
  pair from the cert-manager release namespace.

The listener's TLS Secret `insight-origin-tls` is **minted, never created by
hand** — cert-manager's gateway-shim reads the Gateway annotation and mints a
Certificate per HTTPS listener hostname. The chain is self-signed on purpose:
Cloudflare terminates the public TLS and its "Full" origin-pull accepts a
self-signed origin. **The zone must stay on Full** — "Full (strict)" would
reject the origin certificate, and "Flexible" would fetch over port 80 and turn
the http→https redirect into an infinite loop.

### 7. The Airbyte instance setup call

`POST /api/v1/instance_configuration/setup` with `initialSetupComplete: true`,
an email and an organisation name, authenticated by a token minted from
`POST /api/v1/applications/token` using the instance-admin credentials in
`airbyte-auth-secrets`. It is the consequence of load-bearing setting §7: with
auth enabled a fresh instance boots into a setup wizard and stays
half-initialised until this call is made, and the chart does not make it. Run from a throwaway in-cluster
pod rather than a port-forward, so it exercises the same Service DNS the
ingestion workflows resolve. Idempotent by construction, so it runs on every
deploy rather than guessing.

### 8. Outside the cluster entirely: the DNS record

A public A record for the stand hostname pointing at the edge address,
Cloudflare-proxied with SSL mode Full. The Insight step is **gated** on that
record resolving and refuses to install without it, because the authenticator
resolves `issuerUrl` at startup *and* the browser is redirected to that
hostname — installing before the record exists produces a stand nobody can log
in to.

### 9. Route adoption — migration only

For a stand that was deployed **before** chart 0.5.107, the deploy labels and
annotates any pre-existing `insight-gateway` / `insight-keycloak` HTTPRoute for
Helm to adopt in place; without that the upgrade dies with `invalid ownership
metadata … missing key "app.kubernetes.io/managed-by"`. Idempotent, and a fresh
stand needs nothing. The chart's route names were chosen to match the old ones
exactly, which makes the adoption an update-in-place with no traffic blip and no
creation-timestamp tie (§16).

## One roster, two projections

**This is the invariant a future change is most likely to break, so it is stated
here on its own rather than left implicit in the sections that depend on it.**

The realm and `identity.persons` are two projections of **one** roster. The
realm decides who can authenticate; the seeded rows decide who a login resolves
to. If they disagree, a persona authenticates and resolves to nobody — or, as
observed, three personas work and the fourth silently cannot sign in, with every
pod Ready and the release reporting `deployed`. Adding people to the seed
therefore means regenerating the realm from the same roster, and the seeder now
reads the dev-lead address back from the applied realm so the two cannot drift
on the one value an operator supplies.

Three things follow, and each one is a way this has already gone wrong or could:

* **Only the dev-lead can drift.** Every other persona's address is derived
  deterministically from the roster module and passes through no input at all.
  The dev-lead's was the last operator-supplied value in the seed path, which is
  exactly why it is the one that came apart — and why removing that input, not
  adding a check downstream of it, is the fix.
* **The realm is the source of truth for who exists.** `seed-stand.sh` reads the
  dev-lead address out of the `<release>-keycloak-config-realms` ConfigMap, from
  the user whose `id` is the roster's dev-lead UUID
  (`insight_seed.profiles.DEV_LEAD_UUID`; the generator writes that UUID as each
  realm user's `id`, which is what makes the lookup total and unambiguous
  whatever address the user carries). Regenerate the realm and the next seed
  follows it. `--email` overrides the read for a stand whose realm came from
  somewhere else, and overriding it on a stand whose realm *is* the roster is
  how you reproduce the failure above on purpose.
* **Nothing below this line checks it.** The seed Job's own preflight asserts
  that a dev-lead address is *set*, never that anybody in the IdP answers to it,
  and the only detector further downstream is the smoke gate — which the deploy
  workflow's `stages` input can legitimately skip. Treat any design that leaves
  a realm/roster disagreement as a warning as relying on a gate that is optional
  by construction.

The sibling failures with the same signature are [§11](#11-authenticatoroidcexternalidclaim-idp_sub--not-sub)
(the external-id claim names the wrong claim) and [§15](#15-authenticatoroidcsourcetype-must-equal-what-the-seeder-writes)
(the source type disagrees with what the seeder writes). All three end in a
login that authenticates and then resolves to nobody, against a fully populated
projection, and telling them apart is most of the work of diagnosing one.

## Between deploy and seed

**A seeded stand is not usable between `deploy` and `seed`, and that is not a
fault.** Logins authenticate against Keycloak and then resolve against the
`identity.persons` rows the seeder writes, so an unseeded stand authenticates a
user and immediately denies them. Two consequences follow that get misread as
breakage:

* A login test run before seeding fails on a login that authenticates and
  resolves to nobody. That is true, and it is not a deploy failure — which is
  why the deployment repository's deep validation deliberately does not run it
  as part of the deploy.
* The `identity-resolution` **seed** CronJob is *expected* to fail on a fresh
  stand. It guards an empty `identity_inputs` read and exits non-zero rather
  than publishing an empty projection. A failing seed Job here is the guard
  working. See the comment at the bottom of [`values.yaml`](values.yaml) for the
  converse risk once that table stops being empty.

Seeding itself is a thin wrapper around the application repository's own
`src/ingestion/tools/seed/seed-stand.sh`. It discovers everything from the
cluster — datastore hosts from `insight-platform`, tenant and identity database
from `insight-identity-resolution-config`, the seeder image from the chart's
`ingestion.seedImage`, the login source type from `insight-authenticator-config`,
and the dev-lead persona address from the realm ConfigMap (§4 below, and
[One roster, two projections](#one-roster-two-projections)) — so nothing is
copied from the deployment repository and nothing can drift.

That last one was the exception until recently: the address arrived as a flag,
which meant the realm and the seeded rows had two independent inputs and could
be pointed at different people while both halves reported success. The list
above is now literally complete, which is the property that sentence was always
claiming.

## Stand-specific versus stand-shape

The split below is **enforced, not merely documented**: several names are
pinned by hard assertions in the deploy scripts, so an override is a startup
error rather than a silent divergence.

### What a second stand of this shape changes

| Thing | Notes |
|---|---|
| The kubeconfig and the API server the scripts are allowed to act against | The guard is the server URL, not the context name, because a context name can repeat across kubeconfigs. Neither belongs in this repository. |
| The public hostname | One value; every manifest that names it carries a placeholder. Also the DNS record, the Gateway listener hostname, `gateway.route.host`, `keycloak.route.host`, `issuerUrl`, `redirectUri`, `csrfOrigins`. |
| The edge address | `auto` on a brand-new stand (§24). Never written here. |
| The login mode — `github` or `seeded` | Everything downstream follows from it: the realm name, `sourceType`, `externalIdClaim`, `filesLocations`, whether a GitHub OAuth App is needed at all, and whether seeding is mandatory. |
| The tenant UUID | Technically per-stand, but written in **four coupled places** that must be identical: `global.tenantDefaultId`, `ingestion.reconcile.tenantId`, `authenticator.oidc.defaultTenantId`, and the value handed to the realm generator (which reads it back out of the values file rather than repeating it). Rows written under a tenant the stand does not use are invisible to every login while deploy and seed both report success. `identityResolution.seed.tenantDefaultId` is deliberately **not** set — the composed config Secret already carries it, and setting it adds a fifth copy. |
| The resource profile | Placeholder-rendered sizing for the CR-based datastores; per-stand Helm overlays for the chart-based ones. |

### What a second stand copies verbatim

The deploy order. Every namespace name (`envoy-gateway-system`, `cert-manager`,
`clickhouse-operator`, `clickhouse`, `mariadb-operator`, `mariadb`,
`redis-operator`, `redis`, `redpanda`, `airbyte`, `argo`, `insight`). Every
release name (`eg`, `cert-manager`, `clickhouse-operator`, `mariadb-operator`
and `mariadb-operator-crds`, `redis-operator`, `redis`, `redpanda`, `airbyte`,
`argo-workflows`, `insight`). Every chart repository and pinned version. The
StorageClass name `cinder` and its default-class annotation. The issuer names
`insight-selfsigned` / `insight-ca` and the secret name `insight-ca-key-pair`.
The Gateway name `insight`, GatewayClass `envoy`, EnvoyProxy `insight`, TLS
Secret `insight-origin-tls`, and the listener names. Every in-cluster Service
DNS name in `values.yaml`. Every Secret and ConfigMap name. The ServiceAccount
name `argo-workflow`. And **every one of the load-bearing settings above.**

## Recreating a stand of this shape

Top to bottom. Steps 1–11 are the deployment repository's job; step 12 is where
this repository takes over.

1. **Provision the cluster.** Kubernetes with a CNI, at least three schedulable
   workers (hard anti-affinity, above), and the OpenStack Cinder CSI driver
   registered as `csidriver/cinder.csi.openstack.org`. Verify with
   `kubectl get csidriver` before going further — five later steps hard-fail on
   its absence and one of them takes ten minutes to do so.
2. **Create the `cinder` StorageClass and make it the cluster default.**
   Idempotently applied by five of the deploy scripts, each of which also
   accepts a skip flag.
3. **Choose the per-stand values**: hostname, login mode, tenant UUID, resource
   profile, and `auto` for the edge address on a brand-new stand.
4. **Deploy Envoy Gateway** (step 1 of the order). Its `https` listener will
   finish **unprogrammed** — expected. Note the allocated edge address the
   script prints, and pin it for subsequent runs.
5. **Create the public DNS record** pointing at that address, Cloudflare-proxied
   with SSL mode **Full**. Do this now: the Insight step is gated on it
   resolving, and DNS propagation is the one thing in this list that cannot be
   hurried.
6. **Deploy cert-manager** (step 2), which creates the two-step issuer chain and
   completes the Gateway's certificate. Confirm both ClusterIssuers report
   `Ready=True` before continuing — the Insight preflight will refuse without it,
   but finding out here is cheaper.
7. **Deploy the four datastores in order**: clickhouse, mariadb, redis, redpanda
   (steps 3–6). Each deploy script waits past pod-Ready for the thing that
   actually matters — ClickHouse until the `insight` user authenticates over the
   pod network and `system.clusters` reports the expected topology; Redis until
   exactly one member reports `role:master`, both replicas report
   `master_link_status:up`, **and** the `redis-master` Service has a ready
   endpoint. Do not shortcut those waits: a StatefulSet can report complete
   while the Service the application dials has no endpoint at all.
8. **Confirm `maxUserConnections: 100` on the MariaDB `User/insight`** (§1)
   before anything connects to it. This is the one item on the list that is
   cheap now and expensive later.
9. **Deploy Airbyte** (step 7), including its pre-created PVCs, its
   ownership-init Jobs and the one-time instance setup call
   ([Bootstrap §7](#7-the-airbyte-instance-setup-call)).
10. **Deploy Argo Workflows** (step 8) with `crds.install=true` and
    `crds.keep=true`. Verify `singleNamespace: false` and that the controller's
    `instanceID` matches `ingestion.reconcile.argoInstanceId` (§4) — both are
    silent when wrong.
11. **Run the Insight bootstrap that is not a helm install**: compose
    `insight-db-creds` without a Helm label, generate the ES256 signing key,
    generate the keycloak admin and OIDC client secrets, compose
    `insight-keycloak-config`, generate and pack the realm ConfigMap, and apply
    the `argo-workflow` ServiceAccount + Role + RoleBinding
    ([Bootstrap §§2–5](#2-the-secrets-the-release-expects-to-already-exist)).
    Remove the maintenance route if one was applied (§16).
12. **Hand over to this repository.** The umbrella upgrade is the `helm upgrade`
    spelled out in [`README.md` step 3](README.md#deploying-by-hand) — the same
    command CI runs — followed by the restart in step 4 (§25) and the route
    check in step 5 (§26). Rehearse it read-only first with
    `make diff ENV=test-stand`, which renders the same chart through the same
    values file and contacts no cluster.
    Note that `make deploy ENV=test-stand` is **not** the deploy path for this
    environment and would do damage; see
    [Why not `make deploy`](README.md#why-not-make-deploy).
13. **Seed.** The stand is not usable until this runs
    ([Between deploy and seed](#between-deploy-and-seed)). Capture the seeder's
    manifest from the Job log before its TTL reaps it, and treat that JSON as
    run-internal — it carries persona addresses, UUIDs and in-cluster URLs.

Teardown, for completeness, is the exact reverse: insight, argo-workflows,
airbyte, redpanda, redis, mariadb, clickhouse, envoy-gateway, cert-manager.
Envoy Gateway must come **after** insight because the application HTTPRoutes
attach to its Gateway, and cert-manager must be **last** because both the
authenticator's Certificate and the Gateway's origin certificate must be gone
before their issuer is, and the gateway-shim crashes if the Gateway API CRDs
disappear while it is still running. CRDs and the shared StorageClass are
cluster-scoped and are never removed unless asked for by name. The edge address
survives by design (§24), which is what keeps DNS valid across a rebuild.

## Known drift, and what not to copy

* **The Gateway's listener set.** The reference manifest defines three listeners
  — `https`, `https-default`, `http` — while a stand built before the
  hostname-less catch-all was added carries only `https` and `http` and has
  never been re-applied. A rebuild from the reference will therefore produce a
  three-listener Gateway that differs from an older live one. That is an
  expected difference, not a fault; the behaviour that changes is what a
  bare-address or unknown-Host visitor gets (a 404 from envoy, versus the
  catch-all listener's response).
* **Retired components in the reference repository.** A Patroni/PostgreSQL
  triple, an operator-managed Keycloak triple that depended on it, a Dex
  alternative, and an older seed mechanism all still exist as files and are
  **not** deployed. None appears in the deploy order. They keep literal
  hostnames rather than placeholders precisely so that parameterising them
  cannot imply they are live. Do not copy any of them into a rebuild.
* **Reference-repository prose that predates the current shape.** Its README
  still describes a PostgreSQL layer and a six-member Redis *Cluster*; the live
  shape is no PostgreSQL at all and a three-member Redis *replication* group.
  Trust the manifests and `helm list -A` over the prose, in that repository and
  in this one.
