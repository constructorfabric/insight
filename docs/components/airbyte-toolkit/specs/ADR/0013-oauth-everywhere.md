---
status: accepted
date: 2026-05-08
decision-makers: platform-engineering
---

# ADR-0013: All Airbyte API calls authenticate via OAuth client_credentials

<!-- toc -->
<!-- /toc -->

**ID**: `cpt-insightspec-adr-oauth-everywhere`

## Context and Problem Statement

Reconcile, adoption, and the Argo `airbyte-sync` WorkflowTemplate all need to call Airbyte's HTTP API. Three auth flows existed in the code base:

1. **Self-minted HMAC JWT** — `airbyte-sync` (trigger-sync, poll-job) signs a JWT directly with `jwt-signature-secret` and sets `sub=00000000-...`.
2. **Pre-issued bearer token in a file** — early reconcile read `${AIRBYTE_TOKEN_FILE}`.
3. **OAuth client_credentials** — current reconcile mints via `POST /api/v1/applications/token` using `instance-admin-client-id` / `instance-admin-client-secret` from `airbyte-auth-secrets`.

In Airbyte 1.7+ the self-minted JWT flow is rejected at the security filter for write-class endpoints, but the request hangs ~5 minutes and then resets without a meaningful body. The mode of failure is opaque (no log, no 4xx), and operators see a stuck Argo workflow rather than a clear "auth invalid" message.

## Decision Drivers

- **Single auth surface** — one mechanism to configure, debug, and rotate credentials for.
- **Compatibility with Airbyte 1.7+** — the only flow Airbyte advertises and supports for external clients.
- **Clear failure mode** — auth errors must surface as 401 / 403 with a body, not as a TCP hang.
- **No bespoke crypto** — avoid maintaining HS256 JWT minting in templates.

## Considered Options

- **Option A** — Keep self-minted HMAC JWT in `airbyte-sync` for backwards compatibility.
- **Option B** — Use OAuth client_credentials everywhere (CHOSEN).
- **Option C** — Split: OAuth for reconcile, HMAC for workflow templates.

## Decision Outcome

Chosen option: **Option B — OAuth everywhere**.

**Justification**: Airbyte's OAuth client_credentials flow is the documented and supported way to obtain a bearer token. The `instance-admin-*` credentials live in `airbyte-auth-secrets`, the same Secret that already holds `jwt-signature-secret`, so the chart wiring cost is one extra `secretKeyRef` per script container (no new Secret to create). Token TTL is short, so each script call mints fresh; for the polling loop in `poll-job` we re-mint per iteration to survive long-running syncs.

`airbyte-sync` WorkflowTemplate's `trigger-sync` and `poll-job` script blocks are rewritten to call `/api/v1/applications/token` for their bearer instead of HMAC-signing JWT locally. The HMAC code path and `jwtSecret` value reference are removed from those scripts. (The `jwtSecret` Helm value stays in `values.yaml` for now — other unrelated tooling may still reference it; it can be retired in a follow-up.)

### Consequences

- **Good**, because every API call now fails with 401 / 403 + JSON body when auth is wrong, not a 5-minute TCP hang.
- **Good**, because credential rotation is a single Secret update — no per-component re-issuance.
- **Good**, because operator gets a coherent story: "Airbyte auth = `airbyte-auth-secrets`".
- **Bad**, because `applications/token` requires reachability of `airbyte-server-svc` from the workflow pod. The reconcile SA already needs that for resolve-by-name; same network reachability covers OAuth mint.
- **Bad**, because each script invocation incurs one extra HTTP round-trip (~10 ms in cluster) to mint a token. Negligible vs the actual sync duration.

### Confirmation

- `airbyte-sync` Workflow on dev-vhc 1.8.5 runs end-to-end (resolve → trigger → poll → succeed) using OAuth bearer.
- `reconcile-connectors.sh` already uses the same flow (`ab_get_token`).
- Self-mint code path is removed from chart templates; `grep -r 'mint_token\|HS256\|hmac.new' charts/` returns nothing under `templates/ingestion/`.

## Pros and Cons of the Options

### Option A — Keep HMAC JWT

- Good, because it works on pre-1.7 Airbyte without changes.
- Bad, because broken on 1.7+ for write endpoints, with opaque failure.
- Bad, because mints crypto in template strings — security review surface.

### Option B — OAuth everywhere

- Good, see Decision Outcome.
- Neutral, because slightly more env wiring.

### Option C — Split

- Bad, because two auth paths to debug.
- Bad, because no benefit over Option B once the chart already mounts auth secret.

## More Information

- Implementation: `ab_get_token` in [airbyte.sh](../../../../src/ingestion/reconcile-connectors/lib/airbyte.sh); `trigger-sync` / `poll-job` script blocks in [airbyte-sync.yaml](../../../../charts/insight/templates/ingestion/airbyte-sync.yaml).
- Helm value: `airbyte.authSecret.{name,clientIdKey,clientSecretKey}`.
- Related decisions:
  - `cpt-insightspec-adr-airbyte-workspace-as-namespace` (ADR-0009) — workspace auto-discovery uses the same auth.

## Traceability

- **PRD**: [PRD.md](../PRD.md)
- **DESIGN**: [DESIGN.md](../DESIGN.md)
- **FEATURE**: [feature-reconcile/FEATURE.md](../feature-reconcile/FEATURE.md)
