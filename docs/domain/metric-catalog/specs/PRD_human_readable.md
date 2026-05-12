# Metric Catalog — Plain-English Companion

This document is a human-readable summary of the Metric Catalog PRD. The canonical requirements, IDs, and acceptance criteria live in [`PRD.md`](./PRD.md). When the two disagree, `PRD.md` wins — but if you notice a disagreement, please flag it; drift between the two helps no one.

Audience: product, compliance, customer success, and engineers who want the "why" before diving into the formal spec.

---

## TL;DR in one paragraph

The Metric Catalog is a backend-owned registry that holds two kinds of facts about every metric Insight shows its users: **what it means** (label, unit, format, connector it depends on) and **what "good" looks like** (threshold values for bullet colors and alerts). Today, both kinds of facts are hardcoded across two repos (frontend and backend) and drift over time. v1 moves all of it into one MariaDB-backed store exposed via a read endpoint and a tenant-admin write endpoint, ships with per-tenant / per-role / per-team threshold variance, and includes a lock mechanism so regulated tenants can pin compliance-sensitive thresholds auditably.

---

## The problem

Today, if a compliance team at a tenant says *"we need the focus-time target stricter than engineering, effective Monday"*, the shortest path to that outcome is:

1. A customer-support ticket.
2. A frontend engineer edits `thresholdConfig.ts`.
3. Code review, CI, frontend deploy.
4. The threshold is now live — for **every** tenant, because it's global.
5. If another tenant disagrees, the whole dance repeats in the opposite direction.

Layered on top: metric metadata (the label "Focus time", the unit suffix "%", whether higher is better) also lives in that same `thresholdConfig.ts`, while the metric itself is defined on the backend in `analytics.metrics.query_ref`. Rename a metric on the backend without updating the frontend and the bullet chart silently falls back to generic placeholders nobody notices for weeks.

The catalog replaces that story:

- Metadata goes into a backend MariaDB table, loaded by the frontend via a cacheable read endpoint.
- Thresholds are per-tenant and editable through an admin API — no frontend deploy for a threshold change.
- A small number of scopes (company / role / team / team+role) let different audiences inside a tenant have different bars for the same metric.
- For compliance-sensitive metrics, an admin can **lock** a threshold so narrower scopes can't override it — with an audit trail of who set the lock and who tried to bypass it.

---

## What v1 ships

**Storage:**

- One MariaDB table (`metric_catalog`) with one row per metric. Global, not per-tenant, because metadata is product-level contract.
- One MariaDB table (`metric_threshold`) with per-tenant rows for thresholds. Same metric can have different thresholds in different companies, for different roles, on different teams, or specifically for "this role on this team".
- One MariaDB table (`threshold_lock_audit`) recording every lock-set, lock-clear, and lock-bypass-attempt for at least a year.

**API:**

- `GET /catalog/metrics?role_slug=…&team_id=…` — returns the catalog with resolved thresholds for the caller's `(tenant, role, team)` context. One call hydrates the frontend for an entire session. Cacheable for 5 minutes with cross-replica invalidation.
- `POST/PUT/DELETE /v1/admin/metric-thresholds` — tenant-admin CRUD on thresholds. Validates scope shape, numeric sanity (`warn ≤ good` where `higher_is_better = true`, etc.), and refuses writes shadowed by a broader lock.

**Migration from today:**

- A seed migration imports the current `thresholdConfig.ts` into the catalog so day one looks identical to today.
- The frontend hydrates from the catalog but keeps the old constants as a fallback for one transitional release, then removes the fallback in the immediately following release.
- The orphaned empty `analytics.thresholds` table gets explicitly renamed, dropped, or kept as a generic store — decision lives in the DESIGN doc.

---

## How it works in practice

### Scenario 1 — Tenant admin tunes a threshold

A compliance team at Acme wants a stricter focus-time target than the product default.

1. Admin hits `POST /v1/admin/metric-thresholds` with `{ tenant_id: "acme", metric_key: "focus_time_pct", scope: "tenant", good: 75, warn: 60 }`.
2. API checks that the admin is a tenant admin for Acme, that the metric exists and is enabled, that `warn ≤ good`. Persists the row.
3. The tenant's cache is invalidated across all replicas. Next `GET /catalog/metrics` from anyone in Acme returns the new values.
4. Bullet charts across the product re-render with the new color policy on the next page load. No deploy.

### Scenario 2 — Team lead has a different bar

Acme's PM guild, across all teams, wants a specific bar for the `tasks_shipped` metric that differs from what the engineering team applies.

1. The role-level threshold is set at `{ scope: "role", role_slug: "pm", tenant_id: "acme", good: 12, warn: 8 }`.
2. A specific team in Acme, "Growth", has a special reason to set their own PMs' bar higher again: `{ scope: "team+role", role_slug: "pm", team_id: "growth", tenant_id: "acme", good: 18, warn: 14 }`.
3. When a PM in the Growth team opens their dashboard:
   - Backend resolves `(tenant=acme, role=pm, team=growth)` context.
   - The `team+role` row wins (it's the most specific match).
   - Frontend renders with `good=18, warn=14` and a `resolved_from = "team+role"` hint so admin tooling can explain why the color differs from another team.
4. A PM on a different team in Acme still gets `good=12, warn=8` from the `role` scope.

### Scenario 3 — Regulated tenant locks a security metric

BigBank has a regulatory requirement that the `security_vuln_count` bar must be exactly 0 — no team can relax it.

1. Tenant admin for BigBank sets `{ scope: "tenant", metric_key: "security_vuln_count", good: 0, warn: 0, is_locked: true, lock_reason: "MAS-TRM §4.2 requires zero-tolerance" }`.
2. `lock_reason` is required because the DB enforces it — and because a locked threshold without a reason is useless at audit time.
3. A team lead in BigBank tries `{ scope: "team", team_id: "infra", good: 3, warn: 2 }` to relax the bar for their team.
4. API returns `403 threshold_locked` with `{ blocking_scope: "tenant", blocking_row_id: "abc-…", locked_at: "2026-02-10T…" }`. Nothing persisted.
5. An audit event lands in `threshold_lock_audit`: `{ event_type: "bypass_attempt", actor_id: <team-lead>, tenant_id: "bigbank", metric_key: "security_vuln_count", attempted_scope: "team", attempted_values: { good: 3, warn: 2 }, blocking_scope: "tenant", locked_by: <admin>, locked_at: …, event_at: … }`.
6. When the compliance officer audits six months later, they query `threshold_lock_audit` and see every attempt, with full attribution.

---

## The scope model (how "different bars" work)

Thresholds cascade in a fixed order, most-specific wins:

```
team + role      → this team's take on this role's bar        (most specific)
team             → this team's take across all roles
role             → this role's bar across all teams in the tenant
tenant           → this company's default
product-default  → the global seeded floor                    (least specific)
```

Given a request context `(tenant, role_slug, team_id)`, the backend walks from least-specific to most-specific, collects matching rows, and returns the most specific. If along the way it hits a row with `is_locked = true`, the walk stops there and that row is returned — narrower scopes are ignored. That's how a tenant-level lock blocks team-level or role-level overrides.

Note that **team beats role** by design: a team lead's call on their team's bar wins over a role-wide company standard. If a company wants a role-wide bar to be honored inside a specific team, the team lead has to set `team+role` explicitly. That way, company-wide role policy is the default but team autonomy stays possible without central coordination.

Why not a `dashboard` scope? An earlier draft had one. In practice, admins don't think "change threshold on this dashboard" — they think in role / team / company terms. And since Dashboard Configurator keys dashboards by `(view_type, role)`, a `dashboard` scope would just be a clumsier way of saying "role". It was removed.

---

## Locks and audit

Locks are the feature that turns the catalog from a config store into a compliance mechanism.

**What a lock does:** when `is_locked = true` on a row, no narrower-scope row overrides it during resolution. Write attempts to set a narrower-scope row get `403 threshold_locked` with a structured body telling the admin which lock is in the way.

**Where locks can be set (v1):** only at `product-default` (by the product team, via seed migration) and at `tenant` (by tenant admins, via API). Narrower-scope locks are explicitly out of v1 — they'd be more nuance than the feature currently needs. If someone has a real use case, we widen later.

**What gets recorded:** every lock set, every lock cleared, and every rejected bypass attempt lands in the `threshold_lock_audit` MariaDB table, **and** in the analytics-api structured log stream. The table is the source of truth for long-term audit (≥ 1 year), because log retention tends to be shorter than regulatory cycles; the log stream is for real-time observability.

**What's required on a lock:** `locked_by` (the authenticated actor), `locked_at` (server timestamp), and `lock_reason` (human-authored justification). All three are DB-enforced — a lock without a reason is rejected both by the API and by the DB CHECK constraint. This is deliberate: an audit trail that says "someone locked this, no idea why" fails at audit time.

**Can product-default locks be overridden at runtime?** No. Legitimate legal-override requests flow through backend code migrations. That path is slower (hours, not seconds) but fully auditable in git. If a compliance emergency needs faster turnaround, the deployment pipeline for such migrations may skip non-essential gates (canary, non-blocking lint) to cut hours-to-days down to ~1 hour while preserving the audit trail. Runtime feature-flag overrides were explicitly considered and rejected — they erode the compliance guarantee that makes locks worth shipping.

---

## What v1 does **not** do (and why)

These were in earlier drafts and deliberately deferred:

**Calculation rules** — structured descriptions of how each metric is computed (aggregation function, source views, grain, null policy). Nice in theory, but real opacity lives deeper in the silver/gold data layer (and in per-tenant silver customizations), which can't be captured on a catalog row without its own design. Probably lands together with silver-plugin manifests, when those exist.

**`primary_query_id` linkage and admin diagnostics endpoint** — would let an admin jump from "this metric looks wrong" to the ClickHouse SQL in one click. Useful, but the consumer (admin UI) isn't in v1 scope; adding the column without a reader is premature optimization.

**Invariant tests** — CI jobs that validate `query_ref` SQL against declared calc rules. Depend on calc rules shipping first, and on a fixture-dataset framework we don't have. Deferred with calc rules.

**Dashboard-scope thresholds** — rejected, not deferred. It was a proxy for role that added nuance without adding expressiveness.

**Conditional / filtered thresholds** (e.g., "count only commits with LOC ≤ 5K") — v1 thresholds are numeric scalars. Richer threshold expressions are a possible v2 direction.

**FK constraints on `role_slug` / `team_id`** — v1 ships these as string columns without FK, matching whatever `role_catalog` / team-catalog conventions Dashboard Configurator provides later. The FK-adding migration runs after those tables exist. Meanwhile, `cpt-metric-cat-fr-integrity-check` (a periodic job that flags orphaned references) ships with the FK migration, not before.

**Per-tenant metadata overrides** (a tenant admin renaming a metric for their company) — metadata is product-level contract in v1. If tenant-specific localization ever becomes real, an additive override table lands alongside the current `metric_catalog` without breaking it.

**Tenant-specific custom metrics — behavior deferred, schema ready.** v1 still does not let a tenant add their own metric (e.g., a bank's `mas_compliance_score` or a healthcare tenant's HIPAA-driven KPI). What changed in v1.10 is that the catalog now ships with the **schema slot** for this feature in place: `metric_catalog` has a nullable `tenant_id` column with a v1 CHECK constraint forcing `tenant_id IS NULL`. Every v1 row is global, exactly as before — but the follow-on PRD that turns tenant-custom on lands additively (drop the CHECK, add CRUD for `tenant_id = :caller` rows) without a breaking migration. The model is intentionally symmetric to how thresholds work: product-owned global baseline, plus additive tenant-owned overlay. Tenants *add* their own metrics alongside ours; they don't *edit* ours (`cpt-metric-cat-fr-metadata-writes` still forbids that). The tenant-custom follow-on owns three still-open questions: which query-layer to use (tenant-scoped SQL rows in `analytics.metrics` vs a formula DSL like "metric_X * 0.5 + metric_Y" vs both), whether a tenant can hide a product-owned metric they disagree with (disable-for-me slot), and how silver-schema changes affect tenant-owned custom queries.

---

## Architectural decisions that matter

**Global `metric_catalog`, per-tenant `metric_threshold`, with a reserved extension path for tenant-custom.** Metadata on product-owned metrics is identical across tenants by policy — we forbid per-tenant metadata edits in v1 — so duplicating metadata rows per tenant would tax every write for a capability v1 explicitly rules out. Thresholds are local policy and genuinely per-tenant, which is why they live in their own table with override semantics. In v1.10, `metric_catalog` gains a nullable `tenant_id` column locked to `NULL` by a DB CHECK so the follow-on "tenant adds their own metric" feature can land additively — no breaking migration, no new table to union in on every read. The overall shape is: product owns the global catalog and keeps ownership; tenants extend it with their own rows (follow-on); tenants tune thresholds on *any* row (v1).

**Cross-replica cache invalidation is a hard NFR.** Admin writes must become visible on any replica within 2 seconds. Pure in-process caching silently fails this; DESIGN picks between shared cache (Redis) and pub-sub broadcast. The reason this is non-negotiable: a compliance product that sometimes shows stale values after an admin write isn't a compliance product.

**Constraints are enforced at both the DB and the app layer.** Not one or the other. DB CHECK is the backstop for direct-SQL or migration writes that bypass the API; app-layer validation owns the user-facing error messages. Belt and suspenders.

**Auth is a dependency, not a component.** The catalog assumes the auth layer provides `actor_id`, `tenant_id`, and an `is_tenant_admin(tenant_id)` predicate. If Auth slips, the catalog ships against a trait + test-double stub in staging and local-dev; production deploy gates on the real Auth implementation. This means the catalog's release date is decoupled from Auth's.

**Seed via migration, populate from plugins later.** v1 populates `metric_catalog` via a sea-orm migration importing the current frontend metadata. The table shape is deliberately compatible with future population from silver-plugin manifests — the transition is an additive write-path change, not a schema change.

---

## Glossary (short version)

- **Metric** — a named, quantitative thing Insight shows, e.g., `focus_time_pct`, `cursor_active`.
- **Metric key** — the stable `snake_case` identifier for a metric.
- **Threshold** — a set of numeric values (`good`, `warn`, optional `alert_trigger` and `alert_bad`) that drive bullet colors and alerts.
- **Scope** — one of `product-default` / `tenant` / `role` / `team` / `team+role`. Determines which rows apply to which requests.
- **Lock** — `is_locked = true` on a threshold row, which prevents narrower-scope rows from overriding it. Setting a lock requires a reason.
- **Resolved threshold** — the threshold the catalog returns for a specific `(tenant, role, team)` request context, after walking the scope chain.
- **Source tag** — a short string (or array of strings) naming the connectors a metric depends on, e.g., `["m365", "zoom"]`. Used for connector-readiness diagnostics.
- **Gold layer** — the aggregated, user-facing ClickHouse views that metric queries typically read from. The catalog describes metrics at this layer.
- **Silver layer** — the per-connector cleaned tables that feed gold. Below the catalog's boundary; not described by catalog entries.

---

## Related documents

- [`PRD.md`](./PRD.md) — the canonical requirements document with IDs, acceptance criteria, and traceability markers.
- [`../../../dashboard-configurator/specs/PRD.md`](../../../dashboard-configurator/specs/PRD.md) — sibling PRD for the Dashboard Configurator. Its `role_catalog` and team-identifier conventions are what the catalog references via string columns in v1.
- GitHub PR [#225](https://github.com/cyberfabric/insight/pull/225) — review history and discussion trail.

---

## Changelog of this companion

- **v1.1** (2026-04-24): Aligned with canonical PRD v1.10. Reframed the tenant-custom story from "cross-layer, needs its own PRD" to "schema slot ships in v1 (nullable `tenant_id` column, CHECK forces NULL), behavior ships in a follow-on additively". Updated architectural-decisions section to describe the product/tenant split as *product owns global, tenants extend with their own rows, tenants tune thresholds on any row*.
- **v1.0** (2026-04-24): Initial companion. Matches canonical PRD v1.9 (adds the tenant-specific-custom-metrics clarification in "What v1 does not do").
