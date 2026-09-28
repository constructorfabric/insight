---
status: draft
version: "0.4"
date: 2026-09-28
---

# PRD — Insight v3 Metric Alerts

**Status:** Alert behavior and scheduling are approved. Other decisions in section 11 remain open.

**Revision 0.4:** Make notification requirements destination-neutral and record approved behavior.

<!-- toc -->

- [1. Overview](#1-overview)
  - [1.1 Purpose](#11-purpose)
  - [1.2 Background / Problem Statement](#12-background--problem-statement)
  - [1.3 Goals (Business Outcomes)](#13-goals-business-outcomes)
  - [1.4 Glossary](#14-glossary)
- [2. Actors](#2-actors)
  - [2.1 Human Actors](#21-human-actors)
  - [2.2 System Actors](#22-system-actors)
- [3. Operational Concept & Environment](#3-operational-concept--environment)
  - [3.1 Module-Specific Environment Constraints](#31-module-specific-environment-constraints)
- [4. Scope](#4-scope)
  - [4.1 In Scope](#41-in-scope)
  - [4.2 Out of Scope](#42-out-of-scope)
- [5. Functional Requirements](#5-functional-requirements)
  - [5.1 Rule Administration](#51-rule-administration)
  - [5.2 Evaluation and Notifications](#52-evaluation-and-notifications)
- [6. Non-Functional Requirements](#6-non-functional-requirements)
  - [6.1 Quality Vector Analysis](#61-quality-vector-analysis)
  - [6.2 NFR Inclusions](#62-nfr-inclusions)
  - [6.3 NFR Exclusions](#63-nfr-exclusions)
- [7. Public Library Interfaces](#7-public-library-interfaces)
  - [7.1 Public API Surface](#71-public-api-surface)
  - [7.2 External Integration Contracts](#72-external-integration-contracts)
- [8. Use Cases](#8-use-cases)
- [9. Acceptance Criteria](#9-acceptance-criteria)
- [10. Dependencies](#10-dependencies)
- [11. Assumptions](#11-assumptions)
- [12. Risks](#12-risks)

<!-- /toc -->

## 1. Overview

### 1.1 Purpose

Send a notification when an Insight v3 custom metric meets a threshold condition. Administrators configure alerts through the API or Model Context Protocol (MCP). Alerts run without a dashboard open.

This expands [the parent alert requirement](../PRD.md#58-alerts), `cpt-insightspec-v3-fr-create-alerts`. Rules take effect without a service release. Checks run on a schedule, so detection is not instantaneous.

### 1.2 Background / Problem Statement

On-demand metrics need someone to run them. Alerts check metrics automatically and show whether each check and message delivery succeeded.

### 1.3 Goals (Business Outcomes)

- Configure and inspect alerts through either API or MCP.
- Detect breaches automatically and show the value that triggered them.
- Keep pending work through restarts and show failures. Report delivery only when confirmed.

Verify these goals with synthetic metrics before release. Capacity and timing targets remain open in D7.

### 1.4 Glossary

| Term | Definition |
|------|------------|
| Rule | A metric, selected numeric value, threshold condition, schedule and destination |
| Evaluation | One metric check for a rule |
| Breach | A valid numeric result satisfying the configured condition |
| Episode | Consecutive valid breached results, ending when a valid result no longer breaches |
| Notification | A saved request to send a message; delivery is tracked separately |
| Unknown | A check could not determine whether the condition was met |

## 2. Actors

### 2.1 Human Actors

#### Administrator

**ID**: `cpt-insightspec-v3-alerts-actor-admin`

**Role**: Manages rules and inspects destination details through API or MCP. Must be an authenticated Insight v3 administrator.

**Needs**: Clear validation, understandable results and delivery errors, with credentials hidden.

#### Notification Recipient

**ID**: `cpt-insightspec-v3-alerts-actor-recipient`

**Role**: Reads notifications at a destination configured by an administrator. Destination membership is managed outside Insight.

**Needs**: Metric identity, observed value, condition and evaluation time, without unrelated result rows.

### 2.2 System Actors

#### Notification Provider

**ID**: `cpt-insightspec-v3-alerts-actor-provider`

**Role**: Receives notification requests and reports acceptance or failure. Its availability is outside Insight's control.

## 3. Operational Concept & Environment

### 3.1 Module-Specific Environment Constraints

Checks and delivery run without an active API client. [DESIGN](./DESIGN.md) records the approved job library and pending database compatibility checks.

## 4. Scope

### 4.1 In Scope

**Approved:** Insight v3 custom metrics; administrator API and MCP; one numeric result per alert; threshold checks; notifications to a configured destination; first-breach and interval behavior below.

**Proposed:** retry delivery separately and show check and delivery history. See section 11 for open decisions.

### 4.2 Out of Scope

Excluded: web administration UI, legacy analytics alerts and separate alerts per result group.

First release excludes reminder and recovery messages, cron schedules, and replaying missed checks. The initial set of notification providers is open in D3.

## 5. Functional Requirements

The approved firing and scheduling rules appear below. Other requirements remain proposed until their open decisions are resolved.

### 5.1 Rule Administration

#### Manage Rules

- [ ] `p1` - **ID**: `cpt-insightspec-v3-alerts-fr-manage`

The system **MUST** let administrators create, read, list, update, enable and disable rules through API and MCP, using the same validation.

Each rule selects one existing custom metric, one numeric value, a threshold condition, a schedule and one destination. Report conflicting edits instead of overwriting them. Deletion and history retention remain open in D6.

**Actors**: `cpt-insightspec-v3-alerts-actor-admin`

#### Restrict Administration

- [ ] `p1` - **ID**: `cpt-insightspec-v3-alerts-fr-authorize`

The system **MUST** restrict rule and destination management, results and delivery history to authenticated administrators on both API and MCP. Workers must not depend on a user's login session.

**Actors**: `cpt-insightspec-v3-alerts-actor-admin`

### 5.2 Evaluation and Notifications

#### Evaluate a Scalar

- [ ] `p1` - **ID**: `cpt-insightspec-v3-alerts-fr-evaluate`

The system **MUST** run the same saved metric definition as an on-demand check and compare one numeric result with the threshold. It must not silently choose a row or combine grouped results.

Empty, null, nonnumeric, nonfinite, ambiguous and failed results are not valid non-breaches. Numeric precision, comparison operators and data freshness remain open in D4–D5.

**Actors**: `cpt-insightspec-v3-alerts-actor-admin`

#### Evaluate Unattended

- [ ] `p1` - **ID**: `cpt-insightspec-v3-alerts-fr-schedule`

The system **MUST** check enabled rules at a configured per-rule interval. After downtime, it checks the current value once rather than replaying missed checks. The interval changes neither the metric's data window nor how fresh its source data is.

**Actors**: `cpt-insightspec-v3-alerts-actor-admin`

#### Detect Breach Episodes

- [ ] `p1` - **ID**: `cpt-insightspec-v3-alerts-fr-episodes`

The system **MUST**:

- Notify on the first valid breached result.
- Suppress further notifications until a valid result no longer breaches.
- Allow a new notification when the condition is met again.
- Never treat an unknown result as recovery.

The first release sends neither repeated reminders during a breach nor recovery messages.

D6 covers edits and disabling/re-enabling a rule.

**Actors**: `cpt-insightspec-v3-alerts-actor-recipient`

#### Deliver Notifications

- [ ] `p1` - **ID**: `cpt-insightspec-v3-alerts-fr-deliver`

The system **MUST** send the rule, metric, value, condition and check time to the configured destination.

Retry delivery without rerunning the metric or creating another episode. Show permanent failures and exhausted retries. Never report an uncertain outcome as confirmed delivery; retries may produce duplicate messages.

**Actors**: `cpt-insightspec-v3-alerts-actor-provider`, `cpt-insightspec-v3-alerts-actor-recipient`

#### Inspect Outcomes

- [ ] `p1` - **ID**: `cpt-insightspec-v3-alerts-fr-inspect`

The system **MUST** show rule status, latest check time and outcome, latest valid value, and delivery status separately. Distinguish invalid metrics, delayed work and failed delivery. Audit records must identify configuration changes and the configuration behind each notification, without exposing secrets.

**Actors**: `cpt-insightspec-v3-alerts-actor-admin`

## 6. Non-Functional Requirements

### 6.1 Quality Vector Analysis

| Quality Vector | Show-Stopper Requirement | Rationale |
|----------------|-------------------------|-----------|
| Efficiency | Alert work MUST remain within the parent footprint obligation, `cpt-insightspec-v3-nfr-efficiency`; local allocation is pending | Monitoring must not make the service uneconomical to operate |
| Reliability | Committed notification intents MUST survive process restart, `cpt-insightspec-v3-alerts-nfr-durability` | Silent loss defeats unattended monitoring |
| Performance | None — no show-stopper; obligations covered by `cpt-insightspec-v3-alerts-nfr-timeliness` | Administrators need to know how late a notification may arrive |
| Security | Destination secrets MUST remain absent from ordinary reads and diagnostics, `cpt-insightspec-v3-alerts-nfr-confidentiality` | A leaked credential grants external posting authority |
| Versatility | None — no show-stopper; obligations covered by `cpt-insightspec-v3-nfr-versatility` | Administrators must create new rules without engineering involvement |

### 6.2 NFR Inclusions

#### Durable and Explainable Outcomes

- [ ] `p1` - **ID**: `cpt-insightspec-v3-alerts-nfr-durability`

The system **MUST** preserve committed notification intent and prevent duplicate logical episodes under restart, retry and concurrent workers.

**Threshold**: process interruption loses no saved notifications. Provider messages may still duplicate. Database recovery targets and how long to retry remain open in D7.

**Rationale**: Restarting a worker must not lose an alert.

#### Timeliness and Capacity

- [ ] `p1` - **ID**: `cpt-insightspec-v3-alerts-nfr-timeliness`

The system **MUST** report scheduling and delivery delay separately and enforce explicit capacity limits.

**Threshold**: minimum interval, maximum rule count, concurrency, backlog, history retention, and p95 due-to-evaluation/delivery targets require product-owner and engineering approval in D7. No new numerical SLA is approved.

**Rationale**: Show overload and protect on-demand metric checks.

#### Confidentiality

- [ ] `p1` - **ID**: `cpt-insightspec-v3-alerts-nfr-confidentiality`

The system **MUST** keep destination credentials out of normal API/MCP reads, notifications, queued payloads and diagnostics. Notifications must contain only the approved minimal metric summary, not complete query results.

**Threshold**: zero credential disclosure in those surfaces; destination provisioning and allowed notification content require D8.

**Rationale**: Alerts cross an external data boundary.

#### Parent Obligations

**Inherits**: `cpt-insightspec-v3-nfr-efficiency`, `cpt-insightspec-v3-nfr-reliability`, `cpt-insightspec-v3-nfr-performance`, `cpt-insightspec-v3-nfr-security`, `cpt-insightspec-v3-nfr-versatility`.

Parent targets remain unchanged:

- No increase to the recommended resource footprint.
- Service uptime at least 99.9%.
- Dashboard p95 below 2 seconds; LCP p95 at most 2,500 milliseconds.
- Zero critical scan findings.
- New alerts require no code changes.

Dashboard targets still apply while alerts run; they are not delivery deadlines. Engineering must collect synthetic before/after measurements and security evidence. That evidence is pending.

### 6.3 NFR Exclusions

No parent NFR is excluded. A new visual UI and its browser accessibility measurements are not applicable because no UI is included; API/MCP documentation and readable notification content still apply. External sharing and retention responsibilities require D8.

## 7. Public Library Interfaces

### 7.1 Public API Surface

- [ ] `p1` - **ID**: `cpt-insightspec-v3-alerts-interface-administration`

**Type**: Administrator API and MCP tools.

**Stability**: Proposed, unreleased.

**Description**: Manage rules and inspect destinations and outcomes. API and MCP use the same permissions and behavior.

**Breaking Change Policy**: Contract versioning follows the service policy; exact routes and tool names require design approval before publication.

### 7.2 External Integration Contracts

- [ ] `p1` - **ID**: `cpt-insightspec-v3-alerts-contract-delivery`

**Direction**: Outbound notification with acceptance/failure response.

**Protocol/Format**: Provider-supported message delivery.

**Compatibility**: Provider rate limits and payload constraints apply. The first provider set is pending D3; Insight does not control destination membership or provider retention.

## 8. Use Cases

#### Configure and Monitor a Threshold

- [ ] `p1` - **ID**: `cpt-insightspec-v3-alerts-usecase-monitor`

**Actor**: `cpt-insightspec-v3-alerts-actor-admin`

**Preconditions**: A custom metric and permitted destination exist.

**Main Flow**:

1. Create and enable a rule.
2. Inspect its check result.
3. Receive a notification when the breach policy calls for one.

**Postconditions**: The evaluation and delivery outcome can be inspected independently.

**Alternative Flows**: Invalid scalar output is reported as unknown; provider failure leaves a visible delivery outcome; disabling or editing follows D6.

## 9. Acceptance Criteria

All evidence is pending. Engineering verifies these against synthetic inputs after the referenced decisions are approved.

- [ ] API and MCP both support the approved rule lifecycle and reject non-administrators.
- [ ] Alert evaluation agrees with on-demand execution of the same metric definition and rejects ambiguous scalar output.
- [ ] First-breach notification, suppression during a breach, return below threshold, invalid-result and approved edit behavior are demonstrated.
- [ ] Per-rule interval checks resume with one current check after downtime, without replaying missed intervals.
- [ ] Restart and concurrent execution neither lose committed intents nor create duplicate logical episodes.
- [ ] Delivery failures are visible without metric re-execution; uncertain external results retain their uncertainty.
- [ ] Every selected delivery provider receives the approved summary; no credentials or unrelated metric rows appear in any public surface.
- [ ] Approved capacity targets and unchanged parent quality gates have evidence before release, including parent coverage and non-degradation requirements.

## 10. Dependencies

| Dependency | Description | Criticality |
|------------|-------------|-------------|
| Insight v3 custom metrics | Saved definitions and consistent on-demand evaluation | p1 |
| Insight administrator identity | Authority for API and MCP management | p1 |
| Durable jobs and state | Unattended evaluation and delivery recovery; see DESIGN | p1 |
| Selected notification providers | External delivery and acceptance | p1 |

## 11. Assumptions

The product owner must approve these remaining choices before dependent implementation starts, with engineering and security input where needed. Recommendations are not defaults.

- **D3 — First delivery providers.** Decide which providers ship in the first release. Alert rules and outcomes remain provider-neutral; provider integration details belong in [DESIGN](./DESIGN.md#open-decisions-and-resumption).
- **D4 — Numeric values.** Recommend one selected numeric column in exactly one row, exact conversion for supported numeric types, and `>`, `>=`, `<`, `<=`. Alternatives: restrict values to floating point or add comparison types. Precision and boundary behavior still need agreement.
- **D5 — Unknown or stale data.** Recommend keeping the last valid episode state when a check is unknown. Show check time without claiming the source data is fresh. Alternative: dedicated missing/stale-data conditions, with a definition of freshness.
- **D6 — Edits, disabling and deletion.** Recommend rejecting obsolete results. Edits and re-enabling reset state to unknown and apply the approved first-breach rule. Disabling suppresses unsent messages; deletion keeps history for a limited period. Alternatives: preserve episodes across edits, finish pending deliveries, or erase history immediately. Sends already in progress cannot be recalled.
- **D7 — Limits and recovery.** Agree capacity, interval bounds, how long to retry, history retention and recovery targets using a synthetic workload. Other workload or service-level commitments remain options; no numbers are approved.
- **D8 — Destinations and content.** Recommend operator-provisioned destinations that admins select, with a short summary and explicit time/number formats. Alternatives: admins provision destinations themselves, or richer messages. Sharing and retention responsibilities still need approval.

## 12. Risks

| Risk | Impact | Mitigation |
|------|--------|------------|
| Invalid or stale source data | Misleading alerts | Distinguish unknown, observation time and source freshness; approve D5 |
| Ambiguous provider acceptance | Duplicate messages on retry | Record uncertainty and stable notification identity; no exactly-once claim |
| Metric edits race evaluations | Notification describes obsolete logic | Approve revision and lifecycle behavior before implementation |
| Unsupported job-backend behavior | Lost, stuck or concurrently retried work | Compatibility evidence required by DESIGN |
| External sharing or excessive history | Data exposure and retention conflicts | Minimal content, restricted destinations and D8 review |
