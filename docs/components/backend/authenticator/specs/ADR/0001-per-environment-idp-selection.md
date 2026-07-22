---
status: accepted
date: 2026-07-16
---

# ADR-0001: Per-Environment IdP Selection (fakeidp for all dev environments; production broker deferred)

**ID**: `cpt-insightspec-adr-auth-0001-per-environment-idp-selection`

<!-- toc -->

- [Context and Problem Statement](#context-and-problem-statement)
- [Decision Drivers](#decision-drivers)
- [Considered Options](#considered-options)
- [Decision Outcome](#decision-outcome)
  - [Consequences](#consequences)
  - [Confirmation](#confirmation)
- [Pros and Cons of the Options](#pros-and-cons-of-the-options)
  - [Option A -- fakeidp for all dev environments, chosen](#option-a----fakeidp-for-all-dev-environments-chosen)
  - [Option B -- Dex for local k8s](#option-b----dex-for-local-k8s)
  - [Option C -- heavy broker (Keycloak / Authentik / Zitadel) for local k8s](#option-c----heavy-broker-keycloak--authentik--zitadel-for-local-k8s)
  - [Option D -- authenticator dev-login endpoint](#option-d----authenticator-dev-login-endpoint)
- [More Information](#more-information)
- [Traceability](#traceability)

<!-- /toc -->

## Context and Problem Statement

The authenticator is a confidential OIDC client (`cpt-insightspec-fr-auth-oidc-login`): it logs
in against an upstream IdP, then mints the session-linked ES256 gateway JWT. Since the NGINX_BFF
rollout (EPIC #1583) auth is enforced **everywhere** (R1 -- every downstream verifies the gateway
JWT; there is no `auth_disabled` path). That change retired the old developer shortcut, where the
frontend stamped a dev email into a runtime `oidc-config.js` and the browser forged an *unsigned*
bearer -- it only ever worked because the gateway trusted unauthenticated requests. With real
downstream verification that forged bearer is now a 401, so a non-production login path must still
produce a **genuinely signed** session.

Every non-production environment therefore needs *some* IdP for the authenticator to log in
against, spanning CI / e2e (headless, deterministic, instant), compose (the fast disposable inner
loop), and local k8s (the heavyweight, prod-shaped environment used for FE / browser work).

Two properties make the choice non-trivial:

- The full stack is **tenant-scoped**: analytics rejects a session that carries no Insight tenant.
  The authenticator resolves the Insight `tenant_id` at the **authentication** boundary -- it maps
  an external/IdP tenant identifier to the Insight tenant -- so whatever IdP is used in dev must let
  that claim reach the session, or the app 401s past the login page.
- On local k8s the IdP's browser-facing authorization endpoint must be reachable from the browser,
  while the token/JWKS back-channel must be reachable from the authenticator pod -- from one
  consistent issuer URL. The session cookie (`__Host-sid`) is `Secure`, so the browser only accepts
  it over a **secure context** (HTTPS or `http://localhost`), which constrains the callback origin.

## Decision Drivers

- Auth is always on (R1): any dev login must mint a real signed session, not a mock bearer.
- Use the **published ghcr images unchanged** -- a per-environment login path must not require
  rebuilding a service image.
- The whole stack (analytics tenant-scoping included) must work end-to-end in dev, which means the
  Insight `tenant_id` claim must be present in the dev session.
- CI must stay headless and deterministic; compose must stay fast; local k8s should be as close
  to production as is reasonable while adding no new infrastructure.
- The IdP's issuer URL in k8s must be reachable from both the browser and the authenticator pod,
  and the browser callback must land on a secure context so `__Host-sid` is accepted.
- Prefer lightweight infrastructure -- avoid dragging a database / Redis / JVM into a dev IdP.
- Keep the authenticator IdP-agnostic so a production provider slots in later without code changes.

## Considered Options

- **Option A (chosen)** -- fakeidp for **all** dev/test environments (CI, compose, and local k8s).
- **Option B** -- Dex (`dexidp/dex`) for local k8s.
- **Option C** -- a heavy broker (Keycloak / Authentik / Zitadel) for local k8s.
- **Option D** -- an authenticator dev-login endpoint (`/auth/dev-login?email=...`).

## Decision Outcome

Chosen: **Option A**. **fakeidp is the IdP for every dev/test environment -- CI, compose, and
local k8s.** fakeidp already auto-approves a configured dev user and, crucially, injects the
non-standard `tenants` claim, so the full stack (analytics tenant-scoping included) works
end-to-end with **zero extra infrastructure** -- no new database, Redis, or JVM.

On local k8s, fakeidp is exposed through the ingress at nginx path `/idp` with a rewrite (fakeidp
serves its OIDC routes at root), so the browser login flow reaches it and the authenticator pod
shares the same issuer URL. The browser-facing callback uses `http://localhost/auth/callback`
because the `__Host-sid` session cookie is `Secure` and browsers only accept it over a secure
context (HTTPS or `http://localhost`).

**A production-grade IdP / broker is explicitly deferred to a production decision.** The
authenticator is IdP-agnostic -- `authenticator.oidc.issuerUrl` is a plain config value -- so any
OIDC provider slots in without a code change, and it does not need to run locally to develop
against. See More Information for the leading candidates when that decision is made.

### Consequences

- **CI / compose / local k8s** now all run the same dev IdP (fakeidp), so there is one mechanism
  to reason about and one way the `tenant_id` claim reaches the session.
- **No new infrastructure**: fakeidp is a single small service with no backing store; dev does not
  pay for a database / Redis / JVM to have a working login.
- **Tenant scoping works in dev**: because `tenant_id` is an Insight-domain claim resolved at the
  authentication boundary (a mapping from an external/IdP tenant identifier to the Insight tenant,
  **not** something computed in the identity-resolution/persons service), fakeidp's injected
  `tenants` claim is exactly what lets analytics return data instead of 401.
- **Local k8s wiring**: fakeidp is reached through the ingress (`/idp` + rewrite) so browser and
  pod share one issuer; the browser callback is pinned to `http://localhost/auth/callback` to
  satisfy the `__Host-sid` secure-context requirement. This is a routing/config concern, not a
  code path in the authenticator.
- **Production is unblocked but undecided**: because the authenticator is IdP-agnostic, deferring
  the production IdP costs nothing today -- the eventual provider is a config change plus its own
  provisioning, with no authenticator rebuild.
- **Trade-off accepted**: fakeidp does not exercise a real login page, refresh, logout, or
  consent, so those integration behaviours are validated later against the real production IdP
  rather than locally. This is deliberate: it keeps dev free of heavy IdP infrastructure.

### Confirmation

- `cfs validate --local-only` passes for this ADR and the authenticator DESIGN.
- On local k8s, a browser login through fakeidp (via `/idp`) lands on
  `http://localhost/auth/callback`, the `__Host-sid` cookie is set, and an authenticated request
  returns 200 while an unauthenticated one returns 401.
- Analytics returns tenant-scoped data (not `AUTHN_FAILED`) because the session carries the
  fakeidp-injected `tenants` claim -- the exact check that fails under a claim-less IdP.

## Pros and Cons of the Options

### Option A -- fakeidp for all dev environments, chosen

- Good, because it is one mechanism across CI, compose, and local k8s -- one thing to reason about.
- Good, because it injects the non-standard `tenants` claim, so analytics tenant-scoping and the
  full stack work end-to-end with zero extra infrastructure (no database, Redis, or JVM).
- Good, because auto-approve is ideal for CI and keeps compose fast.
- Good, because it needs no authenticator code change and preserves the published-image contract.
- Bad, because it never exercises a real login page, refresh, logout, or consent -- those surface
  against the production IdP later, not locally. Accepted, given the zero-infrastructure win.
- Neutral: local k8s needs ingress wiring (`/idp` + rewrite) and a `http://localhost` callback for
  the `__Host-sid` secure-context rule -- routing/config, not code.

### Option B -- Dex for local k8s

- Good, because Dex is a tiny single Go binary, database-free in dev, config-only, and can act
  both as a broker and as a standalone provider with a real login page.
- Bad (fatal), because Dex **cannot emit custom claims** -- there is no way to put the Insight
  `tenant_id` into the token. Verified end-to-end: after a real Dex browser login, identity returns
  200 but analytics returns `AUTHN_FAILED` (401) because the session has no tenant.
- Bad, because relying on the IdP to assert *your* Insight tenant is the wrong layer anyway: a real
  IdP (e.g. Entra) emits its own tenant id (`tid`), never your Insight tenant UUID, so the
  external-to-Insight mapping must live at the authentication boundary regardless.

### Option C -- heavy broker (Keycloak / Authentik / Zitadel) for local k8s

- Good, because these **do** satisfy the full feature set: federation broker *and* standalone
  provider, a custom `tenant_id` claim, and a UUID `sub` -- the most production-like option.
- Bad, because each drags in a database / Redis / JVM and realm/tenant administration --
  disproportionate operational footprint for a dev IdP ("don't bring more databases to a small
  install"). Rejected on footprint, not capability.

### Option D -- authenticator dev-login endpoint

- Good, in principle, because it mints a real signed session and removes the IdP hop entirely, so
  there is no browser-reachability or claim-injection problem at all.
- Bad (fatal), because the published ghcr images are built without such a flag; using it locally
  would force a custom image rebuild -- the exact friction we are trying to avoid.
- Bad, because it is a login-bypass backdoor in a security-critical service; even compile-time
  gating adds a permanently sensitive surface. Rejected.

## More Information

- The Insight `tenant_id` is an **Insight-domain claim resolved at the authentication boundary** --
  a mapping from an external/IdP tenant identifier to the Insight tenant. It is **not** produced by
  the identity-resolution/persons service. This is why the IdP choice hinges on getting that claim
  into the session, and why a claim-less IdP (Dex) fails analytics.
- `__Host-sid` requires a **secure context**, so the local-k8s browser callback is
  `http://localhost/auth/callback` (HTTPS or `http://localhost` only); this constraint, not
  preference, pins the callback origin.
- When a real IdP **is** needed in production, the leading lightweight candidate is **Casdoor**
  (Go, can reuse the existing MariaDB -- no new database engine -- with custom claims, standalone
  and broker modes, and a UUID subject). **Keycloak** is the battle-tested heavyweight reference.
  Both keep the authenticator unchanged, since the IdP is only `authenticator.oidc.issuerUrl`.
- EPIC #1583 (NGINX_BFF) establishes R1 (auth everywhere) -- see `NGINX_BFF.md` and the gateway
  ADR `cpt-insightspec-adr-gw-0001-access-by-lua-over-auth-request`.
- Identity brokering / production IdP investigation: #1782.

## Traceability

- Realises the environment wiring behind `cpt-insightspec-fr-auth-oidc-login` (the authenticator's
  OIDC client) without changing its contract.
- Constrains the deployment story for the authenticator's IdP dependency across CI, compose, and
  local k8s, and defers the production IdP to a later decision.
