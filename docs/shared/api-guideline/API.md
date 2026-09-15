# REST API Guideline

> Source: [constructorfabric/DNA — REST/API.md](https://github.com/constructorfabric/DNA/blob/main/REST/API.md)

This document is an actionable playbook for building consistent, evolvable REST APIs across all Insight components.

## Table of Contents

### Core Framework
- [1. Core Principles](#1-core-principles)
- [2. Protocol & Content](#2-protocol--content)
- [3. Resource Modeling & URLs](#3-resource-modeling--urls)
- [4. JSON Conventions](#4-json-conventions)
- [5. Pagination, Filtering, Sorting, Field Projection](#5-pagination-filtering-sorting-field-projection)
- [6. Request Semantics](#6-request-semantics)
- [7. Error Model (Problem Details)](#7-error-model-problem-details)

### Advanced Patterns
- [8. Concurrency & Idempotency](#8-concurrency--idempotency)
- [9. Authentication & Authorization](#9-authentication--authorization)
- [10. Rate Limiting & Quotas](#10-rate-limiting--quotas)
- [11. Asynchronous Operations](#11-asynchronous-operations)
- [12. Webhooks (Outbound)](#12-webhooks-outbound)

### Implementation Details
- [13. Internationalization, Numbers & Time](#13-internationalization-numbers--time)
- [14. Caching](#14-caching)
- [15. Security & CORS](#15-security--cors)
- [16. Observability & Diagnostics](#16-observability--diagnostics)
- [17. Versioning & Deprecation](#17-versioning--deprecation)

### Reference
- [18. Canonical Status Codes](#18-canonical-status-codes)
- [19. Batch & Bulk](#19-batch--bulk)
- [20. OpenAPI & Codegen](#20-openapi--codegen)
- [21. Example Endpoints](#21-example-endpoints)
- [22. Backward Compatibility Rules](#22-backward-compatibility-rules-client-facing)
- [23. Performance & DoS](#23-performance--dos)
- [24. Documentation Style](#24-documentation-style)

### Quick Reference
- [Operational Headers](#operational-headers-quick-reference)
- [References](#references)

---

## 1. Core Principles

- **Consistency over novelty**: Prefer one clear way to do things.
- **Explicitness**: Always specify types, units, timezones, and defaults.
- **Evolvability**: Versioned paths, idempotency, and forward-compatible schemas.
- **Observability**: Every request traceable end-to-end.
- **Security first**: HTTPS only, least privilege, safe defaults.

## 2. Protocol & Content

- **Media type**: `application/json; charset=utf-8` (request & response)
- **Errors**: Problem Details `application/problem+json` (RFC 9457)
- **Encoding**: UTF-8
- **Compression**: gzip/br when client sends `Accept-Encoding`
- **Idempotency**: `Idempotency-Key` header on unsafe methods (see §8)

## 3. Resource Modeling & URLs

- **Nouns, plural**: `/users`, `/tickets`, `/tickets/{ticket_id}`
- **Hierarchy if strict ownership**: `/users/{user_id}/keys`
- **Prefer top-level + filters** over deep nesting: `/tickets?assignee_id=...`
- **Identifiers**: `uuidv7` (or `ulid`). JSON field: `id`
- **Timestamps**: ISO-8601 UTC with `Z`, always include milliseconds `.SSS` (e.g., `2025-09-01T20:00:00.000Z`)
- **Standard fields**: `created_at`, `updated_at`, optional `deleted_at`

## 4. JSON Conventions

- **Naming**: `snake_case` (consistent with backend conventions and databases)
- **Nullability**: Prefer omitting absent fields over `null`
- **Booleans & enums**: Strongly typed; never stringly booleans
- **Money**: Integer minor units + currency code
- **Lists**: Arrays; use `[]` not `null`
- **Envelope**:
  - **Lists**: Use `items` array with optional top-level `page_info` for pagination
  - **Single objects**: Return fields directly at top level (no wrapper)

```json
// List response
{
  "items": [ /* array of objects */ ],
  "page_info": { /* optional: limit, next_cursor, prev_cursor */ }
}

// Single object response (no wrapper)
{
  "id": "01J...",
  "title": "Example",
  "created_at": "2025-09-01T20:00:00.000Z"
}
```

## 5. Pagination, Filtering, Sorting, Field Projection

For complete specification see [QUERYING.md](QUERYING.md):

- **Cursor pagination**: Opaque, versioned cursors with `limit` and `cursor` parameters
- **Filtering**: OData `$filter` with operators (`eq`, `ne`, `gt`, `in`, etc.)
- **Sorting**: OData `$orderby` (e.g., `priority desc,created_at asc`)
- **Field projection**: OData `$select` for sparse field selection (e.g., `$select=id,title,status`)

## 6. Request Semantics

- **Create**: `POST /tickets` → 201 + `Location` + resource in body
- **Partial update**: `PATCH /tickets/{id}` (JSON Merge Patch)
- **Replace**: `PUT /tickets/{id}` (complete representation)
- **Delete**: `DELETE /tickets/{id}` → 204; if soft-delete, return 200 with `deleted_at`

## 7. Error Model (Problem Details)

**Always** return RFC 9457 Problem Details for 4xx/5xx:

```json
{
  "type": "https://api.example.com/errors/validation",
  "title": "Invalid request",
  "status": 422,
  "detail": "email is invalid",
  "instance": "https://api.example.com/req/01J...Z",
  "errors": [
    { "field": "email", "code": "format", "message": "must be a valid email" }
  ],
  "trace_id": "01J...Z"
}
```

**Mappings**: 422 (validation), 401/403 (authz), 404, 409 (conflict), 429, 5xx (no internals)

## 8. Concurrency & Idempotency

- **Optimistic locking**: Representations carry `ETag` (strong or weak). Clients send `If-Match`. On mismatch → 412.
- **Idempotency**: Clients may send `Idempotency-Key` on `POST`/`PATCH`/`DELETE`.
  - Server caches **only successful (2xx) responses** to prevent duplicate side effects.
  - Error responses (4xx/5xx) are NOT cached; retries re-execute to allow fresh validation, permission checks, and recovery from transient failures.
  - Successful replays return the cached response with header: `Idempotency-Replayed: true`.
  - **Retention tiers**:
    - Minimum default: 1 hour (sufficient for network retry protection)
    - Important operations: 24h–7d (e.g., notifications, reports) — must be documented per endpoint
    - Critical operations: Permanent via DB uniqueness constraints (e.g., payments, user registration) → return `409 Conflict` with existing resource after initial creation

## 9. Authentication & Authorization

- **Auth**: OAuth2/OIDC Bearer tokens in `Authorization: Bearer <token>`
- **Scopes/permissions**: Document per endpoint; insufficient → 403
- **Service-to-service**: mTLS optional
- **No secrets in URLs**; short token TTLs; rotate keys; refresh tokens when needed

## 10. Rate Limiting & Quotas

Headers following [IETF RateLimit draft](https://datatracker.ietf.org/doc/draft-ietf-httpapi-ratelimit-headers/):

- `RateLimit-Policy: "default";q=100;w=3600` — defines quota policy: 100 requests per hour
- `RateLimit: "default";r=72;t=1800` — current status: 72 remaining, resets in 1800 seconds
- On 429 also include `Retry-After` (seconds). Quotas are per token by default.
- **Example with partition key**: `RateLimit-Policy: "peruser";q=100;w=60;pk=:dXNlcjEyMw==:`
- **Backward compatibility**: Servers **MAY** also return `X-RateLimit-Limit`, `X-RateLimit-Remaining`, `X-RateLimit-Reset` during a transition period.

## 11. Asynchronous Operations

For long tasks return `202 Accepted` + `Location: /jobs/{job_id}`:

```json
{
  "id": "01J...",
  "status": "queued|running|succeeded|failed|canceled",
  "percent": 35,
  "result": {},
  "error": {},
  "created_at": "...",
  "updated_at": "..."
}
```

Clients poll `GET /jobs/{id}` or subscribe via SSE/WebSocket if available.

## 12. Webhooks (Outbound)

- **Event shape**: `event_type`, `id`, `created_at`, `data`
- **Delivery**: POST JSON to subscriber URL
- **Security**: `X-Signature` HMAC-SHA256 over raw body with shared secret; include `X-Timestamp` (±5 min skew)
- **Retries**: Exponential backoff for ≥24h; dead-letter queue
- **Idempotency**: Include `event_id`; receivers dedupe

## 13. Internationalization, Numbers & Time

- All timestamps UTC (`Z`), always include milliseconds `.SSS` (e.g., `2025-09-01T20:00:00.000Z`). If timezone needed, add separate `timezone` (IANA name)
- JSON numbers for typical values; use strings for high-precision decimals or use integer minor units
- Sorting/filters are locale-agnostic unless documented otherwise
- Errors are not localized; localization is handled by the UI/API client

## 14. Caching

- Reads: `ETag` + `Cache-Control: private, max-age=30` when safe
- Mutations: `Cache-Control: no-store`
- Conditional: `If-None-Match` → 304

## 15. Security & CORS

- HTTPS only; HSTS enabled
- CORS allow-list explicit origins:

```
Access-Control-Allow-Origin: https://app.example.com
Access-Control-Allow-Methods: GET,POST,PUT,PATCH,DELETE,OPTIONS
Access-Control-Allow-Headers: Authorization, Content-Type, Idempotency-Key
Access-Control-Expose-Headers: ETag, Location, RateLimit, RateLimit-Policy
```

- CSRF: only relevant for cookie auth; prefer Bearer in `Authorization` for SPAs
- Content Security Policy on the app domain; avoid wildcard `*/*`

## 16. Observability & Diagnostics

- **Tracing**: accept/propagate `traceparent` (W3C). Emit `trace_id` header on all responses
- **Request ID**: honor `X-Request-Id` or generate one
- **Structured logs**: JSON per request: `trace_id`, `request_id`, `user_id`, `path`, `status`, `duration_ms`, `bytes`
- **Metrics**: RED/USE per route, with p50/p90/p99

## 17. Versioning & Deprecation

- **Path versioning**: `/v1` (breaking changes bump major)
- **Non-breaking changes**: Adding optional fields/params, new endpoints, new enum values, relaxing validation
- **Breaking changes**: Removing fields, changing types/semantics, making optional fields required, changing URL structure
- **Client compatibility**: Must ignore unknown fields, handle new enum values gracefully, not rely on field order
- **Deprecation headers**: `Deprecation: true`, `Sunset: <RFC 8594 date>`, and `Link: <doc>; rel="deprecation"`

See [VERSIONING.md](VERSIONING.md) for the full versioning and deprecation process.

## 18. Canonical Status Codes

For complete definitions and application error code mappings, see [STATUS_CODES.md](STATUS_CODES.md).

Quick reference:
- `200` OK (read/update)
- `201` Created (+ `Location`)
- `202` Accepted (async)
- `204` No Content (delete or idempotent update without body)
- `400` Bad Request
- `401` Unauthorized / `403` Forbidden
- `404` Not Found
- `409` Conflict
- `410` Gone (for deprecated endpoints)
- `412` Precondition Failed (ETag)
- `415` Unsupported Media Type
- `422` Unprocessable Entity (validation)
- `429` Too Many Requests
- `503` Service temporarily overloaded or under maintenance
- `5xx` Other server errors

## 19. Batch & Bulk

For complete specification, see [BATCH.md](BATCH.md).

**Quick Summary:**
- **Endpoint pattern**: `POST /resources:batch` (default maximum 100 items, configurable per endpoint)
- **Response**: `207 Multi-Status` (partial success) or specific status code (all same outcome)
- **Error format**: Full RFC 9457 Problem Details per failed item
- **Atomicity**: Endpoint-specific (best-effort default, atomic for critical operations)
- **Idempotency**: Per-item `idempotency_key` with 1-hour retention
- **Optimistic locking**: Per-item `if_match` field for version checking

## 20. OpenAPI & Codegen

- **Source of truth**: OpenAPI 3.1
- Client SDK: generate TypeScript types (`openapi-typescript`) and React hooks (TanStack Query) with fetch/axios adapter
- Keep schemas DRY via shared components; provide example payloads for every operation

## 21. Example Endpoints

**List Tickets**:

```bash
curl -sS \
  -H "Authorization: Bearer $TOKEN" \
  "https://api.example.com/v1/tickets?limit=25&cursor=...&\$filter=status in ('open','in_progress')&\$orderby=priority desc,created_at asc&\$select=id,title,priority,status,created_at"
```

```json
{
  "items": [
    { "id": "01J...", "title": "Disk full", "priority": "high", "status": "open", "created_at": "2025-08-31T10:05:17.000Z" }
  ],
  "page_info": {
    "limit": 25,
    "next_cursor": "eyJ2IjoxLCJrIjpbIjIwMjUtMDgtMzFUMTA6MDU6MTcuMDAwWiIsIjAxSi4uLiJdLCJvIjoiZGVzYyIsInMiOiJjcmVhdGVkX2F0LGlkIn0",
    "prev_cursor": null
  }
}
```

**Update with Concurrency**:

```
PATCH /v1/tickets/01J...
If-Match: W/"etag-abc"
Idempotency-Key: 5b2f...

{ "status": "in_progress" }
```

**Async Job**:

```
POST /v1/reports → 202 Accepted
Location: /v1/jobs/01J...
```

## 22. Backward Compatibility Rules (Client-facing)

- Clients ignore unknown fields
- Do not rely on property order
- Treat enums laxly: unknown enum → display as string, never crash
- Handle pagination cursors generically

## 23. Performance & DoS

- Enforce max list limits and payload sizes (e.g., 1MB JSON)
- Deny N+1 by default; allow explicit `include=` with documented caps
- Timeouts: handler ≤ 30s; use async jobs for longer work
- Strict input validation with precise 422s

## 24. Documentation Style

Each endpoint must be comprehensively documented to serve both human developers and AI assistants.

### Required Documentation Elements

1. **Summary & Description** — one-line summary, detailed purpose, when to use
2. **Authentication & Authorization** — required auth method, required scopes/permissions
3. **Request Specification** — HTTP method and path, path/query parameters, request headers, request body schema
4. **Response Specification** — success status codes, response headers, response body schema, example responses
5. **Error Documentation** — all possible error codes, Problem Details examples, common scenarios
6. **Rate Limiting** — rate limit class, quota consumption
7. **Code Examples** — curl with realistic data, TypeScript with generated client

### Documentation Template

```
Endpoint: POST /v1/resources
Purpose: Create new resource
Authentication: Required (OAuth2)
Authorization: resources:write scope
Rate Limit: Standard (100/hour)
```

**Request Body Schema**:
```json
{
  "title": "string (required, max 255 chars)",
  "description": "string (optional, max 1000 chars)",
  "priority": "enum: low|medium|high",
  "category": "string (optional)"
}
```

**Success Response** (201):
```json
{
  "id": "res_01JCXYZ...",
  "title": "string",
  "priority": "medium",
  "status": "active",
  "created_at": "2024-01-15T10:30:00.000Z",
  "updated_at": "2024-01-15T10:30:00.000Z"
}
```

**Error Responses**:
- `400` — Invalid request body
- `401` — Missing/invalid authentication
- `403` — Insufficient permissions
- `422` — Validation errors (Problem Details)
- `429` — Rate limit exceeded

---

## Operational Headers (Quick Reference)

- **Requests may include**: `Authorization`, `Idempotency-Key`, `If-Match`, `If-None-Match`, `Accept-Encoding`, `traceparent`, `X-Request-Id`
- **Responses should include**: `Content-Type`, `ETag` (when cacheable), `Location` (201/202), `RateLimit`, `RateLimit-Policy`, `traceId`, `X-Request-Id`

## References

- RFC 9457 Problem Details: [IETF RFC 9457](https://www.rfc-editor.org/rfc/rfc9457)
- IETF RateLimit Headers Draft: [draft-ietf-httpapi-ratelimit-headers](https://datatracker.ietf.org/doc/draft-ietf-httpapi-ratelimit-headers/)
- W3C Trace Context: [traceparent](https://www.w3.org/TR/trace-context/)
