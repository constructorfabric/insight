---
status: accepted
date: 2026-04-23
---

# No-whitelist full-ingestion scope


<!-- toc -->

- [Context and Problem Statement](#context-and-problem-statement)
- [Decision Drivers](#decision-drivers)
- [Considered Options](#considered-options)
- [Decision Outcome](#decision-outcome)
  - [Consequences](#consequences)
  - [Confirmation](#confirmation)
- [Pros and Cons of the Options](#pros-and-cons-of-the-options)
  - [Option 1 — No whitelist](#option-1--no-whitelist)
  - [Option 2 — Project allowlist via K8s Secret](#option-2--project-allowlist-via-k8s-secret)
  - [Option 3 — Tenant-side scoping only (token permissions)](#option-3--tenant-side-scoping-only-token-permissions)
- [More Information](#more-information)
- [Traceability](#traceability)

<!-- /toc -->

**ID**: `cpt-insightspec-adr-youtrack-no-whitelist`
## Context and Problem Statement

The Jira connector accepts a `jira_project_keys` K8s Secret field that restricts ingestion to a specific list of projects. This was driven by Jira-side requirements (Phase 1 customer instances often had hundreds of projects, of which only a handful were relevant for analytics).

For YouTrack, we have to decide whether to mirror this — adding a `youtrack_project_short_names` K8s Secret field that restricts the manifest's JQL-equivalent query and the project-fan-out streams — or to ingest everything the permanent token can reach.

The decision affects:

- The K8s Secret shape (`youtrack.yaml.example`)
- The JQL-equivalent `query` parameter on `youtrack_issue`
- The substream parent partitioning on `youtrack_project_custom_fields`
- Operator UX (one allowlist to maintain vs none)
- Compliance / data residency posture (does the connector see projects it shouldn't?)

## Decision Drivers

- Operational simplicity — one moving part fewer to misconfigure.
- Permission-boundary alignment — the permanent token already constrains what the connector can read.
- Sync cost — ingesting irrelevant projects costs API calls but no extra silver-layer work.
- Reversibility — adding an allowlist later is easier than removing one.
- Customer feedback — early YouTrack tenants in the Insight pipeline are smaller (< 50 projects) and have fewer "private" projects than typical Jira instances.

## Considered Options

1. **No whitelist** — ingest every project the token can reach.
2. **Project-allowlist via K8s Secret** — mirror Jira's `jira_project_keys`.
3. **Tenant-side scoping only** — operators create a service-account token restricted to specific projects in YouTrack; the connector trusts the token's scope.

## Decision Outcome

Chosen option: **no whitelist** (Option 1) combined with reliance on token-scoped permissions (Option 3 as a soft policy, not enforced by the connector). The K8s Secret has no `youtrack_project_short_names` field. The manifest does not filter by project.

If a customer needs project-level scoping, the recommended path is to **issue a YouTrack permanent token with read access only to the desired projects**. YouTrack's permission model supports this natively at the token-creation step.

### Consequences

**Positive**:

- Smaller K8s Secret surface — operators do not maintain a separate allowlist.
- No drift between "what the token can see" and "what the connector ingests".
- Onboarding is faster — no per-project YAML editing during initial setup.
- Token rotation is the single point of change for scope adjustments.

**Negative**:

- Operators who want to ingest a *subset* of projects must work at the YouTrack-token layer rather than at the Insight-config layer. This is a workflow change for teams migrating from a Jira-style operator model.
- API cost is proportional to project count. For tenants with hundreds of projects, this adds noticeable directory-phase latency.
- 403 on individual projects (token has instance-level read but not project-level read on archived/restricted projects) must be soft-fail at the manifest error-handler layer (`cpt-insightspec-fr-youtrack-bronze-retry-policy`).

**Rejected — Option 2 (project allowlist)**: Adds a K8s Secret field whose purpose duplicates the token's permission boundary. Operators rotating the token without updating the allowlist would silently keep ingesting a stale project set. Adds a per-source UI requirement to maintain the allowlist as projects are added/archived.

**Reversal cost (if we change our mind later)**:

- Adding a `youtrack_project_short_names` field to the K8s Secret is a backwards-compatible change (optional field, defaulting to "all projects").
- Updating the manifest to filter by project key in the `youtrack_issue.query` and to filter the `youtrack_projects` parent partitions can be done in a single follow-up PR.
- This ADR can therefore be **revisited** after 6 months of operational data without a costly migration.

### Confirmation

Decision is confirmed when:

- `src/ingestion/secrets/connectors/youtrack.yaml.example` does NOT contain a `youtrack_project_short_names` field.
- `connector.yaml`'s `spec.connection_specification.required` does NOT list a project-allowlist key, and no stream's `query` or `request_parameters` filters by project key.
- Onboarding documentation in the connector README documents the alternative (token-scoped permissions in YouTrack) instead of an Insight-side allowlist.

## Pros and Cons of the Options

### Option 1 — No whitelist

- **Pros**: Smaller K8s Secret surface, no drift between token scope and ingestion scope, faster onboarding, single point of change for scope adjustments.
- **Cons**: Operators wanting a project subset must work at YouTrack token layer (not Insight config). API cost proportional to project count. Per-project 403s must soft-fail.

### Option 2 — Project allowlist via K8s Secret

- **Pros**: Symmetric with Jira; per-source operator UX consistent.
- **Cons**: Duplicates token permission boundary; allowlist can drift from token scope without warning; adds maintenance burden as projects are added / archived.

### Option 3 — Tenant-side scoping only (token permissions)

- **Pros**: Single source of truth (YouTrack's permission model).
- **Cons**: Identical to Option 1 in practice — Option 3 IS Option 1's recommended deployment path.

## More Information

- YouTrack permanent token permission model: <https://www.jetbrains.com/help/youtrack/devportal/Manage-Permanent-Token.html>.
- Equivalent decision in Jira (allowlist retained): `docs/components/connectors/task-tracking/jira/specs/PRD.md` §3.1 `jira_project_keys`.
- 6-month revisit horizon: Insight platform engineering review (Q3 2026 retrospective).

## Traceability

- Implements DESIGN `cpt-insightspec-constraint-youtrack-no-whitelist`.
- Pairs with Connector ADR-001 (project-scoped custom fields) — the per-project substream fans out across every project the token reaches.
- Differs from the equivalent Jira decision (which kept a `jira_project_keys` allowlist) — documented divergence under PRD `cpt-insightspec-principle-youtrack-symmetry-with-jira`.
