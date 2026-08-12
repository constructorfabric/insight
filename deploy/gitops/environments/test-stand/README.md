# `test-stand` — the published test stand

The gitops environment for **insight-test.cfabric.org**: the cluster that CI
upgrades to the umbrella chart it has just published, then seeds and
smoke-tests, on every merge to `main`
([constructorfabric/insight#2244](https://github.com/constructorfabric/insight/issues/2244)).

Its whole job is to answer one question automatically: *does the chart we
just published actually install, hold data, and let a person log in and see
that data?*

> **Read this before touching anything here.** This environment is shaped
> differently from every other environment in `deploy/gitops/`. `make deploy` IS
> the deploy path — CI runs exactly it — but `make bootstrap` and `make system-*`
> are not, and either would damage this stand by installing a second, competing
> copy of something an operator already owns. See
> [How `make deploy` represents this stand](#how-make-deploy-represents-this-stand)
> for the six declarations that make one deploy procedure serve both this
> cluster and a sandbox, and
> [Recovery](#recovery--rebuild-do-not-roll-back) for why a rollback is not how
> this stand is fixed.

## Contents

```text
environments/test-stand/
├── README.md                        # this file
├── INFRA.md                         # what has to exist BENEATH the umbrella, and in what order
├── inventory.yaml                   # cluster address + "this env manages the umbrella only"
└── values.yaml                      # the umbrella overlay — the file the deploy passes to helm
```

There is deliberately no `sealed-secrets/` directory, no `keycloak/realms/`
directory, no `<svc>-values.yaml` and — since the chart grew native Gateway API
templates — no `manifests/` directory either. Each absence is a decision, and
each one is explained below.

[`INFRA.md`](INFRA.md) answers the question this file does not: *how do I get a
cluster to run any of this against?* It is the L0/L2 capture — the operators,
the datastores, the edge, the order they have to come up in, and the handful of
settings whose absence produces a release that reports `deployed` while the
stand silently does not work. Read it before building a second stand; read this
file to change the one that exists.

## The ownership boundary

This tree now owns the **stand's application configuration**. It does not own
the cluster.

| Layer | What | Owned by | Changed how |
|---|---|---|---|
| L0 | Cluster prereqs: cert-manager and its ClusterIssuer, Envoy Gateway and the `Gateway` object, the namespaces themselves | the deployment repository (outside this repo) | a human, deliberately |
| L2 | Datastores — ClickHouse, MariaDB, Redis, Redpanda — each under its own operator in its own namespace; plus Airbyte and Argo Workflows | the deployment repository | a human, deliberately |
| L2 | Generate-once Secrets: `insight-db-creds`, `insight-authenticator-signing-keys`, `insight-oidc`, `insight-keycloak-admin`, `insight-keycloak-config` | the deployment repository | a human, deliberately |
| L2 | The Keycloak realm content (the `insight-keycloak-config-realms` ConfigMap) — on this stand, the roster realm generated from the seeder's own organisation | the deployment repository | a human, deliberately |
| **L3** | **The umbrella Helm release `insight` in namespace `insight` — every value in `values.yaml`** | **this directory** | **CI, on every merge to `main`** |
| L3 | The two `HTTPRoute`s, rendered by the release from `gateway.route` and `keycloak.route` in `values.yaml` | **this directory** | **CI, on every merge to `main`** |
| L3 | The `argo-workflow` ServiceAccount + Role + RoleBinding that the chart's WorkflowTemplates pin but do not create | the deployment repository | a human, deliberately |

Two consequences worth stating plainly:

* **Nothing in this directory creates or rotates a credential.** Every Secret
  the release consumes already exists, was generated once, and is referenced
  by name. That is why `inventory.yaml` lists all of them with
  `enabled: false` — see the long comment there for what each one would break
  if it were re-materialised.
* **The CI deploy credential is the exception, and it has its own runbook.**
  Provisioning, verifying, storing, rotating and revoking the credential CI
  uses to deploy this chart is [`credentials-runbook.md`](credentials-runbook.md),
  beside this file.
* **A change merged here reaches a published stand at merge speed.** There is
  no staging step between this file and the cluster. Review accordingly.

### What this env does not own, but depends on

Five things live outside the Helm release and outside this directory. If any
of them disappears, the release still installs and reports `deployed`, and the
stand is broken anyway:

| Object | Symptom if missing |
|---|---|
| `Gateway/insight` in the gateway controller's namespace — what both routes `parentRef` | the release renders its two `HTTPRoute`s and neither ever attaches: `Accepted` stays false because the parent resolves to nothing. The public URL answers nothing; every smoke check fails at the first request |
| `ClusterIssuer/insight-ca` — what `authenticator.tlsDiscovery.issuerRef` names | the authenticator's authn-TLS Certificate never issues, and the chart mounts the Secret it would have produced **non-optionally** into analytics and identity-resolution. Both sit in `ContainerCreating` until `--wait` gives up, and nothing in that failure names an issuer |
| `ConfigMap/insight-keycloak-config-realms` — the realm content, and now also what the seeder reads the dev-lead address out of | the chart's `keycloak-config` hook Job has nothing to apply and fails the whole upgrade, and step 6's seed stops naming `--email` rather than seeding a roster the realm does not carry. Worse if it is present but wrong: a realm that lost its `idp_sub` mapper authenticates every persona and then denies them, with a fully populated identity projection |
| `ServiceAccount/argo-workflow` (+ its Role/RoleBinding) | every scheduled transform and data-quality run fails with `serviceaccount "argo-workflow" not found`, while all app pods stay healthy — nothing surfaces until a scheduled run |
| The Secrets in the table above | services fail to start, or start with blank configuration |

The two routes used to be committed here, as manifests, because the chart
rendered none and every acceptance criterion travels through them. The umbrella
has since grown native Gateway API templates — `gateway.route` and
`keycloak.route` replaced the old `ingress` blocks — so the release renders both
`HTTPRoute`s and owns them, and the manifests are gone. The check that used to
guard them has not disappeared, it has inverted: `helm --wait` does not wait on
HTTPRoute *status*, so a route helm wrote successfully can still be refused by
the Gateway (a `parentRef` naming a Gateway that is not there, a `sectionName`
naming a listener that is not, a `backendRef` pointed at a Service a chart
upgrade renamed), and every one of those leaves the release `deployed` and the
stand dark. Step 5 below therefore asserts `Accepted` on what the upgrade has
just written — a post-condition of the deploy, not a check on somebody else's
object. The Argo RBAC is still not copied here: it is Argo plumbing rather than
application configuration, and it sits outside the deploy/seed/smoke scope this
environment was created for.

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
      get secret insight-oidc -o jsonpath='{.data.client-secret}' | base64 --decode)" \
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

Then ask the second question, which the upgrade's exit status does not answer:
*is the release the chart you meant?* A resumed run, a hand-deploy that raced
CI, or a mistyped version answers "did it succeed?" with yes and this with no.
CI runs the same check as its own step:

```bash
helm list -n insight --deployed --failed --pending --uninstalling \
  --filter '^insight$' -o json | jq -r '.[] | "\(.status) \(.chart) rev=\(.revision)"'
```

Expect `deployed insight-<chart-version> rev=<n>`. The status flags are
enumerated rather than passed as `--all`, which Helm 4 removed — on a runner or
laptop carrying v4, `--all` exits `unknown flag` and the check silently reports
every release as absent.

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

**5. Check the edge accepted the routes the upgrade just wrote.** The release
renders both of them, but `helm --wait` does not wait on their status — so a
route the Gateway refuses leaves the upgrade green and the stand dark. This is
the command CI runs:

```bash
kubectl -n insight get httproute insight-gateway insight-keycloak \
  -o custom-columns='NAME:.metadata.name,ACCEPTED:.status.parents[0].conditions[?(@.type=="Accepted")].status'
```

Both must read `True`. When one does not, read `ResolvedRefs` in the same
breath — `Accepted` says the Gateway took the attachment, `ResolvedRefs` says
its `backendRef` resolves, and a route that is accepted with unresolved refs
serves 503s rather than nothing:

```bash
kubectl -n insight get httproute insight-gateway insight-keycloak \
  -o 'jsonpath={range .items[*]}{.metadata.name}{"\t"}{range .status.parents[0].conditions[*]}{.type}{"="}{.status}{" "}{end}{"\n"}{end}'
```

**6. Seed.** Seeding reuses the seeder verbatim — no test-stand variant, no
per-stand flags beyond the ones below. It discovers the tenant, the datastore
coordinates, the seed image, the IdP source type **and the dev-lead persona
address** from the cluster itself:

```bash
./src/ingestion/tools/seed/seed-stand.sh \
  -n insight \
  --context insight-test-stand \
  --days 730
```

`--context` is passed even though the ambient kubeconfig already points there:
the script prints the context it resolved before it writes anything, and a run
whose target is stated rather than inherited is one a reader of the log can
check. `--days`, spelled exactly like that — the seeder rejects unknown
arguments, so a plausible-looking synonym fails every time.

**There is no `--email` here, and its absence is the point.** The dev-lead
address is read out of the realm this stand applied — the
`insight-keycloak-config-realms` ConfigMap, the user whose `id` is the roster's
dev-lead UUID — so the realm is the source of truth for who exists and the seed
follows it. The script prints both the address and where it came from before it
writes anything:

```text
==> idp:       source_type=keycloak dev_user=<address> (from ConfigMap insight-keycloak-config-realms)
```

That parenthesis is the check. An address alone cannot be verified by eye; which
side supplied it can. `--email` still exists and still wins when passed — the
origin then reads `(from --email)` — for a stand whose realm is provisioned some
other way. Pass it on *this* stand and the seeder prints a multi-line warning
naming both addresses and seeds yours anyway, because an explicit flag wins;
that warning is the sound of the two projections coming apart, so pass the flag
only deliberately and say why.

If discovery finds nothing the script stops and names `--email`. That is a
statement about the stand rather than about the seeder: the realm ConfigMap is
absent, or holds no user carrying the dev-lead UUID, or the realm was never
applied because step 3 has not run yet. Seeding is downstream of deploying, and
this is where that ordering becomes visible.

The seeder's manifest — the list of personas, fixtures, the tenant and the
data window that the smoke suite reads — is **printed to the seed Job's
stdout** and written to a path inside a pod whose filesystem is discarded.
Capture it from the Job log before the Job's TTL reaps it. Treat that JSON as
run-internal: it carries persona addresses, UUIDs and in-cluster service URLs,
so it must not become a CI artifact on a public repository.

**7. Smoke.** Driven through the public URL exactly as a browser would drive
it — real DNS, real TLS, real IdP redirect. `INSIGHT_STAND_BASE_URL` is what
aims the suite; nothing else does:

```bash
export INSIGHT_STAND_BASE_URL=https://insight-test.cfabric.org
export INSIGHT_STAND_PERSONA_PASSWORD=<the realm generator's persona password — see below>
export INSIGHT_STAND_MANIFEST=<the manifest from step 6>

uv run --project tests --frozen pytest tests/stand -m stand_smoke -ra
```

Nothing in the suite has a default: a missing value is reported by name before
a single request is made.

### Recovery — rebuild, do not roll back

**This stand is disposable, and `make rollback` is not its recovery path.**
Rolling back reinstalls the *previous* chart, which is a downgrade onto a stand
whose entire job is to show what the newest one does. It has been tried: a
downgrade here rewrote two live Deployments into the old chart's shape and every
subsequent upgrade was refused until the stand was rebuilt.

Recovery is fix-forward — the next merge — or a rebuild:

```bash
deploy/gitops/scripts/recreate-test-stand.sh \
  --expect-cluster <cluster> --apply --confirm wipe-test-stand
```

Plan-only without `--apply`; it prints exactly what it would destroy first.

**It drops the databases, and that is the point.** A `helm uninstall` is *not* a
clean stand: this namespace holds six Deployments and nothing else — no
StatefulSets, no PersistentVolumeClaims — so MariaDB and ClickHouse are entirely
untouched by anything helm does here. `identity.persons` is an append-only
observation log, and a single stale row written by an earlier seed once survived
a full uninstall/reinstall and produced five API failures that read exactly like
product defects. A stand you cannot fully recreate is a stand whose red runs you
cannot trust.

What survives, because it is not Helm-managed and the rebuilt stand cannot start
without it: `insight-db-creds`, `insight-oidc`, `insight-keycloak-admin`,
`insight-keycloak-config`, `insight-authenticator-signing-keys`, and the realm
ConfigMap. The script verifies every one of them *before* it destroys anything
and refuses if any is missing or empty.

`make status ENV=test-stand` and `make verify-release ENV=test-stand` remain the
right tools for looking at the stand without changing it.

## How login works here — the seeded realm

The stand's IdP is the bundled Keycloak, published at `/kc` on the same public
hostname, and the realm it serves is `insight`: not the GitHub-federated
`insight-broker` realm the deployment repository's other login mode uses, but a
realm **generated from the demo seed's own organisation** — one local user per
person on the roster, each with a password credential. That single decision is
what makes an automated multi-persona login possible here at all, and it is why
`password` is the correct mode for the smoke suite on this stand.

Five consequences, roughly in the order they bite:

* **The stand is useless between deploy and seed.** Authentication and
  authorisation are two systems here. Keycloak authenticates; the login
  bootstrap then resolves the principal to a row in `identity.persons` by
  `(source_type, external_id)` and **fails closed** when there is no match —
  there is no email fallback, by design. So a freshly deployed, unseeded stand
  accepts a correct password and *then* denies the person, which reads like a
  broken login and is really an empty projection. Deploy → seed → smoke is a
  sequence, not a convenience, and the identity-resolution seed CronJob failing
  on a stand with no data yet is the documented healthy state rather than a
  fault.
* **The external id is `idp_sub`, not `sub`.** The realm generator sets each
  realm user's id to that person's roster UUID, which would be enough for a
  realm *import* — but the chart applies realms with keycloak-config-cli, which
  creates users one at a time through the admin REST API, where Keycloak assigns
  its own id and silently discards the document's. The bring-up outside this
  repository therefore copies the roster UUID into an `idp_sub` user attribute
  and adds the mapper that emits it as a claim, and
  `authenticator.oidc.externalIdClaim` names that claim. Point it at `sub` and
  every login authenticates and is then denied, against a fully populated
  identity projection — the most expensive way this stand can fail, because
  nothing about it looks like a configuration error. [`INFRA.md`](INFRA.md#the-load-bearing-settings)
  carries the full contract the generated realm has to satisfy, including the
  declarative user profile that otherwise discards the attribute before the
  mapper ever sees it.
* **`sourceType` and the seeder have to agree.** `authenticator.oidc.sourceType`
  is `keycloak` here, and the seeder writes
  `(source_type='keycloak', value_type='id', value_id=<the roster UUID>)`.
  Neither value is repeated in CI: `seed-stand.sh` reads the source type back
  out of `insight-authenticator-config` rather than being told. Changing one
  without the other produces exactly the same authenticate-then-deny.
* **The realm decides who exists; the seed follows.** Same argument as the
  source type, applied to the one persona address an operator could supply. The
  dev-lead is the only person on the roster whose address is not derived —
  everyone else's falls out of the roster module and passes through no input at
  all — so the dev-lead is the only one that can drift, and drifting means the
  realm names one person and `identity.persons` names another. `seed-stand.sh`
  therefore reads the address back out of the applied realm ConfigMap, keyed on
  the roster's dev-lead UUID (`insight_seed.profiles.DEV_LEAD_UUID`, which the
  realm generator writes as each realm user's `id`), rather than taking a flag.
  One writer, not two copies. Regenerating the realm moves the address and the
  next seed follows it; passing `--email` overrides the read, which is the
  supported way to seed a stand whose realm came from somewhere else and the
  unsupported way to break this one.
  [`INFRA.md`](INFRA.md#one-roster-two-projections) states the invariant on its
  own, because it is the one a future change is most likely to break.
* **The persona password is one shared value for every seeded user.** The realm
  generator applies `INSIGHT_SEED_PERSONA_PASSWORD` (default: the committed
  `insight-dev` constant, for stands unreachable from outside) to every user it
  emits, so `INSIGHT_STAND_PERSONA_PASSWORD` — and its CI spelling
  `TEST_STAND_PERSONA_PASSWORD` — is a single value per stand. An
  internet-reachable stand must set it when its realm is generated and hand the
  same value to CI. See Known gaps.


## How `make deploy` represents this stand

It did not, once. `make deploy ENV=test-stand` aborted on a sealed-secrets
prerequisite this cluster cannot satisfy; if that hurdle were removed it went on
to overwrite the three chart-owned `insight-*-config` Secrets from a key name
this stand does not use, writing a blank OIDC client secret; it packed a realm
directory this environment deliberately does not ship; it hard-coded `--atomic`
and `--create-namespace`; and its exit status could not gate anything, because
helm was piped into `tee` with no `pipefail` and the recipe ended in `|| true`,
so a failed upgrade reported success.

All of that is fixed in the target rather than worked around here, which is what
the review of #2404 asked for. Every difference is now a DECLARATION this
environment makes about itself:

| declared | where | what it turns off |
| --- | --- | --- |
| `secrets.services` all `enabled: false` | `inventory.yaml` | the sealed-manifest apply and its controller wait |
| `credentials.deploymentMode: helm` | `values.yaml` | `compose-app-secrets.sh` — the chart renders those Secrets |
| `keycloak.realms: external` | `inventory.yaml` | packing a realm ConfigMap owned outside this repo |
| `deploy.atomic: false` | `inventory.yaml` | `--atomic`, i.e. rollback-on-failure |
| `bootstrap.namespaces: false` | `inventory.yaml` | `--create-namespace`, which a namespace-scoped credential cannot use |
| `oidcClientSecret` | `inventory.yaml` | nothing — it says where to READ the client secret at deploy time |

Environments that declare none of these are unaffected: `local` and
`functional-ci` render byte-identical helm invocations to before.

So there is one deploy procedure, this README describes it, and CI executes it.
The command in step 3 above is the command.

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

* **The persona password is a published constant.** Every user in the generated
  realm carries the seeder's `DEV_PASSWORD`, so an internet-reachable stand
  serves local accounts on a value anybody can read out of this repository.
  Nothing in CI is blocked by it — the gate signs in as its personas with it,
  which is the point — but it is not the "no well-known passwords on a public
  stand" posture this environment was specified with. Closing it means teaching
  the realm generator to take a password instead of embedding one, which is a
  change to the seeder rather than to this directory.
* **Realm ownership still sits outside this repository.** The
  `insight-keycloak-config-realms` ConfigMap is written by the bring-up, not by
  this tree, and the deploy only reads it. That is deliberate — see the
  `keycloakConfig` comment in `values.yaml` — but it does mean a login failure
  can have a cause no file here can show you. The seed now reads that same
  object for the dev-lead address, which cuts both ways: a wrong realm is a
  failed or wrongly-scoped seed instead of a silent mismatch, and an object this
  repository does not own has become an input to a stage this repository does.
  That is why `--email` survives as an override rather than being deleted.
* **Nothing asserts that the realm and the seeded rows agree.** Discovery
  removes the one place they could be told to disagree; it does not compare
  them. A realm regenerated from a different roster, or a stale seed manifest,
  still shows up only as a login that authenticates and resolves to nobody. The
  comparison exists (`tests/lib/insight_stand/personas.py`) and skips itself on
  a cluster for want of a realm document — which the applied ConfigMap now is.
* **`authenticator.overrideEnabled: true`** is carried forward from the
  installed release. It is a standing impersonation primitive on an
  internet-reachable stand, gated on that flag alone. The conditional that used
  to hang over it has resolved in the good direction: the smoke suite reaches
  every persona through a real password login and never sends `__override`, so
  turning the flag off is now an unblocked change rather than one that would
  take the gate with it. Tracked separately; see the comment on the key in
  `values.yaml`.
* **The identity CronJobs** can un-seed logins once their input table stops
  being empty. See the comment at the bottom of `values.yaml`.
