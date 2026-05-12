# PRD — ChatGPT Team Connector

> Version 1.0 — March 2026
> Based on: `docs/CONNECTORS_REFERENCE.md` Source 19 (ChatGPT Team)

<!-- toc -->

- [1. Overview](#1-overview)
  - [1.1 Purpose](#11-purpose)
  - [1.2 Background / Problem Statement](#12-background--problem-statement)
  - [1.3 Goals](#13-goals)
  - [1.4 Glossary](#14-glossary)
- [2. Actors](#2-actors)
  - [2.1 Human Actors](#21-human-actors)
  - [2.2 System Actors](#22-system-actors)
- [3. Scope](#3-scope)
  - [3.1 In Scope](#31-in-scope)
  - [3.2 Out of Scope](#32-out-of-scope)
- [4. Functional Requirements](#4-functional-requirements)
  - [4.1 Seat Data Collection](#41-seat-data-collection)
  - [4.2 Activity Data Collection](#42-activity-data-collection)
  - [4.3 Identity Resolution](#43-identity-resolution)
  - [4.4 Silver / Gold Pipeline](#44-silver--gold-pipeline)
- [5. Non-Functional Requirements](#5-non-functional-requirements)
- [6. Open Questions](#6-open-questions)

<!-- /toc -->

---

## 1. Overview

### 1.1 Purpose

The ChatGPT Team connector collects seat assignment and daily AI tool usage data from OpenAI's Admin API for ChatGPT Team/Enterprise workspaces. It enables workspace admins and analytics teams to track AI assistant adoption, seat utilization, and conversational usage patterns across the organization's ChatGPT Team subscription.

### 1.2 Background / Problem Statement

Organizations running ChatGPT Team subscriptions have no centralized visibility into who is actively using AI tools, which models are being used, and how usage varies by client (web, desktop, mobile). The OpenAI Admin API exposes this data, but it must be ingested, identity-resolved, and unified with other AI tool sources (Claude Team Plan) to be actionable.

Unlike the OpenAI API connector (programmatic access), ChatGPT Team covers flat per-seat conversational usage — different billing model, different clients, different analytics purpose.

### 1.3 Goals

- Collect complete seat roster and daily activity data from the ChatGPT Team workspace.
- Resolve user identity (`email`) to canonical `person_id` for cross-system analytics.
- Feed `class_ai_tool_usage` Silver stream alongside Claude Team Plan for unified AI tool adoption reporting.
- Enable Gold-level metrics: active users, conversation volume, model distribution, client breakdown.

### 1.4 Glossary

| Term | Definition |
|------|------------|
| Seat | An assigned ChatGPT Team subscription slot for a specific user |
| Activity | Daily per-user usage record: conversations, messages, tokens by model and client |
| `person_id` | Canonical cross-system person identifier resolved by the Identity Manager |
| Silver stream | Unified, identity-resolved dataset joining multiple Bronze sources |
| `class_ai_tool_usage` | Silver stream for conversational AI tool usage (ChatGPT Team + Claude Team web/mobile) |
| `class_ai_api_usage` | Silver stream for programmatic API usage — distinct from tool usage |
| `class_ai_dev_usage` | Silver stream for IDE/coding AI tool usage (Cursor, Windsurf, Claude Code) |

---

## 2. Actors

### 2.1 Human Actors

#### Workspace Administrator

**ID**: `cpt-insightspec-actor-chatgpt-team-admin`

**Role**: Manages the ChatGPT Team subscription, grants/revokes seat access, monitors usage.
**Needs**: Visibility into seat utilization, inactive seats, and overall adoption trends.

#### Analytics Engineer

**ID**: `cpt-insightspec-actor-chatgpt-team-analytics-eng`

**Role**: Designs and maintains the Silver/Gold pipeline that consumes ChatGPT Team Bronze data.
**Needs**: Reliable, schema-stable Bronze tables with consistent identity fields for joining with other sources.

### 2.2 System Actors

#### OpenAI Admin API

**ID**: `cpt-insightspec-actor-chatgpt-team-openai-api`

**Role**: Source of seat assignment and usage data. Provides workspace user management and usage reports for Team/Enterprise accounts.

#### Identity Manager

**ID**: `cpt-insightspec-actor-chatgpt-team-identity-mgr`

**Role**: Resolves `email` from Bronze tables to canonical `person_id` used in Silver/Gold layers.

---

## 3. Scope

### 3.1 In Scope

- Collection of current seat assignments (who has a seat, their role, status, and activity timestamps).
- Collection of daily usage activity per user, per model, per client (web, desktop, mobile).
- Connector execution logging for monitoring and observability.
- Identity resolution of `email` → `person_id` in the Silver step.
- Feeding `class_ai_tool_usage` Silver stream for conversational usage.

### 3.2 Out of Scope

- Programmatic OpenAI API usage — covered by the OpenAI API connector (`class_ai_api_usage`).
- IDE/coding assistant usage (ChatGPT plugins in IDEs) — not exposed via the Admin API seat/activity endpoints.
- Real-time or sub-daily granularity — the Admin API provides daily aggregates only.
- Historical backfill beyond the Admin API's available lookback window.
- Versioning or history of seat assignment changes (current-state only).

---

## 4. Functional Requirements

### 4.1 Seat Data Collection

#### Collect seat roster

- [ ] `p1` - **ID**: `cpt-insightspec-fr-chatgpt-team-seats-collect`

The connector **MUST** collect all current seat assignments from the OpenAI Admin API, capturing each user's identifier, email, role, status, and activity timestamps.

**Rationale**: Seat roster is the foundation for utilization reporting — without it, activity data cannot be attributed to provisioned users.
**Actors**: `cpt-insightspec-actor-chatgpt-team-admin`, `cpt-insightspec-actor-chatgpt-team-analytics-eng`

#### Represent seat data as current-state snapshot

- [ ] `p2` - **ID**: `cpt-insightspec-fr-chatgpt-team-seats-snapshot`

The seat collection **MUST** represent current-state only (one row per user, no historical versioning), consistent with the source API's snapshot model.

**Rationale**: The OpenAI Admin API does not provide seat change history; the Bronze table must accurately reflect its capabilities.
**Actors**: `cpt-insightspec-actor-chatgpt-team-analytics-eng`

### 4.2 Activity Data Collection

#### Collect daily usage activity

- [ ] `p1` - **ID**: `cpt-insightspec-fr-chatgpt-team-activity-collect`

The connector **MUST** collect daily usage records per user, per model, and per client, capturing conversation count, message count, and token consumption (input, output, reasoning).

**Rationale**: Daily activity is the primary signal for AI tool adoption analytics — frequency of use, model preferences, and client surface breakdown.
**Actors**: `cpt-insightspec-actor-chatgpt-team-admin`, `cpt-insightspec-actor-chatgpt-team-analytics-eng`

#### Include reasoning tokens for o1/o3 models

- [ ] `p2` - **ID**: `cpt-insightspec-fr-chatgpt-team-reasoning-tokens`

The activity collection **MUST** capture `reasoning_tokens` for o1/o3 model usage even though they are not billed separately under the flat subscription.

**Rationale**: Reasoning token visibility allows analytics on model complexity and usage patterns for reasoning-capable models.
**Actors**: `cpt-insightspec-actor-chatgpt-team-analytics-eng`

#### Log connector execution

- [ ] `p1` - **ID**: `cpt-insightspec-fr-chatgpt-team-collection-runs`

The connector **MUST** record each execution run with start/end time, status, record counts, API call count, and error count.

**Rationale**: Execution logs are required for monitoring data freshness, diagnosing failures, and auditing pipeline health.
**Actors**: `cpt-insightspec-actor-chatgpt-team-analytics-eng`

### 4.3 Identity Resolution

#### Resolve email to person_id

- [ ] `p1` - **ID**: `cpt-insightspec-fr-chatgpt-team-identity-resolve`

The Silver pipeline **MUST** resolve `email` from both seat and activity Bronze tables to a canonical `person_id` via the Identity Manager.

**Rationale**: Cross-system analytics (e.g. joining AI tool usage with HR data or task tracker activity) requires a stable, source-independent person identifier.
**Actors**: `cpt-insightspec-actor-chatgpt-team-identity-mgr`, `cpt-insightspec-actor-chatgpt-team-analytics-eng`

#### Use email as the sole identity key

- [ ] `p2` - **ID**: `cpt-insightspec-fr-chatgpt-team-identity-key`

The connector **MUST** treat `email` as the primary identity key for resolution. The OpenAI-internal `user_id` field **MUST NOT** be used for cross-system identity resolution.

**Rationale**: `user_id` is an OpenAI-platform-internal identifier not meaningful outside the OpenAI ecosystem. Email is the stable cross-system key.
**Actors**: `cpt-insightspec-actor-chatgpt-team-identity-mgr`

### 4.4 Silver / Gold Pipeline

#### Feed class_ai_tool_usage

- [ ] `p1` - **ID**: `cpt-insightspec-fr-chatgpt-team-silver-tool-usage`

Daily activity data **MUST** feed the `class_ai_tool_usage` Silver stream, unified with Claude Team Plan web/mobile activity under a common schema.

**Rationale**: Unified AI tool adoption analytics require a single Silver stream spanning all conversational AI tools (ChatGPT Team + Claude Team web/mobile).
**Actors**: `cpt-insightspec-actor-chatgpt-team-analytics-eng`

#### Keep class_ai_tool_usage separate from API and dev usage

- [ ] `p1` - **ID**: `cpt-insightspec-fr-chatgpt-team-silver-separation`

The `class_ai_tool_usage` stream **MUST NOT** merge with `class_ai_api_usage` (programmatic API) or `class_ai_dev_usage` (IDE/coding tools). Cross-stream analysis **MUST** be performed at Gold level using `person_id`.

**Rationale**: Conversational usage (flat-seat, web/mobile), programmatic API usage (pay-per-token, code-driven), and IDE coding assistant usage serve distinct analytics purposes and have incompatible schemas.
**Actors**: `cpt-insightspec-actor-chatgpt-team-analytics-eng`

---

## 5. Non-Functional Requirements

#### Data freshness

- [ ] `p2` - **ID**: `cpt-insightspec-nfr-chatgpt-team-freshness`

The connector **MUST** be executable on a daily schedule such that activity data for day D is available by the start of day D+2 (accounting for the Admin API's reporting lag).

**Threshold**: ≤ 48 hours end-to-end latency from activity occurrence to Bronze availability.
**Rationale**: Daily AI tool adoption reports require timely data; a 48h window accommodates known API reporting delays.

#### Schema stability

- [ ] `p2` - **ID**: `cpt-insightspec-nfr-chatgpt-team-schema-stability`

Bronze table schemas **MUST** remain stable across connector versions. Breaking schema changes **MUST** be versioned with migration guidance.

**Threshold**: Zero unannounced breaking changes to field names or types in `chatgpt_team_seats`, `chatgpt_team_activity`, `chatgpt_team_collection_runs`.
**Rationale**: Downstream Silver/Gold pipelines depend on stable Bronze schemas.

---

## 6. Open Questions

### OQ-CGT-1: ChatGPT Team vs OpenAI API for the same user

A developer may use both ChatGPT Team (via web/desktop) and the OpenAI API (programmatic calls). The same person generates usage in both.

**Status**: CLOSED. `class_ai_tool_usage` (conversational) and `class_ai_api_usage` (programmatic) are separate Silver streams. Cross-stream analysis by `person_id` is performed at Gold level.

### OQ-CGT-2: Unified Silver schema with Claude Admin (Anthropic seats)

ChatGPT Team and the Anthropic Admin API (via `claude-admin`) have similar data shapes for seats + daily Claude Code activity. A unified `class_ai_tool_usage` schema must accommodate both — note that Anthropic does not expose web/mobile chat usage through the Admin API:

- `data_source`: `insight_claude_admin` / `insight_chatgpt_team`
- Shared fields: `date`, `email`, `client`, `model`, token counts, `message_count`, `conversation_count`
- Claude-specific: `tool_use_count`, `cache_write_tokens`, `cache_read_tokens`
- OpenAI-specific: `reasoning_tokens`

**Open**: Should source-specific fields use explicit nullable columns or a jsonb `extras`?
