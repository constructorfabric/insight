# Worked example — scoping tests for a security-architecture EPIC (specs-first)

The fourth shape: not a data feature at all, but a **security/infrastructure replacement**, scoped
before any code exists —
[constructorfabric/insight#1583](https://github.com/constructorfabric/insight/issues/1583)
"PLATFORM 1: nginx + authenticator". It replaces a Rust API gateway with an nginx/OpenResty edge +
a new `authenticator` gear (OIDC + PKCE, Redis sessions, a signed gateway JWT, zero-trust
downstream verification, service tokens). An EPIC with 11 implementation sub-issues.

This example exists so the skill doesn't force its data-feature habits (differential gates, gold
lineage) onto a security feature, and so it remembers to make the scope *operational*.

## What made the shape different

- **No differential.** The old gateway is replaced outright — no installed baseline to
  preserve, nothing to reproduce. Forcing a "reproduces the old behavior" gate would be wrong. The gate is **security
  properties hold** + the spec's own acceptance criteria.
- **EPIC altitude.** The 11 sub-issues own their component tests; this scope is the cross-cutting
  **end-to-end security acceptance of the assembled system** — the tests that only exist once the
  pieces fit together. Say that, so you don't re-scope the sub-issues.
- **Specs-first — no code yet.** The implementation lived on a *merged* branch as PRD/DESIGN docs
  only (found via `gh pr list` / `git branch -r`; read with `git show <branch>:<path>`). For a
  security design that's fine — grounding in the spec is grounding, and the PRD's own §9 acceptance
  criteria + metric labels are the test oracles. A fake provider's **control hooks** (revoke /
  outage / no-refresh / back-channel) are what the ACs are literally written against — a real IdP
  can't fire "outage now," so the fake is load-bearing, not a shortcut.

## The moves that made it usable

1. **Centre on the unhappy path.** Logging in and seeing a dashboard proves almost nothing here.
   The headline group is a **fail-closed matrix** — every bad identity state (no/expired/revoked
   cookie, forged/expired/wrong-issuer/bad-sig JWT, spoofed headers) against each endpoint must
   return 401/403 with nothing reaching upstream.
2. **Minimize at the choke point** (the reason the final scope is small). Enforcement is
   centralized: routes are *generated* from one config, downstream verification is one shared
   middleware, revoke is one pipeline, there's one JWT-verification path. So test each property
   **once where it's enforced** + a cheap invariant it's applied everywhere (a golden-file check
   that *every* generated `/api/` route carries the auth block) — not per route / per service /
   per caller. The cuts are safe because the architecture forces those cases identical.
3. **Lead with the stand.** The first simplification passes were still abstract — the reader
   didn't know *what to bring up or run*. The fix was a short **The stand** section (nginx edge +
   authenticator + Redis + one downstream + SPA, logging in via the fake IdP; compose except the
   NetworkPolicy checks; Dex for one conformance smoke) and rewriting every group to start with an
   action (log in / hit / revoke / kill). A security integration scope needs its environment named;
   the data-feature scopes got away without it only because their stand (ClickHouse + dbt) was
   implicit.
4. **`xfail`-as-gate for deferred behavior, on the user's call.** Warehouse cross-tenant isolation
   is deliberately not implemented (single-tenant posture). Gating on it would fail by design — but
   the user chose to keep it in scope as executable `xfail` so the known gap stays visible and
   turns green when the tenant predicate lands.
5. **Ground surfaced spec ambiguities → pre-work, not tests.** Reading the two DESIGN docs turned
   up genuine internal inconsistencies (the exchange cache key: cookie token vs stable
   `session_id`; whether `/auth/refresh` needs CSRF; 403-vs-401 deny shaping; the open JWT `alg`).
   These became a short "Resolve before writing tests" list — cheap, and skipping them loses
   correctness.

## The shape it settled into

> Goal (fail-closed + PRD §9; unhappy path) → one framing paragraph (no differential; choke-point
> minimization) → **The stand** (what to bring up) → **What to test** (5 action-first groups:
> session lifecycle · fail-closed matrix · revocation & IdP-kill · service tokens · infra
> fail-closed & cutover) → Resolve-first (the 4 spec gaps) → Out of scope → Acceptance.

## What to notice

- The same "lean + action-first + scannable" altitude as the other examples, plus a **stand**
  section on top — the differentiator for anything that needs a new environment.
- The minimization is *principled*: it names the choke point that makes each cut safe. "We test it
  once because the architecture forces every route identical" is a cut a reviewer can trust;
  "we'll just test a few" is not.
- Three simplification rounds happened before it was usable. Expect that: the first draft of a
  broad security scope reads like a spec. Cut meta-commentary (the "why it's minimal" reasoning
  belongs in the chat, not the ticket), keep the operational spine.
