# Workday Connector Specification

> Version 2.0 — June 2026
> Superseded in detail by [specs/PRD.md](./specs/PRD.md) and [specs/DESIGN.md](./specs/DESIGN.md); this page is the overview.

Standalone overview for the Workday (HR) connector. The v1 implementation follows the **BambooHR pattern**: current-state extraction via RaaS custom reports, with SCD2 history constructed at the Silver layer by the shared snapshot chain.

<!-- toc -->

- [Overview](#overview)
- [Extraction Model — Report Contract](#extraction-model--report-contract)
- [Bronze Tables](#bronze-tables)
- [Silver Pipeline](#silver-pipeline)
- [Identity Resolution](#identity-resolution)
- [Testing Without a Workday Tenant](#testing-without-a-workday-tenant)
- [Open Questions](#open-questions)

<!-- /toc -->

---

## Overview

**API**: Workday RaaS (Reports-as-a-Service) — two customer-built custom reports exposed as web services (`/ccx/service/customreport2/{tenant}/{owner}/{report}?format=json`)

**Category**: HR / Directory

**Authentication**: ISU (Integration System User) credentials via HTTP Basic — the ISU security group must hold the domains required by the report fields (missing domains yield silently empty fields, not errors)

**Identity**: `workers.Work_Email` — resolved to canonical `person_id` via Identity Manager through the `fields_history` → `identity_inputs` chain

**Field naming**: contract-defined `Snake_Case` XML aliases, preserved as-is at Bronze level

**Key differences from BambooHR:**

| Aspect | BambooHR | Workday |
|--------|----------|---------|
| Bulk endpoint | Fixed (`POST /reports/custom` with field list) | None — customer-built RaaS report per the Insight report contract |
| Field guarantees | API-defined field set | Only Workday-delivered fields exist everywhere; the report contract pins the actual set |
| Field discovery | `GET /meta/fields` stream | None — the report definition IS the field registry; extra columns land in `raw_data` |
| Record versioning | Current state only | Natively effective-dated, but v1 extracts current state only (parity with BambooHR) |
| Org model | Department name (freeform string) | Supervisory Organization (no standard department/division fields) |
| Worker types | Freeform | `Employee` / `Contingent Worker` (explicit enum) |
| Scale | SMB (100–5,000 employees) | Enterprise (5,000+ employees) |

**Primary use in Insight**: identity resolution, org structure for team analytics, leave history.

---

## Extraction Model — Report Contract

Workday has no fixed bulk API, so the connector ships a **report contract** instead: the customer's Workday administrator builds two Advanced reports, sets the exact column XML aliases, enables them as web services, and shares them with the ISU.

1. **Workers report** — data source All Workers; 20 contract columns (identity, org, job, classification, location, dates, `Last_Functionally_Updated`, `Scheduled_Weekly_Hours`). Extra customer columns are allowed and flow into Bronze `raw_data`.
2. **Leave report** — time-off requests; 9 contract columns plus mandatory `From_Date`/`To_Date` prompts enabled as web service parameters.

The full alias tables live in the connector [README](../../../../src/ingestion/connectors/hr-directory/workday/README.md); the API contract is specified in [DESIGN §3.3](./specs/DESIGN.md).

Both streams are **full refresh** — RaaS returns the complete report result per call (no pagination, no delta). This is the same extraction model as BambooHR and reuses its entire downstream pipeline.

---

## Bronze Tables

| Table | Stream | Primary key | Contents |
|-------|--------|-------------|----------|
| `bronze_workday.workers` | `workers` | `unique_key` = `{tenant}-{source}-{Employee_ID}` | Current-state worker records, contract columns + `raw_data` |
| `bronze_workday.leave_requests` | `leave_requests` | `unique_key` = `{tenant}-{source}-{Request_ID}` | Time-off requests in the configured date range |

Column-level definitions: [DESIGN §3.7](./specs/DESIGN.md). Both tables are promoted to ReplacingMergeTree by `workday__bronze_promoted` (dedup on `unique_key`).

---

## Silver Pipeline

Model-for-model mirror of BambooHR:

| Model | Role |
|-------|------|
| `workday__bronze_promoted` | Bronze → RMT promotion |
| `workday__workers_snapshot` | SCD2 append-only snapshot (`snapshot` macro); tracks contract columns + `workday_custom_fields` var from `raw_data`; `Last_Functionally_Updated` deliberately untracked |
| `workday__workers_fields_history` | Per-field change log (`fields_history` macro) |
| `workday__identity_inputs` | Identity observations (`identity_inputs_from_history` macro); DELETE on `Worker_Status` → `Terminated` |
| `workday__to_class_people` | `class_people` staging; `Supervisory_Organization` → `department_name`; `Worker_Type` → `employment_type` |
| `workday__hr_events` | `class_hr_events` staging (leave); flat columns, no JSON extraction |
| `workday__working_hours` | `class_hr_working_hours` staging; `Scheduled_Weekly_Hours` with 40h fallback |

---

## Identity Resolution

`workers.Work_Email` is the primary identity key — mapped to canonical `person_id` via Identity Manager. The field may legitimately be empty for contingent workers; those records still bind via the `value_type='id'` observation on `Employee_ID`.

`Manager_Work_Email` enables org hierarchy construction from email addresses without requiring `Employee_ID` resolution first.

---

## Testing Without a Workday Tenant

There is no public Workday developer tenant; sandboxes exist only for customers and certified partners. Development therefore uses:

1. **Mock fixtures** — a local HTTP server replaying the RaaS response shape (`{"Report_Entry": [...]}` with contract aliases). Reliable because the response contract is Insight-defined.
2. **Customer Sandbox** — every Workday customer has one; the connector `check` and a data completeness review run there during onboarding before Production.

---

## Open Questions

Tracked in [PRD §13](./specs/PRD.md):

- **OQ-WD-1** — incremental extraction via a prompt-enabled `Last Functionally Updated` filter (also removes the need for the `snapshot()` step; cursor must be entry moment, not effective date, to survive retroactive transactions)
- **OQ-WD-2** — BambooHR + Workday coexistence (identity merge, org precedence)
- **OQ-WD-3** — native effective-dated history backfill via transaction log (pre-onboarding history)
- **OQ-WD-4** — per-customer department/division mapping beyond `Supervisory_Organization`
