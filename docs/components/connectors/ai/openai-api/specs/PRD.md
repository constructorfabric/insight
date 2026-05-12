# PRD — OpenAI API Connector

> Version 1.0 — March 2026
> Based on: `docs/CONNECTORS_REFERENCE.md` Source 18 (OpenAI API), OQ-3

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
  - [4.1 Daily Usage Collection](#41-daily-usage-collection)
  - [4.2 Per-Request Event Collection](#42-per-request-event-collection)
  - [4.3 Identity Resolution](#43-identity-resolution)
  - [4.4 Silver / Gold Pipeline](#44-silver--gold-pipeline)
- [5. Non-Functional Requirements](#5-non-functional-requirements)
- [6. Open Questions](#6-open-questions)

<!-- /toc -->

---

## 1. Overview

### 1.1 Purpose

The OpenAI API connector collects daily token usage aggregates and per-request event data from OpenAI's Usage API. It enables organizations to track programmatic AI API spend, attribute usage to teams or applications, and build cost analytics across API keys and models — including reasoning token accounting for o1/o3 models.

### 1.2 Background / Problem Statement

Organizations using the OpenAI API for internal tooling, automations, or AI-powered product features lack centralized visibility into API spend, per-key utilization, and per-application cost attribution. The OpenAI Usage API provides two complementary data surfaces: daily aggregates (always available) and per-request events (available only when callers instrument their requests with a `user` field in the request body).

The connector mirrors the Claude API connector structure — both are pay-per-token programmatic APIs with optional caller instrumentation for user attribution. The main OpenAI-specific difference is `reasoning_tokens` for o1/o3 models.

### 1.3 Goals

- Collect complete daily API usage aggregates per API key and model.
- Collect per-request events where caller instrumentation permits.
- Enable cost attribution by API key, model, and application tag.
- Account for `reasoning_tokens` correctly in cost analytics for o1/o3 models.
- Resolve `user_id` (when present) to canonical `person_id` for person-level analytics.
- Feed `class_ai_api_usage` Silver stream for cross-provider programmatic API cost analytics.

### 1.4 Glossary

| Term | Definition |
|------|------------|
| `api_key_id` | API key identifier (name or last-4 alias from OpenAI Platform) |
| `user` field | Optional field in OpenAI API request body — caller-defined user identifier |
| `application` | Caller-set convention tag identifying which product or service made an API call |
| `reasoning_tokens` | Internal thinking tokens consumed by o1/o3 reasoning models; billed but not visible in output |
| `person_id` | Canonical cross-system person identifier resolved by the Identity Manager |
| `class_ai_api_usage` | Silver stream for programmatic API usage (OpenAI API + Claude API) |
| Daily aggregate | One row per `(date, api_key_id, model)` — always available, no user attribution |
| Per-request event | One row per API request — available only when caller sets the `user` field |

---

## 2. Actors

### 2.1 Human Actors

#### Platform Engineer / Developer

**ID**: `cpt-insightspec-actor-openai-api-developer`

**Role**: Builds internal tooling or product features that call the OpenAI API.
**Needs**: Visibility into their application's API spend and token consumption, including reasoning token costs.

#### Analytics Engineer

**ID**: `cpt-insightspec-actor-openai-api-analytics-eng`

**Role**: Designs and maintains the Silver/Gold pipeline that consumes OpenAI API Bronze data.
**Needs**: Reliable Bronze tables with stable schemas, consistent cost fields including reasoning tokens, and clear attribution conventions compatible with the Claude API Silver schema.

### 2.2 System Actors

#### OpenAI Usage API

**ID**: `cpt-insightspec-actor-openai-api-openai-api`

**Role**: Source of daily usage aggregates and per-request event data for the organization's OpenAI organization.

#### Identity Manager

**ID**: `cpt-insightspec-actor-openai-api-identity-mgr`

**Role**: Maps `user_id` (caller-defined string from the request body `user` field) to canonical `person_id`. Requires client-specific configuration for each application's user ID convention.

---

## 3. Scope

### 3.1 In Scope

- Collection of daily token usage aggregates per API key per model, including `reasoning_tokens` for o1/o3.
- Collection of per-request events when callers instrument with a `user` field.
- Connector execution logging for monitoring and observability.
- Conditional identity resolution of `user_id` → `person_id` (only when `user_id` is present).
- Feeding `class_ai_api_usage` Silver stream alongside Claude API for cross-provider cost analytics.

### 3.2 Out of Scope

- Conversational ChatGPT Team usage — covered by the ChatGPT Team connector (`class_ai_tool_usage`).
- Enforcement of `user` field instrumentation in calling applications — this is a caller responsibility.
- Real-time or sub-daily granularity for daily aggregates — the Usage API provides daily resolution only.
- Per-prompt or per-token content — the connector collects metadata and counts, not prompt/response content.

---

## 4. Functional Requirements

### 4.1 Daily Usage Collection

#### Collect daily aggregates with reasoning tokens

- [ ] `p1` - **ID**: `cpt-insightspec-fr-openai-api-daily-collect`

The connector **MUST** collect daily token usage aggregates from the OpenAI Usage API, capturing request count, input/output/cached/reasoning tokens, and total cost per API key per model.

**Rationale**: Daily aggregates are always available and provide the baseline for API cost analytics. `reasoning_tokens` must be captured for accurate cost accounting of o1/o3 models.
**Actors**: `cpt-insightspec-actor-openai-api-analytics-eng`

#### Log connector execution

- [ ] `p1` - **ID**: `cpt-insightspec-fr-openai-api-collection-runs`

The connector **MUST** record each execution run with start/end time, status, record counts, API call count, and error count for both daily usage and request event collections.

**Rationale**: Execution logs are required for monitoring data freshness, diagnosing failures, and detecting under-instrumented traffic.
**Actors**: `cpt-insightspec-actor-openai-api-analytics-eng`

### 4.2 Per-Request Event Collection

#### Collect per-request events when available

- [ ] `p1` - **ID**: `cpt-insightspec-fr-openai-api-requests-collect`

The connector **MUST** collect per-request event records when callers have instrumented their requests with a `user` field. Records without this field **MUST NOT** be collected at this granularity.

**Rationale**: Per-request events enable person-level and application-level cost attribution, which is not possible from daily aggregates alone.
**Actors**: `cpt-insightspec-actor-openai-api-analytics-eng`, `cpt-insightspec-actor-openai-api-developer`

#### Treat user_id as nullable

- [ ] `p2` - **ID**: `cpt-insightspec-fr-openai-api-nullable-user`

The `user_id` field in per-request events **MUST** be treated as nullable. Rows with `user_id = NULL` are valid and represent requests where the caller did not set the `user` field.

**Rationale**: Not all callers instrument their requests; unattributed rows must still be collected for cost completeness.
**Actors**: `cpt-insightspec-actor-openai-api-analytics-eng`

### 4.3 Identity Resolution

#### Resolve user_id to person_id conditionally

- [ ] `p1` - **ID**: `cpt-insightspec-fr-openai-api-identity-resolve`

The Silver pipeline **MUST** resolve `user_id` to `person_id` via the Identity Manager when `user_id` is non-null. Rows with `user_id = NULL` **MUST** pass through with `person_id = NULL`.

**Rationale**: Cross-system analytics require `person_id`; NULL rows remain valid for cost attribution by API key and application.
**Actors**: `cpt-insightspec-actor-openai-api-identity-mgr`, `cpt-insightspec-actor-openai-api-analytics-eng`

#### Support caller-convention mapping

- [ ] `p2` - **ID**: `cpt-insightspec-fr-openai-api-identity-convention`

The Identity Manager configuration **MUST** support per-application mapping conventions, as `user_id` may be an email, employee ID, GitHub login, or other caller-defined identifier depending on the calling application.

**Rationale**: The OpenAI `user` field is a caller convention — there is no enforced format. The Identity Manager must be configurable per application.
**Actors**: `cpt-insightspec-actor-openai-api-identity-mgr`

### 4.4 Silver / Gold Pipeline

#### Feed class_ai_api_usage alongside Claude API

- [ ] `p1` - **ID**: `cpt-insightspec-fr-openai-api-silver-api-usage`

Both daily usage aggregates and per-request events **MUST** feed the `class_ai_api_usage` Silver stream. Rows with `person_id = NULL` are valid and **MUST NOT** be filtered out. The Silver schema **MUST** be shared with the Anthropic Admin API connector (`claude-admin`), with a `data_source` field distinguishing `insight_openai_api` from `insight_claude_admin`.

**Rationale**: A unified `class_ai_api_usage` stream enables cross-provider API cost comparison (OpenAI vs Anthropic) using a single analytics query.
**Actors**: `cpt-insightspec-actor-openai-api-analytics-eng`

#### Keep class_ai_api_usage separate from class_ai_tool_usage

- [ ] `p1` - **ID**: `cpt-insightspec-fr-openai-api-silver-separation`

`class_ai_api_usage` (programmatic, pay-per-token) **MUST NOT** be merged with `class_ai_tool_usage` (conversational, flat-seat). There **MUST NOT** be a combined `class_ai_usage` stream.

**Rationale**: Programmatic API usage and conversational tool usage serve different analytics purposes and have incompatible billing models and schemas.
**Actors**: `cpt-insightspec-actor-openai-api-analytics-eng`

---

## 5. Non-Functional Requirements

#### Data freshness

- [ ] `p2` - **ID**: `cpt-insightspec-nfr-openai-api-freshness`

The connector **MUST** be executable on a daily schedule such that daily usage data for day D is available within 48 hours of the end of day D.

**Threshold**: ≤ 48 hours end-to-end latency from API activity to Bronze availability.
**Rationale**: Daily cost reporting requires timely data; a 48h window accommodates known OpenAI Usage API reporting delays.

#### Reasoning token cost integrity

- [ ] `p2` - **ID**: `cpt-insightspec-nfr-openai-api-reasoning-cost`

The connector **MUST** preserve `reasoning_tokens` as a separate field and **MUST NOT** add it to `output_tokens`. Cost calculations at Silver/Gold **MUST** explicitly account for reasoning tokens as an additive cost component.

**Threshold**: `reasoning_tokens` retained as a distinct non-null (zero for non-reasoning models) field in all Bronze and Silver schemas.
**Rationale**: Reasoning tokens are billed separately and are not part of visible output — conflating them with output tokens would misrepresent both cost and output volume.

---

## 6. Open Questions

### OQ-OAPI-1: `reasoning_tokens` — cost accounting in Silver

`reasoning_tokens` are billed but not part of the visible output. For cost aggregation:

- Should Silver `total_tokens` = `input_tokens + output_tokens + reasoning_tokens`?
- Or should `reasoning_tokens` be a separate metric tracked distinctly?
- `cached_tokens` in OpenAI differs from Anthropic's `cache_read_tokens` + `cache_write_tokens` — how are these harmonized in `class_ai_api_usage`?

### OQ-OAPI-2: Unified Silver schema with Claude Admin (Anthropic)

OpenAI API and the Anthropic Admin API (via `claude-admin`) have nearly identical Bronze schemas for programmatic token usage. `class_ai_api_usage` would unify both:

- `data_source`: `insight_claude_admin` / `insight_openai_api`
- Shared fields: `date`, `api_key_id`, `model`, token counts, `cost_cents`
- OpenAI-specific: `reasoning_tokens`, `cached_tokens`
- Anthropic-specific: `cache_read_tokens`, `cache_write_tokens`

**Open**: Should the Silver schema use a jsonb `extras` for source-specific fields, or explicit nullable columns for each?

See also: `CONNECTORS_REFERENCE.md` OQ-3.
