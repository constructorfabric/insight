# DESIGN — Claude Team Connector

| Field | Value |
|---|---|
| Component | `connectors/ai/claude-team` |
| Status | MVP |
| Related PRD | `PRD.md` |

## 1. Architecture overview

```
  ┌──────────────────────────┐                   ┌────────────────────────────┐
  │   Insight cluster        │                   │   Customer environment     │
  │                          │                   │                            │
  │   reconcile-loop         │                   │   claude-team-proxy        │
  │      │                   │                   │   (Docker container,       │
  │      ▼                   │   HTTPS +         │    runs Playwright +       │
  │   Airbyte source ────────┼───Bearer token───►│    Chromium, holds         │
  │   (declarative)          │                   │    sessionKey cookie       │
  │      │                   │                   │    in memory only)         │
  │      ▼                   │                   │            │               │
  │   ClickHouse Bronze      │                   │            ▼               │
  │   bronze_claude_team.*   │                   │       claude.ai            │
  └──────────────────────────┘                   └────────────────────────────┘
```

The Insight side runs an Airbyte declarative source against a customer-
hosted HTTP endpoint. The Insight side never sees the claude.ai session
cookie. The customer-side proxy is opaque to Insight — it lives in
`secure-enclave/proxies/claude_team/` and is owned operationally by the
customer.

The rest of this document covers the **Insight-side** connector only.

## 2. Configuration

### 2.1 Spec fields (Airbyte connector_specification)

| Field | Required | Source | Notes |
|---|---|---|---|
| `claude_org_id` | yes | K8s Secret | UUID of the claude.ai org |
| `proxy_url` | yes | K8s Secret | Customer-exposed proxy base URL (no default) |
| `proxy_auth_token` | yes | K8s Secret | Shared bearer token, masked in logs |
| `insight_tenant_id` | yes | reconcile loop | Tenant slug |
| `insight_source_id` | yes | Secret annotation | `insight.cyberfabric.com/source-id` |
| `start_date` | no | K8s Secret | Earliest `claude_team_code_metrics` date (YYYY-MM-DD); default 7 days ago |

### 2.2 K8s Secret shape

See `src/ingestion/secrets/connectors/claude-team.yaml.example`. The
reconcile loop discovers the Secret via label
`app.kubernetes.io/part-of=insight` and annotations
`insight.cyberfabric.com/{connector,source-id}`.

## 3. Streams

All four streams emit Bronze rows with a common envelope:

```
{
  "tenant_id":    "<from reconcile>",
  "source_id":    "<from annotation>",
  "unique_key":   "<composite>",
  "collected_at": "2026-05-27T08:00:00Z",
  "data_source":  "insight_claude_team",
  ...stream-specific fields
}
```

### 3.1 `claude_team_members`

| Aspect | Value |
|---|---|
| HTTP | `GET {proxy_url}/api/organizations/{claude_org_id}/members` |
| Auth | `Authorization: Bearer {proxy_auth_token}` |
| Response | Plain JSON array — `field_path: []` extractor |
| Primary key | `account/uuid` |
| Sync mode | Full refresh (overwrite) |
| Pagination | None |
| `unique_key` | `{tenant_id}-{source_id}-{account.uuid}` |

Per-record shape (post-envelope):

```
{
  "account": {
    "uuid":          "9bb28bd1-…",
    "tagged_id":     "user_…",            // nullable
    "full_name":     "Jane Doe",          // nullable
    "email_address": "jane@example.com"
  },
  "role":       "user|primary_owner|…",
  "seat_tier":  "team_tier_1|unassigned|…", // nullable
  "created_at": "2026-03-05T12:37:08Z",
  "updated_at": "2026-05-19T09:37:30Z"
}
```

### 3.2 `claude_team_invites`

| Aspect | Value |
|---|---|
| HTTP | `GET {proxy_url}/api/organizations/{claude_org_id}/invites` |
| Auth | Bearer |
| Response | Plain JSON array — `field_path: []` |
| Primary key | `uuid` |
| Sync mode | Full refresh |
| Pagination | None |
| `unique_key` | `{tenant_id}-{source_id}-{uuid}` |

Returns only **pending** invites. Accepted/expired ones drop from the
response. Historical invite events are not reconstructable from this
endpoint.

### 3.3 `claude_team_overage_spend`

| Aspect | Value |
|---|---|
| HTTP | `GET {proxy_url}/api/organizations/{claude_org_id}/overage_spend_limits` |
| Auth | Bearer |
| Response | `{items: [...], page, per_page, total, total_pages}` |
| Extractor | `field_path: [items]` |
| Primary key | `account_uuid` |
| Sync mode | Full refresh |
| Pagination | `PageIncrement(page_size=100, start_from_page=1)` |
| `unique_key` | `{tenant_id}-{source_id}-{account_uuid}` |
| Error handler | HTTP 403 → IGNORE (stream stays empty, sync stays GREEN) |

The 403 case is the dominant failure mode: the sessionKey installed on
the proxy lacks `billing:view`. The connector continues; once a
billing-capable cookie is rotated in on the proxy via
`POST /admin/session-key`, the next sync populates rows automatically.

### 3.4 `claude_team_code_metrics`

| Aspect | Value |
|---|---|
| HTTP | `GET {proxy_url}/api/claude_code/metrics_aggs/users` |
| Auth | Bearer |
| Query params | `organization_uuid={claude_org_id}`, `customer_type=claude_ai`, `subscription_type=team`, `sort_by=total_lines_accepted`, `sort_order=desc`, `start_date=YYYY-MM-DD`, `end_date=YYYY-MM-DD`, `limit=100`, `offset=N` |
| Response | `{users: [...], pagination, pr_attribution, top_users_*, total_users}` |
| Extractor | `field_path: [users]` |
| Primary key | `(metric_date, email)` |
| Sync mode | Incremental |
| Cursor | `DatetimeBasedCursor` on `metric_date`, format `%Y-%m-%d`, `step: P1D` |
| Pagination | `OffsetIncrement(page_size=100)` |
| `unique_key` | `{tenant_id}-{source_id}-{date}-{email}` |
| Backfill floor | `min_datetime: 2025-11-24` |

Date semantics: claude.ai's API treats `start_date` and `end_date` as
**inclusive single-day filters** — `start_date=X, end_date=X` returns
one day's data. We exploit this by walking one day per request.

`metric_date` injection: the per-user objects do NOT carry a date
field. Without injecting the cursor's current value, two days' rows
would collide on the `email` primary key. The connector adds
`metric_date` via a second `AddFields` transformation after the field-
extraction step.

Dropped fields: the response also carries `pr_attribution`,
`top_users_by_prs`, `top_users_by_lines_of_code`. These are
org-aggregates (not per-user) and were empty in the reference org. If
they become populated later they would warrant their own streams
rather than denormalisation into every user row.

Cost field convention: `avg_cost_per_day` and `total_cost` are
returned as decimal-as-string. Bronze keeps them as `type: string` —
the Silver layer is expected to cast.

## 4. Cross-cutting

### 4.1 Authentication

Every `/api/*` call carries `Authorization: Bearer {proxy_auth_token}`.
The proxy validates with a constant-time string compare. On mismatch
the proxy returns 401 → Airbyte fails the sync RED → Argo workflow
status reflects the failure.

### 4.2 Failure semantics

| Upstream condition | Connector behaviour |
|---|---|
| Proxy 200 with non-empty body | Records ingested |
| Proxy 200 with empty `users` page | OffsetIncrement stops the stream |
| Proxy 403 (overage_spend only) | IGNORE — stream empty, sync GREEN |
| Proxy 401 (bad bearer) | Sync RED |
| Proxy 503 "transport not ready" | Sync RED — operator should check the proxy's `/admin/session-key` state |
| Proxy 504 / 502 (transport timeout / upstream issue) | Airbyte retries per its default policy |

### 4.3 Schema enforcement

`metadata.autoImportSchema` is `true` for all four streams. Airbyte
auto-imports the declared JSON-Schema and emits records that match it.
Extra fields from the API (e.g. future additions claude.ai makes) are
allowed via `additionalProperties: true`.

### 4.4 Silver layer

Out of scope for the **Bronze MVP** — but Silver has since landed
(`descriptor.dbt_select: 'tag:claude-team+'`, not `''`). Models are
tagged `claude-team` and contribute to shared Silver classes:

- `claude_team__ai_dev_usage` → `class_ai_dev_usage` (per-user-per-day
  Claude Code usage from `claude_team_code_metrics`; INSIGHT-458).
- `claude_team__ai_overage` → `class_ai_overage` (per-seat-per-month
  spend over the monthly credit limit from `claude_team_overage_spend`;
  descriptor 1.3.0). `overage_cents = max(0, used_credits −
  monthly_credit_limit)`, units already cents (no ×100). Gold surfaces
  it as the `cc_overage` AI bullet. See the cross-connector contract in
  `docs/components/connectors/ai/README.md` and `src/ingestion/silver/ai/schema.yml`.

`bronze_promoted` (ADR-0002) promotes all populated Bronze streams,
including `claude_team_overage_spend`, to ReplacingMergeTree.

## 5. Operational limitations (MVP)

- **sessionKey expiry detection**: the connector has no direct signal
  for an expired cookie. The proxy's `/api/*` responses transition
  from 200 to 503 ("transport not ready") once a stale cookie fails
  CF clearance. Operators rely on Airbyte sync RED + the proxy's
  `/health` endpoint for diagnosis.
- **Single-org per proxy**: `claude_org_id` is duplicated between the
  K8s Secret (Insight side, for path construction) and the proxy env
  (customer side, for clarity/health). They must match — there is no
  enforcement, only convention.
- **Proxy availability**: the customer owns proxy uptime. If the proxy
  is down at sync time, the sync fails RED. Insight does not run a
  health check against the proxy outside of sync time.

## 6. Validation

```
./src/ingestion/tools/declarative-connector/source.sh validate-strict ai/claude-team
./src/ingestion/tools/declarative-connector/source.sh validate        ai/claude-team
```

The first enforces the strict declarative schema (rejects unknown
keys). The second runs Airbyte's `check` operation against a real
proxy + sessionKey — used only for ad-hoc verification, not CI.
