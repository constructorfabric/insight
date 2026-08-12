# `test-stand` — what has to exist beneath the umbrella

[`README.md`](README.md) covers the one thing this directory deploys: the
umbrella release `insight`, from [`values.yaml`](values.yaml). This file covers
everything the release lands *on top of* — the cluster, the datastores, the
edge — none of which this repo deploys and none of which CI touches.

It exists because the release cannot tell you any of it. Almost every failure
below shares one signature: **every pod Ready, the release `deployed`, and the
thing that matters silently not happening.** That sentence is the most useful
one here.

**This file is the spec; the deployment repository is the executable form**
(one `deploy-<svc>.sh` + `validate-<svc>.sh` per component, a `deploy-all.sh`,
per-stand overlays). There is no generation step, so this can drift from it —
treat every version and name below as "true when last verified against a live
stand", not "true now". Drift is cheap to check, read-only, against an admin
kubeconfig:

```bash
helm list -A                                   # against the inventory table below
kubectl get sc                                 # cinder, must be the default
kubectl get clusterissuer                      # insight-selfsigned + insight-ca, both Ready
kubectl -n mariadb get user insight -o jsonpath='{.spec.maxUserConnections}{"\n"}'   # 100
kubectl -n insight get sa argo-workflow
kubectl -n airbyte get role insight-airbyte-auth-reader
```

The CI deploy credential (namespace-scoped SA in `insight`) can run none of
those — it cannot `helm list -A`, read `mariadb/`, or see the Gateway. That is
the right shape for a deploy credential, and the reason this file is maintained
by hand rather than generated.

## The layer model

| Layer | What | Deployed by | This repo's relationship |
|---|---|---|---|
| **L-1** | Cluster: nodes, CNI, OpenStack Cinder CSI driver, public DNS record | the cloud platform | assumed; deploy scripts hard-fail without the CSI driver, Insight is gated on DNS |
| **L0** | Edge + PKI: Envoy Gateway (`GatewayClass`/`Gateway`/`EnvoyProxy`), cert-manager + the two-step `ClusterIssuer` chain, the `cinder` StorageClass | the deployment repository | named in `values.yaml`; never created here |
| **L2** | Datastores under their operators (ClickHouse, MariaDB, Redis, Redpanda), plus Airbyte and Argo — each in its own namespace | the deployment repository | addressed by in-cluster Service DNS; never created here |
| **L2** | Generate-once Secrets + the realm ConfigMap the release consumes by name | the deployment repository | referenced by name; `enabled: false` in `inventory.yaml` |
| **L3** | **The umbrella release `insight` — every value in `values.yaml`** | **this directory** | **owned; changed by CI on every merge to `main`** |

The single most important structural fact: **the release renders its own edge
routes but not the Gateway they attach to.** Since chart 0.5.107 the umbrella
carries native Gateway API templates, so `HTTPRoute/insight-gateway` and
`insight-keycloak` arrive with the release; `Gateway/insight` in
`envoy-gateway-system` stays L0. An HTTPRoute whose `parentRef` names a Gateway
that does not exist is created successfully and serves nothing.

## The deploy order

`deploy-all.sh` runs each `deploy-<svc>.sh` then its `validate-<svc>.sh`,
stopping at the first failure. The order is not interchangeable:

```text
envoy-gateway → cert-manager → clickhouse → mariadb → redis → redpanda
  → airbyte → argo-workflows → insight        ( → seed, deliberately separate )
```

- **envoy-gateway first** — its chart installs the Gateway API CRDs, which
  cert-manager's gateway-shim (run with `config.enableGatewayAPI=true`) needs
  to start.
- **cert-manager back-fills PKI** — on a fresh cluster the Gateway's `https`
  listener comes up *unprogrammed* (no issuer yet); that is expected, and
  cert-manager completes it.
- **seed is not in the list, on purpose** — deploy gives the stand its
  services, seed gives it its people. See [Between deploy and seed](#between-deploy-and-seed).

## Component inventory

Versions cross-checked against `helm list -A` on a live stand; pinned, not
floating (every deploy script passes `--version`).

| # | Layer | Component | Namespace | Chart @ version |
|---|---|---|---|---|
| — | L-1 | CNI | `kube-system` | `cilium` 1.16.3 — its pod CIDR must fall inside the ClickHouse user's IP allow-list (§21) |
| — | L-1 | Cinder CSI + `cinder` StorageClass | — | `WaitForFirstConsumer`, `allowVolumeExpansion`, **annotated cluster-default** (§22) |
| 1 | L0 | Envoy Gateway | `envoy-gateway-system` | `gateway-helm` v1.8.3 — Gateway API CRDs, `GatewayClass/envoy`, `Gateway/insight`, the edge LB Service |
| 2 | L0 | cert-manager | `cert-manager` | v1.21.1 — `ClusterIssuer/insight-ca` + the Gateway origin cert |
| 3 | L2 | ClickHouse operator + CH/Keeper | `clickhouse-operator`, `clickhouse` | operator 0.27.1; the `insight` user; the **per-pod** Service (§2) |
| 4 | L2 | MariaDB operator + Galera×3 | `mariadb-operator`, `mariadb` | operator 26.6.0, `mariadb:11.8.8`; Service `mariadb-primary` (§8) |
| 5 | L2 | Redis operator + replication×3 | `redis-operator`, `redis` | operator 0.25.0, `redis-replication` 0.17.0; Service `redis-master` (§3) |
| 6 | L2 | Redpanda | `redpanda` | `redpanda` 26.1.9; internal Kafka listener on **9093** (§9) |
| 7 | L2 | Airbyte | `airbyte` | `airbyte` 1.8.5; `airbyte-auth-secrets`, the reconcile API |
| 8 | L2 | Argo Workflows | `argo` | `argo-workflows` 1.0.22 — the workflow CRDs **without which the umbrella cannot render** |
| 9 | L3 | **Insight umbrella** | `insight` | `insight` 0.5.111 |

**Node shape is a hard requirement.** Redpanda and MariaDB Galera use *hard*
pod anti-affinity across three members, so fewer than three schedulable workers
leaves both permanently unconverged. Redis uses *soft* anti-affinity on purpose
(a hard spread deadlocks against the operator's own master/replica rule).
Reference: three control-plane + three workers on Kubernetes 1.31. PV sizing:
CH 100Gi, Keeper 10Gi, MariaDB 100Gi + 1Gi Galera-config/member, Redis
10Gi/member, Redpanda 50Gi/broker, Airbyte 10Gi each for PostgreSQL + MinIO.

## The load-bearing settings

Each fails in a way that names something *other than itself* — that is what
earns the list. Correct value on the left, the lie it tells on the right.
Ordered roughly by how badly the failure misleads.

| # | Setting (correct value) | What breaks / how it presents |
|---|---|---|
| 1 | **`maxUserConnections: 100`** on MariaDB `User/insight` (operator default is 10) | One account backs four pools + the bundled Keycloak's own 100. Identity-Resolution and the analytics `migrate` init crash-loop on `1226 … max_user_connections`; the **deploy** dies with a misleading `client rate limiter … context deadline exceeded`. If a deploy times out mentioning a rate limiter, read `insight-identity-resolution` logs first. |
| 2 | **`clickhouse.host` = the per-pod Service** `chi-clickhouse-clickhouse-0-0…` (not the fan-out `clickhouse-clickhouse`) | The chart is not cluster-aware (no `ON CLUSTER`, all `*MergeTree`/`View`). DDL through a fan-out Service scatters across servers that then disagree; migration fails part-way with `UNKNOWN_TABLE`→`HTTP 404`. Per-pod name used even at one replica. |
| 3 | **`redis.host` = replication primary** `redis-master.redis…` (not Redis Cluster, not Sentinel) | The authenticator's Rust client has no `cluster` feature; a clustered Redis answers `MOVED` it can't follow. `/auth/login` → 500 for everyone, pods Ready. `-master` selects the operator's `redis-role=master` label, so it survives failover. |
| 4 | **Argo `singleNamespace: false` + matching `instanceID`** (`argo-workflows-argo`) | With `true` the controller watches only `argo` while the CronWorkflows live in `insight`; a mismatched/empty instanceID makes labelled objects invisible. CronWorkflows sit with `status: {}` forever, zero Workflows — reads as "nothing triggered yet". |
| 5 | **Argo `controller.workflowNamespaces` stays `[argo]`** — do not add `insight` | That list only controls where the chart *creates* the workflow SA/Role; `insight` already has one applied by `kubectl`. Adding it → Argo upgrade fails `invalid ownership metadata`. Cross-namespace *watch* comes from the ClusterRole `singleNamespace: false` renders. |
| 6 | **`airbyte.namespace: airbyte`** | Also decides where `Role/insight-airbyte-auth-reader` renders. Empty → defaults to the release namespace; RBAC lands in `insight` where the Secret isn't, connector provisioning fails at run time, pods healthy. |
| 7 | **Airbyte `global.auth.enabled: true`** | Without it the server never mounts the authorization endpoints, so `POST /applications/token` (the reconcile mint) → 404 while `/health` is 200. Consequence: the one-time instance-setup call (Bootstrap §7). |
| 8 | **`mariadb.host` = `-primary` Service** (not round-robin `mariadb`) | Galera certifies every write cluster-wide; concurrent writers on different members conflict. The analytics `migrate` Job fails `1213` deadlocks. Same for `keycloak.database.host`. |
| 9 | **`redpanda.brokers` = one `host:9093` string** (9093 is the *internal* listener; no 9092 exists) | The chart hands it to a Kafka client verbatim; a bare host silently becomes `:9092`, never connects, pods healthy. Deploy-repo preflight refuses any entry without a colon-port. |
| 10 | **`authenticator.tlsDiscovery.enabled: true` + a Ready `ClusterIssuer/insight-ca`** | Not optional: the chart mounts `insight-authenticator-authn-tls-cert` **non-optionally** into analytics + identity-resolution. Off, or an issuer that can't sign → both sit `ContainerCreating` until `--wait` times out (message names a timeout, not a cert). |
| 11 | **`authenticator.oidc.externalIdClaim: idp_sub`** — not `sub` | keycloak-config-cli creates users via the admin REST API, where Keycloak assigns its own `sub` and discards the document's; the bring-up copies the roster UUID into an `idp_sub` attribute+mapper. With `sub`: every login authenticates then `login_denied_unknown_person` against a full `identity.persons`. |
| 12 | **Generated realm's user profile: `unmanagedAttributePolicy: ENABLED`** | Keycloak 26 always runs the declarative profile and *discards* undeclared attributes. Without it `tenant_id`/`org_unit`/`idp_sub` vanish on import, mappers emit nothing, login fails `id_token carries no non-empty idp_sub`. Realm looks right in the console. |
| 13 | **`keycloakConfig.filesLocations: "/config/*.json"`** (chart default is `*.yaml`) | The roster realm is JSON; default pattern matches zero files and config-cli **errors** on an empty glob → the hook Job fails the whole upgrade (loud, but costs a deploy to learn). |
| 14 | **`keycloak.hostname` ends in `/kc`** | Keycloak 26 hostname-v2 doesn't fold `--http-relative-path` into the advertised issuer. Wrong → server advertises an issuer the browser can't reach; discovery fails looking like TLS/routing. In-cluster pair: `keycloakConfig.url: http://insight-keycloak:8085/kc` + `allowInsecureUrl: true`. |
| 15 | **`authenticator.oidc.sourceType` = what the seeder writes** (`keycloak`) | `seed-stand.sh` reads it back out of `insight-authenticator-config` (one writer). Disagree → both halves succeed, login authenticates then resolves to nobody. Same symptom as §11, different cause. |
| 16 | **`frontend.route.enabled: false`** (the gateway owns the only `/`) | Two HTTPRoutes claiming `/` on one host are resolved by **creation timestamp**, oldest wins, both `Accepted`. A second route (incl. the deploy-repo maintenance page, which claims `/` with no hostname) silently bypasses auth or dead-ends. Removing the maintenance page is a required step. |
| 17 | **App routes attach with `sectionName: https`** (the named listener) | The Gateway also has a hostname-less catch-all 443 listener for bare-address/unknown-Host. A route narrows a listener's hostname, never widens it — so app routes name a hostname and the catch-all doesn't. Wrong section mixes the two populations. |
| 18 | **`insight-db-creds` exists WITHOUT Helm's ownership label** | The chart detects BYO by the *absence* of `managed-by=Helm` and skips its own copy. Anything claiming the name for Helm → install aborts `invalid ownership metadata`. (Dry-run shows the chart emitting its own copy — Helm skips `lookup` on dry runs; don't "fix" it.) |
| 19 | **No `@ : / ? # %` in any `insight-db-creds` value** (incl. root password) | The chart composes DSNs by string interpolation and rejects those chars, failing the install after a datastore was already provisioned with the value. Generators emit hex. `generate: false` only sets the *bootstrap* password — rotate a live server with `ALTER USER` on `root@%` and `root@localhost` together. |
| 20 | **MariaDB `log_bin_trust_function_creators=1`** (in `myCnf`) | The analytics migration creates stored functions as unprivileged `insight`; without it MariaDB refuses `1419 … SUPER privilege`. Galera raises it even with binlog off. Analytics sits `Init:CrashLoopBackOff` until `--wait` gives up. |
| 21 | **ClickHouse `insight` user IP allow-list matches the pod CIDR** | The allow-list silently assumes the cluster's pod CIDR is inside its one private block. A cluster with a pod network outside it → CH healthy, every app connection refused. Compare against `kubectl get nodes -o jsonpath=…podCIDR` before reusing the manifest. |
| 22 | **`cinder` is the cluster DEFAULT StorageClass** (the annotation, not just existence) | Every datastore names `cinder` explicitly, but Airbyte's bundled PostgreSQL/MinIO take the default. The Airbyte deploy script hard-fails unless the default is exactly `cinder`, so this one is caught. |
| 23 | **Non-default securityContexts on MariaDB/Redis/ClickHouse/Airbyte-PVCs** | This Cinder deployment doesn't apply `fsGroup` ownership to new volumes, so each entrypoint chowns its own data dir. Without them: permission-denied crash loops on first start of a fresh volume. Not candidates for "tightening" without fixing volume ownership. |
| 24 | **Edge Service: pinned address + `keep-floatingip: true` + `externalTrafficPolicy: Cluster`** | `keep-floatingip` is why a rebuild keeps the same public name. A pinned address still attached elsewhere → Gateway never gets an address, deploy times out. `Local` costs reachability for nothing (Cloudflare fronts; real client is in `CF-Connecting-IP`). **Brand-new stand: use `auto`.** |
| 25 | **Config-Secret changes don't roll pods — accepted gap, manual restart** | Analytics/Authenticator/Identity-Resolution read `insight-<svc>-config` via `envFrom`; `checksum/config` hashes only each subchart's ConfigMap, not the Secret. A changed Secret leaves the pod spec identical → stale config on Ready pods. After a deliberate change: `kubectl -n insight rollout restart deploy/insight-analytics deploy/insight-authenticator deploy/insight-identity-resolution`. |
| 26 | **Post-deploy HTTPRoute `Accepted` check** — `helm --wait` doesn't wait on route status | A route the Gateway rejects leaves the release `deployed` and the site dark; `Accepted` alone still 503s if the `backendRef` is missing. The deploy/workflow assert `Accepted`+`ResolvedRefs` after the upgrade. |
| 27 | **Helm 4 `--server-side=true --force-conflicts`** named explicitly | Helm 4.2+ defaults `--server-side` to `auto` (= whatever the last revision used), so a release whose last apply went client-side breaks with `forceConflicts enabled when serverSideApply disabled` — inherited from *how someone ran the previous upgrade*. |
| 28 | **Sizing values that move together** | CH + Keeper `replicasCount` in lockstep with the wait counts (and CH not scaled until cluster-wide DDL is issued, §2); `MARIADB_BUFFER_POOL` with the MariaDB memory request (InnoDB never exceeds the buffer pool). Broken → a wait that never fires, or DB that ignores its memory. |
| 29 | **Template placeholders (`__INSIGHT_HOST__` etc.) rendered only via deploy scripts** | A literal `__FOO__` is valid YAML; it reaches the API server as a hostname/quantity and fails at admission naming neither stand nor file. The repo fails a deploy on any leftover placeholder AND if a literal creeps back into a template. |

## Incidents that shaped this environment

Six failure modes, each with the guard that now holds it. Technical conditions,
not facts about any real deployment.

| Incident | What broke | Guard now holding it |
|---|---|---|
| **Fork-downgrade wedge** | Chart version resolved from `.insight-version` in a stale checkout → a downgrade; helm rewrites live Deployments into the older shape and wedges. A post-check comparing only against the *requested* version calls it a pass. | Resolver reads the OCI registry's latest tag first; the deploy workflow + `recreate-test-stand.sh` both refuse a version older than what's deployed. |
| **`TEST_STAND_SEED_EMAIL` drift** | Dev-lead address declared in two places (realm + seeder flag) with no agreement check; only that one login fails. | `seed-stand.sh` reads the address out of the applied realm ConfigMap (keyed on the roster UUID). The flag survives only as an explicit, warned override. |
| **730-day seed window** | Seed window wider than the analytics API's max queryable period → every window-derived request 400s, looking like a broad data problem. | Seed window capped inside the API limit; one shared query-window helper caps both suites at the same value. |
| **Pending-upgrade wedge** | An interrupted `helm upgrade` leaves the release `pending-upgrade`, which helm refuses to upgrade over; every later deploy fails generically. | `make deploy` checks release status first and refuses with an explicit fix; workflow step timeouts stay below helm's own `--timeout`. |
| **Diagnostics-allowlist retreat** | The workflow published curated cluster diagnostics into the public log; an allowlist like that only ever grows. | A failed run prints only the dead stage + edge probe status codes. Full output is read operator-side. |
| **`max_user_connections` crash-loop** | The shared MariaDB user defaulted to the operator's low per-user limit (§1). | `maxUserConnections: 100` on `User/insight`. |

## Bootstrap steps that are not a `helm install`

State the release consumes by name, invisible in `helm get manifest` — anyone
rebuilding from the rendered release alone will miss all of it.

- **Databases split across two layers.** The deployment repo creates only the
  MariaDB `insight` Database/User/Grant and the ClickHouse `default`/`insight`
  users. The rest are created **in-server by the umbrella's own hook Jobs**:
  `insight-mariadb-init-svcdbs` (as root, using `insight-db-creds`'s
  `mariadb-root-password`, `CREATE DATABASE` for `identity`+`keycloak` + grant)
  and `insight-clickhouse-init-svcdbs` (`insight`+`presentation`). That is why
  the root password must be in `insight-db-creds` even though no DSN uses it.
  Other hooks: `insight-keycloak-config` (drift-reverting realm apply) and
  `insight-clickhouse-migrate`. Every hook has `ttlSecondsAfterFinished: 600` —
  **capture hook logs immediately on a failure.**
- **Secrets created outside the release** (in `insight`): `insight-db-creds`
  (recomposed each run, no Helm label — §18), `insight-authenticator-signing-keys`
  (ES256, generate-once — a new key logs everyone out), `insight-keycloak-admin`,
  `insight-oidc` (the config Job re-pushes the same client secret each deploy so
  it can't drift), `insight-keycloak-config` (recomposed). Plus the operator
  Secrets in `clickhouse`/`mariadb`/`redis`, and `airbyte-auth-secrets` (Airbyte
  makes it on first boot; warned about, not failed on). A seeded stand needs
  **no GitHub OAuth App**.
- **The realm ConfigMap** `<release>-keycloak-config-realms` holds exactly one
  key (`realm-insight.json`), written `--dry-run | apply` so it prunes the other
  login mode's file. On a seeded stand the realm is **generated, not checked
  in**, then post-processed to add `idp_sub` (§11) + the user profile (§12); the
  seed reads the dev-lead address back from it (keep `.users[].id` and
  `.users[].email` intact). See [One roster, two projections](#one-roster-two-projections).
- **`argo-workflow` SA + Role + RoleBinding in `insight`**, applied by `kubectl`
  **outside** the release (so `helm uninstall` can't strip it from queued
  workflows). The Role needs `create`/`patch` on `workflowtaskresults` — Argo
  3.4+ has each step report through the pod's own SA, and without it steps die
  `forbidden` before the user container starts.
- **Gateway + two-step ClusterIssuer chain**, as manifests: `Gateway/insight`
  (annotation `cert-manager.io/cluster-issuer: insight-ca`, HTTP→HTTPS 301
  redirect) and `insight-selfsigned` → `Certificate/insight-ca` (in the
  cert-manager namespace) → `insight-ca`. The listener TLS Secret is minted by
  the gateway-shim, never created by hand. Self-signed on purpose — **the
  Cloudflare zone must stay on "Full"** ("Full (strict)" rejects the origin
  cert; "Flexible" turns the redirect into a loop).
- **Airbyte instance setup** — `POST /instance_configuration/setup` with
  `initialSetupComplete: true`, from a throwaway in-cluster pod, idempotent.
  Consequence of §7 (auth enabled → boots into a setup wizard).
- **DNS record** — public A record at the edge address, Cloudflare-proxied, SSL
  **Full**. The Insight step is gated on it resolving.
- **Route adoption (migration only)** — a stand deployed before chart 0.5.107
  gets its pre-existing routes labelled for Helm to adopt in place; a fresh
  stand needs nothing.

## One roster, two projections

**The invariant a future change is most likely to break.** The realm and
`identity.persons` are two projections of one roster: the realm decides who can
authenticate, the rows decide who a login resolves to. Disagree → a persona
authenticates and resolves to nobody, every pod Ready, release `deployed`.

- **Only the dev-lead can drift.** Every other persona's address is derived
  deterministically from the roster module; the dev-lead's was the last
  operator-supplied value, which is why it came apart — and why the fix is
  removing that input, not adding a downstream check.
- **The realm is the source of truth.** `seed-stand.sh` reads the dev-lead
  address from the applied realm ConfigMap, keyed on `DEV_LEAD_UUID` (the
  generator writes it as each user's `id`, making the lookup total). Regenerate
  the realm and the seed follows; `--email` overrides for a stand whose realm
  came from elsewhere (and reproduces the failure above if pointed at this one).
- **Nothing below this line checks it.** The seed preflight asserts an address
  is *set*, not that anyone answers to it; the only downstream detector is the
  smoke gate, which `stages` can skip. Sibling failures with the same signature:
  §11 (wrong external-id claim) and §15 (source type disagrees).

## Between deploy and seed

**A seeded stand is not usable between deploy and seed, and that is not a
fault.** Logins authenticate against Keycloak then resolve against
`identity.persons`, so an unseeded stand authenticates a user and immediately
denies them. Two things get misread as breakage: a login test before seeding
fails (correctly — the deploy-repo's validation doesn't run it), and the
`identity-resolution` seed CronJob is *expected* to fail on a fresh stand (it
guards an empty read rather than publishing an empty projection).

Seeding discovers everything from the cluster — datastore hosts, tenant,
identity DB, seeder image, login source type, and the dev-lead address (from
the realm ConfigMap) — so nothing is copied from the deployment repo and
nothing can drift.

## Recreating a stand of this shape

Steps 1–11 are the deployment repository's job; 12–13 are where this repository
takes over.

1. **Provision the cluster** — CNI, ≥3 schedulable workers (hard anti-affinity),
   Cinder CSI. `kubectl get csidriver` before going further.
2. **`cinder` StorageClass, cluster-default.**
3. **Choose per-stand values** — hostname, login mode, tenant UUID, resource
   profile, `auto` edge address for a new stand.
4. **Deploy Envoy Gateway.** Its `https` listener finishes unprogrammed
   (expected). Note + pin the allocated edge address.
5. **Create the DNS record** at that address, Cloudflare-proxied, SSL Full. Do
   it now — the Insight step is gated on it and propagation can't be hurried.
6. **Deploy cert-manager** — creates the issuer chain, completes the Gateway
   cert. Confirm both ClusterIssuers `Ready=True`.
7. **Deploy the four datastores** (clickhouse, mariadb, redis, redpanda). Each
   validate script waits past pod-Ready for what matters (CH user authenticates
   over the pod network; Redis has exactly one `role:master`, replicas
   `master_link_status:up`, and the `redis-master` Service endpoint ready).
8. **Confirm `maxUserConnections: 100`** on `User/insight` (§1) — cheap now,
   expensive later.
9. **Deploy Airbyte** — PVCs, ownership-init Jobs, the one-time setup call.
10. **Deploy Argo** (`crds.install=true`, `crds.keep=true`). Verify
    `singleNamespace: false` and the `instanceID` match (§4).
11. **Run the not-a-helm-install bootstrap** — compose `insight-db-creds`
    (no Helm label), the ES256 key, keycloak-admin + OIDC secrets,
    `insight-keycloak-config`, generate+pack the realm, apply the
    `argo-workflow` SA. Remove any maintenance route (§16).
12. **Hand over to this repository** — the `helm upgrade` in
    [`README.md` step 3](README.md#deploying-by-hand) (the command CI runs),
    then the restart (§25) and route check (§26). Rehearse read-only with
    `make diff ENV=test-stand`. **`make deploy ENV=test-stand` is NOT the deploy
    path here** and would do damage.
13. **Seed** — the stand isn't usable until this runs. Capture the manifest
    from the Job log before its TTL reaps it; treat that JSON as run-internal.

**Teardown is the exact reverse** — insight, argo, airbyte, redpanda, redis,
mariadb, clickhouse, envoy-gateway, cert-manager. Envoy Gateway *after* insight
(routes attach to its Gateway); cert-manager last (certs must be gone before
their issuer, and the shim crashes if the Gateway API CRDs vanish under it). The
edge address survives by design (§24), keeping DNS valid across a rebuild.

## Known drift, and what not to copy

- **Gateway listeners.** The reference manifest defines three (`https`,
  `https-default`, `http`); an older live stand may carry only two. A rebuild
  produces three — an expected difference, changing only what a bare-address
  visitor gets.
- **Retired components in the reference repo** — a Patroni/PostgreSQL triple, an
  operator-managed Keycloak that depended on it, a Dex alternative, an older
  seed mechanism — all still exist as files, none deployed, none in the deploy
  order. Do not copy them into a rebuild.
- **Stale reference prose.** Its README still describes a PostgreSQL layer and a
  six-member Redis *Cluster*; the live shape is no PostgreSQL and a three-member
  Redis *replication* group. Trust the manifests and `helm list -A` over the
  prose.
