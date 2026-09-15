# HTTP Status Codes & Application Error Codes

> Source: [constructorfabric/DNA — REST/STATUS_CODES.md](https://github.com/constructorfabric/DNA/blob/main/REST/STATUS_CODES.md)

This document outlines the usage of HTTP status codes and application-level error codes for all Insight REST APIs. Responses MUST remain consistent across all endpoints.

---

## Table of Contents

- [Success Codes](#success-codes)
- [Client Errors (4xx)](#client-errors-4xx)
- [Server Errors (5xx)](#server-errors-5xx)
- [5xx vs 4xx Decision](#5xx-vs-4xx-decision)
- [Applicability by HTTP Method](#applicability-by-http-method)
- [Retry Guidance for Clients](#retry-guidance-for-clients)

---

## Success Codes

| Code | Name | Usage |
|------|------|-------|
| `200` | OK | Standard successful `GET` / `PUT` / `PATCH` responses |
| `201` | Created | Resource successfully created. Include `Location` header. |
| `202` | Accepted | Async operation accepted; returns operation status handle (`/jobs/{id}`) |
| `204` | No Content | Successful mutation with no body |
| `304` | Not Modified | Conditional `GET` with matching `If-None-Match` |

---

## Client Errors (4xx)

| Code | Name | When to use | Error codes |
|------|------|-------------|-------------|
| `400` | Bad Request | Malformed syntax, invalid parameters, or invalid cursor | `INVALID_CURSOR`, `ORDER_MISMATCH`, `FILTER_MISMATCH`, `UNSUPPORTED_FILTER_FIELD`, `UNSUPPORTED_ORDERBY_FIELD`, `INVALID_FIELD`, `TOO_MANY_FIELDS`, `FIELD_SELECTION_MISMATCH`, `INVALID_QUERY` |
| `401` | Unauthorized | Missing / invalid / expired authentication | `UNAUTHENTICATED`, `TOKEN_EXPIRED` |
| `403` | Forbidden | Authenticated but not authorized | `FORBIDDEN`, `INSUFFICIENT_PERMISSIONS` |
| `404` | Not Found | Resource or route not found | `NOT_FOUND` |
| `405` | Method Not Allowed | HTTP method not supported for this resource. Include `Allow` header. | `METHOD_NOT_ALLOWED` |
| `406` | Not Acceptable | The requested `Accept` media type is not supported | `NOT_ACCEPTABLE` |
| `408` | Request Timeout | Client took too long to send the complete request | `REQUEST_TIMEOUT` |
| `409` | Conflict | Resource state conflict: duplicate, version conflict, invariant violation | `CONFLICT`, `VERSION_CONFLICT`, `DUPLICATE` |
| `410` | Gone | Resource permanently deleted or endpoint permanently removed/deprecated. Use for hard-deleted resources, sunset API versions after retirement date. | `GONE`, `PERMANENTLY_DELETED`, `ENDPOINT_RETIRED` |
| `412` | Precondition Failed | ETag preconditions failed (`If-Match` mismatch) | `PRECONDITION_FAILED` |
| `413` / `414` | Payload Too Large / URI Too Long | Request exceeds limits (body or URL length) | `REQUEST_TOO_LARGE`, `URI_TOO_LONG` |
| `415` | Unsupported Media Type | `Content-Type` not supported | `UNSUPPORTED_MEDIA_TYPE` |
| `422` | Unprocessable Entity | Validation failed, semantically invalid input | `INVALID_LIMIT`, `VALIDATION_ERROR`, `SCHEMA_MISMATCH` |
| `428` | Precondition Required | Request must be conditional or include a specific precondition (e.g., `If-Match`, `Idempotency-Key`) per API policy | `PRECONDITION_REQUIRED` |
| `429` | Too Many Requests | Rate limit exceeded. Include standard rate limit headers and `Retry-After`. | `RATE_LIMITED` |
| `431` | Request Header Fields Too Large | Request header fields are too large. Reduce header sizes (e.g., cookies) and retry. | `HEADERS_TOO_LARGE` |
| `451` | Unavailable For Legal Reasons | Access blocked due to legal requirements (GDPR, content filtering, geo-restrictions, court orders) | `LEGAL_BLOCK`, `GEO_RESTRICTED`, `CONTENT_BLOCKED` |

---

## Server Errors (5xx)

| Code | Name | When to use | Error codes |
|------|------|-------------|-------------|
| `500` | Internal Server Error | Unexpected server-side error. **Never expose internal details** (stack traces, database errors) to clients. Use for: unhandled exceptions, unexpected state, programming errors. | `INTERNAL_ERROR` |
| `502` | Bad Gateway | Upstream service returned invalid HTTP response (malformed headers, protocol errors, connection refused, closed connection unexpectedly). Use for: returned by API gateways, reverse proxies, load balancers when upstream is at fault. Application servers should NOT return 502 for their own errors. | `UPSTREAM_ERROR`, `INVALID_UPSTREAM_RESPONSE`, `UPSTREAM_PROTOCOL_ERROR` |
| `503` | Service Unavailable | Service temporarily overloaded, under maintenance, or degraded. Include `Retry-After` header when possible. Use for: planned maintenance, circuit breaker open, resource exhaustion, database unreachable. | `SERVICE_UNAVAILABLE`, `MAINTENANCE_MODE`, `OVERLOADED` |
| `504` | Gateway Timeout | Upstream service did not respond within the configured timeout period. Use for: returned by API gateways, reverse proxies when waiting for upstream. Application servers should NOT return 504 for their own slow operations. | `UPSTREAM_TIMEOUT`, `GATEWAY_TIMEOUT` |

---

## 5xx vs 4xx Decision

- Use **4xx** when the client can fix the problem (bad input, missing auth, etc.)
- Use **5xx** when the server/infrastructure has the problem (bugs, outages, dependencies)
- When in doubt: if retrying the identical request might succeed after server recovery → use `5xx`

---

## Applicability by HTTP Method

| Method | Success codes | Common client error codes |
|--------|--------------|--------------------------|
| `GET` | `200`, `304` | `400`, `401`, `403`, `404`, `406`, `408`, `410`, `429`, `431`, `451` |
| `POST` | `201` (created), `202` (async), `200` (idempotent replay) | `400`, `401`, `403`, `404`, `405`, `406`, `408`, `409`, `410`, `415`, `422`, `428`, `429`, `431`, `451` |
| `PUT` | `200` (replaced), `204` (no body), `201` (upsert if supported) | `400`, `401`, `403`, `404`, `405`, `406`, `408`, `409`, `410`, `412`, `415`, `422`, `428`, `429`, `431`, `451` |
| `PATCH` | `200` (updated), `204` (no body) | `400`, `401`, `403`, `404`, `405`, `406`, `408`, `409`, `410`, `412`, `415`, `422`, `428`, `429`, `431`, `451` |
| `DELETE` | `204` (deleted), `202` (async delete), `200` (soft-delete payload) | `400`, `401`, `403`, `404`, `405`, `406`, `408`, `409`, `410`, `412`, `428`, `429`, `431`, `451` |
| `OPTIONS` | `200`, `204` | `401`, `403`, `429` |
| `HEAD` | `200`, `304` | Same as GET (without body) |

---

## Retry Guidance for Clients

Understanding which status codes are safe to retry is critical for building resilient clients without causing duplicate operations or data corruption.

### Safe to Retry (Idempotent Methods Only)

The following status codes are **safe to retry automatically**, but **only** for idempotent HTTP methods (`GET`, `HEAD`, `OPTIONS`):

| Code | Note |
|------|------|
| `408` | Request Timeout |
| `429` | Too Many Requests — respect `Retry-After` header |
| `500` | Internal Server Error |
| `502` | Bad Gateway |
| `503` | Service Unavailable — respect `Retry-After` header |
| `504` | Gateway Timeout |

### Non-Idempotent Methods (`POST`, `PATCH`)

- **Only retry if** the request includes an `Idempotency-Key` header
- Without `Idempotency-Key`, retrying may cause duplicate operations (double charges, duplicate resources, etc.)

### `PUT` and `DELETE` Notes

- `PUT` is idempotent when replacing a complete resource (same request = same result)
- `DELETE` is typically idempotent (deleting an already-deleted resource still results in "not found")
- However, if `PUT` performs side effects (e.g., incrementing counters) or `DELETE` has non-idempotent behavior, treat them as non-idempotent and require `Idempotency-Key` for safe retries
- Check endpoint documentation to determine if a specific endpoint is truly idempotent

### Retry Strategy Best Practices

- Use **exponential backoff** with jitter to avoid thundering herd
- Respect `Retry-After` header when present (`429`, `503`)
- Set a **maximum retry limit** (e.g., 3–5 attempts)
