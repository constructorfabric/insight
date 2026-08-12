# `test-stand` — the published test stand

The gitops environment for **insight-test.cfabric.org**: the cluster CI upgrades
to the umbrella chart it just published, then seeds and smoke-tests, on every
merge to `main`
([#2244](https://github.com/constructorfabric/insight/issues/2244)). Its whole
job is to answer one question automatically — *does the chart we just published
install, hold data, and let a person log in and see that data?*

> **Read before touching anything here.** This environment is shaped unlike
> every other in `deploy/gitops/`. `make deploy` **is** the deploy path (CI runs
> exactly it), but `make bootstrap` and `make system-*` are **not** — either
> would install a second, competing copy of something an operator already owns.
> A rollback is not how this stand is fixed; see [Recovery](#recovery).

```text
environments/test-stand/
├── README.md                 # this file — how to change the stand that exists
├── INFRA.md                  # what must exist BENEATH the umbrella, and in what order
├── credentials-runbook.md    # provisioning/rotating the CI deploy credential
├── inventory.yaml            # cluster address + "this env manages the umbrella only"
└── values.yaml               # the umbrella overlay passed to helm
```

There is deliberately no `sealed-secrets/`, no `keycloak/realms/`, no
`<svc>-values.yaml` and — since the chart grew native Gateway API templates — no
`manifests/`. Each absence is a decision explained below or in `INFRA.md`.

## The ownership boundary

This tree owns the **stand's application config**, not the cluster.

| Layer | What | Owned by | Changed how |
|---|---|---|---|
| L0 | cert-manager + ClusterIssuer, Envoy Gateway + the `Gateway`, the namespaces | the deployment repo | a human |
| L2 | Datastores (ClickHouse, MariaDB, Redis, Redpanda) under their operators; Airbyte; Argo | the deployment repo | a human |
| L2 | Generate-once Secrets (`insight-db-creds`, `insight-oidc`, `insight-keycloak-*`, `insight-authenticator-signing-keys`) and the realm ConfigMap | the deployment repo | a human |
| **L3** | **The umbrella release `insight` — every value in `values.yaml`, plus the two `HTTPRoute`s it renders** | **this directory** | **CI, every merge to `main`** |
| L3 | The `argo-workflow` SA + Role + RoleBinding the chart's WorkflowTemplates pin | the deployment repo | a human |

- **Nothing here creates or rotates a credential** — every Secret already
  exists and is referenced by name (hence `inventory.yaml` lists them
  `enabled: false`). The one exception, the CI deploy credential, has its own
  [`credentials-runbook.md`](credentials-runbook.md).
- **A change merged here reaches a published stand at merge speed.** No staging
  step. Review accordingly.

### What it depends on but does not own

Five things live outside the release; if any disappears the release still
reports `deployed` and the stand is broken anyway:

| Object | Symptom if missing |
|---|---|
| `Gateway/insight` (what both routes `parentRef`) | routes never attach (`Accepted` false), public URL answers nothing, every smoke check fails at the first request |
| `ClusterIssuer/insight-ca` (`authenticator.tlsDiscovery.issuerRef`) | the authn-TLS cert never issues; analytics + identity-resolution sit `ContainerCreating` until `--wait` times out, naming no issuer |
| `ConfigMap/insight-keycloak-config-realms` | the keycloak-config hook fails the upgrade; the seed stops naming `--email`. Present-but-wrong (lost `idp_sub` mapper) is worse: every persona authenticates then is denied |
| `ServiceAccount/argo-workflow` (+ RBAC) | every scheduled transform fails `serviceaccount "argo-workflow" not found`; app pods stay healthy, nothing surfaces until a scheduled run |
| The generate-once Secrets | services fail to start, or start blank-configured |

The routes used to be committed here; the umbrella now renders and owns them.
The check didn't disappear, it inverted: `helm --wait` doesn't wait on HTTPRoute
*status*, so a route helm wrote can still be refused by the Gateway. Step 5
below asserts `Accepted` on what the upgrade just wrote — a post-condition of
this deploy, not a check on someone else's object.

## Deploying by hand

Read-only until the `helm upgrade` line. Full context for any step is in
[`INFRA.md`](INFRA.md).

**0. Point at the stand.** `kubeContext` in `inventory.yaml` is an assertion —
`make` refuses unless `kubectl config current-context` equals it. Keep the
kubeconfig **outside the repo working tree** (`sync-clean` fails on any dirty
file).

```bash
export KUBECONFIG=/path/to/test-stand.kubeconfig     # outside this repo
kubectl config use-context insight-test-stand        # or pass KUBE_CTX= on every make call
```

**1. Confirm the cluster.** The context name only proves a file says so — assert
the API server too, against the address recorded in the deployment repo (not
written here: this repo is public):

```bash
kubectl config view --minify -o jsonpath='{.clusters[0].cluster.server}'
```

**2. See what would change** (read-only, renders offline, needs no context):

```bash
make diff ENV=test-stand INSIGHT_VERSION=<chart-version>
```

**3. Upgrade** — directly, not through `make deploy`:

```bash
helm upgrade --install insight \
  oci://ghcr.io/constructorfabric/charts/insight \
  --version "<chart-version>" --namespace insight \
  --values deploy/gitops/environments/test-stand/values.yaml \
  --set-string authenticator.oidc.clientSecret="$(kubectl -n insight \
      get secret insight-oidc -o jsonpath='{.data.client-secret}' | base64 --decode)" \
  --wait --timeout 10m --history-max 10
```

Three load-bearing details:

- **`--set-string …clientSecret=` is mandatory.** `values.yaml` leaves it empty
  (public repo); the chart writes whatever it's given straight into the
  authenticator config Secret, so omitting it produces a blank client secret —
  pods Ready, release `deployed`, every login broken. Read it from the cluster
  so there's one source of truth.
- **`--wait` but deliberately no `--atomic`.** A failed upgrade is left in place;
  rolling back destroys the evidence this stand exists to produce.
- **`--timeout 10m`**, not the Makefile's 30m — a doomed deploy should fail
  inside the CI budget.

Then confirm the release is the chart you meant (a resumed run or race answers
"did it succeed?" with yes and this with no):

```bash
helm list -n insight --deployed --failed --pending --uninstalling \
  --filter '^insight$' -o json | jq -r '.[] | "\(.status) \(.chart) rev=\(.revision)"'
# expect: deployed insight-<chart-version> rev=<n>
```

Status flags are enumerated, not `--all` — Helm 4 removed `--all` and would
report every release absent.

**4. Restart the three `envFrom` readers.** `checksum/config` hashes each
subchart's ConfigMap, not the umbrella-rendered `insight-*-config` Secrets the
pods read once at start (INFRA.md §25). So a changed datastore host or secret
updates the Secret, leaves the pod spec identical, and never reaches a running
process:

```bash
kubectl -n insight rollout restart \
  deploy/insight-authenticator deploy/insight-analytics deploy/insight-identity-resolution
kubectl -n insight rollout status --timeout=5m \
  deploy/insight-authenticator deploy/insight-analytics deploy/insight-identity-resolution
```

All three, on purpose — a list of two leaves one service holding stale config.

**5. Check the edge accepted the routes** (`helm --wait` doesn't):

```bash
kubectl -n insight get httproute insight-gateway insight-keycloak \
  -o custom-columns='NAME:.metadata.name,ACCEPTED:.status.parents[0].conditions[?(@.type=="Accepted")].status'
```

Both must read `True`. When one doesn't, read `ResolvedRefs` in the same breath
— an accepted route with unresolved refs serves 503s rather than nothing.

**6. Seed** — the seeder verbatim, discovering the tenant, datastore
coordinates, image, IdP source type **and the dev-lead address** from the
cluster:

```bash
./src/ingestion/tools/seed/seed-stand.sh -n insight --context insight-test-stand --days 365
```

`--days 365`, not more — the analytics API rejects a window ≥ 400 days
(INFRA.md incident §3). **No `--email`, and that's the point:** the address is
read out of the applied realm (the `insight-keycloak-config-realms` ConfigMap,
the user whose `id` is the roster's dev-lead UUID), so the realm is the source
of truth and the seed follows it. The script prints where it came from —

```text
==> idp:       source_type=keycloak dev_user=<address> (from ConfigMap insight-keycloak-config-realms)
```

— and that parenthesis is the check. `--email` still wins when passed (for a
stand whose realm came from elsewhere); on *this* stand it prints a warning and
seeds yours anyway, so pass it only deliberately. If discovery finds nothing the
script stops and names `--email` — a statement about the stand (realm absent, or
step 3 hasn't run), since seeding is downstream of deploying.

The seeder's manifest (personas, fixtures, tenant, window — what the smoke suite
reads) is printed to the seed Job's stdout. Capture it from the Job log before
the TTL reaps it, and treat it as run-internal: it carries persona addresses,
UUIDs and in-cluster URLs, so it must never become a CI artifact on a public
repo.

**7. Smoke** — through the public URL, real DNS/TLS/IdP redirect:

```bash
export INSIGHT_STAND_BASE_URL=https://insight-test.cfabric.org
export INSIGHT_STAND_PERSONA_PASSWORD=<the realm generator's persona password — see Known gaps>
export INSIGHT_STAND_MANIFEST=<the manifest from step 6>
uv run --project tests --frozen pytest tests/stand -m stand_smoke -ra
```

Nothing in the suite has a default — a missing value is reported by name before
a single request is made.

## Recovery

**This stand is disposable; `make rollback` is not its recovery path.** A
rollback reinstalls the *previous* chart onto a stand whose job is to show what
the newest one does — and a downgrade here once rewrote two live Deployments
into the old shape and wedged every subsequent upgrade. Recovery is fix-forward
(the next merge) or a rebuild:

```bash
deploy/gitops/scripts/recreate-test-stand.sh \
  --expect-cluster <cluster> --apply --confirm wipe-test-stand
```

Plan-only without `--apply`. **It drops the databases, and that is the point:** a
`helm uninstall` isn't a clean stand — this namespace has six Deployments and no
PVCs, so MariaDB and ClickHouse are untouched by anything helm does, and
`identity.persons` is append-only, where one stale row once survived a reinstall
and produced five API failures that read like product defects. What survives
(not Helm-managed, and the rebuilt stand can't start without it): the
generate-once Secrets and the realm ConfigMap — the script verifies every one
*before* it destroys anything and refuses if any is missing.

`make status ENV=test-stand` and `make verify-release ENV=test-stand` inspect the
stand without changing it.

## How login works — the seeded realm

The IdP is the bundled Keycloak at `/kc` on the same hostname, serving the
`insight` realm **generated from the demo seed's own organisation** — one local
user per roster person, each with a password. That is what makes automated
multi-persona login possible, and why `password` is the correct smoke mode here.
Consequences, roughly in the order they bite:

- **The stand is useless between deploy and seed.** Keycloak authenticates; the
  login bootstrap then resolves the principal to an `identity.persons` row by
  `(source_type, external_id)` and **fails closed** with no email fallback. A
  fresh unseeded stand accepts a correct password and *then* denies the person —
  which looks broken and is really an empty projection. Deploy → seed → smoke is
  a sequence, and the identity-resolution seed CronJob failing on an empty stand
  is the documented healthy state.
- **The external id is `idp_sub`, not `sub`.** keycloak-config-cli creates users
  via the admin REST API, where Keycloak assigns its own `sub` and discards the
  document's; the bring-up copies the roster UUID into an `idp_sub` attribute +
  mapper, and `authenticator.oidc.externalIdClaim` names it. Point it at `sub`
  and every login authenticates then is denied against a full projection — the
  most expensive failure, because nothing looks like a config error. Full
  contract: [INFRA.md](INFRA.md#the-load-bearing-settings) §11–12.
- **`sourceType` and the seeder must agree** — `keycloak` here; `seed-stand.sh`
  reads it back from `insight-authenticator-config` rather than being told.
  Changing one without the other reproduces the same authenticate-then-deny.
- **The realm decides who exists; the seed follows.** The dev-lead is the only
  roster address not derived deterministically, so the only one that can drift;
  `seed-stand.sh` reads it back from the applied realm ConfigMap keyed on
  `DEV_LEAD_UUID`, one writer not two. `--email` overrides (supported for a
  foreign realm, unsupported for breaking this one).
  [INFRA.md](INFRA.md#one-roster-two-projections) states the invariant.
- **The persona password is one shared value.** The realm generator applies
  `INSIGHT_SEED_PERSONA_PASSWORD` (default: the committed `insight-dev`, for
  stands unreachable from outside) to every user, so
  `INSIGHT_STAND_PERSONA_PASSWORD` / `TEST_STAND_PERSONA_PASSWORD` is one value
  per stand. An internet-reachable stand must set it at realm-generation time
  and hand the same value to CI. See Known gaps.

## Why `make deploy` works here, and CI uses it

`make deploy ENV=test-stand` once aborted on a sealed-secrets prerequisite, then
overwrote the chart-owned config Secrets from a key name this stand doesn't use
(blank OIDC secret), packed a realm dir this env doesn't ship, hardcoded
`--atomic`/`--create-namespace`, and couldn't gate anything (helm piped to `tee`,
recipe ended `|| true`). All fixed in the target, per the #2404 review — each
difference is now a **declaration this environment makes**:

| declared | where | turns off |
|---|---|---|
| `secrets.services … enabled: false` | `inventory.yaml` | the sealed-manifest apply + controller wait |
| `credentials.deploymentMode: helm` | `values.yaml` | `compose-app-secrets.sh` |
| `keycloak.realms: external` | `inventory.yaml` | packing a realm ConfigMap owned elsewhere |
| `deploy.atomic: false` | `inventory.yaml` | `--atomic` (rollback-on-failure) |
| `bootstrap.namespaces: false` | `inventory.yaml` | `--create-namespace` (a namespace-scoped credential can't use it) |
| `oidcClientSecret` | `inventory.yaml` | nothing — it says where to READ the client secret at deploy time |

`local` and `functional-ci` declare none of these and render byte-identical helm
invocations to before. One deploy procedure; step 3 above is the command CI runs.

**What CI does:** the deploy is a reusable workflow called from the
chart-publishing workflow after publish, on `main` only, with the chart version
from the publish job's output. Credentials come from the `insight-test-stand`
GitHub environment (main-only); the CI credential is a namespace-scoped SA, admin
kubeconfigs stay human-only. Runs coalesce (never cancel a live upgrade); three
named stages (deploy, seed, smoke); smoke never runs after a failed seed. On
failure it prints only the dead stage + edge probe codes — no `describe`/env
dumps/log artifacts (public logs). A red run belongs to the author of the merge
that produced it: fix forward, or revert.

## Known gaps

- **The persona password is a published constant** on a seed-generated realm
  (every user carries the seeder's `DEV_PASSWORD`). Nothing in CI is blocked by
  it, but it's not the "no well-known passwords on a public stand" posture —
  closing it means the realm generator taking a password instead of embedding
  one (a seeder change).
- **Realm ownership sits outside this repo** — the ConfigMap is written by the
  bring-up; the deploy only reads it, so a login failure can have a cause no
  file here shows. The seed now reads that same object for the dev-lead address,
  which is why `--email` survives as an override.
- **Nothing asserts the realm and seeded rows agree** — discovery removes the
  one place they could be *told* to disagree; it doesn't compare them. The
  comparison exists (`tests/lib/insight_stand/personas.py`) and skips on a
  cluster for want of a realm document, which the applied ConfigMap now is.
- **`authenticator.overrideEnabled: true`** is carried forward — a standing
  impersonation primitive on an internet-reachable stand. The smoke suite uses
  real password login and never sends `__override`, so turning it off is now an
  unblocked change; tracked in the `values.yaml` comment.
- **The identity CronJobs** can un-seed logins once their input table stops
  being empty — see the comment at the bottom of `values.yaml`.
