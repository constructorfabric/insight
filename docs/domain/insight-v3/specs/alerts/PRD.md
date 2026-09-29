---
status: draft
version: "0.5"
date: 2026-09-28
---

# PRD — Insight v3 Metric Alerts

**Status:** Alert behavior, scheduling, the numeric contract, edit semantics, limits and destination provisioning are approved. Delivery to a provider is deferred to the next release; the first provider set stays open in D3.

**Revision 0.5:** Resolve D4–D8, defer delivery, and mark each requirement approved or deferred. Revision 0.4 made notification requirements destination-neutral.

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

**Approved:** Insight v3 custom metrics; administrator API and MCP; one numeric result per alert; threshold checks; a notification owed to a configured destination on the first breach; per-rule interval checks; the latest check and the owed notifications visible to administrators.

**Deferred to the next release:** sending the owed notification to a provider, delivery retries and delivery status. The notification exists and is visible before that; nothing is sent.

### 4.2 Out of Scope

Excluded: web administration UI, legacy analytics alerts and separate alerts per result group.

First release excludes reminder and recovery messages, cron schedules, replaying missed checks, and delivery itself. The initial set of notification providers is open in D3.

This PRD narrows the parent's actor: the parent names the dashboard author (`cpt-insightspec-v3-actor-dashboard-author`); alerts are administered by administrators only, because a rule runs a stored metric unattended and a notification leaves the product. It also widens the parent decomposition's destination, which named one provider: rules are provider-neutral and the provider set is D3.

## 5. Functional Requirements

Each requirement says whether it is approved for this release or deferred.

### 5.1 Rule Administration

#### Manage Rules

- [ ] `p1` - **ID**: `cpt-insightspec-v3-alerts-fr-manage` (approved)

The system **MUST** let administrators create, read, list, update, enable, disable and delete rules through API and MCP, using the same validation.

Each rule has a name administrators read and a generated identity every operation takes; names need not be unique. A rule selects one existing custom metric, one result column, a threshold condition, an interval and one destination. Every write bumps the rule's revision; an update names the revision it replaces and is refused when the rule has moved on. Deleting a rule removes it and everything recorded for it.

**Actors**: `cpt-insightspec-v3-alerts-actor-admin`

#### Restrict Administration

- [ ] `p1` - **ID**: `cpt-insightspec-v3-alerts-fr-authorize` (approved)

The system **MUST** restrict rule management, destinations, check results and notifications to authenticated administrators on both API and MCP. Checks continue while no administrator is signed in.

**Actors**: `cpt-insightspec-v3-alerts-actor-admin`

### 5.2 Evaluation and Notifications

#### Evaluate a Scalar

- [ ] `p1` - **ID**: `cpt-insightspec-v3-alerts-fr-evaluate` (approved)

The system **MUST** run the same saved metric definition as an on-demand check and compare one numeric result with the threshold. It must not silently choose a row or combine grouped results.

A valid result is exactly one row and the named column holding a finite number. Empty, null, non-numeric, non-finite, ambiguous and failed results are unknown, recorded with a reason, and are not valid non-breaches. Operators are `>`, `>=`, `<`, `<=`; integers compare exactly and are never rounded through a float.

**Actors**: `cpt-insightspec-v3-alerts-actor-admin`

#### Evaluate Unattended

- [ ] `p1` - **ID**: `cpt-insightspec-v3-alerts-fr-schedule` (approved)

The system **MUST** check enabled rules at a configured per-rule interval. After downtime, it checks the current value once rather than replaying missed checks. The interval changes neither the metric's data window nor how fresh its source data is.

**Actors**: `cpt-insightspec-v3-alerts-actor-admin`

#### Detect Breach Episodes

- [ ] `p1` - **ID**: `cpt-insightspec-v3-alerts-fr-episodes` (approved)

The system **MUST**:

- Notify on the first valid breached result.
- Suppress further notifications until a valid result no longer breaches.
- Allow a new notification when the condition is met again.
- Never treat an unknown result as recovery.

The first release sends neither repeated reminders during a breach nor recovery messages.

Editing, enabling or re-enabling a rule resets what its checks found, so the first breach seen afterwards is notified. Disabling withdraws notifications not yet sent.

**Actors**: `cpt-insightspec-v3-alerts-actor-recipient`

#### Deliver Notifications

- [ ] `p1` - **ID**: `cpt-insightspec-v3-alerts-fr-deliver` (deferred: next release)

The system **MUST** record the rule, metric, value, condition and check time a notification carries in the same step as the check that owes it, and send them to the configured destination.

Recording is in this release; sending is deferred. When delivery arrives it retries without rerunning the metric or creating another episode, shows permanent failures and exhausted retries, never reports an uncertain outcome as confirmed delivery, and may produce duplicate messages on retry.

**Actors**: `cpt-insightspec-v3-alerts-actor-provider`, `cpt-insightspec-v3-alerts-actor-recipient`

#### Inspect Outcomes

- [ ] `p1` - **ID**: `cpt-insightspec-v3-alerts-fr-inspect` (approved; delivery status deferred with delivery)

The system **MUST** show rule status, revision, latest check time, outcome and reason, latest valid value, and the notifications owed with their status. Each notification carries the revision and configuration it was owed under, and the value and condition the check saw, without exposing secrets.

**Actors**: `cpt-insightspec-v3-alerts-actor-admin`

## 6. Non-Functional Requirements

### 6.1 Quality Vector Analysis

| Quality Vector | Show-Stopper Requirement | Rationale |
|----------------|-------------------------|-----------|
| Efficiency | Alert work MUST remain within the parent footprint obligation, `cpt-insightspec-v3-nfr-efficiency`; local allocation is pending | Monitoring must not make the service uneconomical to operate |
| Reliability | Committed notification intents MUST survive process restart, `cpt-insightspec-v3-alerts-nfr-durability` | Silent loss defeats unattended monitoring |
| Performance | None — no obligation; Performance is material and 6.2 carries no covering NFR (gap): `cpt-insightspec-v3-alerts-nfr-timeliness` states what is reported, its targets await synthetic load | Administrators need to know how late a notification may arrive |
| Security | Destination secrets MUST remain absent from ordinary reads and diagnostics, `cpt-insightspec-v3-alerts-nfr-confidentiality` | A leaked credential grants external posting authority |
| Versatility | None — no show-stopper; obligations covered by `cpt-insightspec-v3-nfr-versatility` | Administrators must create new rules without engineering involvement |

### 6.2 NFR Inclusions

#### Durable and Explainable Outcomes

- [ ] `p1` - **ID**: `cpt-insightspec-v3-alerts-nfr-durability`

The system **MUST** preserve committed notification intent and prevent duplicate logical episodes under restart, retry and concurrent workers.

**Threshold**: a check and the notification it owes are written together or not at all; a check repeated after interruption owes nothing more; a check for a replaced or disabled rule is discarded. Provider messages may still duplicate once delivery exists.

**Rationale**: Restarting a worker must not lose an alert.

#### Timeliness and Capacity

- [ ] `p1` - **ID**: `cpt-insightspec-v3-alerts-nfr-timeliness`

The system **MUST** report check duration per rule and enforce explicit capacity limits.

**Threshold**: interval 60 seconds to 7 days, at most 200 rules, 4 concurrent checks, a 60-second check lock, and the newest 200 notifications kept per rule, each configurable per installation. No latency target is approved; p95 due-to-check targets await synthetic load.

**Rationale**: Show overload and protect on-demand metric checks.

#### Confidentiality

- [ ] `p1` - **ID**: `cpt-insightspec-v3-alerts-nfr-confidentiality`

The system **MUST** keep destination credentials out of normal API/MCP reads, notifications, queued payloads and diagnostics. Notifications must contain only the approved minimal metric summary, not complete query results.

**Threshold**: zero credential disclosure in those surfaces. A destination read answers its name and provider only.

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

- **D3 — First delivery providers.** Open. No provider ships in this release; the notification is recorded and visible. Discord, Telegram and Zulip remain the candidates, and rules stay provider-neutral.
- **D4 — Numeric values.** Resolved: exactly one row and one selected column; `>`, `>=`, `<`, `<=`; integers exact, floats as double precision, an integer compared with a float only where the conversion is exact, otherwise unknown.
- **D5 — Unknown data.** Resolved: an unknown check keeps the last valid finding and records its time and reason. No freshness condition; the check time is shown without claiming the source data is fresh.
- **D6 — Edits, disabling and deletion.** Resolved: every write bumps a revision and an update names the one it replaces; a check for any other revision is discarded. Editing, enabling and re-enabling reset the finding; disabling withdraws unsent notifications; deleting removes the rule and its notifications.
- **D7 — Limits.** Resolved for this release: interval 60 seconds to 7 days, 200 rules, 4 concurrent checks, 60-second check lock, 200 notifications kept per rule, each configurable. Latency targets and delivery retry duration await synthetic load and delivery.
- **D8 — Destinations and content.** Resolved: the operator provisions destinations in configuration; each has a name and a provider; administrators reference the name. Per-administrator destinations follow behind the same interface. The notification carries rule, metric, value, condition and check time; message formatting is decided with delivery.

## 12. Risks

| Risk | Impact | Mitigation |
|------|--------|------------|
| Invalid or stale source data | Misleading alerts | Distinguish unknown, observation time and source freshness; approve D5 |
| Ambiguous provider acceptance | Duplicate messages on retry | Record uncertainty and stable notification identity; no exactly-once claim |
| Rule edits race checks | Notification describes obsolete logic | A check is recorded only at the revision it was scheduled for |
| Job library defect | Lost, stuck or concurrently retried work | Library chosen on measured behaviour ([ADR-0009](../ADR/0009-bullmq-schedules-alert-checks.md)); schedule reconciled from the rules at startup |
| Schedule store loses its data | Checks stop until restart | Operator requirement that Redis persists; startup reconcile |
| External sharing or excessive history | Data exposure and retention conflicts | Minimal content, restricted destinations and D8 review |
