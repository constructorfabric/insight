# Contract Surface — Engineering → Presentation (Phase A)

Reference doc for the read-only contract the Engineering layer exposes to the
Presentation layer (epic #1803, sub-issue #1968). It names the stable read
surface and the additive-only rules so a contract change can be checked against
it. It is the concrete enumeration behind:

- PRD FR `cpt-presentation-fr-contract-surface-doc` and external contract
  `cpt-presentation-contract-read-only-consumption`.
- DESIGN principle `cpt-presentation-principle-additive-contract` and
  §3.5 External Dependencies (ClickHouse).

This is a plain reference doc, not a governed `cfs` artifact. The governed
PRD/DESIGN link here; keep it in sync when the surface changes.

## 1. What the contract is

The contract is the set of ClickHouse objects the Presentation layer is allowed
to read. It is **read-only** to presentation and evolves **additively only**.
Access is enforced by construction: the `presentation_ro` role
(`cpt-presentation-component-read-only-role`) grants `SELECT` on exactly the
contract databases and nothing that can write, alter, or drop them
(`src/ingestion/scripts/bootstrap-db/presentation-role.sql`).

Grant surface (SELECT only):

| Database | Role in the contract |
|----------|----------------------|
| `silver` | Normalized facts: entity/activity classes, grain-level facts, metric rollups, conformed dimensions. |
| `person` | Canonical person records. |
| `identity` | Alias and identity-input artifacts that resolve source accounts to persons. |
| `insight` | Legacy gold (relabel, not migrate) — read-only where dbt builds it today. |

The `presentation` database is **not** part of the contract: it is
presentation-owned (SELECT/INSERT/CREATE) scratch and new gold, not a read
surface produced by Engineering.

## 2. Stable named surface

The families below are the surface presentation builds against. Names are
prefixed by intent so additive evolution is checkable by prefix. The lists are
illustrative of the current shape, not a closed set — new members of a family
are additive (see §3).

### 2.1 Silver facts (`silver.*`)

Engine `ReplacingMergeTree(_version)`; **read with `FINAL`** to see the
deduplicated state (`cpt-presentation-constraint-final-reads`).

| Prefix | Meaning | Examples |
|--------|---------|----------|
| `class_*` | Normalized entity / activity classes, one family per source domain (ai, collaboration, crm, git, hr, support, task-tracking, wiki). | `class_people`, `class_git_pull_requests`, `class_crm_deals`, `class_task_worklogs` |
| `fct_*` | Grain-level fact rows. | `fct_git_commit`, `fct_git_pr`, `fct_git_review`, `fct_git_file_change` |
| `mtr_*` | Pre-aggregated metric rollups. | `mtr_git_person_totals`, `mtr_git_person_weekly` |
| `dim_*` | Conformed dimensions. | `dim_support_agent`, `dim_support_ticket` |

The `class_*` / `fct_*` / `mtr_*` families are the surface named by #1968;
`dim_*` is granted and follows the same additive rules.

### 2.2 Identity (`person.*`, `identity.*`)

Engine `ReplacingMergeTree(updated_at)`; read with `FINAL`.

| Object | Purpose | Key columns (stable) |
|--------|---------|----------------------|
| `person.persons` | Canonical person record. | `id`, `insight_tenant_id`, `display_name`, `email`, `status`, `manager_person_id`, `org_unit_id` |
| `identity.aliases` | Resolved alias → person mapping. | `id`, `insight_tenant_id`, `person_id`, `value_type`, `value` |
| `identity.identity_inputs` | Raw alias observations feeding resolution. | `insight_tenant_id`, `value_type`, `value`, `source_account_id`, `operation_type` |

### 2.3 Contract version stamp (`silver.contract_version`)

The machine-readable version of this surface (#1969,
`cpt-presentation-fr-contract-version-stamp`). A single-row constant view:

| Column | Type | Meaning |
|--------|------|---------|
| `version` | `UInt32` | The contract-surface version currently deployed. |

**Current version: 1** (the surface as documented by #1968).

Stamped by the ledgerless ClickHouse migration
`src/ingestion/scripts/migrations/20260731000000_contract-version-stamp.sql`
(re-applied on every deploy; `CREATE OR REPLACE VIEW` keeps it idempotent).
Readable by `presentation_ro` through the existing `silver.*` grant.

Presentation pins the version it was built against
(`PINNED_CONTRACT_VERSION` in the analytics service,
`src/backend/services/analytics/src/domain/contract_version.rs`) and verifies
the stamp in a periodic post-boot probe (the stamp is created by the migrate
hook after the service boots, and a later in-place bump must surface without a
restart): a mismatch or a missing stamp is logged loudly on every state
change but never gates readiness.

### 2.4 Legacy gold (`insight.*`)

Existing gold views/tables that presentation keeps reading unchanged
(`cpt-presentation-principle-relabel-not-migrate`). These are **not extended**
by presentation; new gold is authored in the `presentation` namespace. Legacy
gold stays where dbt builds it until an optional later physical move
(#1979-#1981).

## 3. Additive-only rules

A contract change is allowed iff every existing reader keeps working unchanged:

1. **Add, never rewrite.** New tables and new columns are allowed. Renaming,
   dropping, or retyping an existing table or column is a breaking change and is
   not permitted within a contract version.
2. **Stable engines and read semantics.** Silver/identity stay
   `ReplacingMergeTree`; readers keep `FINAL`. Changing the engine or the
   dedup/versioning column of an existing table is breaking.
3. **New columns are nullable/defaulted.** A new column must not change the
   result of an existing `SELECT` that does not name it.
4. **Tenant column.** Presentation scopes reads by tenant. Silver exposes the
   tenant as `tenant_id`; identity/person expose `insight_tenant_id`. Do not
   rename or drop the tenant column of a contract object outside the coordinated
   engineering retrofit (#1829). New presentation gold that carries the tenant
   puts `insight_tenant_id` first in `ORDER BY`
   (`cpt-presentation-constraint-tenant-id-retrofit`).

## 4. Not part of the stable surface

Granted-but-internal or out-of-scope objects that presentation must not treat as
a stable contract:

- **Bronze / staging.** Raw connector-landed data is not granted to
  `presentation_ro` and is not a read surface.
- **`seed_*` tables** in `person`/`identity` (e.g. `seed_persons_from_*`,
  `seed_aliases_from_*`). Bootstrap/staging inputs to identity resolution, not a
  consumer surface; their shape may change.
- **The `presentation` database.** Presentation-owned write scratch, not
  Engineering-produced contract (§1).

## 5. Checking a change against the surface

When Engineering changes a contract object:

1. Confirm it is additive per §3 (new table/column, no rename/drop/retype, no
   engine change, tenant column preserved).
2. Update the family/object lists here if a new stable object or column is
   introduced.
3. Bump the contract version stamp (§2.3) in all three places together: the
   constant in `20260731000000_contract-version-stamp.sql` (edited in place),
   its copy in the `connectors-ddl/silver.sql` snapshot, and the
   **Current version** line above. Presentation raises its
   `PINNED_CONTRACT_VERSION` when it adopts the new surface; until then its
   periodic probe reports the mismatch.

A change that cannot be expressed additively is a new contract version and a
coordinated migration, not a Phase-A contract edit.
