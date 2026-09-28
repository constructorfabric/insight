---
status: draft
version: "0.1"
date: 2026-09-28
---

# PRD — Insight v3 Metric Alerts


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

**Approval status:** Scope is confirmed; behavior marked **proposed** awaits product-owner approval. This draft is not implementation authorization. Version 0.1 introduces the alert requirements.
## 1. Overview

### 1.1 Purpose

Notify a configured Discord destination when an Insight v3 custom metric breaches an administrator-defined threshold. Administrators manage alerts through the API and Model Context Protocol (MCP), without needing a dashboard open.

This refines [the parent alert capability](../PRD.md#58-alerts), `cpt-insightspec-v3-fr-create-alerts`. Creating a rule takes effect without a service release; scheduled evaluation is not a promise of streaming or instantaneous detection.

### 1.2 Background / Problem Statement

Custom metrics can be run on demand, but threshold monitoring needs unattended evaluation and visible delivery outcomes. A successful metric check and a successful notification are different outcomes; administrators need to distinguish them.

### 1.3 Goals (Business Outcomes)

- Configure and inspect an alert entirely through either approved administration surface.
- Detect a qualifying breach without an interactive request and explain which value caused it.
- Preserve pending work across process restart and expose failures without claiming delivery that was not confirmed.

These outcomes are evaluated before release using synthetic metrics. Quantitative capacity and timeliness targets remain an explicit approval item in section 11; no measured baseline is claimed.

### 1.4 Glossary

| Term | Definition |
|------|------------|
| Rule | A metric reference, scalar selection, threshold condition, evaluation schedule and destination |
| Evaluation | One execution of a rule against its referenced custom metric |
| Breach | A valid numeric result satisfying the configured condition |
| Episode | Consecutive valid breached results, ending when a valid result no longer breaches |
| Notification | A recorded intent to tell Discord about an episode; delivery has its own outcome |
| Unknown | Evaluation could not establish a valid breached or non-breached result |

## 2. Actors

### 2.1 Human Actors

#### Administrator

**ID**: `cpt-insightspec-v3-alerts-actor-admin`

**Role**: An authenticated Insight v3 administrator creates, changes, enables, disables and inspects alert rules and destination metadata through API or MCP.
**Needs**: Clear validation, an explainable latest result, and delivery diagnostics without exposing credentials.

#### Discord Recipient

**ID**: `cpt-insightspec-v3-alerts-actor-recipient`

**Role**: Reads messages in a Discord destination configured by an administrator. Channel membership is managed outside Insight.
**Needs**: Metric identity, observed value, condition and evaluation time, without unrelated result rows.

### 2.2 System Actors

#### Discord

**ID**: `cpt-insightspec-v3-alerts-actor-discord`

**Role**: Receives outbound notification requests and reports acceptance or failure. Its availability is outside Insight's control.

## 3. Operational Concept & Environment

### 3.1 Module-Specific Environment Constraints

The feature is limited to the Insight v3 service and its custom metrics. Scheduling and delivery must operate without an active API client. The approved job-library constraint is recorded in [DESIGN](./DESIGN.md); backend compatibility is not yet verified.

## 4. Scope

### 4.1 In Scope

**Confirmed:** administrator API and MCP, one numeric result per alert, threshold monitoring, and Discord as the only initial destination provider.

**Proposed:** per-rule intervals, first-valid-breach notification, one notification per episode, independent delivery retries, and inspectable evaluation/delivery history. Section 11 distinguishes these proposals from confirmed scope.

### 4.2 Out of Scope

A web administration UI, legacy analytics alerts, per-group fan-out, Telegram and Zulip adapters, and Temporal are excluded from this release. This feature does not define a general workflow platform. Reminder and recovery messages, cron schedules, historical catch-up, and additional Discord channel types remain scope decisions rather than implicit commitments.

## 5. Functional Requirements

Except for confirmed scope, the following requirements specify the **proposed baseline** for review.

### 5.1 Rule Administration

#### Manage Rules

- [ ] `p1` - **ID**: `cpt-insightspec-v3-alerts-fr-manage`

The system **MUST** let administrators create, read, list, update, enable and disable rules through API and MCP with equivalent validation. A rule identifies one existing custom metric, one numeric result selection, a threshold condition, a schedule and one destination. Conflicting changes must be reported rather than silently overwriting another change. Removal and associated history retention are unresolved in decision D6.

**Actors**: `cpt-insightspec-v3-alerts-actor-admin`

#### Restrict Administration

- [ ] `p1` - **ID**: `cpt-insightspec-v3-alerts-fr-authorize`

Only authenticated administrators **MUST** be permitted to manage rules and destinations or inspect their results and delivery history. Non-administrators and unauthenticated callers must be denied those operations on both API and MCP. A background worker's authority must not depend on retaining a user's login session.

**Actors**: `cpt-insightspec-v3-alerts-actor-admin`

### 5.2 Evaluation and Notifications

#### Evaluate a Scalar

- [ ] `p1` - **ID**: `cpt-insightspec-v3-alerts-fr-evaluate`

The system **MUST** evaluate the same saved custom-metric definition used by on-demand execution and compare one unambiguous numeric result with the configured threshold. It must not silently pick a row or aggregate grouped results. Empty, null, nonnumeric, nonfinite, ambiguous and failed results must be distinguishable from a valid non-breach. Numeric representation, operators and freshness policy await D4 and D5.

**Actors**: `cpt-insightspec-v3-alerts-actor-admin`

#### Evaluate Unattended

- [ ] `p1` - **ID**: `cpt-insightspec-v3-alerts-fr-schedule`

The system **MUST** evaluate enabled rules without an interactive client and resume eligible work after restart. Proposed D2: run on a per-rule interval and coalesce missed checks into one current evaluation. Evaluation cadence does not change the metric's own data window or establish source-data freshness.

**Actors**: `cpt-insightspec-v3-alerts-actor-admin`

#### Detect Breach Episodes

- [ ] `p1` - **ID**: `cpt-insightspec-v3-alerts-fr-episodes`

Under proposed D1, the system **MUST** create one notification when the first valid result breaches, suppress further notifications while that episode remains breached, and allow another notification after a valid non-breach. Unknown results must not be represented as recovery. Rule or metric edits and disable/re-enable behavior require D6 before implementation.

**Actors**: `cpt-insightspec-v3-alerts-actor-recipient`

#### Deliver to Discord

- [ ] `p1` - **ID**: `cpt-insightspec-v3-alerts-fr-deliver`

The system **MUST** send a qualifying notification to its configured Discord destination, including rule and metric identity, value, condition and evaluation time. Temporary delivery failures must not rerun the metric or create a new episode. Exhausted or permanent failures must remain inspectable. An uncertain external outcome must not be represented as confirmed delivery; retries may produce duplicate Discord messages.

**Actors**: `cpt-insightspec-v3-alerts-actor-discord`, `cpt-insightspec-v3-alerts-actor-recipient`

#### Inspect Outcomes

- [ ] `p1` - **ID**: `cpt-insightspec-v3-alerts-fr-inspect`

The system **MUST** expose configuration status, latest evaluation time and outcome, latest valid value, and notification delivery status separately. Administrators must be able to distinguish an invalid metric, delayed work and failed delivery. Audit records must identify configuration changes and the configuration used for a notification without exposing destination secrets.

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
**Threshold**: zero lost committed intents from process interruption; no external exactly-once guarantee. Database-loss recovery objectives and delivery retry horizon await D7.
**Rationale**: Reliability must cover interrupted work, not just successful requests.

#### Timeliness and Capacity

- [ ] `p1` - **ID**: `cpt-insightspec-v3-alerts-nfr-timeliness`

The system **MUST** report scheduling and delivery delay separately and enforce explicit capacity limits.
**Threshold**: minimum interval, maximum rule count, concurrency, backlog, history retention, and p95 due-to-evaluation/delivery targets require product-owner and engineering approval in D7. No new numerical SLA is approved.
**Rationale**: An overloaded monitor must remain diagnosable and must not starve interactive metric use.

#### Confidentiality

- [ ] `p1` - **ID**: `cpt-insightspec-v3-alerts-nfr-confidentiality`

The system **MUST** keep destination credentials out of normal API/MCP reads, notifications, queued payloads and diagnostics. Notifications must contain only the approved minimal metric summary, not complete query results.
**Threshold**: zero credential disclosure in those surfaces; destination provisioning and allowed notification content require D3 and D8.
**Rationale**: Alerts cross an external data boundary.

#### Parent Obligations

**Inherits**: `cpt-insightspec-v3-nfr-efficiency`, `cpt-insightspec-v3-nfr-reliability`, `cpt-insightspec-v3-nfr-performance`, `cpt-insightspec-v3-nfr-security`, `cpt-insightspec-v3-nfr-versatility`.

The parent targets remain unchanged: no increase to its recommended footprint; service uptime at least 99.9%; dashboard p95 below 2 seconds and LCP p95 at most 2,500 milliseconds; zero critical scan findings; creating an alert without code changes. Dashboard targets are coexistence obligations, not alert-delivery SLAs. Engineering owns synthetic before/after measurements and security evidence; none has been collected for this feature.

### 6.3 NFR Exclusions

No parent NFR is excluded. A new visual UI and its browser accessibility measurements are not applicable because no UI is included; API/MCP documentation and readable Discord content still apply. External sharing and retention responsibilities require D8.

## 7. Public Library Interfaces

### 7.1 Public API Surface

- [ ] `p1` - **ID**: `cpt-insightspec-v3-alerts-interface-administration`

**Type**: Administrator API and MCP tools.
**Stability**: Proposed, unreleased.
**Description**: Rule lifecycle, destination metadata and outcome inspection; both surfaces must expose equivalent authority and domain behavior.
**Breaking Change Policy**: Contract versioning follows the service policy; exact routes and tool names require design approval before publication.

### 7.2 External Integration Contracts

- [ ] `p1` - **ID**: `cpt-insightspec-v3-alerts-contract-discord`

**Direction**: Outbound notification with acceptance/failure response.
**Protocol/Format**: Discord-supported HTTP integration and message payload.
**Compatibility**: Provider rate limits and payload constraints apply; delivery transport is pending D3. Insight does not control channel membership or Discord retention.

## 8. Use Cases

#### Configure and Monitor a Threshold

- [ ] `p1` - **ID**: `cpt-insightspec-v3-alerts-usecase-monitor`

**Actor**: `cpt-insightspec-v3-alerts-actor-admin`
**Preconditions**: A custom metric and permitted destination exist.
**Main Flow**: The administrator creates and enables a rule, inspects its evaluation outcome, and receives a Discord notification when the approved breach policy qualifies.
**Postconditions**: The evaluation and delivery outcome can be inspected independently.
**Alternative Flows**: Invalid scalar output is reported as unknown; Discord failure leaves a visible delivery outcome; disabling or editing follows D6.

## 9. Acceptance Criteria

All evidence is pending. Engineering verifies these against synthetic inputs after the referenced decisions are approved.

- [ ] API and MCP both support the approved rule lifecycle and reject non-administrators.
- [ ] Alert evaluation agrees with on-demand execution of the same metric definition and rejects ambiguous scalar output.
- [ ] Approved first-run, repeated-breach, recovery, invalid-result and edit behavior is demonstrated.
- [ ] Restart and concurrent execution neither lose committed intents nor create duplicate logical episodes.
- [ ] Delivery failures are visible without metric re-execution; uncertain external results retain their uncertainty.
- [ ] Discord receives the approved summary and no credentials or unrelated metric rows appear in any public surface.
- [ ] Approved capacity targets and unchanged parent quality gates have evidence before release, including parent coverage and non-degradation requirements.

## 10. Dependencies

| Dependency | Description | Criticality |
|------------|-------------|-------------|
| Insight v3 custom metrics | Saved definitions and consistent on-demand evaluation | p1 |
| Insight administrator identity | Authority for API and MCP management | p1 |
| Durable jobs and state | Unattended evaluation and delivery recovery; see DESIGN | p1 |
| Discord | External notification acceptance and channel access | p1 |

## 11. Assumptions

Confirmed scope does not resolve the choices below. The product owner owns approval, with engineering and security input where indicated; all are required before the dependent implementation. Recommendations are drafts, not defaults.

| Decision | Recommended proposal | Alternative requiring a choice |
|----------|----------------------|-------------------------------|
| D1: firing | First valid breach fires; once per episode; no reminders or recovery messages | Recovery messages, reminders, or baseline-only first observation |
| D2: schedule | Per-rule intervals; one current check after missed intervals | Cron schedules or historical catch-up |
| D3: Discord delivery | Discord is the only initial provider; the technical integration proposal is recorded in DESIGN | Implementation approach requires design approval |
| D4: numeric contract | One selected numeric column in exactly one row; comparisons `>`, `>=`, `<`, `<=`; exact supported numeric conversion | Restricted floating-point domain or additional comparison types; engineering must settle precision and boundary semantics |
| D5: unknown and freshness | Unknown preserves last valid episode state; display observation time without claiming data freshness | Dedicated missing/stale-data conditions, requiring a freshness contract |
| D6: lifecycle | Obsolete work cannot change current outcomes; edits/re-enable reset to unknown and apply D1 initial policy; disable suppresses unsent notifications; removal retains bounded audit history | Preserve episode across edits, drain existing deliveries, or immediate erasure; in-flight external sends cannot be recalled |
| D7: limits and recovery | Agree capacity, interval bounds, retry horizons, retention and recovery objectives from a synthetic workload before release | Different workload/SLA commitments; no numerical defaults approved |
| D8: destinations and content | Operators provision permitted destinations; admins select them; minimal plain-language summary with explicit time/number formats | Administrators provision destinations themselves, or richer messages; approve sharing and retention responsibilities |

## 12. Risks

| Risk | Impact | Mitigation |
|------|--------|------------|
| Invalid or stale source data | Misleading alerts | Distinguish unknown, observation time and source freshness; approve D5 |
| Ambiguous Discord acceptance | Duplicate messages on retry | Record uncertainty and stable notification identity; no exactly-once claim |
| Metric edits race evaluations | Notification describes obsolete logic | Approve revision and lifecycle behavior before implementation |
| Unsupported job-backend behavior | Lost, stuck or concurrently retried work | Compatibility evidence required by DESIGN |
| External sharing or excessive history | Data exposure and retention conflicts | Minimal content, restricted destinations and D8 review |
