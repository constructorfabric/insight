# Querying: Pagination, Filtering, Sorting & Field Projection

> Source: [constructorfabric/DNA — REST/QUERYING.md](https://github.com/constructorfabric/DNA/blob/main/REST/QUERYING.md)

This document defines the full contract for cursor-based pagination, OData filtering/sorting, and field projection (`$select`) used across all Insight REST APIs.

---

## Table of Contents

- [Goals](#goals)
- [Cursor Pagination](#cursor-pagination)
  - [Request Parameters](#request-parameters)
  - [Request Phases](#request-phases)
  - [Response Envelope](#response-envelope)
  - [Canonical Sort Requirements](#canonical-sort-requirements)
  - [Ordering Fields](#ordering-fields)
  - [Indexed Fields & Allowed Parameters](#indexed-fields--allowed-parameters)
  - [Cursor Format (Opaque, Versioned)](#cursor-format-opaque-versioned)
  - [Validation Rules](#validation-rules)
  - [Implementation Recipe (SQL/ORM)](#implementation-recipe-sqlorm)
  - [Pagination Navigation Patterns](#pagination-navigation-patterns)
  - [Edge Cases & Guarantees](#edge-cases--guarantees)
  - [Do's and Don'ts](#dos-and-donts)
- [Field Projection with `$select`](#field-projection-with-select)
  - [Overview](#overview)
  - [Default Projection](#default-projection)
  - [Allowlisting & Security](#allowlisting--security)
  - [Response Format](#response-format)
  - [Interaction with Pagination](#interaction-with-pagination)
  - [Error Handling](#error-handling)
  - [OpenAPI Integration](#openapi-integration)
- [OpenAPI Contract Requirements](#openapi-contract-requirements)
- [Examples](#examples)

---

## Goals

- **Stable ordering**: No duplicates or gaps when paging.
- **Opaque cursors**: Clients never rely on internal fields.
- **Simple client contract**: One optional `cursor` and `limit` param per request.
- **Filter safety**: Cursors are bound to the query/filter they were created from.
- **Extensible**: Versioned cursor payloads for painless future changes.

## Terminology

- **Cursor**: Opaque token representing a position in a sorted result set.
- **Page**: Up to `limit` items returned for a given cursor.
- **Canonical sort**: Endpoint-defined, immutable ordering (includes a unique tiebreaker).

---

## Cursor Pagination

### Request Parameters

Every paginated endpoint MUST accept:

| Parameter | Type | Default | Min | Max | Description |
|-----------|------|---------|-----|-----|-------------|
| `limit` | integer | 25 | 1 | 200 | Number of items to return (endpoints may choose a lower max) |
| `cursor` | string | — | — | — | Opaque token from a previous response; returns next page after cursor position |
| `$filter` | string | — | — | — | OData-style filter expression (see below) |
| `$orderby` | string | — | — | — | OData-style order-by list (see below) |
| `$select` | string | — | — | — | Comma-separated field names for sparse projection |

**`$filter` examples**:
```
price gt 10 and startswith(name,'Pro')
status in ('paid','shipped') and created_at ge 2025-01-01T00:00:00Z
```

**Supported operators**: `eq`, `ne`, `gt`, `ge`, `lt`, `le`, `and`, `or`, `not`, `in` (set membership), functions `startswith`, `endswith`, `contains`. Strings use single quotes; timestamps are RFC3339 UTC.

**`$orderby` examples**:
```
created_at desc, id desc
priority desc, created_at asc
```

Must include a unique tiebreaker (typically `id`) last for stable pagination; if omitted, the server appends `id` automatically.

### Request Phases

**First request** (no `cursor`):
- Client may supply `limit`, `$orderby`, and `$filter`.
- Server validates `$orderby` tokens, ensures a unique tiebreaker is last.
- Server applies `$filter` and `$orderby`, returns page, and encodes `s` (effective `$orderby`), `o` (direction of the primary key), and `f` (normalized filter hash) into the cursor.

**Subsequent requests** (with `cursor`):
- `cursor` fully defines ordering (`s`, `o`) and filters (`f`).
- Server ignores any provided `$orderby` when a `cursor` is present. If provided and it differs from the cursor → `400 ORDER_MISMATCH`.
- Changing filters between pages is not allowed; if detected (via `f`) → `400 FILTER_MISMATCH`.

### Response Envelope

```json
{
  "items": [ /* ... */ ],
  "page_info": {
    "next_cursor": "<opaque>",
    "prev_cursor": "<opaque>",
    "limit": 20
  }
}
```

Rules:
- `items` are in the endpoint's canonical sort order (do not reverse for backward navigation).
- `next_cursor` points to the position immediately after the last item in `items`; may be omitted if there is no next page.
- `prev_cursor` points to the position immediately before the first item in `items`; may be omitted on the first page.
- **Do not include `total_count`** in cursor pagination responses.

### Canonical Sort Requirements

- Must be total and stable: combine the primary sort key with a unique tiebreaker (e.g., `created_at DESC, id DESC`).
- The unique tiebreaker must be monotonic with respect to insertion order or at least unique. **UUIDv7 is recommended**; its lexicographic order aligns with creation time.
- Example canonical sorts:
  - Timelines: `created_at DESC, id DESC`
  - Oldest-first logs: `created_at ASC, id ASC`

### Ordering Fields

By default, only `created_at` and `id` are allowed for ordering. Specific endpoints may add more ordering fields if they are indexed and documented.

| Field | Type | Notes |
|-------|------|-------|
| `created_at` | timestamp (RFC3339/UTC) | Recommended primary key for feeds and most lists. Must be non-null. |
| `id` | string (UUIDv7) | Required unique tiebreaker in all canonical sorts. Lexicographic order preserves creation-time ordering with UUIDv7. |

Field comparison semantics:
- **Strings**: case-insensitive NFKC with `en-US` unless endpoint specifies otherwise
- **Timestamps**: compare by instant in UTC
- **Numbers**: IEEE-754; NaN not allowed; nulls not allowed in ordering fields

### Indexed Fields & Allowed Parameters

- Filtering and ordering are allowed only on fields backed by suitable database indexes.
- For string functions (`startswith`, `contains`), enable only when supported by an index (prefix, trigram, full-text); otherwise reject the filter.
- **Recommended cap per endpoint**: 10 queryable indexed fields.
- Each endpoint MUST publish an allowlist of `$filter` fields (and supported operators per field) and `$orderby` fields (with allowed directions) in the generated API spec.

**Suggested OpenAPI vendor extensions**:
```yaml
x-odata-filter:
  allowedFields:
    created_at: [ge, gt, le, lt, eq]
    id: [eq, in]

x-odata-orderby:
  allowedFields:
    - created_at desc
    - created_at asc
    - id asc
```

### Cursor Format (Opaque, Versioned)

Base64URL (no padding) encoded JSON object. **Treat as completely opaque to clients.**

```json
{
  "v": 1,                      // version
  "k": [<primary_key>, <tiebreaker_key>],
  "o": "desc|asc",             // order of the primary key
  "s": "created_at,id",        // sort keys used to build the cursor
  "f": "<filter-hash>"         // hash of normalized filters (optional but recommended)
}
```

Guidelines:
- `k` are the raw values needed for comparison in the database (e.g., ISO8601 timestamp string and id string).
- When multiple sort keys are used, `k` MUST include values for each sort key in order.
- For mixed-direction sorts, encode per-field direction in `s` using optional `+`/`-` prefixes (e.g., `"-score,+created_at,+id"`).

### Validation Rules

- If `cursor` is present, decode and validate: known `v`, matches endpoint's canonical sort (`s` and `o`), current request's normalized parameters hash to the same `f`.
- `400 INVALID_CURSOR` — cursor validation failure
- `422 INVALID_LIMIT` — `limit` is out of bounds
- `400 UNSUPPORTED_FILTER_FIELD` — field referenced in `$filter` not in allowlist
- `400 UNSUPPORTED_ORDERBY_FIELD` — field referenced in `$orderby` not in allowlist
- `400 ORDER_MISMATCH` — `$orderby` provided with cursor but differs from cursor's sort
- `400 FILTER_MISMATCH` — `$filter` changed mid-pagination

### Implementation Recipe (SQL/ORM)

Assume canonical sort `created_at DESC, id DESC`:

1. Normalize and clamp `limit` to `[1, MAX_LIMIT]`. Use `page_size = min(request.limit || 20, 100)`.
2. If `cursor` is provided, decode it to `(cursor_created_at, cursor_id)`.
3. Build the predicate for forward pagination:
   - **DESC**: `(created_at, id) < (cursor_created_at, cursor_id)`
   - **ASC**: `(created_at, id) > (cursor_created_at, cursor_id)`
4. Apply filters first, then the cursor predicate.
5. Order by the canonical sort.
6. Fetch `page_size + 1` rows.
7. Trim `items = rows.slice(0, page_size)`.
8. Compute cursors from the first/last items in `items`.

### Pagination Navigation Patterns

**Key Principles**:
1. **Forward** means "next page in the canonical sort direction"
2. **Backward** means "previous page in the canonical sort direction"
3. **`ORDER BY` always remains the same** regardless of navigation direction
4. Only the **WHERE clause comparison operators are reversed** for backward navigation
5. **Results are always returned in canonical order**, never reversed

#### Scenario 1: Forward Pagination (DESC ordering)

```sql
-- $orderby=created_at desc, id desc — get next (older) items
WHERE (
  created_at < :cursor_created_at
  OR (created_at = :cursor_created_at AND id < :cursor_id)
)
ORDER BY created_at DESC, id DESC
LIMIT :page_size_plus_one
```

#### Scenario 2: Backward Pagination (DESC ordering)

```sql
-- $orderby=created_at desc, id desc — get previous (newer) items
WHERE (
  created_at > :cursor_created_at
  OR (created_at = :cursor_created_at AND id > :cursor_id)
)
ORDER BY created_at DESC, id DESC
LIMIT :page_size_plus_one
```

#### Scenario 3: Forward Pagination (ASC ordering)

```sql
-- $orderby=created_at asc, id asc — get next (newer) items
WHERE (
  created_at > :cursor_created_at
  OR (created_at = :cursor_created_at AND id > :cursor_id)
)
ORDER BY created_at ASC, id ASC
LIMIT :page_size_plus_one
```

#### Scenario 4: Backward Pagination (ASC ordering)

```sql
-- $orderby=created_at asc, id asc — get previous (older) items
WHERE (
  created_at < :cursor_created_at
  OR (created_at = :cursor_created_at AND id < :cursor_id)
)
ORDER BY created_at ASC, id ASC
LIMIT :page_size_plus_one
```

#### Mixed-Direction Sort

For `$orderby=score desc, created_at asc, id asc`:

```sql
-- Forward pagination
WHERE (
  score < :cursor_score
  OR (score = :cursor_score AND created_at > :cursor_created_at)
  OR (score = :cursor_score AND created_at = :cursor_created_at AND id > :cursor_id)
)
ORDER BY score DESC, created_at ASC, id ASC
LIMIT :page_size_plus_one
```

### Edge Cases & Guarantees

- Returning fewer than `limit` items is allowed (last page).
- `next_cursor` may be omitted if there is no further item; `prev_cursor` may be omitted on the first page.
- Inserts/deletes during pagination: the strictly monotonic tuple comparison `(primary, tiebreaker)` prevents duplicates and minimizes gaps. Absolute consistency is not guaranteed without snapshots; this is acceptable for feed-like listings.
- Cursors are stateless and may be reused; optionally expire old versions server-side by rejecting outdated `v`.

### Do's and Don'ts

- ✅ Include a unique tiebreaker in the sort
- ✅ Over-fetch by one to compute existence of a next page
- ✅ Validate that filters/sort match what the cursor encodes
- ❌ Don't expose database IDs or raw fields as cursors; keep them opaque and versioned
- ❌ Don't provide `total_count` in cursor pagination
- ❌ Don't reverse item order for backward navigation; keep order canonical

### Cursor Versioning

- Start at `v = 1`.
- On breaking changes to cursor content or comparison semantics, bump `v` and reject older tokens with `INVALID_CURSOR`.

---

## Field Projection with `$select`

### Overview

Sparse field selection via OData-style `$select` query parameter. Returns only requested fields to reduce payload size.

**Syntax**: `$select=field1,field2,field3` (comma-separated, no whitespace, case-sensitive)

```http
GET /v1/tickets/01J...?$select=id,title,status,priority
GET /v1/tickets?$filter=status eq 'open'&$orderby=priority desc&$select=id,title,status
```

### Default Projection

When `$select` is omitted, servers return a **default projection**:
- **Includes**: `id`, `type`, core business fields (`title`, `status`), timestamps, small reference IDs
- **Excludes**: Large text fields, binary data, sensitive fields, expensive computed fields

Default projection documented per resource in OpenAPI using `x-odata-select` vendor extension.

### Allowlisting & Security

- Only **allowlisted fields** may appear in `$select` → `400 INVALID_FIELD` if not allowed
- **Max fields per request**: 50 (configurable) → `400 TOO_MANY_FIELDS` if exceeded
- Authorization via allowlist definition (at design time), not runtime checks

### Response Format

```http
GET /v1/tickets?limit=2&$select=id,title,status
```

```json
{
  "items": [
    { "id": "01J...", "title": "Database timeout", "status": "open" },
    { "id": "01J...", "title": "Login error", "status": "in_progress" }
  ],
  "page_info": {
    "limit": 2,
    "next_cursor": "eyJ2IjoxLCJrIjpbIjIwMjUtMDktMTRUMTI6MzQ6NTcuMTAwWiIsIjAxSi4uLiJdLCJvIjoiZGVzYyIsInMiOiJjcmVhdGVkX2F0LGlkIn0"
  }
}
```

### Interaction with Pagination

Cursors encode `$filter`, `$orderby`, **and `$select`** to ensure consistent response shape across pages.

**Field selection is locked per pagination session**. Attempting to change `$select` with an existing cursor returns `400 FIELD_SELECTION_MISMATCH`. To use different fields, start a new pagination session without a cursor.

### Error Handling

All errors return RFC 9457 Problem Details format.

| Code | Status | Description |
|------|--------|-------------|
| `INVALID_FIELD` | 400 | Requested field not in allowlist. Returns `invalid_fields` and `allowed_fields` arrays. |
| `TOO_MANY_FIELDS` | 400 | Exceeded max field limit (default 50). Returns `max_fields` and `requested_fields` integers. |
| `FIELD_SELECTION_MISMATCH` | 400 | Cannot change `$select` during pagination. Returns `cursor_select` and `request_select` strings. |

### OpenAPI Integration

```yaml
x-odata-select:
  defaultFields: [id, title, status, priority, created_at, updated_at]
  allowedFields: [id, title, description, status, priority, created_at, updated_at, assignee_id, reporter_id]
  maxFields: 50
```

---

## OpenAPI Contract Requirements

Generated OpenAPI MUST explicitly document the allowed filter and order parameters per endpoint:

```yaml
components:
  parameters:
    FilterParam:
      name: $filter
      in: query
      required: false
      schema:
        type: string
      description: |
        OData filter over allowlisted indexed fields. See x-odata-filter.allowedFields.
    OrderByParam:
      name: $orderby
      in: query
      required: false
      schema:
        type: string
      example: created_at desc, id desc
      description: |
        OData order-by over allowlisted indexed fields. See x-odata-orderby.allowedFields.

paths:
  /v1/items:
    get:
      parameters:
        - $ref: '#/components/parameters/FilterParam'
        - $ref: '#/components/parameters/OrderByParam'
      x-odata-filter:
        allowedFields:
          created_at: [ge, gt, le, lt, eq]
          id: [eq, in]
      x-odata-orderby:
        allowedFields:
          - created_at desc
          - created_at asc
          - id asc
```

---

## Examples

### Request

```http
GET /v1/messages?limit=20&$filter=active eq true&$orderby=created_at desc, id desc
```

```http
GET /v1/messages?limit=20&cursor=eyJ2IjoxLCJrIjpbIjIwMjUtMDktMTRUMTI6MzQ6NTYuNzg5WiIsIjEyM2U0NTY3Il0sIm8iOiJkZXNjIiwicyI6ImNyZWF0ZWRfYXQsaWQiLCJmIjoiZjg2OWJhIn0
```

### Response

```json
{
  "items": [
    {
      "id": "018f6c9e-2c3b-7b1a-8f4a-9c3d2b1a0e5f",
      "created_at": "2025-09-14T12:34:57.100Z",
      "text": "..."
    }
  ],
  "page_info": {
    "next_cursor": "eyJ2IjoxLCJrIjpbIjIwMjUtMDktMTRUMTI6MzQ6NTcuMTAwWiIsIjEyM2U0NTY3Il0sIm8iOiJkZXNjIiwicyI6ImNyZWF0ZWRfYXQsaWQiLCJmIjoiZjg2OWJhIn0",
    "prev_cursor": null,
    "limit": 20
  }
}
```
