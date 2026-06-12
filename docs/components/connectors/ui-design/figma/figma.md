# Figma Connector Specification

> Version 1.1 — June 2026 (verified against the official OpenAPI spec, figma/rest-api-spec)
> Based on: `docs/components/connectors/ui-design/README.md` (Design Tools domain)
> Implementation: `src/ingestion/connectors/ui-design/figma/` (nocode declarative manifest, bronze-only)

Standalone specification for the Figma (Design Tools) connector. Expands the Design Tools domain schema with Figma-specific API details, authentication, endpoint mapping, and known limitations.

**v1.1 corrections over v1.0** (March 2026 draft was written before API verification):

- `GET /v1/teams/{team_id}/members` **does not exist** in the public REST API — there is no team/org member enumeration endpoint (it is an open feature request on the Figma forum). `design_users` cannot be populated from the REST API.
- The `User` object returned by versions/comments/meta endpoints carries **only `id`, `handle`, `img_url` — never `email`**. Email exists solely on `GET /v1/me` (the token owner).
- `design_file_activity` is **not** built by the connector — connectors are thin extractors (ADR `cpt-insightspec-adr-connector-responsibility-scope`); aggregation is a dbt concern. The connector ships raw streams instead.
- Rate limits are now publicly documented as per-endpoint tiers tied to the token owner's seat type (see below) — the old "~100–120 req/min" estimate is obsolete.

<!-- toc -->

- [Overview](#overview)
- [Authentication](#authentication)
- [API Endpoints Used](#api-endpoints-used)
- [Bronze Streams (implemented)](#bronze-streams-implemented)
- [Rate Limiting and Pagination](#rate-limiting-and-pagination)
- [Known Limitations](#known-limitations)
- [Identity Resolution](#identity-resolution)
- [Silver / Gold Notes](#silver-gold-notes)
- [Open Questions](#open-questions)

<!-- /toc -->

---

## Overview

**API**: Figma REST API v1 — `https://api.figma.com/v1/`

**Category**: Design Tools (`ui-design`)

**`data_source`**: `insight_figma`

**Authentication**: Personal Access Token (`X-Figma-Token` header)

**Identity**: `author_id` / `author_handle` (Figma user ID + display name) on versions, comments, and file metadata. **No email is available anywhere in the REST API** except `/v1/me`. Resolution to canonical `person_id` is deferred to Silver (handle-based matching via Identity Manager, or an Enterprise SCIM-sourced directory later).

**Critical limitations**:

- No per-user activity aggregates or activity feed (no `/v1/activity` equivalent). Activity is inferred from version history and comments.
- No team enumeration — `figma_team_ids` is required config, copied from team URLs.
- No member enumeration — user directory requires Enterprise SCIM API (separate token, SSO required) or Enterprise Activity Logs API (org admin, `org:activity_log_read`).
- File view counts: Enterprise-only Library Analytics / admin Analytics; no usable public API for per-file views.
- `GET /v1/files/{key}` (full document tree) is rate-limit **Tier 1** — 6 requests/month on viewer seats — and is deliberately not used. Design content never leaves Figma.

---

## Authentication

**Personal Access Token** (implemented):

- Generate at: **Figma → Settings → Security tab → Personal access tokens** — any user can create one, no admin required
- Header: `X-Figma-Token: {token}`
- Scopes: select read scopes covering projects, file metadata, file versions, file comments
- Access scope: the token sees exactly what its owner sees — the owner must be a member of every configured team
- Rate limits are tied to the owner's **seat type** — use a Dev/Full seat account
- The token is displayed once at creation

**OAuth 2.0** (not implemented): Figma supports OAuth apps (`https://www.figma.com/oauth`), but PAT is sufficient for server-side batch collection and avoids refresh-token management. Revisit if a customer requires it.

**Connector configuration fields** (see `src/ingestion/connectors/ui-design/figma/README.md` for the K8s Secret):

| Field | Required | Description |
|-------|----------|-------------|
| `figma_token` | Yes | Personal access token |
| `figma_team_ids` | Yes | Comma-separated team IDs (no default — fail-fast) |
| `figma_start_date` | No | Earliest date of interest, default `2020-01-01` |
| `figma_page_size` | No | Versions page size, default 50 (API max) |

---

## API Endpoints Used

| Endpoint | Tier | Pagination | Used for |
|----------|------|------------|----------|
| `GET /v1/teams/{team_id}/projects` | 2 | none | `design_projects` |
| `GET /v1/projects/{project_id}/files` | 2 | none | `design_files` (`key`, `name`, `last_modified`) |
| `GET /v1/files/{file_key}/meta` | 3 | none | `design_file_meta` (`creator`, `last_touched_by`, `editorType`, `folder_name`, `link_access`) |
| `GET /v1/files/{file_key}/versions` | 2 | `pagination.next_page` URL, `page_size` ≤ 50 | `design_file_versions` |
| `GET /v1/files/{file_key}/comments` | 2 | none | `design_file_comments` (top-level + replies via `parent_id`) |

Endpoints **not** used and why:

| Endpoint | Why not |
|----------|---------|
| `GET /v1/files/{key}` (full node tree) | Tier 1 rate limit; design content not needed for analytics |
| `GET /v1/teams/{id}/components` / `styles` | Design-system metrics — out of scope for v1 |
| SCIM `/Users` | Enterprise + SSO only; candidate for an optional `design_users` stream later |
| `GET /v1/activity_logs` | Enterprise only, org admin token |
| Webhooks v2 | Batch polling model; no push infrastructure needed |

---

## Bronze Streams (implemented)

Raw extraction, namespace `bronze_figma`. Every record carries `tenant_id`, `source_id`, `unique_key`, `data_source='insight_figma'`, `collected_at`.

| Stream | Natural key | Sync mode | Notes |
|--------|-------------|-----------|-------|
| `design_projects` | `project_id` | full refresh | Partitioned by configured `figma_team_ids`; `team_id` denormalized |
| `design_files` | `file_key` | incremental (client-side cursor on `last_modified`) | `project_id`, `project_name`, `team_id` denormalized from parent |
| `design_file_meta` | `file_key` | full refresh (substream) | `creator_id/handle`, `last_touched_by_id/handle`, `last_touched_at`, `editor_type` (figma/figjam/slides/…), `link_access` |
| `design_file_versions` | `file_key` + `version_id` | full refresh (substream) | `author_id/handle`, `created_at`, `label`, `description`; bounded by `figma_start_date` (record filter + pagination stop) |
| `design_file_comments` | `file_key` + `comment_id` | full refresh (substream) | `parent_comment_id` for replies, `resolved_at`, `message`, `order_id`, `reaction_count` |

Differences from the v1.0 draft's `design_*` table plan:

- `design_users` — **dropped**: no REST endpoint exists. See Identity Resolution.
- `design_file_activity` — **moved to dbt** (planned staging model), not a connector output.
- `design_collection_runs` — superseded by platform-level run tracking (Argo/Airbyte job logs); modern connectors do not emit a runs stream.

---

## Rate Limiting and Pagination

Figma publishes per-endpoint **tiers**; budgets depend on the token owner's seat type and the workspace plan ([developers.figma.com/docs/rest-api/rate-limits](https://developers.figma.com/docs/rest-api/rate-limits/)):

| Tier | Example endpoints | Viewer/Collab seat | Dev/Full seat (Pro / Org) |
|------|-------------------|--------------------|---------------------------|
| 1 | full file, images | **6/month** | 15–20/min |
| 2 | projects, files, versions, comments | 5/min | 50–100/min |
| 3 | file meta, components, users | 10/min | 100–150/min |

- All five endpoints used by the connector are Tier 2/3.
- On `429` the API returns `Retry-After`; the manifest honours it with a 600 s cap (confluence pattern), retries 5xx with backoff.
- Per-file fan-out is 3 requests (meta + versions + comments). At 50 req/min (Pro, Dev seat) a 500-file workspace takes ~30 min per full pass; client-side incremental on `last_modified` keeps subsequent syncs short.
- File-level `403`/`404` (invite-only projects, deleted files) are IGNOREd; team-level `401`/`403`/`404` FAIL the run (bad token or team ID).

**Pagination**: only `/versions` paginates — `pagination.next_page` is a full URL injected via `RequestPath`, `page_size` max 50, newest-first ordering (which makes the `figma_start_date` pagination stop-condition correct).

---

## Known Limitations

| Limitation | Impact | Mitigation |
|-----------|--------|------------|
| No member enumeration in REST API | No `design_users` stream; no emails for authors | Identity via handle-matching in Silver; optional SCIM-based directory stream for Enterprise tenants later |
| `User` objects have no email | Author attribution is `id` + display name only | Same as above |
| No activity feed | Activity inferred from versions + comments; editing without version checkpoints is undercounted | Document as proxy metric; recommend enabling autosave-version setting in Figma org settings |
| No server-side date filter on versions/comments | First sync walks history | `figma_start_date` record filter + pagination stop on versions; comments are a single response per file |
| Seat-type-dependent rate limits | Viewer-seat token makes the connector unusably slow | README requires a Dev/Full seat token owner |
| No team enumeration | New Figma teams require a config update | Documented in README; `figma_team_ids` has no default (fail-fast) |
| View counts unavailable | No `files_viewed` metric | Revisit if Figma ships a public Analytics API |

---

## Identity Resolution

**What the API gives us**: `author_id` (stable Figma user ID) and `author_handle` (display name) on versions, comments, file meta. No email.

**Resolution chain (Silver, future)**:

```text
design_file_versions.author_id / author_handle
  → Identity Manager name-matching (handle ≈ display name in HR/git sources)
    → person_id
```

Fallbacks, in order of preference:

1. **Enterprise SCIM directory** (if tenant has Figma Enterprise + SSO): `userName` = email, full name → straightforward email join. Candidate optional stream.
2. **Handle-based matching** via Identity Manager (display-name aliases, same mechanism as first-name alias matching in `inbox/IDENTITY_RESOLUTION.md`).
3. Stable `author_id` keeps per-person continuity inside Figma even when unresolved to `person_id`.

This mirrors the confluence situation (no email in the v2 API; resolved via `jira_user` join) — except Figma has no companion product table to join, so name-matching or SCIM is the path.

---

## Silver / Gold Notes

Bronze-only delivery for now; `dbt_select: tag:figma+` runs only `figma__bronze_promoted` (RMT promotion).

Planned Silver step 1 (staging) models, by analogy with confluence:

- `figma__design_activity` → `class_design_activity`: per-author per-day rollup from `design_file_versions` (session-collapse like `confluence__wiki_activity` — Figma autosave checkpoints need the same 30-min gap treatment) plus comment counts from `design_file_comments`.
- `design_files` + `design_file_meta` serve as dimension tables.

**Designer↔engineer correlation (Gold)**: `class_design_activity` joined with `class_commits` on `(person_id, date ± N days)`; `design_files.project_name` correlated with repo / Jira project names.

---

## Open Questions

### OQ-FIGMA-1: Figma Analytics API (Enterprise)

Unchanged from v1.0: no public API for view/edit analytics. Decision: no undocumented endpoints; revisit when Figma announces an Analytics API.

### OQ-FIGMA-2: Team enumeration

**Resolved (June 2026)**: there is no org-level team enumeration endpoint in the REST API at any plan tier. `figma_team_ids` manual configuration is the only option. The Enterprise Activity Logs API exposes team-related events but is not a directory.

### OQ-FIGMA-3: Version vs. autosave — activity undercount

Unchanged: versions are checkpoint-based; editing without checkpoints is invisible. Mitigation: recommend the org setting that auto-creates versions; treat `versions_created` as a proxy metric. The Silver session-collapse must also avoid over-counting autosave checkpoint bursts (see `confluence__wiki_activity` precedent).

### OQ-FIGMA-4 (new): SCIM-based `design_users` stream

For Enterprise + SSO tenants, the SCIM API (`GET /scim/v2/Users`, max 3000/page) provides the full directory with emails — exactly what identity resolution wants. Needs a separate SCIM token and applies to a minority of tenants. Decide whether to add it as an optional conditional stream when the first Enterprise tenant lands.
