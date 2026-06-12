---
status: accepted
date: 2026-06-04
decision-makers: Insight ingestion team
---

# Collect ChatGPT Team data via a customer-hosted browser proxy, not the OpenAI Admin API

**ID**: `cpt-insightspec-adr-chatgpt-team-browser-proxy`

## Context and Problem Statement

PRD/DESIGN v1.0 (March 2026) assumed ChatGPT Team seat and activity data would be read from the **OpenAI Admin API** (`api.openai.com/v1/organization/...`) with an admin API key. That assumption is wrong: the Admin API exposes only the *programmatic* surface (org users, projects, API keys, per-key/per-model completions usage, costs, audit logs) — which is already collected by the existing `openai` connector and feeds `class_ai_api_usage`. The **conversational** ChatGPT Team data this connector actually needs — per-seat roster, per-user daily messages/sessions, Codex usage, subscription spend — is exposed **only** by the authenticated `chatgpt.com/backend-api/*` web endpoints, behind a browser session cookie and Cloudflare bot protection. How should Insight collect data that has no API and requires a real browser session?

## Decision Drivers

* **Data availability** — the required seat/activity/Codex/subscription data exists *only* behind `chatgpt.com` session auth; the Admin API does not return it.
* **Security** — the customer's `chatgpt.com` session cookie (and derived access token) must never enter Insight's infrastructure.
* **Cloudflare** — `chatgpt.com` requires a real Chromium TLS fingerprint and JS challenge clearance; a plain HTTP client is blocked.
* **Consistency** — the Claude Team connector already solved the identical problem (`claude-team` + `secure-enclave/proxies/claude_team`); converging on one pattern reduces operational and review cost.
* **No duplication** — programmatic OpenAI usage is already owned by the `openai` Admin-API connector; this connector must not re-collect it.

## Considered Options

* **Option 1 — OpenAI Admin API** (the v1.0 premise): read seats/activity from `api.openai.com/v1/organization/*` with an admin key.
* **Option 2 — Insight calls `chatgpt.com` directly** holding the customer's session cookie inside the Insight cluster.
* **Option 3 — Customer-hosted browser proxy + declarative connector** (mirror `claude-team`): a customer-deployed headless-Chromium proxy holds the session, Insight talks to it over HTTP with a shared bearer token.

## Decision Outcome

Chosen option: **Option 3 — customer-hosted browser proxy + declarative connector**, because it is the only option that can reach the required data (rejecting Option 1) without placing the customer's session cookie in Insight infrastructure or fighting Cloudflare from a non-browser client (rejecting Option 2). It also reuses the proven `claude-team` architecture end to end.

Concretely:

- A new proxy `secure-enclave/proxies/chatgpt_team` (forked from `claude_team`) runs Playwright + stealth, holds the `chatgpt.com` session in memory, clears Cloudflare, and exposes `GET /api/*` (bearer-authenticated passthrough to `chatgpt.com/backend-api/*`) plus `POST /admin/session-key`.
- A declarative connector `connectors/ai/chatgpt-team` points `url_base` at `proxy_url`, authenticates with `proxy_auth_token`, and reads the seat/activity/Codex/subscription streams.
- The OpenAI **admin API key is not used** by this connector; the `openai` connector keeps the programmatic surface.

### Consequences

* Good, because the required conversational/seat/Codex data becomes reachable at all.
* Good, because the session cookie never leaves the customer environment (only `proxy_url` + `proxy_auth_token` reach Insight).
* Good, because it converges with `claude-team`: same proxy skeleton, same declarative + dbt shape, one operational runbook.
* Good, because it cleanly separates from the `openai` (Admin-API) connector — no double counting; `class_ai_assistant_usage` / `class_ai_dev_usage` vs `class_ai_api_usage`.
* Bad, because `chatgpt.com` auth is heavier than `claude.ai`: it needs the full session cookie **and** a short-lived bearer `access_token` (obtained via `GET /api/auth/session`) that must be refreshed — a token-exchange layer the `claude_team` proxy did not need. See risk below.
* Bad, because two upstream identifiers (`account_id` and `org_id`) are used by different endpoints, vs the single `org_id` in `claude-team`.
* Bad, because it inherits the browser-proxy fragility: Cloudflare/stealth churn, ~hourly access-token expiry, manual session bootstrap, one proxy per workspace.

### Confirmation

This decision is confirmed when:

- `connectors/ai/chatgpt-team/connector.yaml` sets `url_base: {{ config['proxy_url'] }}` and `Authorization: Bearer {{ config['proxy_auth_token'] }}` — and contains **no** `api.openai.com` host and **no** admin-key field.
- `descriptor.yaml:secret.required_fields` lists `proxy_url`, `proxy_auth_token` and `chatgpt_account_id` (with `chatgpt_org_id` optional — only the subscription streams need it) — and does **not** list an OpenAI admin key.
- A proxy exists at `secure-enclave/proxies/chatgpt_team` exposing `/api/*` (bearer) and `/admin/session-key`.
- PRD §1 and DESIGN §1 describe the source as the customer-hosted proxy over `chatgpt.com`, not the OpenAI Admin API.

## Pros and Cons of the Options

### Option 1 — OpenAI Admin API

* Good, because it is a stable, documented API with a simple bearer key.
* Good, because no browser, no Cloudflare, no session rotation.
* Bad, because it **does not expose** ChatGPT Team conversational seat/activity/Codex data — the core requirement is unmet.
* Bad, because what it *does* expose is already collected by the `openai` connector — adopting it here duplicates that connector.

### Option 2 — Insight calls `chatgpt.com` directly

* Good, because no extra deployable component.
* Bad, because it requires the customer's session cookie to live inside Insight infrastructure — an unacceptable security posture.
* Bad, because a non-browser client is blocked by Cloudflare on `chatgpt.com`.

### Option 3 — Customer-hosted browser proxy + declarative connector

* Good, because it reaches the data, keeps the cookie on-prem, and clears Cloudflare with a real browser.
* Good, because it reuses the `claude-team` proxy + declarative + dbt pattern.
* Neutral, because it adds a customer-deployed container the customer must operate (already true for Claude Team).
* Bad, because of the `chatgpt.com`-specific access-token exchange/refresh and the dual `account_id`/`org_id` identifiers (new work vs `claude_team`).

## More Information

- Prototype evidence (`data_collector/apps/openai`): browser transport against `https://chatgpt.com/backend-api/*`, session cookie + optional bearer `ACCESS_TOKEN`, separate `ACCOUNT_ID` and `ORG_ID`; endpoints include `/accounts/{account_id}/users`, `/accounts/{account_id}/analytics/user_list`, `/wham/analytics/daily-sessions-messages-counts`, `/wham/analytics/usage-leaderboard`, `/subscriptions/{org_id}/usage`.
- Reference architecture: `claude-team` ADR/DESIGN and `secure-enclave/proxies/claude_team` (Playwright + stealth, `/admin/session-key`, bearer passthrough).
- **Open risk to verify before building the proxy**: whether `chatgpt.com` requires the bearer `access_token` for the target endpoints and its exact TTL/refresh path (`GET /api/auth/session`). This determines whether the proxy must perform token exchange + refresh, or can rely on the session cookie alone (as `claude_team` does). Tracked as the first de-risking step.

## Traceability

- **PRD**: [PRD.md](../PRD.md)
- **DESIGN**: [DESIGN.md](../DESIGN.md)

This decision directly addresses the following requirements or design elements:

* `cpt-insightspec-fr-chatgpt-team-seats-collect` — seat roster is read from `chatgpt.com/backend-api/accounts/{account_id}/users` via the proxy, not the Admin API.
* `cpt-insightspec-fr-chatgpt-team-activity-collect` — daily per-user activity is read from the browser analytics endpoints exposed through the proxy.
* `cpt-insightspec-fr-chatgpt-team-identity-key` — `email` remains the cross-system identity key; the browser source still returns it.
* `cpt-insightspec-fr-chatgpt-team-silver-separation` — keeps this connector's conversational/dev usage distinct from the `openai` connector's `class_ai_api_usage`.
