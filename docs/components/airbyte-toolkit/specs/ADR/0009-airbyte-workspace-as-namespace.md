---
status: accepted
date: 2026-05-06
decision-makers: platform-engineering
---

# ADR-0009: Insight connectors live in one Airbyte workspace, identified by `custom: true`


<!-- toc -->

- [Context and Problem Statement](#context-and-problem-statement)
- [Decision Drivers](#decision-drivers)
- [Considered Options](#considered-options)
- [Decision Outcome](#decision-outcome)
  - [Consequences](#consequences)
  - [Confirmation](#confirmation)
- [Pros and Cons of the Options](#pros-and-cons-of-the-options)
  - [Option A — Prefix descriptor names](#option-a--prefix-descriptor-names)
  - [Option B — Use Airbyte's `custom: true` flag](#option-b--use-airbytes-custom-true-flag)
  - [Option C — Separate Airbyte workspace + filter](#option-c--separate-airbyte-workspace--filter)
- [More Information](#more-information)
- [Traceability](#traceability)

<!-- /toc -->

**ID**: `cpt-insightspec-adr-airbyte-workspace-as-namespace`

## Context and Problem Statement

Airbyte stores `source_definitions` globally per-instance, keyed by name. Public registry definitions (e.g., `m365 7.18.1` published by Airbyte) coexist in the same store with custom-built ones. Without a namespace separator, `list_for_workspace` followed by filter-by-name returns both, and our reconcile loop cannot tell which definition is "ours". This caused real prod-like bugs on dev-vhc where adopt updated a public registry definition or skipped our custom one entirely. We need a deterministic, no-rename way to identify Insight-managed definitions inside a single Airbyte instance.

## Decision Drivers

- **Deterministic namespace** that survives Airbyte version upgrades and registry refreshes.
- **Zero rename cost** — the 16 existing connector descriptors and secret examples must not be touched.
- **Operator simplicity** — no extra workspace UUID management; consumers should not have to know the UUID.
- **Fail-fast on ambiguity** — if the Airbyte instance ever holds more than one workspace, reconcile must refuse to run rather than guess.
- **Compatibility with reconcile** — filter MUST be expressible as a single attribute check inside ID-iteration code.

## Considered Options

- **Option A** — Prefix descriptor names (`insight-m365`, `insight-zoom`, …).
- **Option B** — Use Airbyte's `custom: true` flag on every Insight definition (CHOSEN).
- **Option C** — Separate Airbyte workspace per Insight instance + name filter.

## Decision Outcome

Chosen option: **Option B — single dedicated workspace + `custom: true` filter**.

**Justification**: every custom-built Airbyte definition has `custom: true`; built-in registry definitions are `custom: false`. The filter is a one-line check (`if def.custom != true: skip`), no rename is required, and no extra workspace lifecycle is introduced.

The workspace UUID itself is **discovered at runtime**, not configured. Reconcile calls `POST /api/v1/workspaces/list_by_organization_id` with the Airbyte built-in default organization id (`00000000-0000-0000-0000-000000000000`) and asserts exactly one workspace. This is implemented in `ab_workspace_id` ([airbyte.sh](../../src/ingestion/reconcile-connectors/lib/airbyte.sh)) and is the **only** path used by reconcile, adoption, GC, and the migrate-orphan tool. Rationale: every supported deploy of Insight runs against a single-workspace Airbyte instance (DESIGN §2.2); the operator already provisions the instance — making them re-type its workspace UUID into Helm values is needless ceremony, prone to drift (the value can be wrong even when Airbyte is healthy, leading to silent "0 connectors" runs), and gives no additional safety the `custom: true` filter does not already provide. Fail-fast is preserved: if `len(workspaces) != 1`, `ab_workspace_id` exits non-zero and reconcile aborts with a clear stderr message.

Multi-tenant isolation continues to be expressed at the descriptor / connection-name level, not workspace level.

### Consequences

- **Good**, because all ID-iteration code (`extract_definition_ids.py` + inline filters) reduces to `def.custom == true`.
- **Good**, because the workspace UUID is auto-discovered — no Helm value, no env var, no operator typo surface.
- **Good**, because public registry connectors with the same name as ours never collide.
- **Good**, because zero descriptor / example rename cost (16 connectors untouched).
- **Bad**, because a second Insight installation in the same Airbyte instance would share the same `custom: true` namespace; multi-tenant isolation must therefore be handled at descriptor / connection-name level (this matches the existing single-workspace constraint in DESIGN §2.2).

### Confirmation

- `reconcile-connectors.sh --dry-run` against a workspace containing both a public `m365` (custom=false) and an Insight `m365` (custom=true) reports exactly one definition under management.
- `ab_workspace_id` exits non-zero with a clear stderr message when the Airbyte instance has zero or more than one workspace; reconcile aborts.
- CI smoke check: `extract_definition_ids.py` returns only `custom: true` IDs.

## Pros and Cons of the Options

### Option A — Prefix descriptor names

Rename every connector to `insight-<name>` so name-prefix matching disambiguates Insight definitions from public registry ones.

- Good, because filter is a string-prefix check, trivially expressible.
- Bad, because mass-rename of 16 descriptors + secret examples + workflows + dashboards.
- Bad, because Airbyte UI displays the prefixed name (operator UX regression).
- Bad, because future connectors must remember the convention; one missed prefix breaks the filter.

### Option B — Use Airbyte's `custom: true` flag

Every custom-built definition already carries `custom: true`; built-in registry definitions are `custom: false`. Filter on this attribute.

- Good, because Airbyte already maintains the flag — no new convention to enforce.
- Good, because zero rename cost.
- Good, because filter is a single attribute check.
- Neutral, because the flag is set by Airbyte at registration time, not by us; we rely on a soft contract that custom-built definitions always carry it (verified by current Airbyte versions).
- Bad, because two parallel Insight installations in the same Airbyte instance would share the namespace — but this is forbidden by the single-workspace constraint anyway.

### Option C — Separate Airbyte workspace + filter

Provision a dedicated Airbyte workspace for Insight, filter by `workspace_id == INSIGHT_AIRBYTE_WORKSPACE_ID`.

- Good, because hard isolation at workspace level.
- Bad, because operator must provision and track an extra workspace UUID.
- Bad, because `custom: true` already gives namespace isolation — this option layers a second mechanism with no extra benefit.
- Bad, because cross-workspace tooling (UI, CLI) becomes more cumbersome.

## More Information

- The `custom: true` flag is a stable attribute of `source_definitions` across Airbyte 0.50+ versions in our deployment matrix.
- Workspace identity is auto-discovered via `ab_workspace_id` ([airbyte.sh](../../src/ingestion/reconcile-connectors/lib/airbyte.sh)) — no env var, no Helm value, no CLI flag.
- Related decisions:
  - `cpt-insightspec-adr-version-driven-reconcile` (ADR-0001) — overall reconcile flow that consumes the filter.
  - `cpt-insightspec-adr-nocode-via-builder-projects` (ADR-0010) — nocode publish flow that depends on this namespace.

## Traceability

- **PRD**: [PRD.md](../PRD.md)
- **DESIGN**: [DESIGN.md](../DESIGN.md)
- **FEATURE-reconcile**: [feature-reconcile/FEATURE.md](../feature-reconcile/FEATURE.md) — algorithm `cpt-insightspec-algo-reconcile-filter-custom-definitions` for the filter.

This decision directly addresses:

- `cpt-insightspec-fr-version-driven-reconcile` — Insight-namespace identification underpins the reconcile diff.
- `cpt-insightspec-fr-orphan-gc` — orphan GC must skip non-custom (public) definitions.
- `cpt-insightspec-component-reconcile-engine` — consumes the filter on every iteration.
