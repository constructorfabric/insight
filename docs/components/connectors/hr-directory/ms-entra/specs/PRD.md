# PRD — MS Entra Connector

> Version 1.0 — May 2026
> Based on: HR Directory domain (`docs/components/connectors/hr-directory/README.md`)

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
  - [5.1 Data Collection](#51-data-collection)
  - [5.2 Data Integrity](#52-data-integrity)
  - [5.3 Privacy](#53-privacy)
- [6. Non-Functional Requirements](#6-non-functional-requirements)
  - [6.1 NFR Inclusions](#61-nfr-inclusions)
  - [6.2 NFR Exclusions](#62-nfr-exclusions)
- [7. Public Library Interfaces](#7-public-library-interfaces)
  - [7.1 Public API Surface](#71-public-api-surface)
  - [7.2 External Integration Contracts](#72-external-integration-contracts)
- [8. Use Cases](#8-use-cases)
- [9. Acceptance Criteria](#9-acceptance-criteria)
- [10. Dependencies](#10-dependencies)
- [11. Assumptions](#11-assumptions)
- [12. Risks](#12-risks)
- [13. Open Questions](#13-open-questions)
  - [OQ-MSENTRA-1: Delta-token incremental sync](#oq-msentra-1-delta-token-incremental-sync)
  - [OQ-MSENTRA-2: Group memberships and manager relationships](#oq-msentra-2-group-memberships-and-manager-relationships)
  - [OQ-MSENTRA-3: Multi-tenant Entra accounts](#oq-msentra-3-multi-tenant-entra-accounts)
  - [OQ-MSENTRA-4: Guest user filtering](#oq-msentra-4-guest-user-filtering)

<!-- /toc -->

---

## 1. Overview

### 1.1 Purpose

The MS Entra connector extracts the user directory from Microsoft Entra ID (formerly Azure AD) via the Microsoft Graph REST API into the Insight platform's Bronze layer. The data feeds the Identity Manager so users authenticated against Entra in any product can be resolved to their accounts in other services (GitHub, Slack, Jira, BambooHR), and contributes to the canonical person registry alongside HR directory connectors.

### 1.2 Background / Problem Statement

Microsoft Entra ID is the cloud identity provider for Microsoft-365-based organisations. When a workforce signs into the Insight platform via SSO, the JWT carries the Entra Object ID (`oid` claim) — but `oid` alone tells nothing about who the person is in GitHub, Slack, Jira, or HR. Without the directory data, Insight cannot:

1. **Resolve the authenticated user to a canonical person** — link `oid` to `mail`, `proxyAddresses`, `employeeId`, `onPremisesSamAccountName`, the signals other connectors carry.
2. **Build the org graph for non-HRIS deployments** — for organisations that use Entra as the source of truth instead of (or alongside) BambooHR / Workday, manager and department fields ride directly on the user object.
3. **Detect identity lifecycle events** — `accountEnabled` flipping to `false` is the authoritative termination signal in Microsoft-only shops.

The Microsoft Graph user object also exposes a long tail of personal fields (`birthday`, `aboutMe`, `interests`, `mobilePhone`, `streetAddress`, `schools`) that have no analytics value and would create privacy risk if collected. The connector must extract enough for identity resolution and explicitly refuse the rest.

### 1.3 Goals (Business Outcomes)

1. Enable identity resolution for every Insight workspace authenticated via Entra ID — the JWT `oid` claim resolves to a `person_id` for cross-service attribution.
2. Provide a directory feed for organisations whose source-of-truth for org structure is Entra (not BambooHR/Workday).
3. Surface identity lifecycle events (`accountEnabled = false`, account deletion) so Insight can mark `class_people.status = 'terminated'` in near real time.
4. Collect only the directory fields needed for identity resolution — minimise privacy exposure even when the granted permission (`User.Read.All`) would allow the full profile.

### 1.4 Glossary

| Term | Definition |
|------|-----------|
| **Entra Object ID (`oid`)** | The immutable, application-independent identifier for a user object in Entra ID. Equals the JWT `oid` claim and the `id` field on the Microsoft Graph user resource. The cross-service join key for identity resolution. |
| **JWT `sub` claim** | A pairwise pseudonymous subject — unique per (user, application) within a tenant. **Not** used as a cross-service identifier; `oid` is. |
| **App Registration** | An Entra application identity (client_id + client_secret) used for OAuth2 client credentials flow. The connector requires its own dedicated App Registration. |
| **Application permission** | An Entra permission granted to an app for app-only access (no signed-in user). Distinct from delegated permissions. The connector requires `User.Read.All` of type Application. |
| **Admin consent** | A tenant administrator's grant that activates an application permission. Without admin consent, the role is requested but inactive — the access token does not carry the role. |
| **`$select` allowlist** | A Microsoft Graph query parameter that restricts the response to a fixed list of fields. The connector uses it as a privacy control: even though `User.Read.All` permits the full profile, the connector requests only the identity-resolution subset. |
| **Identity key** | The field used for cross-system person resolution. For MS Entra: `id` (= Entra Object ID = JWT `oid`). |

---

## 2. Actors

### 2.1 Human Actors

#### Platform Engineer

**ID**: `cpt-insightspec-actor-msentra-platform-engineer`

Configures Entra connections (creates the App Registration, grants `User.Read.All` admin consent, supplies `azure_tenant_id` / `azure_client_id` / `azure_client_secret` to the K8s Secret), monitors collection runs, and troubleshoots extraction failures.

**Needs**: A reliable way to provision the connector for a new Entra tenant; clear errors when admin consent is missing or credentials are wrong; visibility into collection runs.

#### Data Analyst

**ID**: `cpt-insightspec-actor-msentra-data-analyst`

Consumes Entra Bronze data through the Silver/Gold layers for org-aware metric scoping, headcount reporting, and identity resolution audits.

**Needs**: Stable canonical IDs that survive UPN/email changes; clear lineage from a `class_people` row back to the Bronze record.

### 2.2 System Actors

#### Orchestrator

**ID**: `cpt-insightspec-actor-msentra-orchestrator`

Triggers MS Entra connector runs on schedule and routes the output to the destination.

#### Identity Manager

**ID**: `cpt-insightspec-actor-msentra-identity-manager`

Consumes identity signals emitted from `users` (`mail`, `userPrincipalName`, `proxyAddresses`, `employeeId`, `onPremisesSamAccountName`) to maintain the canonical `oid → person_id` mapping used by all Silver streams.

#### Destination (ClickHouse)

**ID**: `cpt-insightspec-actor-msentra-destination`

Receives extracted records and writes them to Bronze tables.

---

## 3. Operational Concept & Environment

### 3.1 Module-Specific Environment Constraints

- The connector requires a dedicated Entra App Registration with an Application-type permission scoped to read all users, and tenant admin consent granted. The exact Graph permission name and consent procedure live in [DESIGN §3.3](./DESIGN.md#33-api-contracts).
- Authentication is app-only (no signed-in user). Delegated user auth is out of scope.
- All traffic is HTTPS to Microsoft Graph and the Microsoft identity platform. Endpoint URLs and the OAuth flow are specified in [DESIGN §3.3](./DESIGN.md#33-api-contracts).
- The source API rate-limits requests per tenant / per app / per service. The connector must respect server-supplied retry hints and back off on transient overload responses.

---

## 4. Scope

### 4.1 In Scope

- Extraction of the user directory limited to an explicit allowlist of identity-resolution fields (the technical allowlist lives in [DESIGN §3.3](./DESIGN.md#33-api-contracts) and is mirrored by the manifest).
- OAuth2 client-credentials authentication with token caching.
- Cursor-based pagination so that tenants of any practical size are extracted in a single sync.
- Full-refresh sync mode.
- Retry on transient API failures with backoff and respect for server-supplied retry hints.
- `tenant_id`, `source_id`, and composite `unique_key` injection on every record (platform invariant for tenant isolation and idempotent writes).
- Multi-instance support — multiple Entra tenants synced in parallel via separate K8s Secrets, each with its own `source-id` annotation.

### 4.2 Out of Scope

- Silver/Gold layer transformations (handled by the dbt pipeline).
- Identity resolution logic (handled by the Identity Manager).
- Group memberships and manager relationships — deferred to a follow-up iteration.
- Delta / change-tracking incremental sync — deferred (rationale and constraint in [DESIGN §2.2](./DESIGN.md#22-constraints)).
- Personal-life and biographic profile attributes (date of birth, free-form biography, hobbies, schools, phone numbers, physical addresses, social-identity providers, age-group / consent metadata, etc.) — explicitly excluded for privacy. The full exclusion list and the audit-point mechanism live in [DESIGN §4 Source-Specific Considerations](./DESIGN.md#source-specific-considerations).
- Mailbox content, calendar events, file content, chat / channel messages — require separate, distinct API permissions that are not granted to this App Registration.
- Last-sign-in telemetry — requires an additional permission and licence tier; deferred.
- Delegated user authentication (interactive sign-in flow) — out of scope.
- Write operations (user creation, update, deletion).

---

## 5. Functional Requirements

> Technical contract for every requirement below — endpoint URLs, exact response shapes, query parameters, and error codes — lives in [DESIGN §3.3 API Contracts](./DESIGN.md#33-api-contracts) and [DESIGN §3.7 Database schemas & tables](./DESIGN.md#37-database-schemas--tables). The PRD captures **what** must hold; the DESIGN document captures **how**.

### 5.1 Data Collection

#### User Directory Collection

- [ ] `p1` - **ID**: `cpt-insightspec-fr-msentra-collect-users`

The system **MUST** extract every user record from Microsoft Entra ID, capturing only the categories of attributes with clear identity-resolution value: stable identifier, principal name and email aliases, display attributes, org context (employee number, department, job title), account state, and account-creation provenance.

**Rationale**: User directory data is the foundation for resolving the SSO subject (the Entra Object ID carried in the JWT `oid` claim) to a canonical person and for joining Entra-authenticated users to records in other source systems (GitHub, Slack, Jira, BambooHR).

**Actors**: `cpt-insightspec-actor-msentra-orchestrator`, `cpt-insightspec-actor-msentra-destination`

#### Pagination

- [ ] `p1` - **ID**: `cpt-insightspec-fr-msentra-pagination`

The system **MUST** follow the source API's pagination cursor until exhausted, so that tenants larger than the per-page maximum are fully extracted in a single sync.

**Rationale**: Real tenants commonly exceed the per-page cap. Without correct cursor following, only the first page would be collected.

**Actors**: `cpt-insightspec-actor-msentra-orchestrator`, `cpt-insightspec-actor-msentra-destination`

### 5.2 Data Integrity

#### Deduplication

- [ ] `p1` - **ID**: `cpt-insightspec-fr-msentra-deduplication`

The system **MUST** define a primary key on every emitted record as the composite `unique_key = {tenant_id}-{source_id}-{natural_key}`, where the natural key is the Entra Object ID. The destination uses this key to deduplicate across collection runs.

**Rationale**: Composite keys with `tenant_id` and `source_id` prevent collisions between Insight tenants and between multiple Entra tenants synced into the same workspace.

**Actors**: `cpt-insightspec-actor-msentra-destination`

#### Identity Key

- [ ] `p1` - **ID**: `cpt-insightspec-fr-msentra-identity-key`

The system **MUST** collect the Entra Object ID for every user — the immutable, application-independent identifier that equals the JWT `oid` claim issued by the Microsoft identity platform. The connector **MUST NOT** use the JWT `sub` claim as a cross-service identity key — `sub` is pairwise pseudonymous per application and is not exposed by the directory API.

**Rationale**: A single canonical key for every Entra-authenticated person is the precondition for joining Insight identities across services. The choice of `oid` over `sub` follows Microsoft's own guidance and prevents cross-app identity fragmentation.

**Actors**: `cpt-insightspec-actor-msentra-identity-manager`

#### Sync Mode

- [ ] `p1` - **ID**: `cpt-insightspec-fr-msentra-sync-mode`

The connector **MUST** produce idempotent output across runs: re-running against the same source state yields the same Bronze rows after destination-side deduplication. Full-refresh is the operational sync mode for v1; change-tracking incremental sync is deferred (constraint and rationale in [DESIGN §2.2](./DESIGN.md#22-constraints)).

**Rationale**: Directory sizes are typically 1k–50k users; a daily full pull is operationally acceptable. Idempotency is a platform invariant; the destination engine and `unique_key` PK enforce it.

**Actors**: `cpt-insightspec-actor-msentra-orchestrator`

#### Fault Tolerance

- [ ] `p1` - **ID**: `cpt-insightspec-fr-msentra-fault-tolerance`

The system **MUST** retry on transient API failures with exponential backoff and **MUST** honour server-supplied retry hints. Authentication errors **MUST** fail fast without retry, with an operator-facing message that names either the missing permission or the credential issue.

**Rationale**: The source API throttles unpredictably, but credential errors are deterministic — retrying them only wastes time and burns rate budget. Clear error messages cut diagnosis time when admin consent is missing or a secret has rotated.

**Actors**: `cpt-insightspec-actor-msentra-orchestrator`

#### Collection Runs

- [ ] `p2` - **ID**: `cpt-insightspec-fr-msentra-collection-runs`

The system **MUST** emit collection-run metadata (start time, end time, status, record count, API call count, error count) so operators can monitor extraction health and detect anomalies (sudden drop in user count, rising error rate).

**Rationale**: Without run metadata, silent failures or partial pulls go unnoticed until downstream metrics break.

**Actors**: `cpt-insightspec-actor-msentra-platform-engineer`

### 5.3 Privacy

#### Field Allowlist

- [ ] `p1` - **ID**: `cpt-insightspec-fr-msentra-field-allowlist`

The system **MUST** restrict the data extracted from the source directory to an explicit allowlist of identity-resolution attributes, applied at extraction time. The system **MUST NOT** collect personal-life or biographic profile attributes — date of birth, free-form biography, interests, hobbies, skills, past projects, schools, personal site, phone numbers, physical addresses, instant-messaging addresses, hire/leave dates, age-group and consent metadata, or external social-identity providers — even when the granted API permission would allow them.

The allowlist **MUST** be expressed as a single declarative configuration point that a reviewer can audit in one place. The exact field list, the request parameter, and the design rationale are in [DESIGN §3.3](./DESIGN.md#33-api-contracts) and [DESIGN §4 Source-Specific Considerations](./DESIGN.md#source-specific-considerations).

**Rationale**: Identity resolution does not require personal-life or biographic attributes. Collecting them creates privacy and compliance exposure with no analytics benefit. Restricting the allowlist at extraction time keeps Bronze free of fields that would otherwise need scrubbing downstream, and enforcing the allowlist in a single declarative location makes scope changes review-visible.

**Actors**: `cpt-insightspec-actor-msentra-platform-engineer`, `cpt-insightspec-actor-msentra-destination`

---

## 6. Non-Functional Requirements

### 6.1 NFR Inclusions

#### Authentication Flexibility

- [ ] `p1` - **ID**: `cpt-insightspec-nfr-msentra-auth-flexibility`

The connector **MUST** support OAuth2 client credentials authentication. The Entra tenant ID, application client ID, and application client secret **MUST** be configurable via the source connection specification (sourced at runtime from a K8s Secret), never hardcoded in the manifest or descriptor.

**Threshold**: Three configuration fields — `azure_tenant_id`, `azure_client_id`, `azure_client_secret` — present in `spec.connection_specification` with `airbyte_secret: true` on the secret.

#### Rate-Limit Compliance

- [ ] `p1` - **ID**: `cpt-insightspec-nfr-msentra-rate-limit-compliance`

The connector **MUST** honour Microsoft Graph rate limits by retrying with exponential backoff on 429 and 503, and **MUST** wait for the duration specified in the `Retry-After` header when present.

**Threshold**: On a 429 response with `Retry-After: 60`, the connector waits ≥ 60 s before retry.

#### Schema Compliance

- [ ] `p1` - **ID**: `cpt-insightspec-nfr-msentra-schema-compliance`

All Bronze records **MUST** use Microsoft Graph's source-native field names (camelCase) with no renaming. Schema transformations occur at the Silver layer.

**Threshold**: Field names in the `bronze_ms_entra.users` table match exactly the keys returned in the Microsoft Graph response (`id`, `userPrincipalName`, `proxyAddresses`, `accountEnabled`, …).

#### Idempotent Writes

- [ ] `p1` - **ID**: `cpt-insightspec-nfr-msentra-idempotent-writes`

Re-running the connector **MUST** produce identical Bronze records given identical source state. The connector does not mutate source data; idempotency is ensured by deterministic API responses and primary-key-based deduplication at the destination.

**Threshold**: Two consecutive runs against an unchanged Entra tenant yield zero net new rows in `bronze_ms_entra.users`.

#### Privacy by Default

- [ ] `p1` - **ID**: `cpt-insightspec-nfr-msentra-privacy-by-default`

The connector **MUST** declare its field allowlist explicitly in the manifest (via the `$select` request parameter) so reviewers can audit, in a single place, exactly what data leaves Microsoft Graph. Adding a new field **MUST** require a manifest change and an explicit code review.

**Threshold**: A grep for `$select` in `connector.yaml` returns the canonical allowlist; no code path constructs the field list dynamically from configuration.

### 6.2 NFR Exclusions

- **Performance SLAs**: Not applicable — Microsoft Graph response times depend on tenant size and Microsoft's infrastructure. The connector runs as a scheduled batch and has no latency guarantee.
- **High availability**: Scheduled batch job; no real-time availability requirement.
- **Encryption at rest**: Handled by the destination (ClickHouse) infrastructure, not the connector.
- **Encryption in transit**: Provided by the platform's HTTPS calls to `graph.microsoft.com` and `login.microsoftonline.com`; not a connector-specific requirement.

---

## 7. Public Library Interfaces

### 7.1 Public API Surface

Not applicable. The MS Entra connector is a declarative manifest (YAML) executed by the Airbyte Declarative Connector framework. It does not expose a public API.

### 7.2 External Integration Contracts

#### Microsoft Graph v1.0

- [ ] `p1` - **ID**: `cpt-insightspec-contract-msentra-graph-v1`

**Direction**: required from external system

**Protocol/Format**: HTTPS / JSON over `https://graph.microsoft.com/v1.0/`

**Compatibility**: Backward-compatible — Microsoft commits to v1.0 stability. Field additions are non-breaking; field removals are deprecated for ≥ 30 days. The connector relies on `additionalProperties: true` in the inline schema so unknown fields pass through.

Endpoint and authentication details are specified in the [DESIGN](./DESIGN.md) §3.3.

---

## 8. Use Cases

#### UC-1: Initial Full Sync

- [ ] `p1` - **ID**: `cpt-insightspec-usecase-msentra-initial-full-sync`

**Actor**: `cpt-insightspec-actor-msentra-platform-engineer`, `cpt-insightspec-actor-msentra-orchestrator`

**Preconditions**:
- Dedicated Entra App Registration created with `User.Read.All` Application permission and admin consent granted.
- K8s Secret `insight-ms-entra-{source-id}` applied with `azure_tenant_id`, `azure_client_id`, `azure_client_secret`.

**Main Flow**:
1. Platform engineer adds the Secret; the orchestrator discovers the source by label.
2. Orchestrator triggers the connector.
3. Connector exchanges `client_credentials` for an access token.
4. Connector iterates `GET /v1.0/users?$select=...&$top=999` following `@odata.nextLink` until exhausted.
5. For each record, the connector injects `tenant_id`, `source_id`, `unique_key` and emits to the destination.
6. Destination writes records to `bronze_ms_entra.users`.

**Postconditions**:
- `bronze_ms_entra.users` is populated with the full directory.
- Collection-run metadata is recorded.

**Alternative Flows**:
- **Admin consent missing**: Token issuance succeeds but `/v1.0/users` returns `403 Authorization_RequestDenied`. Connector fails fast with a message naming the missing permission.
- **Wrong client secret**: Token endpoint returns `401 invalid_client`. Connector fails fast with a message instructing to verify the secret value (not the secret ID).

---

#### UC-2: Scheduled Full Refresh

- [ ] `p1` - **ID**: `cpt-insightspec-usecase-msentra-scheduled-sync`

**Actor**: `cpt-insightspec-actor-msentra-orchestrator`

**Preconditions**:
- UC-1 has completed successfully at least once.
- Schedule is active in the descriptor (`0 5 * * *`).

**Main Flow**:
1. Orchestrator triggers the connector at the scheduled time.
2. Connector performs a full pull (same as UC-1 step 3–5).
3. Destination receives the records; `ReplacingMergeTree` deduplicates by `unique_key`.

**Postconditions**:
- `bronze_ms_entra.users` reflects the current state of the Entra directory.
- Disabled accounts surface as `accountEnabled = false`; deleted accounts are absent.

**Alternative Flows**:
- **Rate-limited (429)**: Connector waits per `Retry-After` and retries.
- **Transient 5xx**: Connector retries with exponential backoff up to a configured max.

---

#### UC-3: Identity Manager Feed

- [ ] `p1` - **ID**: `cpt-insightspec-usecase-msentra-identity-feed`

**Actor**: `cpt-insightspec-actor-msentra-identity-manager`

**Preconditions**:
- Fresh `users` records have landed in Bronze.

**Main Flow**:
1. Identity Manager reads new/updated records from the snapshot/fields-history pipeline.
2. For each record, it ingests identity signals: `mail`, `userPrincipalName`, `proxyAddresses` items, `employeeId`, `displayName`, `onPremisesSamAccountName`, with the source-account-id being the Entra `id` (= `oid`).
3. Identity Manager updates the `oid → person_id` mapping; downstream Silver streams join through this mapping.

**Postconditions**:
- Every Entra-authenticated user can be resolved to a canonical `person_id`.
- When `accountEnabled` flips to `false`, the deactivation event is emitted and the canonical person's `status` becomes `terminated`.

**Alternative Flows**:
- **No `mail` value (some guest users)**: Identity Manager falls back to `userPrincipalName` as the email signal.

---

## 9. Acceptance Criteria

- [ ] The connector successfully extracts user records from a real Entra tenant with `User.Read.All` granted and writes them to the destination.
- [ ] The connector paginates through `@odata.nextLink` and collects every user, verified against the count returned by an independent `GET /v1.0/users/$count` call.
- [ ] Every Bronze record carries `tenant_id`, `source_id`, and a `unique_key` of the form `{tenant_id}-{source_id}-{id}`, with no nulls and no duplicates.
- [ ] The connector retries on 429 and 503, honours `Retry-After`, and fails fast with a clear message on 401 / 403 / `Authorization_RequestDenied`.
- [ ] No fields outside the declared allowlist appear in any Bronze record.
- [ ] Inline schemas in the manifest match the field set returned by Microsoft Graph for the configured `$select`.
- [ ] Multiple Entra tenants can be synced concurrently via separate K8s Secrets without collisions in the destination.

---

## 10. Dependencies

| Dependency | Description | Criticality |
|------------|-------------|-------------|
| Microsoft Graph v1.0 | Source system API | p1 |
| Airbyte Declarative Connector framework (CDK v7.0+) | Runtime execution engine | p1 |
| ClickHouse destination connector | Bronze table writes | p1 |
| Identity Manager | Downstream consumer of identity signals | p1 |
| Entra App Registration with `User.Read.All` Application permission and admin consent | Authentication | p1 |
| Kubernetes Secret with the App Registration credentials | Credential provisioning | p1 |

---

## 11. Assumptions

1. The dedicated Entra App Registration has `User.Read.All` of type Application granted with admin consent — without it the access token's `roles` claim is empty and `/v1.0/users` returns 403.
2. Tenant directory size is bounded — typical 1k–50k users; up to ~500k is paginated through `@odata.nextLink` within a single run window.
3. `id` (Entra Object ID) is immutable per Microsoft's specification and does not change across renames or re-provisioning.
4. `mail` is null for some user types (unlicensed / guest); `userPrincipalName` is always present and serves as a fallback email signal.
5. `proxyAddresses` is sometimes null on guest accounts but is otherwise populated for licensed users.
6. Microsoft Graph's `Retry-After` header reliably reflects throttle wait time.

---

## 12. Risks

| Risk | Impact | Mitigation |
|------|--------|------------|
| Admin consent revoked or App Registration disabled | Hard sync failure (403) | Fail-fast error with clear remediation; alert on consecutive 403s in collection runs. |
| Microsoft Graph schema additions surface unknown fields | Silent expansion of collected data | `$select` allowlist guarantees only declared fields are returned; new fields require explicit manifest change. |
| Tenant exceeds practical pagination limits (very large directories, > 500k users) | Sync window overruns daily schedule | Document the threshold; future enhancement: switch to `/users/delta` once a custom cursor component is available. |
| `proxyAddresses` empty for some users | Reduced cross-service match coverage | Identity Manager already accepts multiple email forms (`mail`, `userPrincipalName`); coverage degrades gracefully. |
| Client secret rotation drifts out of sync with K8s Secret | 401 invalid_client | Document rotation runbook; alert on sustained 401. |
| Personal-life fields accidentally added to `$select` during edits | Privacy regression | Code review checklist + privacy-by-default NFR; manifest is the single audit point. |

---

## 13. Open Questions

### OQ-MSENTRA-1: Delta-token incremental sync

**Owner**: Platform Eng
**Target resolution**: Q3 2026

The Microsoft Graph `/users/delta` endpoint supports change tracking via an opaque `$deltatoken`. The declarative Airbyte runtime cannot drive an opaque-token cursor without a custom Python component.

- Should the connector ship a small custom CDK extension to support delta sync, or wait for declarative cursor flexibility?
- What is the directory-size threshold above which delta sync becomes essential rather than nice-to-have?

### OQ-MSENTRA-2: Group memberships and manager relationships

**Owner**: Identity Eng
**Target resolution**: Q3 2026

`/v1.0/groups`, `/v1.0/groups/{id}/transitiveMembers`, and `$expand=manager` provide org-graph data that is not in v1 of this connector.

- Should group membership be a separate stream (`group_memberships`) or expanded inline on `users`?
- Manager via `$expand=manager` is one extra request per page — acceptable for tenants up to ~10k users; above that, a separate `users/{id}/manager` walk may be needed.

### OQ-MSENTRA-3: Multi-tenant Entra accounts

**Owner**: Platform Eng
**Target resolution**: Q4 2026

Some customers have multiple Entra tenants (acquisitions, regional splits). The K8s Secret model already supports multi-instance via distinct `source-id` annotations.

- Do we need cross-tenant identity unification at the Identity Manager layer, or are separate `person_id` namespaces per tenant the right model?
- How should B2B guest users (homed in tenant A, present in tenant B's directory) be deduplicated?

### OQ-MSENTRA-4: Guest user filtering

**Owner**: Identity Eng
**Target resolution**: Q3 2026

The default `GET /v1.0/users` returns both `Member` and `Guest` users. Guests are typically external collaborators (vendors, partners, customers).

- Should the connector filter out guests by default (`$filter=userType eq 'Member'`) and require an opt-in to include them?
- Or is including guests but tagging them via `userType` sufficient — letting downstream models decide?
