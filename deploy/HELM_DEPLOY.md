# Deploying Insight with Helm (single umbrella chart)

This runbook shows a platform or DevOps engineer how to install the Insight business app on an existing Kubernetes cluster using only `helm` and `kubectl` — no GitOps controller, no CI pipeline. You edit a small set of values, secret, and connector files, then apply them directly with tools already on your workstation. This is the opposite of GitOps: instead of a reconciler (like Argo CD) continuously syncing this repo's manifests, you run each command once, in order, and re-run `helm upgrade` whenever something changes.

## Contents

<!-- toc -->

- [Contents](#contents)
- [Overview](#overview)
- [Prerequisites](#prerequisites)
  - [Cluster and CLI tools](#cluster-and-cli-tools)
  - [Cluster-level dependencies](#cluster-level-dependencies)
  - [Running external infrastructure](#running-external-infrastructure)
- [Step 0 — Collect the values Step 1 needs](#step-0--collect-the-values-step-1-needs)
  - [Generate the tenant ID](#generate-the-tenant-id)
  - [Look up the external service addresses](#look-up-the-external-service-addresses)
  - [Find the Airbyte namespace](#find-the-airbyte-namespace)
  - [Compose the Redpanda brokers string](#compose-the-redpanda-brokers-string)
  - [Get the OIDC client details](#get-the-oidc-client-details)
  - [Read the Argo workflow-controller instance ID](#read-the-argo-workflow-controller-instance-id)
- [Step 1 — Configure values/umbrella.yaml](#step-1--configure-valuesumbrellayaml)
- [Step 2 — Fill the secret files](#step-2--fill-the-secret-files)
  - [secrets/insight-db-creds.yaml](#secretsinsight-db-credsyaml)
  - [secrets/insight-authenticator-signing-keys.yaml](#secretsinsight-authenticator-signing-keysyaml)
- [Step 3 — Create the namespace and apply the secrets](#step-3--create-the-namespace-and-apply-the-secrets)
- [Step 4 — Install with Helm](#step-4--install-with-helm)
- [Step 5 — Verify the install](#step-5--verify-the-install)
- [Step 6 — Configure connectors (optional)](#step-6--configure-connectors-optional)
- [Appendix — Reference](#appendix--reference)
  - [values/umbrella.yaml placeholders](#valuesumbrellayaml-placeholders)
  - [secrets/insight-db-creds.yaml keys](#secretsinsight-db-credsyaml-keys)
  - [secrets/insight-authenticator-signing-keys.yaml keys](#secretsinsight-authenticator-signing-keysyaml-keys)

<!-- /toc -->

## Overview

Insight reads engineering and collaboration data from your tools (Jira, Slack, GitHub, and so on), pipelines it through ClickHouse, and serves metrics to a dashboard behind an OIDC (OpenID Connect, the login protocol) login. It installs as five first-party services in one Helm "umbrella" chart — bundled sub-charts, so a single `helm install` deploys everything — published at `oci://ghcr.io/constructorfabric/charts/insight`. The five are:

- **Gateway** (`insight-gateway`, alias `gateway`) — the OpenResty edge. It owns the public ingress and is the single entrance to the cluster: it routes `/*` to the Frontend and `/api/*` to Analytics/Identity, performing a cached cookie-to-JWT exchange against the Authenticator's `/internal/authz` endpoint (a per-pod Lua cosocket lookup, not nginx's `auth_request`) and injecting the resulting gateway JWT into upstream requests.
- **Authenticator** (`insight-authenticator`, alias `authenticator`) — a separate pod that performs the OIDC login with your IdP, keeps Redis-backed sessions, and mints the ES256 gateway JWT the Gateway injects downstream.
- **Analytics** (`insight-analytics`, alias `analytics`) — serves metrics from the ClickHouse Gold layer.
- **Identity Resolution** (`insight-identity-resolution`, alias `identityResolution`) — resolves people and org data from MariaDB. `identityResolution.deploy: true` is the chart default and effectively required: the Authenticator's login-bootstrap person lookup only exists on this service (constructorfabric/insight#1960), so the chart's `insight.validate` render-time check refuses to install with it off.
- **Frontend** (`insight-frontend`, alias `frontend`) — the web UI (dashboard); optional (`frontend.deploy`, default `true`).

Two more subcharts are bundled for local development only, off by default: `keycloak` (dev mode, embedded database, known admin login) and `fakeidp` (a stateless stub). Neither is a stand's IdP — this runbook expects the real one from [Prerequisites](#cluster-level-dependencies).

You supply one values file, secret files, and optionally one Secret per connector (see [deploy/CONNECTORS.md](./CONNECTORS.md)) — no GitOps repo, CI or auto-reconciliation. The data infrastructure is your side of the contract; see [Prerequisites](#running-external-infrastructure).

## Prerequisites

### Cluster and CLI tools

- A Kubernetes cluster you can already reach with `kubectl`, with permission to create namespaces, Secrets, workloads, and Roles/RoleBindings — including in Airbyte's namespace when it differs from the app's, where the chart installs a Role letting its jobs read Airbyte's auth Secret. That namespace must exist before you install.
- `helm` ≥ 3.8 — the chart is pulled as an OCI artifact, and OCI support is stable from 3.8 onward.
- `kubectl`, plus `openssl`, `uuidgen` (or `python3`) and `base64` for the commands in Steps 0–3.

### Cluster-level dependencies

Install all three of these before the chart — it bundles none of them:

- **An ingress controller.** Install ingress-nginx, or point `gateway.ingress.className` at what you run (default `nginx`). The gateway owns the only Ingress: UI at `/`, APIs under `/api/`.
- **A real OIDC identity provider** — Entra ID, Okta, Auth0, or your own. OIDC is mandatory and there is no auth-off switch. **No IdP on the stand? Install Keycloak as its own release**, on a hostname the browser and the authenticator pod resolve identically, then read its issuer, client ID and client secret in Step 0. The bundled `keycloak`/`fakeidp` subcharts are dev-mode servers for local development, not this.
- **cert-manager, plus a `ClusterIssuer` of your own.** The authenticator's TLS-discovery sidecar (`authenticator.tlsDiscovery.enabled`, default `true`) creates a `cert-manager.io/v1` `Certificate`, and Analytics verifies the authenticator's JWKS against that CA — load-bearing, not optional. Point `authenticator.tlsDiscovery.issuerRef.name` at an issuer your cluster actually has: the chart's `local-ca` default exists only in this repo's local k3s sandbox (`deploy/gitops/bootstrap/local/selfsigned-issuer.yaml`). Any issuer works, self-signed included — the certificate is internal-only and unrelated to the ingress certificate in `<TLS_SECRET>`. (Identity Resolution verifies the same way as Analytics.)

Confirm the cluster-side pieces (the IdP gets verified in Step 0, once you have its issuer URL):

```sh
kubectl get ingressclass nginx                  # the className the gateway Ingress uses
kubectl get crd certificates.cert-manager.io    # cert-manager CRDs installed
kubectl get clusterissuer                       # pick one for tlsDiscovery.issuerRef.name
```

### Running external infrastructure

All six systems below must already run and be reachable from the cluster. The chart installs none of them: ClickHouse, MariaDB, Redis and Redpanda are wired in by host/credentials, Airbyte and Argo via `airbyte.namespace` and `ingestion.reconcile.argoInstanceId` — Step 0 reads those off your cluster.

Argo is the exception to "only wired in": the chart installs WorkflowTemplates and CronWorkflows into `insight`, so its CRDs must be present, at >= 3.5 for the plural `schedules:` field. Without Argo, set `ingestion.templates.enabled: false` or the install fails with `no matches for kind "WorkflowTemplate"`.

| System | Used for |
|--------|----------|
| ClickHouse | The Bronze (raw), Silver (conformed) and Gold (query-ready) layers; Analytics serves metrics from Gold |
| MariaDB | The `identity` database Identity Resolution resolves people and org data from |
| Redis | Analytics' cache and the authenticator's session store |
| Redpanda | Kafka-compatible event stream |
| Airbyte | Runs the connectors (Jira, Slack, GitHub, …) that load Bronze |
| Argo Workflows | Runs the dbt transforms Bronze → Silver → Gold, and the Airbyte sync workflows |

Three things on those systems are yours to create:

- **MariaDB: the `insight` database and login.** The pre-install hook creates only Identity Resolution's `identity`, and Analytics' blocking `migrate` initContainer needs this one — without it `--wait` burns its whole timeout and fails.

  ```sql
  CREATE DATABASE IF NOT EXISTS `insight`
    CHARACTER SET utf8mb4 COLLATE utf8mb4_unicode_ci;
  CREATE USER IF NOT EXISTS `insight`@`%` IDENTIFIED BY '<mariadb-password>';
  GRANT ALL ON `insight`.* TO `insight`@`%`;
  ```

- **ClickHouse: the `clickhouse.username` account, holding CREATE DATABASE.** The pre-install hook creates the Bronze/Silver/Gold databases as that user; the chart has no separate admin account.
- **Argo: an `argo-workflow` ServiceAccount in `insight`,** plus a Role granting create/patch on `workflowtaskresults.argoproj.io`. The chart pins every ingestion workflow to it and ships neither, so without them dbt transforms and data-quality checks fail with `serviceaccount "argo-workflow" not found` (connector provisioning still works).

Run every command below from the directory holding your `values/` and `secrets/` files; Steps 1 and 2 give you their full contents.

## Step 0 — Collect the values Step 1 needs

Generate the tenant ID, then read the rest off the cluster. Each dependency may sit in its own namespace, or outside the cluster entirely.

### Generate the tenant ID

A lowercase UUID, used verbatim for both `global.tenantDefaultId` and `ingestion.reconcile.tenantId`, and never changed after the first sync. (Local/dev against the compose wizard, the seed generators or `fakeidp` reuses their fixed `00000000-df51-5b42-9538-d2b56b7ee953`.) `global.tenantDefaultId` is only a fallback — the request tenant comes from the id_token claim named by `authenticator.oidc.tenantClaim`, default `tenant_id` — so if your IdP asserts that claim, give it this same UUID.

```sh
uuidgen | tr '[:upper:]' '[:lower:]'    # no uuidgen? python3 -c 'import uuid; print(uuid.uuid4())'
```

### Look up the external service addresses

Every host is `<svc>.<its-own-namespace>.svc.cluster.local`, or any resolvable host or IP off-cluster. The ClickHouse, MariaDB and Redis ports are already in the skeleton, so you supply only the host.

```sh
kubectl get svc -A | grep -Ei 'clickhouse|mariadb|redis|redpanda|airbyte'
```

### Find the Airbyte namespace

Set `airbyte.namespace` to the namespace the Airbyte release runs in. It drives both the computed API URL and the namespace the jobs read `airbyte-auth-secrets` from, so `airbyte.apiUrl` stays empty unless your Airbyte sits behind a non-standard URL.

```sh
kubectl get svc -A | grep airbyte-server
  # the namespace in that row is airbyte.namespace
  # computed URL: http://<releaseName>-airbyte-server-svc.<that-namespace>.svc.cluster.local:8001
```

### Compose the Redpanda brokers string

`redpanda.brokers` takes one comma-separated `host:port` bootstrap string, not the host/port pair the other datastores take, aimed at the internal Kafka API listener. Read the port rather than assuming `9093` — that is the `redpanda/redpanda` chart's default, while this repo's compose stack uses `9092`.

```sh
kubectl -n <redpanda-ns> get svc <redpanda-svc> -o jsonpath='{range .spec.ports[*]}{.name}={.port}{"\n"}{end}'
  # compose <redpanda-svc>.<redpanda-ns>.svc.cluster.local:<kafka port>
  # e.g. redpanda.insight-infra.svc.cluster.local:9093
```

### Get the OIDC client details

In the IdP from Prerequisites, register a **confidential** client with redirect URI `https://<HOST>/auth/callback` and collect its issuer URL, client ID and client secret. On Keycloak the issuer is `<base-url>/realms/<realm>` and the secret is on the client's *Credentials* tab.

```sh
# the issuer must be the SAME URL the browser and the authenticator pod resolve, or `iss` won't validate
kubectl run oidc-probe --rm -i --restart=Never --image=curlimages/curl -- \
  curl -sS <OIDC_ISSUER>/.well-known/openid-configuration | head -c 200
```

### Read the Argo workflow-controller instance ID

Set `ingestion.reconcile.argoInstanceId` to the controller's configured `instanceID` only when it is pinned to one; no match means leave it empty — the common case, where the reconcile workflows go unlabelled and any controller accepts them.

```sh
kubectl -n <argo-ns> get cm | grep workflow-controller     # name varies by chart version
kubectl -n <argo-ns> get cm <cm> -o jsonpath='{.data.config}' | grep -i instanceID   # newer charts nest it under `config:`
kubectl -n <argo-ns> get cm <cm> -o jsonpath='{.data.instanceID}{"\n"}'              # older ones use a top-level key
```

## Step 1 — Configure values/umbrella.yaml

Create `values/umbrella.yaml` from the skeleton below and replace every `<...>` placeholder. No passwords here — they go in the Step 2 secret files.

```yaml
## values/umbrella.yaml — the only values file you need.
## Fill every <...> placeholder. Passwords are NEVER here — see the secret files.
credentials:
  deploymentMode: helm               # helm | gitops (gitops forbids autoGenerate:true)
  autoGenerate: true                 # BYO compose; won't overwrite a labelless insight-db-creds

global:
  tenantDefaultId: "<TENANT_ID>"     # the UUID from Step 0; must equal ingestion.reconcile.tenantId
  # imagePullSecrets: []             # [{name: my-regcred}] for a private registry

# Datastore wiring — every dep is external; the chart only dials it.
clickhouse:
  host: <CLICKHOUSE_HOST>            # e.g. clickhouse.<its-ns>.svc.cluster.local
  port: 8123               # reachable from the insight namespace
  database: insight
  username: insight
mariadb:
  host: <MARIADB_HOST>
  port: 3306
  database: insight
  username: insight
redis:
  host: <REDIS_HOST>
  port: 6379
redpanda:
  brokers: "<REDPANDA_BROKERS>"      # e.g. redpanda.<its-ns>.svc.cluster.local:9093

# Ingestion — point at existing Airbyte + Argo; install the dbt WorkflowTemplates.
ingestion:
  templates:
    enabled: true
  reconcile:
    tenantId: "<TENANT_ID>"
    destinationName: clickhouse-bronze
    argoInstanceId: "<ARGO_INSTANCE_ID>"     # match the controller's instanceID (Step 0); leave "" if unpinned
airbyte:
  namespace: "<AIRBYTE_NAMESPACE>"   # where the Airbyte release runs; "" = the app namespace
  apiUrl: ""                         # "" = computed from releaseName + namespace; set only for a non-standard URL

analytics:
  replicaCount: 1                    # chart default 2; bump for HA
  resources:
    requests: { cpu: 100m, memory: 128Mi }
    limits:   { cpu: 500m, memory: 512Mi }

gateway:
  replicaCount: 1
  ingress:                           # the only Ingress the chart publishes; the gateway
    enabled: true                    # itself routes / to the UI and /api/* to Analytics
    className: nginx                 # and Identity, from subchart defaults you need not set
    host: <HOST>
    tls:
      enabled: true
      secretName: <TLS_SECRET>
  resources:
    requests: { cpu: 100m, memory: 128Mi }
    limits:   { cpu: 500m, memory: 256Mi }

authenticator:
  replicaCount: 1
  # ES256 signing keys — see Step 2. MUST already exist as a Secret before install.
  signingKeysSecret: "insight-authenticator-signing-keys"
  # cert-manager Certificate for the JWKS-discovery sidecar (internal TLS only).
  tlsDiscovery:
    enabled: true
    issuerRef:
      name: <CLUSTER_ISSUER>          # your cluster's ClusterIssuer; the chart's `local-ca`
                                      # default only exists in the local k3s sandbox
      kind: ClusterIssuer             # or Issuer, if yours is namespaced into `insight`
  oidc:
    issuerUrl: "<OIDC_ISSUER>"        # MUST be set — your IdP's issuer URL
    clientId: "<OIDC_CLIENT_ID>"
    clientSecret: "<OIDC_CLIENT_SECRET>"
    redirectUri: "https://<HOST>/auth/callback"   # MUST be set — browser-facing callback through the gateway
    scopes: ["openid", "profile", "email", "offline_access"]
    sourceType: "<IDP_SOURCE_TYPE>"   # MUST be set — the identity-resolution source_type
                                      # your IdP's connector seeds `persons` under (e.g. "ms-entra").
                                      # Scopes the login-bootstrap resolve (constructorfabric/insight#1960).
    externalIdClaim: "sub"           # id_token claim carrying your IdP's stable external user id
                                      # for sourceType. Default "sub" is correct when `sub` itself is
                                      # that stable id; Entra needs "oid" instead (its `sub` is
                                      # pairwise-unique per client, NOT the directory-stable id).
  # csrfOrigins: ["https://<HOST>"]  # fail-closed by default: if the UI's POST /auth/logout,
                                     # /auth/refresh or DELETE /auth/sessions return 403, set this

identityResolution:
  deploy: true                       # chart default — the authenticator's login-bootstrap resolve
                                     # only exists here (constructorfabric/insight#1960)
  replicaCount: 1
  databaseName: "identity"
  resources:
    requests: { cpu: 100m, memory: 128Mi }
    limits:   { cpu: 500m, memory: 512Mi }

frontend:                            # the web UI (dashboard)
  deploy: true                       # served through the gateway at / — no ingress of its own
  replicaCount: 1
```

To start from the chart's full defaults instead of typing the skeleton:

```sh
helm show values oci://ghcr.io/constructorfabric/charts/insight > values/umbrella.yaml
```

Fill each placeholder:

| Placeholder | What it should be |
|-------------|--------------------|
| `<TENANT_ID>` | The tenant UUID you generated in Step 0. Must be the same value in `global.tenantDefaultId` and `ingestion.reconcile.tenantId` |
| `<CLICKHOUSE_HOST>` | ClickHouse host only — no port, no scheme; the skeleton's `port: 8123` supplies the port |
| `<MARIADB_HOST>` | MariaDB host only — no port; the skeleton's `port: 3306` supplies it |
| `<REDIS_HOST>` | Redis host only — no port; the skeleton's `port: 6379` supplies it |
| `<REDPANDA_BROKERS>` | The bootstrap string you composed in Step 0 — comma-separated `host:port` pointing at the internal Kafka API listener |
| `<AIRBYTE_NAMESPACE>` | Namespace of the Airbyte release from Step 0, for example `insight-infra`. Leave `""` if Airbyte shares the app namespace |
| `<ARGO_INSTANCE_ID>` | The `instanceID` your Argo workflow controller is pinned to — read it off the controller config map in Step 0. Leave empty (`""`) if the controller is unpinned, the common case |
| `<HOST>` | Public FQDN for the gateway ingress — the single entrance for both the UI and the APIs, for example `insight.example.com` |
| `<TLS_SECRET>` | TLS Secret for that domain. The chart only references it: pre-create it, or add `gateway.ingress.annotations: {cert-manager.io/cluster-issuer: <issuer>}`. Missing, ingress-nginx quietly serves its own fake certificate |
| `<CLUSTER_ISSUER>` | A cert-manager `ClusterIssuer` in your cluster, for the authenticator's internal JWKS certificate. Self-signed is fine; the chart's `local-ca` default exists only in this repo's local sandbox |
| `<OIDC_ISSUER>` | Your IdP's issuer URL. Its `/.well-known/openid-configuration` document must resolve from inside the cluster |
| `<OIDC_CLIENT_ID>` / `<OIDC_CLIENT_SECRET>` | Your OIDC client / application registration credentials |
| `<IDP_SOURCE_TYPE>` | The identity-resolution `insight_source_type` your IdP's connector seeds `persons` under (e.g. `ms-entra`) — required, scopes the login-bootstrap resolve |

For infrastructure in the same cluster, use `<service>.<namespace>.svc.cluster.local`. Any resolvable host or IP also works.

Check these before installing:

- `identityResolution.deploy: true` is the chart default — leave it alone unless you have a specific reason to disable it. This is what the authenticator's login-bootstrap resolve actually depends on; `insight.validate` refuses to render without it.
- Set real values for `authenticator.oidc.issuerUrl`, `redirectUri`, and `sourceType`. The chart wraps all three in Helm's `required`, and there is no auth-off switch.
- Create the Secret named in `authenticator.signingKeysSecret` before installing (Step 2). The chart does not generate it.
- Point the OIDC fields at the real IdP from Prerequisites. The bundled `keycloak`/`fakeidp` subcharts are dev-mode servers for local development, not a stand's IdP.

## Step 2 — Fill the secret files

### secrets/insight-db-creds.yaml

Create this Secret with all four datastore passwords — the chart fails fast if any key is missing. Use the passwords your datastores already run with, but check them first: the chart composes DSNs by string interpolation and rejects any password containing `@ : / ? # %`, so those must be rotated to `[A-Za-z0-9._~-]` before you install.

```yaml
apiVersion: v1
kind: Secret
metadata: { name: insight-db-creds, namespace: insight }
type: Opaque
stringData:
  clickhouse-password:   "CHANGE_ME"   # password of clickhouse.username -> Analytics + hooks
  mariadb-password:      "CHANGE_ME"   # MariaDB app-user password    -> Analytics + Identity
  mariadb-root-password: "CHANGE_ME"   # password of the account literally named `root` -> identity-DB init hook
  redis-password:        "CHANGE_ME"   # Redis password               -> Analytics + Authenticator
```

If those passwords already live in Secrets in your infrastructure namespace, copy them across instead of retyping them:

```sh
# each password lives in a Secret in its own datastore's namespace — they need not be the same namespace
kubectl -n <clickhouse-ns> get secret <ch-secret>         -o jsonpath='{.data.<ch-key>}'    | base64 -d; echo   # clickhouse-password
kubectl -n <mariadb-ns>    get secret <maria-secret>      -o jsonpath='{.data.<app-key>}'   | base64 -d; echo   # mariadb-password (app user)
kubectl -n <mariadb-ns>    get secret <maria-root-secret> -o jsonpath='{.data.<root-key>}'  | base64 -d; echo   # mariadb-root-password
kubectl -n <redis-ns>      get secret <redis-secret>      -o jsonpath='{.data.<redis-key>}' | base64 -d; echo   # redis-password
```

Paste each decoded value into the matching field.

> **Never label this Secret `app.kubernetes.io/managed-by: Helm`.** The chart reads the label's *absence* as "bring your own" and composes `insight-analytics-config` and `insight-identity-resolution-config` from your values; with the label Helm claims ownership of a Secret it did not create and the install aborts with `invalid ownership metadata`.

### secrets/insight-authenticator-signing-keys.yaml

Generate the authenticator's ES256 (EC P-256) gateway-JWT key as PKCS#8 and create the Secret — the chart does not generate it:

```sh
openssl ecparam -name prime256v1 -genkey -noout | openssl pkcs8 -topk8 -nocrypt -out current.pem
kubectl create secret generic insight-authenticator-signing-keys \
  --namespace insight --from-file=current.pem \
  --dry-run=client -o yaml > secrets/insight-authenticator-signing-keys.yaml
```

## Step 3 — Create the namespace and apply the secrets

```sh
kubectl create namespace insight
kubectl -n insight apply -f secrets/

# verify
kubectl -n insight get secret insight-db-creds insight-authenticator-signing-keys   # expect 4 keys / 1 key (current.pem)
```

Do **not** copy Airbyte's own `airbyte-auth-secrets` into `insight`. The reconcile loop and the airbyte-sync workflows read it from Airbyte's namespace at run time — that is what `airbyte.namespace` in Step 1 is for, and the chart renders a Role/RoleBinding there granting `get` on that one Secret. A copy would freeze credentials Airbyte regenerates on reinstall.

## Step 4 — Install with Helm

Install the umbrella chart against your values file:

```sh
helm upgrade --install insight oci://ghcr.io/constructorfabric/charts/insight \
  -n insight -f values/umbrella.yaml --wait --timeout 15m
```

- Add `--version <x.y.z>` to pin a chart release; omit it for the latest published one.
- `--wait --timeout 15m` blocks until every resource is ready, giving a pass/fail signal instead of a detached rollout.
- The install also runs the `insight-clickhouse-migrate` hook Job. It creates the staging/silver/app databases, seeds bronze placeholders, ALTERs bronze/silver tables, applies `src/ingestion/scripts/migrations/*.sql`, then rebuilds the gold models with dbt — that last part dominates on a cluster with real data, so raise `--timeout` if it runs close. It fires on **every** upgrade (gated by `clickhouse.runMigrations`, default `true`) and a failure fails the upgrade. Because it drops and recreates every gold object each run, a failure points at Bronze/Silver schema or data, not a stale-object conflict.
- If `--wait` stalls with the authenticator and analytics in `ContainerCreating`, the cert-manager Certificate `insight-authenticator-authn-tls` has not issued — `--wait` does not wait on Certificates. Check `kubectl -n insight describe certificate insight-authenticator-authn-tls`.

## Step 5 — Verify the install

Run all four checks:

```sh
kubectl -n insight get pods
  # expect insight-gateway, -authenticator, -analytics, -identity-resolution, -frontend all Running
  # (fakeidp/keycloak only appear with their own deploy flag)

kubectl -n insight get secret insight-analytics-config insight-authenticator-config insight-identity-resolution-config
  # the chart composes these from insight-db-creds

helm -n insight history insight
  # the ClickHouse migration runs as a post-install/post-upgrade hook Job; Helm deletes it
  # on success (hook-delete-policy: hook-succeeded), so "no jobs found" is the healthy
  # state — Step 4 exiting 0 is the pass signal. On failure the Job survives:
  #   kubectl -n insight logs job/insight-clickhouse-migrate

kubectl -n insight get cronworkflow
  # expect two: insight-reconcile-loop (provisions Airbyte sources/connections)
  # and insight-data-quality (ingestion.dataQuality.enabled, default true)
```

Then open `https://<HOST>` — the host from Step 1 — and confirm the login redirect to your OIDC provider.

## Step 6 — Configure connectors (optional)

Configure connectors after the app is up. Each of the 25 connectors is a single Kubernetes Secret; the `insight-reconcile-loop` CronWorkflow discovers it and provisions the Airbyte source automatically, so there is nothing else to run.

See [deploy/CONNECTORS.md](./CONNECTORS.md) for the connector list and a copy-paste Secret for each.

## Appendix — Reference

### values/umbrella.yaml placeholders

| Placeholder | Field(s) | Notes |
|-------------|----------|-------|
| `<TENANT_ID>` | `global.tenantDefaultId`, `ingestion.reconcile.tenantId` | Generated in Step 0; a lowercase UUID, identical across both |
| `<CLICKHOUSE_HOST>` | `clickhouse.host` | Always external; port fixed at `8123` in the file |
| `<MARIADB_HOST>` | `mariadb.host` | Always external; port fixed at `3306` |
| `<REDIS_HOST>` | `redis.host` | Always external; port fixed at `6379` |
| `<REDPANDA_BROKERS>` | `redpanda.brokers` | Always external; a single comma-separated `host:port` string, not a host/port pair. `9093` for the `redpanda/redpanda` chart's internal listener — read yours in Step 0 |
| `<AIRBYTE_NAMESPACE>` | `airbyte.namespace` | Namespace of the Airbyte release; `""` = app namespace. Drives the computed `apiUrl` and where the jobs read `airbyte-auth-secrets` |
| `<ARGO_INSTANCE_ID>` | `ingestion.reconcile.argoInstanceId` | Match the controller's configured `instanceID` (Step 0); empty if unpinned |
| `<HOST>` | `gateway.ingress.host` | Public FQDN on the gateway's Ingress, the only one the chart publishes (`/` → UI, `/api/*` → Analytics/Identity) |
| `<TLS_SECRET>` | `gateway.ingress.tls.secretName` | Kubernetes TLS Secret name; referenced only — the chart never creates it |
| `<CLUSTER_ISSUER>` | `authenticator.tlsDiscovery.issuerRef.name` | A cert-manager `ClusterIssuer` that exists in your cluster; internal cert, so self-signed is fine |
| `<OIDC_ISSUER>` | `authenticator.oidc.issuerUrl` | Your IdP's issuer URL |
| `<OIDC_CLIENT_ID>` / `<OIDC_CLIENT_SECRET>` | `authenticator.oidc.clientId`/`clientSecret` | Your OIDC client / application registration credentials. The authenticator is the only OIDC client — the frontend does not register one |
| `<IDP_SOURCE_TYPE>` | `authenticator.oidc.sourceType` | The identity-resolution source_type your IdP's connector seeds `persons` under; required, no default |

Other notable (non-placeholder) settings in this file:

- Image tags are omitted deliberately. Each subchart renders `image.tag | default .Chart.AppVersion`, so a chart release already carries a tested set of product images. Set `<service>.image.tag` only to pin one service to a different build.
- `credentials.deploymentMode: helm` and `credentials.autoGenerate: true` — this enables the "bring your own" credentials path, where the chart keeps a labelless `insight-db-creds` Secret instead of generating random passwords.
- `identityResolution.deploy: true` — the chart default; don't flip it off.
- `authenticator.tlsDiscovery.issuerRef.name` — the cert-manager `ClusterIssuer` the JWKS-discovery Certificate is issued from. Always set this: the chart ships `local-ca`, which is the self-signed root that `make bootstrap-cert-manager ENV=local` creates for the local k3s sandbox, not anything a real cluster has.
- There is no auth-off toggle anywhere in this chart. `authenticator.oidc.issuerUrl` and `authenticator.oidc.redirectUri` are hard `required` fields, so a real IdP is a prerequisite; install Keycloak as a separate release if the stand has none. The bundled `keycloak`/`fakeidp` subcharts are local-development servers (embedded database, known passwords) and not a substitute.

### secrets/insight-db-creds.yaml keys

| Key | Meaning | Consumed by |
|-----|---------|--------------|
| `clickhouse-password` | Password of the `clickhouse.username` account | Analytics, Identity, and the init/migrate hooks |
| `mariadb-password` | MariaDB app-user password | Analytics + Identity |
| `mariadb-root-password` | Password of the account literally named `root` — the hook runs `mariadb -uroot` and no values key renames it. A wrong one surfaces as `MariaDB did not become reachable within 2 minutes`, not as an auth error | identity-DB init hook |
| `redis-password` | Redis password | Analytics + Authenticator |

Recall: this Secret must never carry an `app.kubernetes.io/managed-by: Helm` label.

### secrets/insight-authenticator-signing-keys.yaml keys

One required key, `current.pem`: the active ES256 (EC P-256) signing key, unencrypted PKCS#8 PEM.
