---
status: draft
version: "0.9"
date: 2026-10-15
---

# PRD — Insight v3 Metric Alerts

**Status:** Alert behavior, scheduling, the condition model, edit semantics, limits, destination provisioning, delivery and the administrator screens in the portal are approved. The first providers are Discord, Telegram, Slack and Zulip.

**Revision 0.9:** A rule may name context columns, result columns the witness carries beside the ones the condition reads, so a notification can say who or what crossed the threshold (D8, D9). Revision 0.8: conditions cover every row one metric run answers: a row quantifier, nested groups, and number, text and date tests (D4, D9). Notifications and checks show the rows that met the condition, and the portal's form previews a draft on the server. Revision 0.7 moved the administrator screens into scope; 0.6 approved delivery and resolved D3; 0.5 resolved D4–D8.

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

Send a notification when an Insight v3 custom metric's result meets a condition. Administrators configure alerts in the portal's Custom zone, through the API or through the Model Context Protocol (MCP). Alerts run without a dashboard open.

This expands [the parent alert requirement](../PRD.md#58-alerts), `cpt-insightspec-v3-fr-create-alerts`. Rules take effect without a service release. Checks run on a schedule, so detection is not instantaneous.

### 1.2 Background / Problem Statement

On-demand metrics need someone to run them. Alerts check metrics automatically and show whether each check and message delivery succeeded.

### 1.3 Goals (Business Outcomes)

- Configure and inspect alerts in the portal, through the API or through MCP, with one set of rules behind all three.
- Detect breaches automatically and show the rows that triggered them.
- Keep pending work through restarts and show failures. Report delivery only when confirmed.

Verify these goals with synthetic metrics before release. Capacity and timing targets remain open in D7.

### 1.4 Glossary

| Term | Definition |
|------|------------|
| Rule | A metric, a condition over its result rows, a schedule and a destination |
| Condition | A row quantifier and a test on each row: one column test, or an `all`, `any` or `not` group of tests |
| Evaluation | One metric check for a rule |
| Breach | A valid result whose rows satisfy the condition's test as its row quantifier requires |
| Witness | The rows a check shows for its finding, at most five, with how many rows matched out of how many the metric answered |
| Context column | A result column a rule names so the witness carries it beside the columns the condition reads |
| Episode | Consecutive valid breached results, ending when a valid result no longer breaches |
| Notification | A saved request to send a message; delivery is tracked separately |
| Unknown | A check could not determine whether the condition was met |

## 2. Actors

### 2.1 Human Actors

#### Administrator

**ID**: `cpt-insightspec-v3-alerts-actor-admin`

**Role**: Manages rules and inspects destination details in the portal's Custom zone, through API or through MCP. Must be an authenticated Insight v3 administrator.

**Needs**: Clear validation, understandable results and delivery errors, with credentials hidden.

#### Notification Recipient

**ID**: `cpt-insightspec-v3-alerts-actor-recipient`

**Role**: Reads notifications at a destination configured by an administrator. Destination membership is managed outside Insight.

**Needs**: Metric identity, the condition, the witness and evaluation time, without unrelated result rows or columns.

### 2.2 System Actors

#### Notification Provider

**ID**: `cpt-insightspec-v3-alerts-actor-provider`

**Role**: Receives notification requests and reports acceptance or failure. Its availability is outside Insight's control.

## 3. Operational Concept & Environment

### 3.1 Module-Specific Environment Constraints

Checks and delivery run without an active API client. [DESIGN](./DESIGN.md) records the approved job library and the completed database compatibility findings.

## 4. Scope

### 4.1 In Scope

**Approved:** Insight v3 custom metrics, scalar, grouped and raw-row alike; administrator API and MCP; a condition over the rows one metric run answers, with a row quantifier and number, text and date tests in nested groups; a server-side preview of a draft; a notification owed to a configured destination on the first breach; per-rule interval checks; the latest check and the owed notifications visible to administrators.

**Approved, delivery:** the owed notification is sent to its destination through the configured provider, retried while the outcome is unknown, and shown with its delivery status.

**Approved, administrator screens:** an Alerts entry in the portal's Custom zone lists the rules with a switch for their checks, opens each rule with its latest check and its notifications, and creates or edits a rule on a page of its own. The form builds the whole condition, nested groups included, and switches between the editor and the rule draft as JSON, the same body the API and MCP take. It shows what a check would find for the draft by asking the server to preview it. The screens call the administrator API and add no behaviour of their own; the list shows name, metric and whether checks are on, and the latest check appears on the rule's page.

### 4.2 Out of Scope

Excluded: legacy analytics alerts and separate alerts per result group. A grouped metric is watched through the row quantifier, and its breaches form one episode per rule.

A condition reads one metric run: no arithmetic between columns, no comparison with an earlier window or another metric, and no condition held over several checks. First release excludes severity tiers, reminder and recovery messages, cron schedules, replaying missed checks, and administrator-managed destinations.

This PRD narrows the parent's actor: the parent names the dashboard author (`cpt-insightspec-v3-actor-dashboard-author`); alerts are administered by administrators only, because a rule runs a stored metric unattended and a notification leaves the product. It also widens the parent decomposition's destination, which named one provider: rules are provider-neutral and the provider set is D3.

## 5. Functional Requirements

Every requirement below is approved for this release.

### 5.1 Rule Administration

#### Manage Rules

- [ ] `p1` - **ID**: `cpt-insightspec-v3-alerts-fr-manage` (approved)

The system **MUST** let administrators create, read, list, update, enable, disable and delete rules through API and MCP, using the same validation. The portal's screens perform these operations through the API and surface its refusals on the field they concern.

Each rule has a name administrators read and a generated identity every operation takes; names need not be unique. A rule selects one existing custom metric, a condition over its result rows, up to five context columns, an interval and one destination. Each test in the condition is checked against the kind of column it reads when the rule is written, and a refusal names the path of the part it concerns; each context column must be one the metric answers and does not group by, named once. An administrator can preview a draft: the system evaluates it as a check would and answers the outcome, reason and witness, recording nothing and owing no notification. Every write bumps the rule's revision; an update names the revision it replaces and is refused when the rule has moved on. Deleting a rule removes it and everything recorded for it.

**Actors**: `cpt-insightspec-v3-alerts-actor-admin`

#### Restrict Administration

- [ ] `p1` - **ID**: `cpt-insightspec-v3-alerts-fr-authorize` (approved)

The system **MUST** restrict rule management, destinations, check results and notifications to authenticated administrators on both API and MCP. The portal's screens sit behind the Custom zone's administrator gate and inherit the API's refusals. Checks continue while no administrator is signed in.

**Actors**: `cpt-insightspec-v3-alerts-actor-admin`

### 5.2 Evaluation and Notifications

#### Evaluate a Condition

- [ ] `p1` - **ID**: `cpt-insightspec-v3-alerts-fr-evaluate` (approved)

The system **MUST** run the same saved metric definition as an on-demand check and evaluate the rule's condition over every row it answers. It must not silently choose a row.

A condition is a row quantifier and a test on each row. The test is one column test, or an `all`, `any` or `not` group of tests. A number column takes `>`, `>=`, `<`, `<=`, `==`, `!=` and `between`; a text column takes `==`, `!=`, `contains`, `starts_with`, `ends_with`, `in` and `not_in`, compared exactly and case-sensitively; a date column takes `before`, `after`, `between`, `older_than` and `newer_than`, the last two measured back from the check's own time. The quantifier is `one`, `any`, `all`, `none`, or a count of passing rows compared with a number (D9).

A test on a missing column, a null, or a cell its operator cannot read is unknown, not false. A group or a quantifier is unknown when an unknown part could change its answer. Unknown and failed checks are recorded with a reason and are not valid non-breaches. Integers compare exactly and are never rounded through a float.

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

An episode belongs to the rule: it spans consecutive breached results whichever rows breach. The first release sends neither repeated reminders during a breach nor recovery messages.

Editing, enabling or re-enabling a rule resets what its checks found, so the first breach seen afterwards is notified. Disabling withdraws notifications not yet sent.

**Actors**: `cpt-insightspec-v3-alerts-actor-recipient`

#### Deliver Notifications

- [ ] `p1` - **ID**: `cpt-insightspec-v3-alerts-fr-deliver` (approved)

The system **MUST** record the rule, metric, condition, witness and check time a notification carries in the same step as the check that owes it, and send them to the configured destination. The message states the condition in words, and for a condition over many rows how many rows matched out of how many the metric answered; a witness row is written with the columns the condition reads and the rule's context columns, each as its name and value.

Delivery retries an unconfirmed send without rerunning the metric or creating another episode, gives up after the configured attempts, shows rejections and exhausted retries, never reports an unconfirmed send as delivered, and may produce a duplicate message when a send landed but its confirmation did not.

**Actors**: `cpt-insightspec-v3-alerts-actor-provider`, `cpt-insightspec-v3-alerts-actor-recipient`

#### Inspect Outcomes

- [ ] `p1` - **ID**: `cpt-insightspec-v3-alerts-fr-inspect` (approved)

The system **MUST** show rule status, revision, latest check time, outcome and reason, the latest check's witness, and the notifications owed with their delivery status, attempts, last error and provider receipt. Each notification carries the revision and configuration it was owed under, and the condition, context columns and witness the check saw, without exposing secrets.

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

**Threshold**: interval 60 seconds to 7 days, at most 200 rules, 4 concurrent checks, a 60-second check lock, and the newest 200 notifications kept per rule, never dropping one still owed, each configurable per installation. No latency target is approved; p95 due-to-check targets await synthetic load.

**Rationale**: Show overload and protect on-demand metric checks.

#### Confidentiality

- [ ] `p1` - **ID**: `cpt-insightspec-v3-alerts-nfr-confidentiality`

The system **MUST** keep destination credentials out of normal API/MCP reads, notifications, queued payloads and diagnostics. Notifications must contain only the approved minimal metric summary - the witness, cut to the columns the condition reads and the rule names - not complete query results.

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

No parent NFR is excluded. The administrator screens inherit the parent's dashboard and accessibility targets; their component and story tests check validation, conflict handling, paging and phone-width layout, and browser measurements are pending. API/MCP documentation and readable notification content still apply. External sharing and retention responsibilities require D8.

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

**Compatibility**: Provider rate limits and payload constraints apply. The first provider set is D3; Insight does not control destination membership or provider retention.

## 8. Use Cases

#### Configure and Monitor a Condition

- [ ] `p1` - **ID**: `cpt-insightspec-v3-alerts-usecase-monitor`

**Actor**: `cpt-insightspec-v3-alerts-actor-admin`

**Preconditions**: A custom metric and permitted destination exist.

**Main Flow**:

1. Preview a draft, then create and enable the rule.
2. Inspect its check result and witness.
3. Receive a notification when the breach policy calls for one.

**Postconditions**: The evaluation and delivery outcome can be inspected independently.

**Alternative Flows**: A result the condition cannot decide is reported as unknown with its reason; provider failure leaves a visible delivery outcome; disabling or editing follows D6.

## 9. Acceptance Criteria

All evidence is pending. Engineering verifies these against synthetic inputs after the referenced decisions are approved.

- [ ] API and MCP both support the approved rule lifecycle and reject non-administrators.
- [ ] The portal's screens create, edit, enable, disable and delete a rule through the API, keep the input and offer the latest version when a save lost a race, and show the latest check with its witness and the notifications with their delivery status.
- [ ] The portal builds every condition the API accepts, nested groups and the row quantifier included, in the editor or as the rule's JSON, and switches between the two without loss.
- [ ] Alert evaluation agrees with on-demand execution of the same metric definition, and a preview answers the outcome a check records for the same draft and rows.
- [ ] A test whose operator does not fit its column's kind, groups nested deeper than 3, more than 20 tests, and an `in` list past 50 values are refused with the path of the part concerned.
- [ ] Over zero rows, `any` is not a breach, a count compares zero, and `one`, `all` and `none` are unknown; `one` over several rows is unknown.
- [ ] A null cell inside `all` makes the check unknown, never a clear; inside `any` beside a passing test it passes. `older_than` and `newer_than` measure back from the check's own time.
- [ ] A rule over a grouped metric with `any` breaches when one group meets the test, names at most five matching rows with their group columns, and reports how many rows matched out of how many answered.
- [ ] First-breach notification, suppression during a breach, a valid clear, invalid-result and approved edit behavior are demonstrated.
- [ ] Per-rule interval checks resume with one current check after downtime, without replaying missed intervals.
- [ ] Restart and concurrent execution neither lose committed intents nor create duplicate logical episodes.
- [ ] Delivery failures are visible without metric re-execution; uncertain external results retain their uncertainty.
- [ ] Every selected delivery provider receives the approved summary; no credentials, and no metric rows or columns beyond the witness, appear in any public surface.
- [ ] Approved capacity targets and unchanged parent quality gates have evidence before release, including parent coverage and non-degradation requirements.

## 10. Dependencies

| Dependency | Description | Criticality |
|------------|-------------|-------------|
| Insight v3 custom metrics | Saved definitions and consistent on-demand evaluation | p1 |
| Insight administrator identity | Authority for API and MCP management, and the Custom zone's administrator gate in the portal | p1 |
| Durable jobs and state | Unattended evaluation and delivery recovery; see DESIGN | p1 |
| Selected notification providers | External delivery and acceptance | p1 |

## 11. Assumptions

D3–D9 are resolved; the product owner approved each, with engineering and security input where needed.

- **D3 — First delivery providers.** Resolved: Discord (incoming webhook), Telegram (bot), Slack (bot to a channel) and Zulip (bot to a stream topic). Rules stay provider-neutral; a destination names its provider in configuration.
- **D4 — Conditions.** Resolved: a condition is a row quantifier (D9) and a tree of tests. Groups are `all`, `any` and `not`, nested at most 3 deep, holding at most 20 tests in all. Each test reads one column with an operator its kind takes, as `cpt-insightspec-v3-alerts-fr-evaluate` lists them; the kind comes from the metric definition and is checked when the rule is written. `in` and `not_in` take at most 50 values; a text value holds at most 256 characters; `older_than` and `newer_than` reach back at most 100 years. Integers exact, floats as double precision, an integer compared with a float only where the conversion is exact, otherwise unknown. A boolean cell compares as the text `true` or `false`. Tests combine three-valued: `all` is false when any part is false, `any` is true when any part is true, `not` keeps an unknown unknown, and otherwise an unknown part leaves the group unknown.
- **D5 — Unknown data.** Resolved: an unknown check keeps the last valid finding and records its time and reason. No freshness condition; the check time is shown without claiming the source data is fresh.
- **D6 — Edits, disabling and deletion.** Resolved: every write bumps a revision and an update names the one it replaces; a check for any other revision is discarded. Editing, enabling and re-enabling reset the finding; disabling withdraws unsent notifications; deleting removes the rule and its notifications.
- **D7 — Limits.** Resolved for this release: interval 60 seconds to 7 days, 200 rules, 4 concurrent checks, 60-second check lock, 200 notifications kept per rule, 5 delivery attempts with a doubling wait from 30 seconds, a 10-second provider call, 4 concurrent sends; each configurable. Latency targets await synthetic load.
- **D8 — Destinations and content.** Resolved: the operator provisions destinations in configuration; each has a name, a provider and that provider's credentials; administrators reference the name and read only the name and provider. Per-administrator destinations follow behind the same interface. The message carries the rule, the metric, the condition in words, the witness and the check time in UTC, within a fixed length. A rule names up to five context columns; the witness carries them after the condition's columns, so an administrator decides which columns leave the product.
- **D9 — Row quantifier and witness.** Resolved: `one` breaches when the metric answers exactly one row and it passes; `any` when at least one row passes; `all` when every row passes; `none` when no row passes; a count when the number of passing rows meets its comparison. Over zero rows `any` is not a breach, a count compares zero, and `one`, `all` and `none` are unknown (`no_rows`); `one` over several rows is unknown (`many_rows`). The witness is at most 5 rows in result order, each holding only the metric's group-by columns, the columns the condition names and the rule's context columns, each once, with how many rows matched out of how many answered. A context column the run no longer answers is left out; it never makes a check unknown. Under `one` it holds the one row whatever the test found; under every other quantifier it holds rows that passed, so a breach under `none` shows no rows and the counts carry it. An unknown check has no witness. A run stops at the service's row bound when the metric names no bound of its own, or names more than the service allows; rows past that bound count as unknown, so a cut run decides only what those rows could not change, and is unknown (`row_limit`) otherwise. Episodes stay per rule: one spans consecutive breached checks whichever rows breach, and an unknown check never ends one.

## 12. Risks

| Risk | Impact | Mitigation |
|------|--------|------------|
| Invalid or stale source data | Misleading alerts | Distinguish unknown, observation time and source freshness; approve D5 |
| Ambiguous provider acceptance | Duplicate messages on retry | Record uncertainty and stable notification identity; no exactly-once claim |
| Rule edits race checks | Notification describes obsolete logic | A check is recorded only at the revision it was scheduled for |
| Job library defect | Lost, stuck or concurrently retried work | Library chosen on measured behaviour ([ADR-0009](../ADR/0009-bullmq-schedules-alert-checks.md)); schedule reconciled from the rules at startup |
| Schedule store loses its data | Checks stop until the schedule is reconciled | Operator requirement that Redis persists; reconcile at startup and every five minutes |
| External sharing or excessive history | Data exposure and retention conflicts | Minimal content, restricted destinations and D8 review |
