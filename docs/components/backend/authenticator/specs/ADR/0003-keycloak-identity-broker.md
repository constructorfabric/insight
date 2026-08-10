---
status: accepted
date: 2026-08-04
---

# ADR-0003: Keycloak as the Identity Broker (Configured as Code)

**ID**: `cpt-insightspec-adr-auth-0003-keycloak-identity-broker`

**Status history**:

- 2026-08-07: NOTE -- fakeidp retirement COMPLETED (issue #2198): the fakeidp crate, subchart
  and compose service are deleted; the functional-CI environment and the gateway e2e rigs run
  the in-stack Keycloak with the seed-generated roster realm (imported via keycloak-config-cli /
  `--import-realm`). Compose and the authenticator e2e were already Keycloak-only.
- 2026-08-06: AMENDED -- claim-value-to-tenant translation (the advanced claim-to-group mapper
  sketched in the Decision Outcome) is REJECTED: the tenant is always the fixed per-registration
  pin from environment values, an IdP's own tenancy assertions are never consulted, and a
  customer with several IdPs pins the same tenant on each registration. Two customers sharing
  an IdP vendor do not intersect: realm-per-customer gives each its own registration, client,
  and pin. A CI guard asserts the contract against the canonical realm (fail-closed without the
  pin, single-string claim, tenant-bearing groups inert and flagged).
- 2026-08-05: AMENDED -- of the instance-deployment options this ADR left open, the umbrella
  subchart is chosen: the broker runs **in-stack** (production-mode `insight-keycloak`,
  MariaDB-backed) as part of each environment's auth services, amending ADR-0002's shared
  external instance. The compose Keycloak remains the local-machine dev counterpart and MUST
  keep the same token contract as the broker realms (the canonical `insight` client scope:
  allow-listed `email` + single-string `tenant_id`); the tenant is pinned per provider
  registration from environment values, never imported from upstream claims.

<!-- toc -->

- [Context and Problem Statement](#context-and-problem-statement)
- [Decision Drivers](#decision-drivers)
- [Considered Options](#considered-options)
- [Decision Outcome](#decision-outcome)
  - [Consequences](#consequences)
  - [Confirmation](#confirmation)
- [Pros and Cons of the Options](#pros-and-cons-of-the-options)
  - [Option A -- Keycloak broker + keycloak-config-cli, chosen](#option-a----keycloak-broker--keycloak-config-cli-chosen)
  - [Option B -- Dex](#option-b----dex)
  - [Option C -- multi-issuer support in the authenticator](#option-c----multi-issuer-support-in-the-authenticator)
  - [Option D -- another self-hosted broker (Zitadel / Authentik / Casdoor)](#option-d----another-self-hosted-broker-zitadel--authentik--casdoor)
- [More Information](#more-information)
- [Traceability](#traceability)

<!-- /toc -->

## Context and Problem Statement

The authenticator is a confidential OIDC client against exactly **one** configured issuer
(`authenticator.oidc.issuerUrl`). Customers bring heterogeneous IdPs -- Entra, Okta, Google
Workspace, generic OIDC, SAML-only shops -- each with its own quirks (audience/scope shapes,
`offline_access` behaviour, back-channel-logout support), and onboarding each one today means
bespoke per-environment configuration. Issue #1782 asked whether an **identity broker** in front
of the authenticator should absorb that heterogeneity.

Issue #2163 sharpens the question: "Login with GitHub" (and later Google, Facebook, Apple).
GitHub OAuth Apps are **plain OAuth 2.0, not OIDC** -- no `id_token`, no discovery document, no
JWKS -- so GitHub cannot be wired to the authenticator at all without an adapting layer. Social
logins force the broker decision that heterogeneous customer IdPs only motivated.

Two constraints carry over from the earlier ADRs:

- The gateway JWT carries a **single** `tenant_id` claim resolved at the authentication boundary
  (DD-AUTH-04); ADR-0001 already rejected Dex because it cannot inject such a claim -- verified
  end-to-end, analytics returns `AUTHN_FAILED` without it.
- The authenticator must stay IdP-agnostic: the issuer is a config value, never a code change or
  an image rebuild.

A further requirement from #2163: broker configuration must be expressible **as code** -- reviewed
in pull requests, applied by automation, secrets held outside the repository -- never click-ops in
an admin UI.

## Decision Drivers

- One uniform OIDC issuer toward the authenticator, regardless of what a customer runs upstream.
- Coverage: OIDC and SAML customer IdPs, plus social providers -- GitHub (#2163), Google,
  Facebook, Apple -- including GitHub's OAuth-only protocol gap.
- Per-provider claim control: inject the Insight `tenant_id`, normalise `email`/`sub`, and pass
  **only** allow-listed claims into the token (exactly one `tenant_id`, never an array).
- Configuration as code: declarative, idempotent, gitops-applied; secrets via sealed secrets;
  no admin-UI drift.
- Operational footprint must stay bounded: ADR-0002 already pays for one shared, pre-provisioned
  Keycloak; a broker should extend that investment, not add a second system.
- Dev/CI parity: the same issuer technology everywhere removes prod-versus-test drift and the
  cost of maintaining a test double.

## Considered Options

- **Option A (chosen)** -- Keycloak as the identity broker, realm content managed declaratively
  with `keycloak-config-cli` from gitops.
- **Option B** -- Dex as a lightweight OIDC adapter in front of the authenticator.
- **Option C** -- native multi-issuer support in the authenticator (one RP, many configured IdPs,
  per-provider adapters including a bespoke GitHub OAuth adapter).
- **Option D** -- another self-hosted broker (Zitadel, Authentik, Casdoor).

## Decision Outcome

Chosen: **Option A**. Keycloak fronts the authenticator as an **identity broker**: a realm
federates the upstream IdPs (customer OIDC/SAML, GitHub, Google, Facebook, Apple) and presents a
single uniform OIDC issuer. The authenticator's flow logic is unchanged -- the existing
code + PKCE, refresh, and back-channel-logout machinery against one issuer shape; its only
change is configuration surface: the single `issuerUrl` generalises to a host-keyed issuer map
for multi-customer (cloud) installations (see realm selection below).

- **Provider coverage is built in.** Keycloak ships identity providers for GitHub, Google,
  Facebook, Microsoft, LinkedIn, GitLab and others, and Apple since Keycloak 24. GitHub's
  OAuth-only protocol is absorbed by the broker; the authenticator never sees it.
- **Configuration is code, exclusively.** The Keycloak *instance* is deployed declaratively (the
  existing chart/subchart mechanism, or the Keycloak Operator `Keycloak` CR where the platform
  provides one). *Realm content* -- brokered IdPs, mappers, the `insight-authenticator` client --
  is YAML applied idempotently by [`keycloak-config-cli`](https://github.com/adorsys/keycloak-config-cli)
  as a gitops sync job. Client secrets enter via environment-variable substitution from sealed
  secrets; nothing lands in the repository or an image (the #2163 credentials criterion). The
  admin UI is read-only in practice: `KeycloakRealmImport` and hand edits are not configuration
  channels, and config-cli re-applies the versioned realm on every sync, reverting drift.
- **Claims are shaped at the broker.** Every provider registration carries a
  `hardcoded-attribute-idp-mapper` pinning the fixed per-registration Insight `tenant_id` from
  environment values, plus an attribute-importer stamping `idp_sub` (the upstream's stable
  directory id, e.g. Entra `oid` -- the login-bootstrap external id). An upstream's own tenancy
  assertions are never consulted (amended 2026-08-06; the claim-to-group translation once
  sketched here is rejected): a customer with several IdPs pins the same tenant on each
  registration, and two customers sharing an IdP vendor never intersect -- realm-per-customer
  gives each its own registration, client, and pin. The client's **protocol mappers / client
  scopes** are the allow-list: the token contains only what is explicitly emitted -- `sub`,
  `email`, one string `tenant_id`, `idp_sub` -- and deliberately does NOT aggregate attributes
  over groups, so a group-sourced tenant is mechanically impossible; a CI guard asserts the
  contract on every committed realm file and against a live import.
- **Topology: one realm per customer**, holding that customer's brokered IdPs and one
  confidential client. The single-`tenant_id` rule holds because each provider registration (or
  upstream tenancy claim) maps to exactly one Insight tenant. Realm-per-tenant remains available
  if isolation inside a customer is ever required -- realms are just more YAML.
- **IdP selection is the realm's login page; realm selection is host-based.** Within one
  customer, no Insight code chooses a provider: an unauthenticated request lands on the broker
  realm's login page, which offers exactly the options its YAML configures (password, social,
  corporate SSO); a single-IdP realm auto-redirects (default IdP / `kc_idp_hint`). Across
  customers -- the cloud case, where the issuer must be known while the user is still anonymous
  -- the customer's hostname selects the realm: the authenticator's `issuerUrl` becomes a
  host-keyed issuer map, with a single-entry map preserving today's behaviour for dedicated
  installs. Keycloak **Organizations** (one shared realm, email-domain discovery) is rejected
  for now: it weakens the per-customer realm isolation this decision builds on, and ADR-0002
  already scoped it out; revisit only if per-customer hostnames are unavailable.
- **Unknown users are refused by the existing boundary.** Brokering does not change person
  resolution: the authenticator still resolves the (now broker-issued) identity via the Identity
  Service, so a social login whose email matches no person is refused with the existing
  unknown-person behaviour and audit event (#2163: no auto-created accounts).
- **fakeidp is retired.** With Keycloak the issuer everywhere, the test double loses its reason
  to exist -- this amends ADR-0002's survival clause (compose inner loop, in-process rig,
  time-boxed CI scaffold). The compose stack's existing `AUTH_MODE=keycloak` path becomes the
  only mode; the generated realm (`gen-realm.py` from the seed roster) already provisions users,
  the confidential client, and the `tenant_id` mapper, so Keycloak itself is the user store in
  dev and CI -- no upstream IdP and no brokering needed there. The in-process integration rig
  follows via a container-provisioned realm (same generated JSON, `--import-realm`); tests that
  need no real HTTP IdP keep mocking at the OIDC-client seam.

```text
customer IdPs (Entra / Okta / SAML / ...) ─┐
social (GitHub / Google / Facebook / Apple)┴─brokered──▶ Keycloak realm ──OIDC──▶ authenticator ──▶ gateway JWT ──▶ services
```

### Consequences

- Onboarding a customer IdP or enabling a social provider becomes a realm-YAML pull request plus
  a sealed secret -- no authenticator change, no per-IdP code, no admin-UI session.
- The authenticator gains one bounded config change: `issuerUrl` becomes a host-keyed issuer map
  (OIDC client and JWKS caches keyed by issuer); a single-entry map is the dedicated-install
  degenerate case, so existing deployments are untouched.
- The authenticator's configurable tenant-claim name (`idp.tenant_claim`) loses its purpose once
  every environment is brokered -- the broker always emits `tenant_id`. It is defaulted to
  `tenant_id` and frozen (kept as a chart value for third-party consumers wiring a non-broker
  IdP directly, mirroring ADR-0002's `authDisabled` precedent), not removed.
- The per-IdP branching at our edge (audience/scope quirks, `account_person_map`-style seams)
  collapses: the authenticator sees one issuer shape, and claim normalisation lives in versioned
  mapper definitions.
- Keycloak moves from "stand IdP" (ADR-0002) to a **production-path dependency**: availability,
  upgrade cadence, and realm-change procedure now need the same ownership rigour ADR-0002 already
  demanded for stands, extended to production environments.
- Two token hops exist upstream of the session (upstream IdP -> broker, broker -> authenticator).
  Session lifetime follows the **broker's** refresh tokens; upstream-IdP revocation reaches us
  only as fast as the broker learns of it (its own token validation against the upstream). The
  PoC sharpened this: between Keycloak instances the upstream can push logout into the broker
  (the `keycloak-oidc` provider's endpoint), but a **non-Keycloak** upstream has no inbound
  back-channel path into a Keycloak 26 broker at all -- for those, the broker realm's SSO
  session and token lifetimes bound the revocation delay and become a per-realm tuning knob.
- Retiring fakeidp deletes a maintained service and the prod-versus-test issuer drift, at the
  cost of Keycloak start-up wherever a live login flow is exercised; the in-process rig keeps
  sub-second tests by mocking at the client seam.
- The broker inherits the redirect-URI registration story: social providers and customer IdPs
  register **one** callback (the broker's), not one per environment consumer.

### Confirmation

The proof of concept ran 2026-08-04 as the adoption EPIC's Phase 0 gate (compose stack; the
findings notes and the reproducible realm YAML live on the Phase 0 issue, #2194). Verdict:
**go** -- every gated behaviour passed, with one Keycloak limitation recorded below.

- **Broker login path -- pass.** A broker realm defined only in config-cli YAML, brokering one
  upstream OIDC provider, logs in through the **unchanged** authenticator (the published image;
  only `issuerUrl` and client secret re-configured) and mints a gateway JWT carrying the single
  string `tenant_id`; an API call through the gateway is accepted downstream (that verifier
  fails closed without `tenant_id`). Single-IdP auto-redirect (identity-provider redirector)
  and a prompt-free first broker login (upstream-asserted email + name, `trustEmail`) both
  work declaratively.
- **Refresh-token passthrough -- pass.** The background refresher rotates broker-issued refresh
  tokens on schedule under strict rotation (`revokeRefreshToken`, zero reuse); `offline_access`
  requested via authenticator configuration yields offline tokens that rotate identically. The
  broker does not background-refresh its stored upstream tokens -- session lifetime follows the
  broker, as the consequences above record. Revoked sessions fail closed at the next refresh
  (`invalid_grant` -> session revoked), with no false logouts observed across rotations.
- **Logout propagation -- pass, with a recorded gap.** Broker-to-authenticator uses spec OIDC
  Back-Channel Logout (client `backchannel.logout.url`); the authenticator matches `iss`+`sid`
  and revokes the session. Upstream-to-broker: Keycloak 26's generic `oidc` provider exposes
  **no inbound back-channel receiver** (a spec logout token from the upstream has nowhere to
  land); between Keycloak instances, the `keycloak-oidc` provider type plus the upstream
  client's `adminUrl` (`k_logout` push) delivers the full cascade: upstream logout -> broker
  session terminated -> back-channel logout -> authenticator session revoked. The gap and its
  mitigation for non-Keycloak upstreams are recorded in the consequences above.
- **Config-as-code mechanics -- pass**, with operational notes for the gitops sync job
  (Phase 1): config-cli skips checksum-unchanged files, so drift reverts only when the file
  changes unless the import cache is disabled; client-attribute removal needs an explicit empty
  value (absent keys are merged, not deleted); an identity provider's `providerId` cannot change
  in place (recreating it drops federated-identity links, forcing an account-relink step on
  next login); environment-variable substitution also scans YAML comments.
- `cfs validate --local-only` passes for this ADR, the amended ADR-0002, and the DESIGN.
- On an environment with a social provider enabled, a login whose email matches no person is
  refused with the unknown-person audit event; other configured providers keep working (#2163
  acceptance criteria).
- `git grep` finds no secret material in realm YAML: credentials resolve only through
  environment-variable substitution at apply time.

## Pros and Cons of the Options

### Option A -- Keycloak broker + keycloak-config-cli, chosen

- Good, because provider coverage is built in -- GitHub, Google, Facebook, Apple (KC 24+),
  Microsoft, LinkedIn, plus OIDC and SAML brokering for customer IdPs -- no adapters to write.
- Good, because claim shaping is first-class: per-provider mappers inject `tenant_id`, and the
  protocol-mapper allow-list means upstream claims do not leak into tokens by default.
- Good, because the operational investment is already made: ADR-0002 provisioned shared Keycloak,
  and the compose stack already generates and imports a realm; this extends working mechanisms.
- Good, because `keycloak-config-cli` is idempotent, actively maintained, uses the realm-export
  schema, and substitutes secrets from the environment -- a clean gitops fit.
- Bad, because Keycloak is a JVM with its own database and a fast upgrade cadence, now on the
  production login path -- weight the config-as-code discipline mitigates but does not remove.
- Bad, because refresh and logout semantics compose across two hops and must be proven (the PoC
  confirmation), not assumed.

### Option B -- Dex

- Good, because it is a single Go binary whose only configuration mode is a file -- gitops-native
  by construction, with connectors for GitHub, Google, Microsoft, LinkedIn, OIDC, SAML.
- Bad (fatal), because Dex cannot inject custom claims -- the `tenant_id` failure ADR-0001
  already verified end-to-end -- and offers no per-provider claim allow-listing.
- Bad (fatal), because there is no Apple and no Facebook connector, so it cannot cover the
  stated social-login roadmap.

### Option C -- multi-issuer support in the authenticator

- Good, because it adds no new runtime dependency and keeps every behaviour in one codebase.
- Bad, because it rebuilds a broker inside a security-critical service: per-provider adapters
  (including a bespoke GitHub OAuth-to-OIDC shim), N sets of refresh/logout quirks, and a
  provider-selection UI contract -- permanent bespoke surface where Option A uses commodity.
- Bad, because it contradicts the design's own boundary: the authenticator is deliberately a
  single-issuer RP (`issuerUrl` is one config value), and every ADR so far has preserved that.

### Option D -- another self-hosted broker (Zitadel / Authentik / Casdoor)

- Good, because each supports brokering and declarative configuration to some degree (Terraform
  provider, blueprints), and Casdoor could reuse the existing MariaDB.
- Bad, because none matches Keycloak's brokering maturity plus built-in social coverage
  (Apple/Facebook gaps or weaker SAML), and adopting one now would strand the Keycloak mechanisms
  ADR-0002 and the compose stack already run.
- Rejected as not better where it matters; Casdoor remains noted in ADR-0001's record.

## More Information

- Investigation report with the full comparison and config sketches: #1782
  (issuecomment-5176199376); social-login requirements: #2163.
- Existing realm generation: `deploy/compose/keycloak/README.md`, `deploy/compose/keycloak/gen-realm.py`
  (roster-driven users, `tenant_id` protocol mapper, confidential client) -- the dev/CI half of
  this decision, already in place.
- Realm-content tooling: [`adorsys/keycloak-config-cli`](https://github.com/adorsys/keycloak-config-cli).
  The Keycloak Operator's `KeycloakRealmImport` was considered for realm content and set aside:
  import-oriented, weak day-2 updates; the Admin API v2 declarative resources are still preview.
  The Terraform provider is mature but introduces state management foreign to the gitops flow.
- Migration order: provision broker realms as code first; move social providers and new customer
  IdPs behind the broker immediately; re-point each environment's `issuerUrl` from its directly
  wired IdP to the broker realm as it is onboarded -- per environment, no flag day; retire
  fakeidp last, once compose and CI default to the Keycloak realm. **Done 2026-08-07 (#2198)**:
  fakeidp is deleted; compose, CI and the e2e rigs all run the Keycloak roster realm.

## Traceability

- Resolves the investigation this component's ADRs deferred twice: ADR-0001 ("production
  IdP/broker deferred") and ADR-0002 ("the production IdP choice remains deferred to #1782").
- Amends `cpt-insightspec-adr-auth-0002-real-idp-on-deployed-stands`: its fakeidp survival
  clause (compose, in-process rig, CI scaffold) is retired; its shared pre-provisioned Keycloak
  and realm-generation decisions are extended, unchanged, to brokering.
- Realises the environment wiring behind `cpt-insightspec-fr-auth-oidc-login` without changing
  that contract; the single-`tenant_id` token shape of DD-AUTH-04 is preserved by broker-side
  mappers.
