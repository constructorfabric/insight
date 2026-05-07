# PRD — Confluence Connector

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
  - [5.1 Space and Page Extraction](#51-space-and-page-extraction)
  - [5.2 Activity Extraction](#52-activity-extraction)
  - [5.3 User Directory](#53-user-directory)
  - [5.4 Connector Operations](#54-connector-operations)
  - [5.5 Data Integrity](#55-data-integrity)
  - [5.6 Identity Resolution](#56-identity-resolution)
- [6. Non-Functional Requirements](#6-non-functional-requirements)
  - [6.1 NFR Inclusions](#61-nfr-inclusions)
  - [6.2 NFR Exclusions](#62-nfr-exclusions)
- [7. Public Library Interfaces](#7-public-library-interfaces)
  - [7.1 Public API Surface](#71-public-api-surface)
  - [7.2 External Integration Contracts](#72-external-integration-contracts)
- [8. Use Cases](#8-use-cases)
  - [UC-001 Configure Confluence Connection](#uc-001-configure-confluence-connection)
  - [UC-002 Incremental Sync Run](#uc-002-incremental-sync-run)
- [9. Acceptance Criteria](#9-acceptance-criteria)
- [10. Dependencies](#10-dependencies)
- [11. Assumptions](#11-assumptions)
- [12. Risks](#12-risks)
- [13. Resolved Questions](#13-resolved-questions)
- [14. Non-Applicable Requirements](#14-non-applicable-requirements)

<!-- /toc -->

## 1. Overview

### 1.1 Purpose

The Confluence Connector extracts page metadata, version history (edit activity), per-user view analytics (Premium tier), space directory, and user directory from the Atlassian Confluence REST API v2 (Cloud) and loads them into the Insight platform's Bronze layer using the unified wiki schema (`wiki_*` tables). It enables knowledge management analytics — page authorship, editorial velocity, documentation health, and knowledge consumption patterns — alongside the existing Outline connector in a unified wiki analytics domain.

### 1.2 Background / Problem Statement

Confluence is the most widely used enterprise wiki and knowledge base platform. Insight defines a unified wiki schema (`wiki_*` tables) that supports multiple wiki sources via a `data_source` discriminator. The Confluence connector must populate this shared schema with Confluence-specific data while handling several platform-specific challenges.

The Confluence REST API v2 (Cloud) returns `authorId` (Atlassian `accountId`) on page and version responses but does not include email. Email resolution requires separate calls to the Atlassian User API (v1 endpoint: `/rest/api/user/bulk`), adding API call overhead and introducing a dependency on the v1 API alongside v2.

Per-user view analytics (`GET /analytics/content/{id}/viewers`) is available only on Confluence Premium and Enterprise tiers. Standard tier instances return 403/404 for this endpoint. The connector must gracefully degrade — collecting edit activity on all tiers while only collecting view data when the analytics endpoint is available.

The wiki data model is document-centric rather than event-centric: Bronze tables store current page state (`wiki_pages`) and aggregated per-user per-day activity metrics (`wiki_page_activity`), not raw events. The Silver layer applies SCD Type 2 to track page evolution over time.

**Target Users**:

- Platform operators who configure Confluence API credentials, space scope, and monitor extraction runs
- Data analysts who consume wiki activity data in Silver/Gold layers alongside Outline for unified knowledge management metrics
- Engineering and documentation leads who use editorial velocity, knowledge consumption, and documentation health data for team effectiveness analysis

**Key Problems Solved**:

- Lack of Confluence data in the Insight platform, preventing unified wiki analytics across Confluence and Outline
- No visibility into knowledge creation patterns (who writes, how often, which spaces are active vs. stale)
- Missing per-user view data needed to measure documentation consumption and identify high-value content
- No cross-system identity resolution between Confluence users and other Insight sources (Jira, GitHub, M365, Slack)
- Email resolution from Atlassian `accountId` requires separate API calls not available in the v2 page response

### 1.3 Goals (Business Outcomes)

**Success Criteria**:

- Confluence wiki data extracted with no missed sync windows over a 90-day period (Baseline: no Confluence extraction; Target: v1.0)
- Per-user edit and view activity available for identity resolution within 24 hours of extraction (Baseline: N/A; Target: v1.0)
- Confluence data unified with Outline in the `wiki_*` Silver tables for cross-source knowledge management analytics (Baseline: Outline only; Target: v1.0)

**Capabilities**:

- Extract Confluence spaces, pages with parent hierarchy, and version history (edit activity)
- Extract per-user per-page view analytics when Confluence Premium tier is available
- Graceful degradation on Standard tier — edit activity collected without view data
- Email resolution from Atlassian `accountId` via the User API bulk endpoint
- Incremental extraction using client-side cursor via `sort=-modified-date` with `is_client_side_incremental` for pages
- All data written to the unified `wiki_*` schema with `data_source = 'insight_confluence'`

### 1.4 Glossary

| Term | Definition |
|------|------------|
| Confluence REST API v2 | Atlassian's current REST API for Confluence Cloud (`/wiki/api/v2/`). Provides access to spaces, pages, and versions. Coexists with v1 for user-related endpoints. |
| Confluence REST API v1 | Legacy API (`/wiki/rest/api/`) still required for user profile lookup (`/rest/api/user/bulk`). No v2 equivalent exists for single-user or bulk-user lookup. |
| Atlassian Account ID (`accountId`) | Opaque, immutable identifier for a user across the Atlassian platform (Confluence, Jira, Bitbucket). Used as the internal user key — not suitable for cross-system identity resolution. |
| Confluence Premium | A paid tier that includes the Analytics endpoint (`/analytics/content/{id}/viewers`). Standard tier does not have this endpoint. |
| Space | A Confluence organizational container for pages. Types: `global` (team/project spaces) and `personal` (individual user spaces). |
| Version | A saved revision of a Confluence page. Each version records the author (`authorId`), creation timestamp, and version number. One version = one edit. |
| Bronze Table | Raw data table in the destination using the unified wiki schema (`wiki_*`), with `data_source = 'insight_confluence'` discriminator. |

## 2. Actors

### 2.1 Human Actors

#### Platform Operator

**ID**: `cpt-insightspec-actor-conf-operator`

**Role**: Configures Confluence instance credentials (Basic Auth with email + API token, or OAuth 2.0), selects space scope, enables/disables analytics collection, and monitors extraction runs.
**Needs**: Ability to configure the connector with Confluence credentials, filter by space scope, verify data flow, and understand whether analytics (view data) is available for the connected instance.

#### Data Analyst

**ID**: `cpt-insightspec-actor-conf-analyst`

**Role**: Consumes Confluence page, edit activity, and view data from Silver/Gold layers to build dashboards for editorial velocity, knowledge consumption, documentation health, and space utilization — alongside Outline data in unified wiki views.
**Needs**: Complete, gap-free page and activity data with identity resolution to canonical person IDs for cross-platform aggregation.

### 2.2 System Actors

#### Confluence REST API

**ID**: `cpt-insightspec-actor-conf-api`

**Role**: External REST API providing access to spaces, pages, versions, analytics, and user profiles. Uses v2 endpoints for spaces/pages/versions and v1 endpoints for user lookup. Enforces rate limits via token-bucket model (varying by endpoint category) and requires Basic Auth or OAuth 2.0 authentication.

#### Identity Manager

**Ref**: `cpt-insightspec-actor-identity-manager`

**Role**: Resolves `email` from Confluence user directory to canonical `person_id` in Silver step 2. Enables cross-system joins (Confluence + Outline + Jira + GitHub + M365 + Slack, etc.).

## 3. Operational Concept & Environment

### 3.1 Module-Specific Environment Constraints

- Requires an Atlassian account with API access to the target Confluence Cloud instance
- Authentication via Basic Auth (email + Atlassian API token from `id.atlassian.com`) or OAuth 2.0 (3-legged flow via Atlassian developer console)
- OAuth 2.0 requires scopes: `read:confluence-space.summary`, `read:confluence-content.all`, `read:confluence-content.summary`, `read:confluence-user`, and optionally `read:confluence-analytics` (Premium only)
- The connector uses v2 API endpoints for spaces, pages, and versions, and v1 API endpoints for user profile lookup (no v2 equivalent for `/rest/api/user/bulk`)
- The connector operates as a batch collector; the recommended run frequency is daily
- Atlassian enforces rate limits using a token-bucket model with varying budgets per endpoint category (content vs. analytics vs. user API). The connector must handle both HTTP 429 (rate limited) and HTTP 503 (service unavailable, also used for throttling) responses with exponential backoff. Analytics endpoints may have stricter limits than content endpoints
- Per-user view analytics (`/analytics/content/{id}/viewers`) is available only on Confluence Premium and Enterprise tiers; the connector must detect tier and degrade gracefully on Standard tier
- The analytics endpoint requires iterating over every page individually — API call volume scales with page count

## 4. Scope

### 4.1 In Scope

- Extraction of Confluence spaces with type, status, and URL
- Extraction of pages with metadata, version number, parent hierarchy, author, and last editor
- Extraction of version history (edit activity) as per-user per-page per-day edit counts
- Extraction of footer and inline page comments (top-level + replies for both kinds) — issue #285
- Extraction of per-user per-page view analytics from the Premium analytics endpoint (graceful degradation when unavailable)
- Extraction of user directory via `accountId` → email resolution through the Atlassian User API bulk endpoint
- User directory history preserved with SCD Type 2 (`valid_from`/`valid_to`)
- Connector execution monitoring via collection runs stream
- Incremental sync using client-side cursor via `sort=-modified-date` with `is_client_side_incremental` for pages
- Identity resolution via `email` resolved from Atlassian `accountId`
- All data written to unified `wiki_*` schema with `data_source = 'insight_confluence'`
- `insight_source_id` and `tenant_id` stamped on every record
- All timestamps normalized to UTC

### 4.2 Out of Scope

- Silver/Gold layer transformations — responsibility of the wiki domain pipeline
- Silver step 2 (identity resolution: `email` → `person_id`) — responsibility of the Identity Manager
- Real-time streaming — this connector operates in batch mode
- Confluence Server/Data Center — this connector targets Confluence Cloud only
- Page body/content extraction — only metadata and activity metrics are collected
- Attachment downloads or file metadata
- Blog posts — deferred to a future release
- Confluence webhooks or event-driven collection
- Outline connector implementation — separate connector using the same `wiki_*` schema

## 5. Functional Requirements

### 5.1 Space and Page Extraction

#### Extract Spaces

- [ ] `p1` - **ID**: `cpt-insightspec-fr-conf-space-extraction`

The connector **MUST** extract the Confluence space directory from `GET /spaces` (paginated), including: space ID, name, description (plain text), space type (`global`/`personal`), status (mapped: `current` → `active`, `archived` → `archived`), web URL, and collection timestamp.

**Rationale**: Spaces are the organizational structure of Confluence. Space type and status enable filtering (e.g., exclude personal spaces from team analytics) and documentation health assessment (active vs. archived spaces).

**Actors**: `cpt-insightspec-actor-conf-api`, `cpt-insightspec-actor-conf-analyst`

#### Extract Pages with Metadata

- [ ] `p1` - **ID**: `cpt-insightspec-fr-conf-page-extraction`

The connector **MUST** extract Confluence pages from `GET /pages` (paginated, filtered by space), including: page ID, space ID, title, status (`current`/`archived`/`trashed`/`draft`), author ID (`accountId`), author email (resolved), last editor ID (`accountId` from `version.authorId`), last editor email (resolved), creation timestamp, last update timestamp (`version.createdAt`), version number, parent page ID, and collection timestamp.

The page query **MUST** include `status=current,archived,trashed` to capture trashed pages (soft-deleted, in the Confluence trash). Including trashed pages enables downstream analytics to detect deletions — a page transitioning from `current` to `trashed` between runs is a soft-delete signal. Draft pages (`status=draft`) are excluded because they are unpublished private content.

The connector **MUST** support incremental sync using `sort=-modified-date` with client-side cursor filtering (`is_client_side_incremental: true`). The cursor value is `max(version.createdAt)` from the previous successful run. On subsequent runs, the connector fetches all pages sorted by descending modification date and filters client-side against the stored cursor. **Note**: The v2 `/pages` endpoint does NOT support a `lastModifiedAfter` query parameter -- client-side incremental filtering is the only available approach.

**Known limitation**: If a page is **permanently purged** from the Confluence trash (not just trashed), it disappears from all API responses. The incremental cursor will never see it. The connector **SHOULD** support a periodic full-refresh reconciliation run (e.g., weekly) to detect permanently purged pages by comparing Bronze page IDs against the current full page list. Between reconciliation runs, purged pages persist in Bronze as `status = trashed` or `status = current`.

**Known behavior**: New pages appear in incremental results because their `createdAt` qualifies as a modification timestamp and the client-side cursor compares against it. This resolves OQ-CONF-3.

**Rationale**: Pages are the core entity for wiki analytics. Parent hierarchy enables documentation structure analysis. Per-space incremental sync is required for sustainable operation on large instances and for correct handling of scope changes.

**Actors**: `cpt-insightspec-actor-conf-api`, `cpt-insightspec-actor-conf-analyst`

### 5.2 Activity Extraction

#### Extract Edit Activity from Version History

- [ ] `p1` - **ID**: `cpt-insightspec-fr-conf-edit-activity`

The connector **MUST** extract version history for collected pages from `GET /pages/{id}/versions` (paginated). Each version **MUST** produce one edit activity row in `wiki_page_activity` with: page ID, user ID (`accountId` of version author), user email (resolved), date (UTC date part of `version.createdAt`), `edit_count = 1`, `view_count = null`, and collection timestamp.

The connector **MUST** only fetch versions created after the last successful cursor position to avoid re-processing historical versions on incremental runs.

**Rationale**: Version history is the edit log for Confluence — each version represents one save/edit event. Per-user per-day edit counts are the foundation for editorial velocity metrics and contributor activity analysis.

**Actors**: `cpt-insightspec-actor-conf-api`, `cpt-insightspec-actor-conf-analyst`

#### Extract Page Comments (Footer + Inline, with Replies)

- [ ] `p1` - **ID**: `cpt-insightspec-fr-conf-comment-extraction`

The connector **MUST** capture page comments — both kinds Confluence exposes (footer comments at the page bottom and inline comments anchored to a highlighted text fragment) — and **MUST** capture replies to those comments, attributed to the comment author and to the parent page.

Each captured comment **MUST** carry, at minimum, enough information for downstream Silver to compute per-page-day engagement (commenter identity, parent page, creation date, comment kind, top-level vs reply distinction).

**Rationale**: Comments are how reviewers, doc owners, and stakeholders engage with a page after publish. Today's per-edit `wiki_page_activity` undercounts collaboration because someone who comments on others' pages but rarely authors is invisible. Comment data feeds engagement KPIs like "this RFC got 12 comments from 5 reviewers" vs "this RFC was published and ignored". Issue #285.

**Stream wiring, endpoint choreography, and Bronze field definitions** are specified in [DESIGN §1.2 / §3.7](./DESIGN.md) — the v2 page-level listing endpoints don't return replies (verified empirically), so the implementation needs a 2-level parent-child substream chain.

**Actors**: `cpt-insightspec-actor-conf-api`, `cpt-insightspec-actor-conf-analyst`

#### Extract View Analytics (Premium Tier)

- [ ] `p2` - **ID**: `cpt-insightspec-fr-conf-view-analytics`

The connector **MUST** attempt to collect per-user per-page view data from `GET /analytics/content/{id}/viewers` (Premium-only endpoint). Each viewer record **MUST** produce one view activity row in `wiki_page_activity` with: page ID, user ID (`accountId`), user email (available directly in analytics response), date (UTC date part of `viewedAt`), `view_count = 1`, `edit_count = null`, and collection timestamp.

**Graceful degradation**: If the analytics endpoint returns HTTP 403 or 404 (Standard tier), the connector **MUST**:
1. Detect the tier limitation on the first failed analytics call during the run.
2. Skip all remaining analytics collection for the current run.
3. Log the tier limitation in the collection run log (`analytics_available = false`).
4. Set all view-related fields (`view_count` in `wiki_page_activity`, `view_count` and `distinct_viewers` in `wiki_pages`) to `null`.
5. **MUST NOT** retry analytics calls or report the 403/404 as an error.

This resolves OQ-CONF-2 — the connector detects tier on first failure and skips gracefully.

The analytics viewers endpoint is paginated (default limit: 100 per response). The connector **MUST** follow pagination (`_links.next`) until all viewers are retrieved for each page.

**Data quality caveat**: The `viewedAt` field in the analytics response represents the **last** time a user viewed the page — not a per-day event log. If a user viewed a page on Day 1 and Day 5, only Day 5 is returned. Therefore, `wiki_page_activity` view rows represent "last known view date per user per page" — not "the user viewed the page on exactly this date." Gold-layer metrics like "views per page per week" will undercount because intermediate view dates are lost. This is a fundamental limitation of the Confluence Analytics API.

**Populating `wiki_pages.view_count` and `distinct_viewers`**: These aggregate fields **MUST** be sourced from the analytics summary endpoint (`GET /analytics/content/{id}`) which returns `viewCount` directly — not computed by counting rows from the per-user viewers endpoint. The per-user endpoint only reports unique viewers, not total view count. If analytics is unavailable (Standard tier), both fields **MUST** be `null`.

**Rationale**: View data measures knowledge consumption — which pages are actually read, by whom. This is a high-value metric for documentation health but is Premium-only. Graceful degradation ensures the connector is useful on all Confluence tiers.

**Actors**: `cpt-insightspec-actor-conf-api`, `cpt-insightspec-actor-conf-analyst`

### 5.3 User Directory

#### Resolve Email from accountId

- [ ] `p2` - **ID**: `cpt-insightspec-fr-conf-email-resolution`

> **Phase 1 deferral**: Email resolution is delivered by the Silver/dbt layer via JOIN with the `jira_user` stream (shared Atlassian `accountId` namespace). The Confluence connector emits `accountId` only on `author_id` / `last_editor_id`; `author_email` / `last_editor_email` are populated downstream. Rationale: the v1 `/rest/api/user/bulk` endpoint cannot be expressed in the declarative YAML runtime without a CDK fallback, and the `jira_user` JOIN provides equivalent coverage for any organization that also operates Jira. Priority reduced from `p1` to `p2` per DESIGN §1.1 and §3.2 (driver row 5). Restore to `p1` with a dedicated User API stream when connector-level resolution is required (e.g., Confluence-only deployments with no Jira connector).

The connector **MUST** collect all unique `accountId` values encountered in page responses (`authorId`, `version.authorId`) and analytics viewer records, and batch-resolve them to email via `GET /rest/api/user/bulk?accountId={id1}&accountId={id2}` (v1 API, up to 200 IDs per request).

The resolved email **MUST** be:
1. Normalized (lowercase, trimmed).
2. Backfilled into `author_email` and `last_editor_email` in `wiki_pages`, and `user_email` in `wiki_page_activity`.
3. Stored in `wiki_users` as the identity resolution key.

The connector **MUST NOT** fail if the User API returns `null` for `email` on some accounts. Email may be unavailable for:
- Deactivated accounts with redacted email
- Active managed accounts provisioned via SCIM (Atlassian Access / Atlassian Guard) where the API token user lacks org-admin privileges
- App/bot accounts without email

Records with unresolvable email **MUST** retain the `accountId` in `user_id` and set `email = null`. The collection run log **MUST** record `unresolved_email_count` so operators can detect when identity resolution coverage is degrading. This resolves OQ-CONF-1.

**Rationale**: Confluence v2 API returns only `accountId` (opaque, Atlassian-internal). Email is the only cross-system identity key. Without explicit resolution via the User API, Confluence users cannot be linked to their Jira, GitHub, or Slack identities.

**Actors**: `cpt-insightspec-actor-conf-api`, `cpt-insightspec-actor-identity-manager`

#### Preserve User Directory History (SCD Type 2)

- [ ] `p2` - **ID**: `cpt-insightspec-fr-conf-user-scd`

> **Phase 1 deferral**: The `wiki_users` stream is not emitted by the Confluence connector in Phase 1 (see FR `cpt-insightspec-fr-conf-email-resolution` deferral). SCD Type 2 versioning is therefore delivered by whichever Silver model assembles the canonical person timeline (from `jira_user`, HR, and any future connector-level Confluence user stream). Priority reduced from `p1` to `p2` per DESIGN §1.1 and §3.2 (driver row 5). Restore to `p1` alongside a dedicated `wiki_users` stream when connector-level user emission is required.

The `wiki_users` table **MUST** preserve historical state changes using the SCD Type 2 pattern. When the connector detects a change in a user's attributes (email, display name, active status) between the current resolution and the most recent stored record, it **MUST** close the previous record and insert a new record with updated state.

The Airbyte sync mode for `wiki_users` **MUST** be **Full Refresh | Append** (not overwrite). SCD Type 2 versioning (`valid_from`/`valid_to` or equivalent) **MUST** be applied at the destination layer via merge logic (e.g., ClickHouse ReplacingMergeTree with `_version` column, or destination-level MERGE statement). The implementation approach is deferred to DESIGN; note that Declarative YAML does not natively support stateful change detection, so destination-level MERGE is the expected path.

**Note on Bronze schema**: The current `wiki_users` Bronze schema in `README.md` does not define `valid_from`/`valid_to` columns. SCD Type 2 is implemented at the destination/Silver level, not as explicit Bronze columns emitted by the connector. If a future `wiki_users` stream is added, it would emit the current-state snapshot and rely on the framework-managed version column (`_airbyte_extracted_at`, not a custom `_version`) for deduplication; the destination merge logic derives `valid_from`/`valid_to` from successive snapshots.

**Rationale**: The User API returns current state only. Without SCD Type 2, an email change or account deactivation silently overwrites history, breaking historical identity resolution.

**Actors**: `cpt-insightspec-actor-conf-analyst`, `cpt-insightspec-actor-identity-manager`

### 5.4 Connector Operations

#### Track Collection Runs

- [ ] `p2` - **ID**: `cpt-insightspec-fr-conf-collection-runs`

The connector **MUST** produce a collection run log entry for each execution in the `confluence_collection_runs` table (connector-specific, not part of the unified `wiki_*` schema), recording: run ID, start/end time, status, per-stream record counts (spaces, pages, versions/edits, analytics/views, users), API call count, error count, per-page error count (skipped pages), unresolved email count, analytics availability flag, spaces visible count, and collection settings (domain, space filter, analytics enabled, incremental cursor position).

**Note**: `confluence_collection_runs` is deliberately a connector-specific table (not the generic `wiki_collection_runs` from the unified schema) because it includes Confluence-specific fields (`analytics_records_collected`, `analytics_available`, `versions_collected`) that do not apply to Outline. The unified `wiki_collection_runs` schema in `README.md` is a generic template; Confluence uses its own table with richer instrumentation.

**Rationale**: Operational visibility into connector health. The analytics availability flag is critical for operators to understand whether view data is being collected (Premium) or skipped (Standard). `unresolved_email_count` surfaces identity resolution degradation. `spaces_visible_count` helps operators verify permission coverage.

**Actors**: `cpt-insightspec-actor-conf-operator`

### 5.5 Data Integrity

#### Deduplicate by Primary Key

- [ ] `p1` - **ID**: `cpt-insightspec-fr-conf-deduplication`

Each stream **MUST** define a primary key that ensures re-running the connector does not produce duplicate records.

The connector **MUST** generate URN-based surrogate primary keys for entity records:
- Pages: `urn:confluence:{tenant_id}:{insight_source_id}:{page_id}`
- Spaces: `urn:confluence:{tenant_id}:{insight_source_id}:{space_id}`

Activity records use `(insight_source_id, page_id, user_id, date, data_source)` as the composite dedup key.

The Airbyte sync mode for `wiki_pages` and `wiki_spaces` **MUST** be **Incremental | Append + Deduped** (upsert semantics). The `wiki_page_activity` stream **MUST** use **Incremental | Append + Deduped** with the composite key. The `wiki_users` stream **MUST** use **Full Refresh | Append** with SCD Type 2 handling.

Every record **MUST** be deduplicated deterministically at the destination: later records supersede earlier ones for the same `unique_key`. In Phase 1 this is satisfied by Airbyte's destination framework, which auto-generates `_airbyte_extracted_at` (DateTime64, per-write timestamp) on every row and creates the table as `ReplacingMergeTree(_airbyte_extracted_at) ORDER BY unique_key`. No custom `_version` field is emitted by the manifest — project-wide convention shared with other no-code connectors (jira, zoom). See DESIGN §3.7.

**Rationale**: Without upsert semantics, overlapping incremental windows produce duplicate rows. URN keys provide unambiguous cross-instance identity. The framework-managed `_airbyte_extracted_at` ensures deterministic merge resolution at the destination layer.

**Actors**: `cpt-insightspec-actor-conf-api`

#### Support Incremental Collection

- [ ] `p1` - **ID**: `cpt-insightspec-fr-conf-incremental-sync`

The connector **MUST** support incremental collection using `sort=-modified-date` with client-side cursor filtering (`is_client_side_incremental: true`). The cursor value is `max(version.createdAt)` from the previous successful run. The connector fetches all pages sorted by descending modification date and filters client-side against the stored cursor. Version history collection is scoped by the parent page set returned by the incremental query. **Note**: The v2 `/pages` endpoint does NOT support a `lastModifiedAfter` query parameter.

Spaces are collected as full refresh on every run (small cardinality).

**Rationale**: Full page scans on large Confluence instances (10,000+ pages) are expensive. Client-side incremental cursor with descending sort order minimizes the number of pages processed by encountering fresh records first. On large instances, this is less efficient than server-side filtering but is the only option given the v2 API constraints.

**Actors**: `cpt-insightspec-actor-conf-operator`

#### Stamp Instance and Tenant Context

- [ ] `p1` - **ID**: `cpt-insightspec-fr-conf-instance-context`

Every record emitted by the connector **MUST** include `tenant_id` (identifying the Insight tenant), `insight_source_id` (identifying the specific Confluence instance), and `data_source` (set to `insight_confluence` for all records). All three fields are injected by the connector manifest via `AddFields` transformations using values from the connector configuration (`insight_tenant_id`, `insight_source_id` config parameters).

**Note on Bronze schema**: The connector manifest injects `tenant_id`, `source_id`, and `data_source` onto every record via `AddFields`. URN-based primary keys (`unique_key`) incorporate `{tenant_id}-{source_id}-{natural_key}`, ensuring cross-instance uniqueness. See DESIGN §3.3 for the injection pattern.

**Rationale**: Multiple Confluence instances may feed into the same Bronze store. The `data_source` field enables the Silver pipeline to distinguish Confluence-originated records from Outline-originated records in the unified `wiki_*` schema.

**Actors**: `cpt-insightspec-actor-conf-operator`

#### Normalize Timestamps to UTC

- [ ] `p1` - **ID**: `cpt-insightspec-fr-conf-utc-timestamps`

All timestamps persisted in the Bronze layer **MUST** be stored in UTC. Confluence API returns ISO 8601 timestamps; the connector **MUST** normalize any timezone offsets. Activity dates **MUST** be bucketed to calendar day in UTC.

**Rationale**: Consistent UTC normalization prevents timezone-related errors in cross-platform analytics.

**Actors**: `cpt-insightspec-actor-conf-analyst`

### 5.6 Identity Resolution

#### Expose Identity Key

- [ ] `p1` - **ID**: `cpt-insightspec-fr-conf-identity-key`

All user-attributed streams **MUST** include both `user_id` (Atlassian `accountId`) and `email` (resolved via User API). The Identity Manager resolves `email` to canonical `person_id` in Silver step 2.

When email is unavailable (deactivated accounts with redacted email), the connector **MUST** emit the record with `email = null` and a valid `accountId`. The Identity Manager stores the `accountId` as an isolated node — if the email later becomes resolvable, the node is retroactively merged.

**Rationale**: `accountId` is shared across the Atlassian platform (Confluence, Jira, Bitbucket) but is opaque and not suitable for cross-vendor identity resolution. Email is the canonical cross-system key.

**Actors**: `cpt-insightspec-actor-identity-manager`

## 6. Non-Functional Requirements

### 6.1 NFR Inclusions

#### Data Freshness

- [ ] `p1` - **ID**: `cpt-insightspec-nfr-conf-freshness`

The connector **MUST** deliver extracted data to the Bronze layer within 24 hours of the connector's scheduled run.

**Threshold**: Data available in Bronze ≤ 24h after scheduled collection time.

**Rationale**: Timely page and edit data enables near-real-time documentation health dashboards.

#### Extraction Completeness

- [ ] `p1` - **ID**: `cpt-insightspec-nfr-conf-completeness`

The connector **MUST** extract all pages and versions matching the configured space scope and date range on each successful run. Failed or partial runs must be detectable and retryable without data loss.

**Rationale**: Incomplete extraction leads to understated editorial velocity and unreliable documentation health metrics.

### 6.2 NFR Exclusions

- **Real-time streaming latency**: Not applicable — this connector operates in batch mode with daily collection.
- **Throughput / high-volume optimization**: The analytics endpoint requires per-page iteration; rate limit handling is covered in Section 3.1.
- **Availability**: Batch connector — availability is determined by the orchestrator's scheduling.

## 7. Public Library Interfaces

### 7.1 Public API Surface

#### Confluence Stream Contract

- [ ] `p1` - **ID**: `cpt-insightspec-interface-conf-streams`

**Type**: Data format (Bronze table schemas)

**Stability**: stable

**Description**: Three Bronze streams in Phase 1 — `wiki_spaces`, `wiki_pages`, `wiki_page_versions` (all with `data_source = 'insight_confluence'`). Pages use client-side incremental cursor via `sort=-modified-date` with `is_client_side_incremental`. Spaces are full-refresh. Page versions are a substream scoped by the incremental parent page set. Additional streams deferred to Phase 2: `wiki_page_activity` (edit + view aggregation, depends on Silver/dbt), `wiki_users` (email resolution, see FR `cpt-insightspec-fr-conf-email-resolution`), and `confluence_collection_runs` (operational log, see FR `cpt-insightspec-fr-conf-collection-runs`).

**Field-level schemas**: Defined in [`confluence.md`](../confluence.md) (field-level mapping from Confluence API to Bronze columns) and [`README.md`](../README.md) (unified wiki schema).

**Breaking Change Policy**: Adding new fields is non-breaking. Removing or renaming fields requires a migration.

### 7.2 External Integration Contracts

#### Confluence REST API

- [ ] `p1` - **ID**: `cpt-insightspec-contract-conf-rest-api`

**Direction**: required from external system

**Protocol/Format**: REST / JSON

| Stream | Endpoint | API Version | Method |
|--------|----------|-------------|--------|
| `wiki_spaces` | `GET /spaces` | v2 | Full refresh |
| `wiki_pages` | `GET /pages?sort=-modified-date&status=current,archived,trashed` | v2 | Incremental (client-side cursor) |
| `wiki_page_activity` (edits) | `GET /pages/{id}/versions` | v2 | Child of pages — incremental |
| `wiki_page_activity` (views) | `GET /analytics/content/{id}/viewers` | v2 | Child of pages — Premium only; paginated (100/page) |
| `wiki_pages` (view_count) | `GET /analytics/content/{id}` | v2 | Summary: aggregate `viewCount` — Premium only |
| `wiki_users` | `GET /rest/api/user/bulk?accountId=...` | **v1** | Batch resolution (up to 200 IDs) |
| Tier detection | `GET /analytics/content/{id}/viewers` (first call) | v2 | 200 = Premium; 403/404 = Standard |

**Authentication**: Basic Auth (email + Atlassian API token) or OAuth 2.0 (3-legged flow)

**Compatibility**: Confluence REST API v2 (Cloud) for spaces/pages/versions. REST API v1 for user lookup (no v2 equivalent). Cursor-based pagination via `_links.next`. Field additions are non-breaking.

## 8. Use Cases

### UC-001 Configure Confluence Connection

- [ ] `p1` - **ID**: `cpt-insightspec-usecase-conf-configure`

**Actor**: `cpt-insightspec-actor-conf-operator`

**Preconditions**:

- Confluence Cloud instance accessible
- API token generated at `id.atlassian.com` (Basic Auth) or OAuth 2.0 app configured with required scopes

**Main Flow**:

1. Operator selects authentication method (Basic Auth or OAuth 2.0)
2. For Basic Auth: operator provides email address and API token
3. For OAuth: operator completes the 3-legged OAuth flow
4. System validates credentials against the Confluence API (`GET /spaces?limit=1`)
5. System attempts analytics endpoint (`GET /analytics/content/{any_page}/viewers`) to detect Confluence tier (Premium vs. Standard)
6. If Standard tier detected: system informs operator that view analytics will not be collected; edit activity will be collected
7. System lists available spaces; operator selects space scope (default: all non-personal spaces)
8. System initializes the connection with empty cursor state

**Postconditions**:

- Connection is ready for first sync run
- Confluence tier (Premium/Standard) recorded in connection metadata
- Analytics collection enabled/disabled based on tier detection

**Alternative Flows**:

- **Invalid credentials**: System reports authentication failure; operator corrects credentials
- **Insufficient OAuth scopes**: System reports which scopes are missing; operator updates the OAuth app
- **No spaces accessible**: API returns empty space list; operator verifies permissions
- **Analytics endpoint returns 403**: System classifies as Standard tier; analytics collection disabled with informational message (not an error)

### UC-002 Incremental Sync Run

- [ ] `p1` - **ID**: `cpt-insightspec-usecase-conf-incremental-sync`

**Actor**: `cpt-insightspec-actor-conf-operator`

**Preconditions**:

- Connection configured and credentials valid
- Previous state available (or empty for first run)

**Main Flow**:

1. Orchestrator triggers the connector with current state
2. Connector refreshes space directory from `GET /spaces` (full refresh)
3. Connector queries pages via `GET /pages?sort=-modified-date&status=current,archived,trashed` and filters client-side against the stored cursor (`is_client_side_incremental`)
4. For each modified page: fetch version history (`GET /pages/{id}/versions`) for versions created after cursor; produce edit activity rows
5. If analytics enabled (Premium tier): for each modified page, fetch per-user view data from `GET /analytics/content/{id}/viewers`; produce view activity rows
6. Collect all unique `accountId` values from pages and versions; batch-resolve to email via `GET /rest/api/user/bulk` (up to 200 IDs per request); apply SCD Type 2 versioning for user attribute changes
7. Backfill `author_email`, `last_editor_email`, `user_email` into page and activity records
8. Write all records with upsert semantics
9. Update per-space cursors to `max(version.createdAt)` from this run
10. Collection run log entry written (including `unresolved_email_count`, `spaces_visible_count`, per-page error count)

**Postconditions**:

- Bronze tables contain new and updated records
- Per-space cursors updated with latest modification timestamps
- Collection run log records success/failure, per-stream counts, analytics availability, and error counts

**Alternative Flows**:

- **First run**: Connector extracts all pages across all spaces in scope (full initial load); per-space cursors initialized from max timestamp per space
- **New space added to scope**: Cursor for that space starts at epoch zero; full initial load for the new space only
- **API throttling (HTTP 429 or 503)**: Connector respects `Retry-After` header and retries with exponential backoff
- **Per-page API failure (versions or analytics)**: Connector retries up to 3 times with backoff. If a single page consistently fails, the connector **MUST** skip that page, log the error, increment the per-page error counter, and continue. A run **MUST NOT** fail due to a single page error
- **Analytics endpoint fails mid-run**: If first analytics call returns 403 (tier change or permission revoked), connector disables analytics for remainder of run and logs the change
- **User API returns null email**: Connector emits user record with `email = null`; increments `unresolved_email_count`

## 9. Acceptance Criteria

- [ ] Spaces extracted from a live Confluence Cloud instance with type, status, and URL
- [ ] Pages extracted with metadata, version number, parent hierarchy, author, and last editor; trashed pages included (`status=current,archived,trashed`)
- [ ] Edit activity extracted from version history as per-user per-page per-day counts
- [ ] View analytics extracted on Premium tier instances (with paginated viewer endpoint); gracefully skipped on Standard tier with null view fields
- [ ] `wiki_pages.view_count` and `distinct_viewers` populated from analytics summary endpoint (`GET /analytics/content/{id}`); null on Standard tier
- [ ] User directory populated via `accountId` → email resolution from User API bulk endpoint; managed/SCIM accounts with null email handled gracefully
- [ ] User directory preserves historical state with SCD Type 2 (destination-level merge via framework-managed version column) — **deferred, see FR `cpt-insightspec-fr-conf-user-scd` Phase 1 note**
- [ ] Per-space incremental cursors: second run extracts only modified pages per space; adding a new space triggers full load for that space only
- [ ] `email` resolved and backfilled into page and activity records; null when unavailable
- [ ] Every record carries a destination-level version column used for deduplication (framework-managed `_airbyte_extracted_at` in Phase 1; see DESIGN §3.7)
- [ ] URN-based surrogate primary keys on entity streams
- [ ] `tenant_id`, `insight_source_id`, and `data_source = 'insight_confluence'` injected by connector manifest via `AddFields`
- [ ] All timestamps stored in UTC
- [ ] Collection run log (`confluence_collection_runs`) records success, per-stream counts, API call count, analytics availability, unresolved email count, spaces visible count, and per-page error count
- [ ] API throttling (HTTP 429 and 503) handled with exponential backoff
- [ ] Per-page API failures retried 3 times then skipped without failing the run

## 10. Dependencies

| Dependency | Description | Criticality |
|------------|-------------|-------------|
| Confluence REST API v2 (Cloud) | Spaces, pages, versions, and analytics endpoints | `p1` |
| Confluence REST API v1 | User profile bulk lookup (`/rest/api/user/bulk`) — no v2 equivalent | `p1` |
| Atlassian credentials | API token (Basic Auth) or OAuth 2.0 tokens with required scopes | `p1` |
| Airbyte Connector framework | Execution model for running the connector. Declarative YAML for standard streams; email resolution and activity row expansion may require Python components | `p1` |
| Identity Manager | Resolves `email` to `person_id` in Silver step 2 | `p2` |
| Destination store (PostgreSQL / ClickHouse) | Target for Bronze tables | `p1` |

## 11. Assumptions

- The Confluence instance is Cloud-hosted; Confluence Server/Data Center is not supported
- Basic Auth uses email + Atlassian API token (not password); tokens are generated at `id.atlassian.com`
- Confluence REST API v2 is the primary API for spaces, pages, and versions; v1 is used only for user lookup
- The `/rest/api/user/bulk` endpoint (v1) accepts up to 200 `accountId` values per request and returns email when available; deactivated accounts may have `email = null`
- The analytics endpoint (`/analytics/content/{id}/viewers`) is Premium-only; Standard tier returns 403 or 404
- The analytics endpoint reports "user X viewed page Y at time Z" — not a count of views. `view_count = 1` means "at least one view by this user on this date"
- ~~`lastModifiedAfter` on the `/pages` endpoint filters by the latest version timestamp; new pages appear in results because their `createdAt` qualifies~~ **Corrected in DESIGN**: `lastModifiedAfter` does NOT exist on the v2 `/pages` endpoint; client-side incremental cursor is used instead
- `GET /wiki/api/v2/spaces` returns `createdAt` (ISO 8601 UTC timestamp) — **corrected**: earlier assumption that `createdAt` was not available on the v2 spaces endpoint was incorrect (confirmed via live API testing against `darthvolt.atlassian.net`)
- Spaces and users are small-cardinality entities collected as full refresh on every run
- Pages are collected incrementally using client-side cursor filtering via `sort=-modified-date` with `is_client_side_incremental: true` (the v2 API has no `lastModifiedAfter` parameter)
- Atlassian enforces rate limits via a token-bucket model with varying budgets per endpoint category (content, analytics, user API). Both HTTP 429 and HTTP 503 are used for rate limiting. Analytics endpoints may have stricter limits than content endpoints
- The `accountId` is shared across Confluence, Jira, and Bitbucket within the Atlassian platform — the same `accountId` in Confluence and Jira refers to the same person
- Email resolution from `accountId` requires the `read:confluence-user` OAuth scope; without it, the User API returns 403. Both deactivated accounts and active managed accounts (SCIM/Atlassian Guard provisioned) may return `email = null` — this is not limited to inactive accounts
- The `/rest/api/user/bulk` endpoint (v1) is accessible with `read:confluence-user` scope; `manage:confluence-user` is not required. If this assumption is incorrect, the DESIGN must specify the minimum required scope
- The Airbyte Declarative Connector framework (YAML) can handle standard pagination and incremental sync but email resolution (batch lookup + backfill) and version-to-activity row expansion may require Python CDK components
- The unified wiki schema (`wiki_*` tables) is shared with Outline; `data_source = 'insight_confluence'` discriminates Confluence records
- The `wiki_*` Bronze schemas in `README.md` do not define `tenant_id` or SCD Type 2 columns (`valid_from`/`valid_to`). `tenant_id` is injected by the manifest via `AddFields`; SCD Type 2 is implemented at the destination/Silver layer via merge logic against the framework-managed version column (`_airbyte_extracted_at`)
- `confluence_collection_runs` is a connector-specific table (not `wiki_collection_runs` from the unified schema) because it includes Confluence-specific instrumentation fields
- `GET /spaces` returns only spaces visible to the authenticated API token user. Spaces restricted by space permissions are silently excluded — no error is returned for invisible spaces
- ~~The `/pages` endpoint requires `?expand=version` to include `version.authorId` and `version.createdAt` in the response. Without this expansion parameter, version metadata fields are absent from the response with no error~~ **Corrected in DESIGN**: v2 returns the `version` object by default; no `expand` parameter needed (confirmed via live API testing)
- The `/analytics/content/{id}/viewers` endpoint is paginated (default: 100 per response) and returns only the last `viewedAt` timestamp per user per page — not a per-view event log. Per-day view granularity is not available from this endpoint
- `wiki_pages.view_count` is sourced from the analytics summary endpoint (`GET /analytics/content/{id}`) which returns aggregate `viewCount`, not from the per-user viewers endpoint
- Trashed pages are included in the page query (`status=current,archived,trashed`) to enable soft-delete detection. Permanently purged pages are invisible to all API endpoints
- Blog posts are a separate content type in Confluence with a separate API endpoint (`GET /blogposts`); they are excluded from this connector's scope, which means editorial velocity metrics undercount for teams that use blog posts as a primary content format

## 12. Risks

| Risk | Impact | Mitigation |
|------|--------|------------|
| Analytics endpoint unavailable (Standard tier) | No per-user view data; documentation consumption analytics limited to edit activity only | Graceful degradation: detect tier on first call, skip analytics, set view fields to null — see FR `cpt-insightspec-fr-conf-view-analytics`. **Update (live API testing)**: the aggregate analytics summary endpoint (`GET /wiki/rest/api/analytics/content/{id}/viewers`) returned HTTP 200 with `{"id":720897,"count":0}` on a Free-plan instance (`darthvolt.atlassian.net`). This suggests the aggregate viewers count may work on all tiers while the per-user viewers list may still be Premium-only. Tier detection logic should distinguish between aggregate summary and per-user detail endpoints — requires further investigation before Phase 2 implementation |
| Analytics endpoint requires per-page iteration | API call volume scales with page count; large instances (10,000+ pages) may consume significant quota on analytics calls alone | Limit analytics collection to pages modified since cursor (incremental); monitor API call counts; consider sampling strategy for very large instances |
| Email redacted for deactivated accounts | Identity resolution fails for departed users; historical attribution gaps | Emit `accountId` as fallback; support deferred merge when email becomes available; log unresolvable accounts in collection run |
| v1 User API deprecation | Atlassian may deprecate v1 endpoints; no v2 equivalent for user lookup exists today | Monitor Atlassian deprecation announcements; if v2 user endpoint is released, migrate. The `accountId` is stable and can be used for Atlassian-ecosystem joins (Jira, Bitbucket) as a fallback |
| Rate limit exhaustion on large instances | Per-user rate limit (10 req/s) shared across all API calls; version history + analytics fan-out per page may saturate the limit | Implement exponential backoff; prioritize page and version collection over analytics; log throttle events in collection run |
| Client-side cursor edge cases | Pages modified at exactly the cursor timestamp may be skipped or double-counted | Client-side cursor uses `>=` comparison; accept minimal overlap as safe (upsert handles duplicates) |
| Personal spaces pollute analytics | Personal spaces (drafts, scratch pages) may skew editorial velocity and consumption metrics | Default space scope excludes personal spaces; operator can override |
| Confluence API v2 field additions | New fields in v2 responses may change JSON structure | Field additions are non-breaking; the connector extracts only documented fields |
| Email reuse on deactivated accounts | Departed user's email reassigned to new hire; two `wiki_users` records share the same email | SCD Type 2 with temporal bounds enables disambiguation — see FR `cpt-insightspec-fr-conf-user-scd` |
| Version history fan-out (N+1) | Each page requires a separate `/versions` call; pages with hundreds of versions produce many API calls. First run on 10K pages × 50 versions ≈ 30K+ API calls for versions alone | Scope version collection to versions after cursor; paginate with reasonable page size. For first run: consider phased rollout (start with a few spaces, expand). DESIGN should specify a version depth cap for initial loads (e.g., last 90 days) |
| Permanently purged pages invisible | Pages purged from Confluence trash disappear from all API responses; incremental cursor never sees them; Bronze retains stale `current` status | Include `status=trashed` in page query to catch soft-deletes; periodic full-refresh reconciliation (weekly) to detect purged pages by comparing Bronze IDs against current full list |
| Managed/SCIM accounts with null email | Active accounts provisioned via Atlassian Access/Guard may return `email = null` even though the user is active; identity resolution fails silently | Log `unresolved_email_count` in collection run; surface in operator dashboard; use `accountId` for Atlassian-ecosystem joins (Jira, Bitbucket) as fallback |
| Blog posts excluded from editorial velocity | Blog posts are a first-class content type in Confluence used for announcements, retrospectives, and knowledge sharing. Excluding them understates editorial velocity for teams that use blogs heavily | Documented as out of scope for v1.0. Blog post extraction via `GET /blogposts` (same schema as pages) is a natural v1.1 extension |
| Standard-tier: zero consumption signals | Without Premium analytics AND without comments, Standard-tier instances provide only edit counts — no view data, no engagement signals. The "knowledge consumption" goal (Section 1.3) is unachievable for Standard-tier customers | Document limitation clearly. Consider promoting comment count extraction to P2 as a tier-independent consumption signal (comments are available on all tiers via `GET /pages/{id}/footer-comments`) |
| Space visibility limited to API token permissions | `GET /spaces` returns only spaces the authenticated user can browse; restricted spaces are silently excluded with no error indication | Display `spaces_visible_count` in UC-001 and collection run log so operators can verify coverage. Document that the API token user must have Browse permission on all target spaces |
| Analytics viewers endpoint: lossy `viewedAt` | The per-user viewers endpoint returns only the last view date per user per page, not a per-view event log. Gold metrics like "views per page per week" will undercount because intermediate view dates are lost | Document as a data quality caveat. `wiki_pages.view_count` from the summary endpoint provides accurate aggregate counts; per-user per-day breakdown is approximate |
| ~~`expand=version` required on /pages~~ | ~~Without `?expand=version` in the pages query, `version.authorId` and `version.createdAt` are absent from the response~~ | **Resolved**: v2 returns the `version` object by default; no `expand` parameter needed (confirmed via live API testing) |

## 13. Resolved Questions

All open questions from the connector specification (`confluence.md`) have been resolved and incorporated into the PRD as concrete requirements:

| ID | Summary | Resolution | Incorporated In |
|----|---------|------------|-----------------|
| OQ-CONF-1 | Email resolution from `accountId` | Batch resolve via `GET /rest/api/user/bulk` (v1 API, up to 200 IDs per request). Deactivated accounts may return `email = null` — the connector handles this gracefully by emitting the record with `accountId` only and `email = null`. The Identity Manager stores unresolvable accounts as isolated nodes with deferred merge capability. | FR `cpt-insightspec-fr-conf-email-resolution` |
| OQ-CONF-2 | Analytics endpoint tier and rate limiting | Detect tier on first analytics call: 200 = Premium (collect views), 403/404 = Standard (skip all analytics). Do not retry failed analytics calls. Log `analytics_available` flag in collection run. Rate limit budget: analytics calls share the per-user limit with other calls; incremental collection (only modified pages) limits the blast radius. | FR `cpt-insightspec-fr-conf-view-analytics` |
| OQ-CONF-3 | Incremental page sync strategy | Store `max(version.createdAt)` per run as the cursor. Use `sort=-modified-date` with `is_client_side_incremental: true` on the `/pages` endpoint (the v2 API has no `lastModifiedAfter` parameter). New pages appear in incremental results because `createdAt` qualifies as a modification timestamp. Version history collection is scoped by the parent page set. | FR `cpt-insightspec-fr-conf-page-extraction`, FR `cpt-insightspec-fr-conf-incremental-sync` |

## 14. Non-Applicable Requirements

The following checklist domains have been evaluated and determined not applicable for this connector:

| Domain | Reason |
|--------|--------|
| **Security (SEC)** | The connector handles an API token or OAuth 2.0 tokens, stored as `airbyte_secret` by the Airbyte framework. No custom authentication, authorization, or encryption logic exists in the connector. Credential storage and secret management are delegated to the Airbyte platform. |
| **Safety (SAFE)** | Pure data extraction pipeline. No interaction with physical systems, no potential for harm to people, property, or environment. |
| **Performance (PERF)** | Batch connector with cursor-based pagination. Rate limit handling (HTTP 429 with exponential backoff) is the primary performance concern, covered in Section 3.1. Analytics fan-out is managed by incremental scoping. |
| **Reliability (REL)** | Idempotent extraction via deduplication keys. No distributed state, no transactions. Recovery is handled by re-running the sync (Airbyte framework manages state). |
| **Usability (UX)** | No user-facing interface. Configuration is a credential form and space scope selection in the Airbyte UI. No accessibility, internationalization, or inclusivity requirements apply. |
| **Compliance (COMPL)** | User emails are personal data under GDPR. Page content is NOT extracted — only metadata and aggregate activity counts. Retention, deletion, and data subject rights are delegated to the Airbyte platform and destination operator. |
| **Maintainability (MAINT)** | Initial implementation uses Declarative YAML for standard streams. Email resolution (batch User API lookup + backfill) and version-to-activity row expansion may require Python CDK components. The unified wiki schema is shared with Outline — schema changes affect both connectors. |
| **Testing (TEST)** | Connector behavior must satisfy PRD acceptance criteria (Section 9). Validation includes: Airbyte framework connection check, schema validation, and connector-specific acceptance tests (Premium tier detection, graceful degradation, email resolution). |
