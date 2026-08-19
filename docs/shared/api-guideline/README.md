# REST API Guideline

> Source: [cyberfabric/DNA — REST](https://github.com/cyberfabric/DNA/tree/main/REST)

This directory contains the canonical REST API guidelines for all Insight components. Every HTTP API — backend endpoints, connector APIs, orchestrator interfaces — **MUST** conform to these rules.

---

## Documents

| File | Description |
|------|-------------|
| [API.md](API.md) | Core playbook: principles, resource modeling, JSON conventions, pagination, error model, auth, rate limiting, async, webhooks, caching, security, observability |
| [QUERYING.md](QUERYING.md) | Cursor pagination spec, OData `$filter` / `$orderby` / `$select`, indexed fields, cursor format, validation rules, SQL implementation recipe |
| [STATUS_CODES.md](STATUS_CODES.md) | Canonical HTTP status codes, application error codes, per-method applicability, retry guidance |
| [BATCH.md](BATCH.md) | Batch/bulk endpoint patterns, request/response format, atomicity, idempotency, optimistic locking, performance limits |
| [VERSIONING.md](VERSIONING.md) | Versioning strategy, breaking vs non-breaking changes, deprecation process, migration, client guidelines |

---

## Quick Rules

- **Path versioning**: `/v1/`, `/v2/` — major version only
- **JSON**: `snake_case`, `items` envelope for lists, direct fields for single objects
- **Errors**: RFC 9457 Problem Details (`application/problem+json`) for all `4xx`/`5xx`
- **Pagination**: cursor-based (`cursor` + `limit`); never offset; never `total_count`
- **Filtering/Sorting**: OData `$filter`, `$orderby`, `$select` on allowlisted indexed fields
- **Timestamps**: ISO-8601 UTC with milliseconds — `2025-09-01T20:00:00.000Z`
- **IDs**: `uuidv7` (or `ulid`); JSON field name `id`
- **Batch writes**: `POST /resources:batch` → `207 Multi-Status` for partial success
- **Async**: `202 Accepted` + `Location: /v1/jobs/{job_id}`
- **Idempotency**: `Idempotency-Key` header on `POST`/`PATCH`/`DELETE`
- **Auth**: `Authorization: Bearer <token>` (OAuth2/OIDC); no secrets in URLs
- **HTTPS only** — no plaintext HTTP in production

---

## Enforcement

These guidelines apply to:
- `src/backend/` — REST API services
- `src/frontend/` — API client contract
- `src/ingestion/connectors/` — connector HTTP interfaces

When proposing changes to these guidelines, open an ADR under `docs/shared/glossary/ADR/`.
