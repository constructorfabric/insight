# `test-stand` — the published test stand

The gitops environment for **insight-test.cfabric.org**: the cluster that CI
upgrades to the umbrella chart it has just published, then seeds and
smoke-tests, on every merge to `main`
([constructorfabric/insight#2244](https://github.com/constructorfabric/insight/issues/2244)).

Its whole job is to answer one question automatically: *does the chart we
just published actually install, hold data, and let a person log in and see
that data?*

> **Read this before touching anything here.** This environment is shaped
> differently from every other environment in `deploy/gitops/`. The usual
> `make bootstrap` → `make system-*` → `make deploy` sequence is **not** the
> deploy path for this stand, and two of those three targets would damage it.
> The [Why not `make deploy`](#why-not-make-deploy) section is not optional
> reading.

## Contents

```text
environments/test-stand/
├── README.md                        # this file
├── inventory.yaml                   # cluster address + "this env manages the umbrella only"
├── values.yaml                      # the umbrella overlay — the file the deploy passes to helm
└── manifests/
    ├── httproute.yaml               # public hostname -> insight-gateway (the chart renders none)
    └── httproute-keycloak.yaml      # /kc on the same hostname -> the bundled Keycloak
```

There is deliberately no `sealed-secrets/` directory, no `keycloak/realms/`
directory and no `<svc>-values.yaml` here. Each absence is a decision, and
each one is explained below.

## The ownership boundary

This tree now owns the **stand's application configuration**. It does not own
the cluster.

| Layer | What | Owned by | Changed how |
|---|---|---|---|
| L0 | Cluster prereqs: cert-manager and its ClusterIssuer, Envoy Gateway and the `Gateway` object, the namespaces themselves | the deployment repository (outside this repo) | a human, deliberately |
| L2 | Datastores — ClickHouse, MariaDB, Redis, Redpanda — each under its own operator in its own namespace; plus Airbyte and Argo Workflows | the deployment repository | a human, deliberately |
| L2 | Generate-once Secrets: `insight-db-creds`, `insight-authenticator-signing-keys`, `insight-oidc`, `insight-keycloak-admin`, `insight-keycloak-config` | the deployment repository | a human, deliberately |
| L2 | The Keycloak realm content (the `insight-keycloak-config-realms` ConfigMap) | the deployment repository | a human, deliberately |
| **L3** | **The umbrella Helm release `insight` in namespace `insight` — every value in `values.yaml`** | **this directory** | **CI, on every merge to `main`** |
| L3 | The two `HTTPRoute`s in `manifests/` | source of truth here; applied by the deployment repository | a human, from the files here |
| L3 | The `argo-workflow` ServiceAccount + Role + RoleBinding that the chart's WorkflowTemplates pin but do not create | the deployment repository | a human, deliberately |

Two consequences worth stating plainly:

* **Nothing in this directory creates or rotates a credential.** Every Secret
  the release consumes already exists, was generated once, and is referenced
  by name. That is why `inventory.yaml` lists all of them with
  `enabled: false` — see the long comment there for what each one would break
  if it were re-materialised.
* **A change merged here reaches a published stand at merge speed.** There is
  no staging step between this file and the cluster. Review accordingly.

### What this env does not own, but depends on

Four objects live outside the Helm release and outside this directory. If any
of them disappears, the release still installs and reports `deployed`, and the
stand is broken anyway:

| Object | Symptom if missing |
|---|---|
| `HTTPRoute/insight-gateway` | the public URL answers nothing; every smoke check fails at the first request |
| `HTTPRoute/insight-keycloak` | `/kc` is unreachable, so OIDC discovery fails and nobody can log in |
| `ServiceAccount/argo-workflow` (+ its Role/RoleBinding) | every scheduled transform and data-quality run fails with `serviceaccount "argo-workflow" not found`, while all app pods stay healthy — nothing surfaces until a scheduled run |
| The Secrets in the table above | services fail to start, or start with blank configuration |

The two routes are committed here (`manifests/`) because the acceptance
criteria travel through them. They are **verified, not applied**, by the
deploy — see the header comment in `manifests/httproute.yaml` for why re-applying
would create two writers on one object. The Argo RBAC is not copied here: it
is Argo plumbing rather than application configuration, and it sits outside the
deploy/seed/smoke scope this environment was created for.

## Deploying by hand

Everything below is read-only until the `helm upgrade` line, and each step is
worth running on its own the first time.

**0. Point at the stand.** The `kubeContext` in `inventory.yaml` is an
assertion, not a lookup — `make` refuses to act unless
`kubectl config current-context` equals it. Either rename your context to
match, or pass `KUBE_CTX=` on every make command line:

```bash
export KUBECONFIG=/path/to/your/test-stand.kubeconfig     # outside this repo's working tree
kubectl config rename-context <your-context> insight-test-stand
kubectl config use-context insight-test-stand
```

Keep the kubeconfig **outside the repository working tree**. The Makefile's
`sync-clean` prerequisite fails on any dirty file, so a kubeconfig written next
to these files blocks every make target that touches the cluster.

**1. Confirm you are on the right cluster.** The context-name check above
proves only that a file says so. Assert the API server as well — this is the
one mechanical guard against an upgrade landing somewhere it should not:

```bash
kubectl config view --minify -o jsonpath='{.clusters[0].cluster.server}'
```

Compare it against the address recorded for this stand in the deployment
repository. It is deliberately not written down here: this repository is
public, and a cluster API endpoint is not something to publish.

**2. See what would change.** Read-only, and it renders through the same
values file the upgrade will use:

```bash
make diff ENV=test-stand INSIGHT_VERSION=<chart-version>
```

`make diff` works for this env unchanged. It renders offline and never
contacts the cluster, so it needs no context; its prerequisites are
`sync-clean`, `values-present` and `chart-present` — none of which trips over
the sealed-secrets problem described below. `sync-clean` is why step 0 insists
the kubeconfig lives outside the working tree.

**3. Upgrade.** Called directly, not through `make deploy`:

```bash
helm upgrade --install insight \
  oci://ghcr.io/constructorfabric/charts/insight \
  --version "<chart-version>" \
  --namespace insight \
  --values deploy/gitops/environments/test-stand/values.yaml \
  --set-string authenticator.oidc.clientSecret="$(kubectl -n insight \
      get secret insight-oidc -o jsonpath='{.data.client-secret}' | base64 -d)" \
  --wait --timeout 10m --history-max 10
```

Three things about that command are load-bearing:

* **`--set-string authenticator.oidc.clientSecret=…` is mandatory.** The
  values file leaves it empty because this repository is public. The chart
  writes whatever it is given straight into the authenticator's config Secret,
  so an upgrade without this flag produces a confidential OIDC client with a
  blank secret: pods Ready, release `deployed`, every login broken. Read it
  from the cluster Secret, as above, so there is one source of truth rather
  than a second copy to rotate.
* **`--wait` but deliberately no `--atomic`.** A failed upgrade is left in
  place. Rolling back automatically destroys the evidence of *why* it failed,
  and this stand exists to produce that evidence. Recovery is the next merge,
  or a human running `make rollback` (below).
* **`--timeout 10m`**, not the Makefile's 30m default. A deploy that is going
  to fail should say so inside the CI budget.

**4. Restart what the chart cannot know to restart.** Each subchart's
`checksum/config` annotation hashes its own ConfigMap. It does **not** cover
the umbrella-rendered `insight-*-config` Secrets, which the pods consume with
`envFrom` and therefore read exactly once, at container start. So a changed
datastore host, tenant, or client secret updates the Secret, leaves the pod
spec identical, and never reaches a running process — a `deployed` release
with all-Ready pods running stale configuration:

```bash
kubectl -n insight rollout restart \
  deploy/insight-authenticator deploy/insight-analytics deploy/insight-identity-resolution
kubectl -n insight rollout status --timeout=5m \
  deploy/insight-authenticator deploy/insight-analytics deploy/insight-identity-resolution
```

All three are listed on purpose. Each of the three consumes its config Secret
with `envFrom`, so all three are subject to the same staleness — a restart
list of two would leave one service holding old configuration.

**5. Check the edge is still routing.** The chart renders no route, so a
successful upgrade tells you nothing about whether the stand is reachable:

```bash
kubectl -n insight get httproute insight-gateway insight-keycloak \
  -o custom-columns='NAME:.metadata.name,ACCEPTED:.status.parents[0].conditions[?(@.type=="Accepted")].status'
```

**6. Seed and smoke.** Seeding reuses the seeder verbatim — no test-stand
variant, no per-stand flags beyond the ones below (it discovers the tenant,
the datastore coordinates and the IdP source type from the cluster itself):

```bash
src/ingestion/tools/seed/seed-stand.sh -n insight --email <address> --days 730
```

The seeder's manifest — the list of personas, fixtures, the tenant and the
data window that the smoke suite reads — is **printed to the seed Job's
stdout** and written to a path inside a pod whose filesystem is discarded.
Capture it from the Job log before the Job's TTL reaps it. Treat that JSON as
run-internal: it carries persona addresses, UUIDs and in-cluster service URLs,
so it must not become a CI artifact on a public repository.

### Rollback

```bash
make status   ENV=test-stand
make rollback ENV=test-stand
```

Both work for this env unchanged (they take the context from
`inventory.yaml` and assert it against your current one, so step 0 is a
prerequisite for both). `rollback` is a human action by design: an
automated rollback on failure is exactly what `--atomic` would have done, and
what step 3 deliberately does not do.

## Why not `make deploy`

`make deploy ENV=test-stand` does not work against this stand, and would not
be the right tool even if it did. Three separate reasons, in the order you
would hit them:

1. **It aborts on the sealed-secrets prerequisite.** `deploy-insight` depends
   on `apply-app-secrets`, which requires at least one
   `*-sealedsecret.yaml` under `environments/<ENV>/sealed-secrets/insight/`
   and `kubectl apply`s every file it finds. This env has none, on purpose,
   and the cluster has neither the sealed-secrets controller nor the
   `SealedSecret` CRD. There is no documented skip switch.

2. **If that hurdle were removed, the target would then do damage.**
   `apply-app-secrets` goes on to run `compose-app-secrets.sh`, which
   overwrites the three `insight-*-config` Secrets with locally composed
   content and reads the OIDC client secret from a key name this stand does
   not use — writing a blank client secret into the authenticator's config.
   In `helm` credentials mode the same-run `helm upgrade` rewrites them back,
   so the visible result is churn plus a broken-login window; if the helm step
   fails, the broken state is what is left. Sealing `insight-db-creds` into
   this directory would be worse still: the controller would overwrite the
   live Secret composed from the operators' own credentials, and every service
   would lose its datastore login.

3. **Its exit status cannot gate anything.** The deploy recipe is a single
   backslash-continued shell line whose last command ends in `|| true`, and
   the helm call is piped into `tee` with no `pipefail`. A failed upgrade
   reports success. Acceptance criterion (2) — the workflow result is gated by
   the deploy, seed and smoke — cannot be built on that. `--atomic` is also
   hard-coded with no override, which contradicts the leave-it-failed
   disposition in step 3 above.

Making `make deploy` usable here means changing the Makefile: an optional
sealed-manifest glob, `set -o pipefail` plus a status-bearing final command,
and a way to opt out of `--atomic`. That is worth doing, and it is a separate
change from introducing this environment. Until then, the `helm upgrade` in
step 3 **is** the deploy path, and CI runs that same command.

## What CI does

The deploy runs as a reusable workflow called from the chart-publishing
workflow's final job, on `main` only, with the chart version passed in from
the publish job's output rather than read from `.insight-version` (which is
only committed at the very end of publishing, so a checkout of the trigger
commit would read the previous version).

* Credentials come from the `insight-test-stand` GitHub environment,
  restricted to `main`. The CI credential is a namespace-scoped
  ServiceAccount in `insight` — admin kubeconfigs stay human-only, and no
  credential for this stand lives in the repository.
* Runs are coalesced, never cancelled: an upgrade already in flight is allowed
  to finish.
* Three named stages — deploy, seed, smoke — and smoke never runs after a
  failed seed.
* On failure the run publishes a curated, redacted set of diagnostics only.
  This repository is public and so are its run logs, so there are no
  `describe` dumps, no environment dumps and no log artifacts.
* A red run belongs to the author of the merge that produced it. There is no
  freeze: fix forward, or revert.

## Known gaps

* **Scripted login.** The stand's realm federates login to an external OAuth
  provider and has no local password users, so a username+password login
  cannot be scripted against it as configured. Resolving that is a decision
  about who owns the realm and what credential CI is allowed to hold — see the
  `keycloakConfig` comment in `values.yaml` for the ownership half of it.
* **`authenticator.overrideEnabled: true`** is carried forward from the
  installed release. It is a standing impersonation primitive on an
  internet-reachable stand, gated on that flag alone. Tracked separately;
  see the comment on the key in `values.yaml`.
* **The identity CronJobs** can un-seed logins once their input table stops
  being empty. See the comment at the bottom of `values.yaml`.
