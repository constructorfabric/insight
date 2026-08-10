---
status: accepted
date: 2026-07-30
---

# ADR-0002: Real IdP on Deployed Stands (Pre-Provisioned Keycloak)

**ID**: `cpt-insightspec-adr-auth-0002-real-idp-on-deployed-stands`

**Status history**:

- 2026-08-04: AMENDED by `cpt-insightspec-adr-auth-0003-keycloak-identity-broker` -- the fakeidp
  survival clause is retired (Keycloak becomes the issuer in compose and the in-process rig too),
  and the deferred production IdP/broker question is decided (adopt Keycloak as broker). The
  shared pre-provisioned Keycloak and realm-generation decisions stand unchanged.
- 2026-08-05: AMENDED -- the Keycloak instance is deployed **in-stack**: the umbrella's
  `insight-keycloak` subchart gains a production mode (`start`, MariaDB-backed via the existing
  L2 MariaDB -- no second DBMS -- bootstrap admin from a sealed Secret, no realm import), so each
  environment runs its own broker as part of its auth services. This supersedes Option A's
  "externally operated, stands never administer it" property: realm content is administered as
  code (ADR-0003 config-cli Job) with a sealed per-environment admin credential; the admin UI
  remains a non-channel. Option B's per-stand footprint objection is answered by reusing the
  stack's MariaDB and by the broker being production auth infrastructure, not test scaffolding.
  The one-realm-per-environment and realm-generation decisions stand.
- 2026-08-07: NOTE -- the fakeidp retirement that the 2026-08-04 amendment recorded is now
  executed (issue #2198): the fakeidp crate, subchart and compose service are deleted; the
  functional-CI environment and the gateway e2e rigs run the in-stack Keycloak with the
  seed-generated roster realm.

<!-- toc -->

- [Context and Problem Statement](#context-and-problem-statement)
- [Decision Drivers](#decision-drivers)
- [Considered Options](#considered-options)
- [Decision Outcome](#decision-outcome)
  - [Consequences](#consequences)
  - [Confirmation](#confirmation)
- [Pros and Cons of the Options](#pros-and-cons-of-the-options)
  - [Option A -- One Shared Pre-Provisioned Keycloak, chosen](#option-a----one-shared-pre-provisioned-keycloak-chosen)
  - [Option B -- Keycloak Per Stand](#option-b----keycloak-per-stand)
  - [Option C -- Keep fakeidp on Stands](#option-c----keep-fakeidp-on-stands)
  - [Option D -- Adopt Casdoor Now](#option-d----adopt-casdoor-now)
- [More Information](#more-information)
- [Traceability](#traceability)

<!-- /toc -->

## Context and Problem Statement

ADR-0001 chose `fakeidp` as the IdP for every non-production environment -- CI, compose, and local
k8s -- and deferred a production-grade IdP. Two things have changed since.

First, the project is standing up **deployed Kubernetes stands** whose purpose includes asserting
that a real login works end to end. `fakeidp` auto-approves and never exercises a real
authorization endpoint, a login form, or a real token exchange, so it cannot prove that a login
works.

Second, **the implementation has already diverged from ADR-0001**.
`deploy/gitops/environments/local/values.yaml.template` ships `keycloak.deploy: true` with
`fakeidp.deploy: false`; `charts/insight` bundles both an `insight-fakeidp` and an
`insight-keycloak` subchart (both `deploy: false` by default); and `deploy/gitops/Makefile` carries
a `keycloak-realm` target, gated on `keycloak.deploy == true`, that generates a realm from the seed
roster via `deploy/compose/keycloak/gen-realm.py` and applies it as a `<release>-keycloak-realm`
ConfigMap. The record and the code disagree, and a decision is needed either way.

ADR-0001 rejected Keycloak on operational footprint -- a JVM and its database per environment. That
objection is answered rather than overridden here: the Keycloak instance is provisioned **once**,
shared, and lives outside every stand, so no stand and no CI run pays for it.

## Decision Drivers

- A deployed stand must be able to assert that a real user login succeeds; a fake IdP cannot
  demonstrate this.
- Auth is enforced everywhere since the NGINX_BFF rollout (EPIC #1583); there is no
  `auth_disabled` path downstream, so any stand must mint genuinely signed sessions.
- The Insight `tenant_id` claim must reach the session, or analytics rejects it -- the failure
  ADR-0001 recorded for claim-less IdPs.
- Operational footprint must not be paid per stand or per CI run; CI runs on every pull request.
- The authenticator must stay IdP-agnostic -- the IdP is `authenticator.oidc.issuerUrl` config,
  never a code change or an image rebuild.
- Redirect URIs must be registrable for two very different shapes: per-run CI namespaces with
  unstable in-cluster names, and one stable public hostname for the long-lived stand.
- The record must stop contradicting the deployed configuration.

## Considered Options

- **Option A (chosen)** -- one pre-provisioned, externally operated Keycloak, shared by all
  deployed stands, with one realm per environment.
- **Option B** -- a Keycloak bundled per stand via the existing `insight-keycloak` subchart
  (`keycloak.deploy: true` per environment).
- **Option C** -- keep `fakeidp` on the stands, as ADR-0001 decided.
- **Option D** -- adopt Casdoor now as the real IdP, resolving the deferred production choice at
  the same time.

## Decision Outcome

Chosen: **Option A**. Real Keycloak is the IdP for every deployed Kubernetes stand -- the CI stand,
the E2E stand, and local k8s. It is **pre-provisioned and externally operated**; stands consume it
and never administer it, and no Keycloak admin credential is present on any stand.

- **One realm per environment** (`insight-ci`, `insight-e2e`), each with one confidential client,
  its own roster users, and a `tenant_id` protocol mapper. The Insight tenant is fixed per realm.
- **Redirects**: a wildcard redirect URI on the CI client, because CI namespaces are created and
  destroyed per run and their in-cluster callback hostnames are not knowable in advance; a single
  fixed redirect URI on the E2E client, which has a stable public hostname.
- **"Organisation" means the Insight tenant**, not Keycloak's Organizations feature, which is
  explicitly out of scope. Provisioning an organisation means creating, from one roster in one
  operation, the realm with its clients, roles, users and `tenant_id` mapper, together with the
  matching tenant and organisation tree in Insight's own data.
- **Realm generation is a deliberate administrative import.** `gen-realm.py` moves into the shared
  seed package so one roster produces the data, the identities and the realm; it runs when the
  roster changes, not on every deploy. The realm definition is versioned in the repository.
- **`authDisabled: true` is forbidden in any stand values file.** It remains a chart value for
  third-party consumers. The prohibition is enforced at runtime, not by convention: the stand smoke
  asserts that an unauthenticated request returns 401.
- **`fakeidp` survives in exactly three places**: the docker-compose inner loop, the in-process
  integration rig, and as an explicitly time-boxed scaffold while the `insight-ci` realm is being
  provisioned. It is not a supported stand configuration.
- **Casdoor is not adopted here.** The production IdP choice remains deferred to the
  identity-brokering investigation (#1782); this ADR governs stands only.

The login path across a stand:

```text
browser ──▶ gateway (nginx, auth_request) ──▶ authenticator
authenticator ──▶ Keycloak realm  /authorize            (real login form, real session)
Keycloak      ──▶ authenticator  /auth/callback          (code → token exchange)
authenticator ──▶ sets __Host-sid                        (session cookie, browser)
browser ──▶ gateway  auth_request ──▶ authenticator ──▶ injects ES256 gateway JWT (tenant_id claim)
gateway ──▶ analytics                                    (tenant-scoped data, not 401)
```

### Consequences

- CI availability becomes coupled to the shared Keycloak: if it is unreachable, stand runs fail.
  This is mitigated, not eliminated -- stand failures are classified by stage, and an unreachable
  IdP reports as a neutral infrastructure result routed to the stand owner rather than as a failure
  attributed to the pull request author.
- The shared Keycloak needs a named owner, a stated availability expectation, and a documented
  realm-change procedure, because it is now a dependency of every pull request and is operated
  outside the repositories that depend on it.
- The gateway ES256 signing key (`current.pem`, PKCS#8) is **not auto-generated** by the chart --
  it is supplied as a BYO Secret. It therefore becomes a required input on every stand; without it
  there is no working session regardless of the IdP.
- ADR-0001's accepted trade-off -- that no real login page, refresh, logout, or consent is
  exercised -- is resolved for deployed stands and still holds for compose and the in-process rig.
- Local k8s changes behaviour relative to ADR-0001: it now points at the shared Keycloak instead of
  an in-cluster `fakeidp`, which means a developer's local stand depends on reaching that IdP.
- Zero additional infrastructure is deployed per stand or per CI run; the JVM and its database are
  paid once, centrally.

### Confirmation

- `cfs validate --local-only` passes for this ADR and for the amended ADR-0001.
- On a deployed stand, an unauthenticated request to analytics returns 401, and the same request
  after a real OIDC login through the authenticator returns 200.
- Analytics returns tenant-scoped data rather than an authentication failure, because the session
  carries the `tenant_id` claim emitted by the realm's protocol mapper.
- No stand values file contains `authDisabled: true`.

## Pros and Cons of the Options

### Option A -- One Shared Pre-Provisioned Keycloak, chosen

- Good, because it exercises a genuine login flow, which is the point of a deployed stand.
- Good, because the footprint is paid once rather than per stand or per run, which answers
  ADR-0001's objection directly.
- Good, because realm generation already exists and is already shared between compose and the
  gitops path, so this reuses a working mechanism instead of inventing one.
- Good, because it needs no authenticator code change -- the IdP remains a config value.
- Bad, because every stand run now depends on a service operated outside these repositories, so CI
  availability inherits the IdP's availability.
- Bad, because a wildcard redirect URI on the CI client is looser than a fixed one; acceptable only
  because that realm is test-only and holds synthetic identities.
- Neutral: realm changes become a deliberate administrative step rather than an automatic
  consequence of deploying.

### Option B -- Keycloak Per Stand

- Good, because there is no external dependency; each stand is self-contained and reproducible in
  isolation.
- Good, because the subchart already exists and is wired for the local environment.
- Bad, because it reintroduces exactly the footprint ADR-0001 rejected, and multiplies it: a JVM
  and a database in every per-run CI namespace, on every pull request.
- Bad, because Keycloak start-up and realm import would dominate a short CI budget.
- Rejected on cost per run, not on capability.

### Option C -- Keep fakeidp on Stands

- Good, because it is zero change, zero new dependency, and it keeps ADR-0001 intact.
- Bad (fatal), because a stand whose login is fake cannot assert that login works, which removes
  the main reason to deploy a stand at all.
- Bad, because it also leaves the record contradicting the shipped local-environment configuration.

### Option D -- Adopt Casdoor Now

- Good, because it would resolve the deferred production IdP question at the same time, and
  Casdoor can reuse the existing MariaDB rather than adding a database engine.
- Bad, because nothing in the repository implements or exercises Casdoor today, whereas Keycloak
  realm generation is already working in two places.
- Bad, because it couples a stands decision to a production decision that is still under
  investigation.
- Deferred rather than rejected.

## More Information

- Supersedes the CI and local-k8s clause of `ADR-0001`
  (`cpt-insightspec-adr-auth-0001-per-environment-idp-selection`); its compose decision is
  unchanged.
- EPIC #1583 (NGINX_BFF) establishes auth-everywhere -- see `NGINX_BFF.md` and the gateway ADR
  `cpt-insightspec-adr-gw-0001-access-by-lua-over-auth-request`.
- Identity brokering / production IdP investigation: #1782.
- Existing realm generator and its claim set: `deploy/compose/keycloak/README.md` and
  `deploy/compose/keycloak/gen-realm.py`.
- Chart-side IdP values: `charts/insight/values.yaml` (`authenticator.oidc`, `fakeidp`,
  `keycloak`).
- Kube realm application: the `keycloak-realm` target in `deploy/gitops/Makefile`.

## Traceability

- Supersedes `cpt-insightspec-adr-auth-0001-per-environment-idp-selection` (its Option A decision
  for CI and local k8s only).
- Realises the environment wiring behind `cpt-insightspec-fr-auth-oidc-login` without changing
  that contract.
- Constrains the deployment stand topology decision recorded as
  `cpt-insightspec-adr-deploy-0001-stand-topology` (authored separately; reference it by ID).
