---
status: accepted
date: 2026-05-04
decision-makers: platform-engineering
---

# ADR-0003: Credential Rotation via `sources/update`, Not Environment-Variable Injection


<!-- toc -->

- [Context and Problem Statement](#context-and-problem-statement)
- [Decision Drivers](#decision-drivers)
- [Considered Options](#considered-options)
- [Decision Outcome](#decision-outcome)
  - [Consequences](#consequences)
  - [Confirmation](#confirmation)
- [Pros and Cons of the Options](#pros-and-cons-of-the-options)
  - [Option A — `SecretPersistence` plugin pointing at external secret store](#option-a--secretpersistence-plugin-pointing-at-external-secret-store)
  - [Option B — Inject credentials as environment variables in the Airbyte worker](#option-b--inject-credentials-as-environment-variables-in-the-airbyte-worker)
  - [Option C — `sources/update` on cfg-hash mismatch](#option-c--sourcesupdate-on-cfg-hash-mismatch)
- [More Information](#more-information)
- [Traceability](#traceability)

<!-- /toc -->

**ID**: `cpt-insightspec-adr-credential-rotation-no-env`
## Context and Problem Statement

Connector credentials live in K8s Secrets in the `data` namespace. When an admin rotates a credential (in 1Password, propagated through the operator), the new value lands on the K8s Secret. Airbyte, however, stores its copy of the credential inside `source.connectionConfiguration` in the Airbyte database. After Secret rotation, the Airbyte source still reads the old value until something writes the new value to it.

We considered whether Airbyte sync workers could read credentials directly from K8s at sync time — bypassing the database copy entirely — so that rotation in K8s would propagate without any toolkit involvement. This question matters because the obvious alternative (`sources/update` after every rotation) requires the toolkit to be invoked on rotation events.

## Decision Drivers

- **Operator simplicity**: ideally, the admin rotates a Secret and walks away.
- **Airbyte OSS compatibility**: solution must work on the OSS Airbyte we deploy, not just Cloud/Enterprise.
- **No fork or patch of Airbyte**: any modification to Airbyte's worker pod spec or image breaks our upgrade path.
- **State preservation**: rotation must not invalidate connection sync state.
- **Observable**: when rotation happens, there must be a clear log of what was updated when.

## Considered Options

- **Option A** — Configure Airbyte's `SecretPersistence` to point at HashiCorp Vault, AWS Secrets Manager, or GCP Secret Manager. Airbyte stores references; values are fetched at sync time from the external store. Rotation in the external store propagates to Airbyte automatically.
- **Option B** — Inject credentials as environment variables into the Airbyte sync worker pod via a custom worker image or a sidecar; modify connectors (CDK or declarative) to read from `os.environ` instead of `connectionConfiguration`.
- **Option C** — Continue storing credentials in `source.connectionConfiguration`; on rotation, the toolkit detects the Secret change (via `cfg-hash:<sha256>` tag mismatch) and calls `POST /api/v1/sources/update` with fresh values. `sources/update` is idempotent and does not invalidate the connection's `connectionId` or sync state.

## Decision Outcome

Chosen option: **Option C — `sources/update` on cfg-hash mismatch**.

**Justification**: in Airbyte OSS, the `SecretPersistence` plugin (`SECRET_PERSISTENCE` env var on `airbyte-server`) supports HashiCorp Vault, AWS Secrets Manager, and GCP Secret Manager — but **not** K8s Secrets as a backend. Adopting Option A would require provisioning Vault or AWS SM and migrating credentials there, which is a significant infrastructure investment for a single rotation problem. Option B requires modifying the Airbyte worker image or pod spec, which breaks our upgrade path and introduces a maintenance burden. Option C accepts that rotation requires a single API call but achieves it with one idempotent operation that the toolkit already needs for other reasons (initial create, version bump cascade). It works on every Airbyte deployment we run today.

### Consequences

- **Good**, because no infrastructure addition: works on virtuozzo and any future cluster as-is.
- **Good**, because state is preserved: `sources/update` is documented to keep the source's `sourceId` and the connection's `connectionId`.
- **Good**, because rotation cost is one API call per rotated Secret, not a full reconcile cycle.
- **Good**, because the rotation event is observable via Airbyte's audit logs (`sources/update` is a discrete, logged operation).
- **Neutral**, because the toolkit must be invoked on rotation events. Triggering this is left to operators / cron / pre-sync hooks (out of ADR scope; see `cpt-insightspec-component-reconcile-engine`).
- **Bad**, because credentials still live encrypted in Airbyte's database — exposure surface is unchanged from the pre-refactor state. Mitigation: rely on Airbyte's `AIRBYTE_SECRET_PERSISTENCE_KEY` for at-rest encryption.
- **Bad**, because if the toolkit is never invoked after rotation, syncs continue with stale credentials until they fail authentication. Mitigation: run reconcile as a pre-sync step in the Argo workflow (recommended; out of ADR scope).

### Confirmation

- After rotating a Secret in K8s, running `reconcile-connectors.sh` shows the affected connector's `cfg-hash` tag updated and a single `sources/update` API call in the toolkit's log.
- The connection's `connectionId` is unchanged before vs after; running the next sync produces results without re-fetching from cursor zero.
- Airbyte's API audit log records the `sources/update` event with a non-zero diff between previous and current `connectionConfiguration`.

## Pros and Cons of the Options

### Option A — `SecretPersistence` plugin pointing at external secret store

Airbyte stores secret references; values are resolved at sync time from Vault / AWS SM / GCP SM. The K8s Secret would not be the source of truth — the external store would.

- Good, because rotation in the external store propagates to Airbyte without toolkit involvement.
- Good, because Airbyte's database stops holding plaintext credentials (only references).
- Bad, because OSS Airbyte does not support K8s Secrets as a backend — would require provisioning Vault / SM as new infrastructure.
- Bad, because adds a runtime dependency: every sync requires the secret backend reachable.
- Bad, because forces a global migration of every existing connector credential out of K8s into the external store.

### Option B — Inject credentials as environment variables in the Airbyte worker

Modify the Airbyte worker pod (or sidecar) so credentials are mounted from K8s Secrets as env vars; modify connectors to read from `os.environ`.

- Good, because rotation in K8s Secret is picked up at the next pod restart (and sync workers are short-lived).
- Bad, because requires patching every connector to read env instead of `connectionConfiguration` — including upstream-maintained declarative manifests we cannot fork.
- Bad, because Airbyte expects `connectionConfiguration` to be the source of credentials; injecting env vars side-channels around the contract and breaks UI display, validation, and connection schema discovery.
- Bad, because patched worker image requires manual re-application on every Airbyte upgrade.

### Option C — `sources/update` on cfg-hash mismatch

Compute `cfg-hash:<sha256(secret.data)>` per Secret. The reconcile engine compares against the `cfg-hash:` tag on the corresponding connection. On mismatch, call `sources/update` with fresh credentials and re-tag the connection.

- Good, because uses only documented Airbyte APIs already in use elsewhere in the toolkit.
- Good, because preserves state (no recreation of source or connection).
- Good, because cfg-hash provides cheap drift detection — no API call when nothing changed.
- Neutral, because requires the toolkit to run after rotation. Acceptable when reconcile is a pre-sync step.
- Bad, because credentials remain in Airbyte's encrypted-at-rest store. Same exposure as today.

## More Information

- Airbyte OSS `SecretPersistence` documentation lists supported backends: testing-local, GCP_SECRET_MANAGER, AWS_SECRET_MANAGER, VAULT. K8s is not in the list as of Airbyte 2026.04.
- Airbyte API: `POST /api/v1/sources/update` accepts the same payload as `create` minus `workspaceId`; sources cumulatively update without invalidating downstream connections.
- Related decisions:
  - `cpt-insightspec-adr-version-driven-reconcile` (ADR-0001) — defines version anchor; cfg-hash is the per-instance complement.
  - `cpt-insightspec-adr-adoption-of-existing-resources` (ADR-0002) — adoption uses the same `cfg-hash` tag scheme.

## Traceability

- **PRD**: [PRD.md](../PRD.md)
- **DESIGN**: [DESIGN.md](../DESIGN.md)

This decision directly addresses:

- `cpt-insightspec-fr-cli-surface` — `sources/update` is one of the operations exposed via the reconcile CLI.
- `cpt-insightspec-fr-version-driven-reconcile` — config drift detection complements version drift detection.
- `cpt-insightspec-component-reconcile-engine` — owns the cfg-hash comparison and `sources/update` call.
- `cpt-insightspec-component-secret-discovery` — computes the desired `cfg-hash` value.
- `cpt-insightspec-component-secret-validator` — surfaces structural drift on the K8s side before reconcile runs.
