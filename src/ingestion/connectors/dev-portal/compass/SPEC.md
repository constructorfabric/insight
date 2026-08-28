# Compass + Atlassian Teams connector — design spec

Status: **implemented (bronze-only).** Manifest, descriptor, bronze→RMT
promotion and mock suites are in place; every stream has been read against a
live site. Silver is deliberately deferred (see §7).

Source: Atlassian Compass (internal developer portal — a catalog of software
components) plus the Atlassian Teams directory that Compass uses for component
ownership. One connector covers both; see [Placement](#8-placement).

All examples below are synthetic.

## 1. Scope

In scope — six streams:

| Stream | Grain |
| --- | --- |
| `components` | one component (service / library / application / other) |
| `scorecards` | one scorecard definition, with its criteria |
| `component_scorecard_scores` | one component x scorecard result |
| `teams` | one team in the Atlassian Teams directory |
| `team_members` | one team x person membership |
| `deployment_events` | one event of Compass type `DEPLOYMENT` on a component |

`links`, `labels`, `event_sources` and `relationships` are JSON columns on
`components` rather than streams of their own — see [Links](#33-links-a-column-not-a-stream)
and [Dependency edges](#34-dependency-edges-a-column-not-a-stream).

Out of scope for v1, with reasons:

- **Metric time series** (`metricValuesTimeSeries`). Compass computes
  DORA-adjacent metrics per component, but reading them costs one query per
  metric source per component, and most of these quantities are derivable from
  data the git and CI sources already deliver.
- **`PUSH` / `BUILD` / `INCIDENT` events.** Push and build duplicate, at lower
  fidelity, what the git connectors already carry. Incident coverage depends on
  an incident-tool integration being wired up; revisit when one is.
- **Atlassian groups.** A group is an access-control primitive carrying only a
  name and an id — no owner, no hierarchy, no typed relationship to a team.
  Group names may resemble team names, but no id linkage exists, so any join
  would be fuzzy string matching. Groups answer "who has access", which is a
  different question from ownership.
- **Team hierarchy** (`TeamV2.hierarchy`). Useful, but adds a fourth
  experimental opt-in (see [Beta surface](#24-beta-surface)). Add once the
  base streams are stable.

## 2. Transport

### 2.1 Endpoint and auth

Single GraphQL gateway for every stream:

```
POST https://api.atlassian.com/graphql
Authorization: Basic base64(<atlassian_email>:<atlassian_api_token>)
Content-Type: application/json
```

An Atlassian account email plus API token is sufficient; no OAuth app and no
additional grants are required. The token is the same *kind* of credential the
Jira and Confluence connectors use, but this connector declares its own secret
(see [Placement](#8-placement)) — the repo has no shared-secret mechanism.

`cloudId` is a required argument on most Compass fields and is discovered once
per site:

```
GET <atlassian_instance_url>/_edge/tenant_info  ->  {"cloudId": "..."}
```

The Teams fields take a site-scoped ARI rather than a second identifier:
`ari:cloud:platform::site/<cloudId>`. The instance URL is only ever used
out-of-band, to read the cloud id above — it is not a connector setting.

### 2.2 Protocol rules that shape the manifest

1. **Operation names are mandatory.** Anonymous queries return a warning that
   they will be rejected in future. Every `request_body_json.query` must be a
   named operation.
2. **Errors arrive as HTTP 200** with a top-level `errors[]` array. The
   requester needs `HttpResponseFilter` entries keyed on `response['errors']` —
   the same treatment `git/github-directory` already applies to the GitHub
   GraphQL API. A predicate that only inspects the status code will treat every
   failure as a successful empty page.
3. **Unions everywhere.** Most fields return a union of a connection type and
   `QueryError`, so every selection needs `... on <Connection>` plus
   `... on QueryError { message }`. A `QueryError` payload is a *successful*
   response body carrying a business error — record selectors must not mistake
   it for data.
4. **Argument types are inconsistent** between sibling fields (the same logical
   `cloudId` is `String!` on one field and `ID!` on another). Validate each
   query against introspection rather than by analogy.

### 2.3 Rate budget

The gateway applies cost-based per-user limiting (points per minute) and
answers `429` above it. Costs are not published per field, so treat the budget
as an unknown to be measured, not modelled. Consequences for the manifest:

- keep a conservative `max_concurrency` and a `429` backoff filter;
- the request count is dominated entirely by `deployment_events`
  (see [Volume](#6-volume)), so pace that stream rather than the cheap ones.

### 2.4 Beta surface

Several required fields are experimental and need a **field-level** directive —
`@optIn(to: "<key>")` on the field itself. Neither the query-level directive
position nor the `X-ExperimentalApi` header works.

| Field | Opt-in key | Needed for |
| --- | --- | --- |
| `scorecards(...).appliedToComponents` | `compass-beta` | `component_scorecard_scores` |
| `componentScorecardRelationship` | `compass-beta` | score history backfill |
| `scorecardScoreHistories` | `compass-beta` | score history backfill |
| `teamsV2` | `Team` | bulk team resolve |
| `TeamV2.hierarchy` | `Team-hierarchy` | (out of scope for v1) |

**Risk to record explicitly:** experimental fields may change shape without a
deprecation cycle. `components`, `scorecards`, `teams`, `team_members` and
`deployment_events` are all reachable on stable fields. Only
`component_scorecard_scores` (and the optional history backfill) depend on
beta. If beta stability becomes a problem, that one stream can be dropped
without touching the rest.

## 3. Streams

### 3.1 Dependency graph

```text
scorecards                      [dictionary  · full refresh]
│
teams                           [root        · full refresh]
└─► team_members                [substream: partition = team_id · full refresh]

components                      [root        · full refresh]
│      ownerId ───────────────► teams.id
│      links[] · labels[] · event_sources[] · relationships[]  — JSON columns,
│      all carried by the same catalog response
├─► component_scorecard_scores  [traversed via scorecards, not components · full refresh]
└─► deployment_events           [substream: partition = component_id · INCREMENTAL]
```

`component_scorecard_scores` is the one substream that is **not** partitioned by
component: iterating scorecards and reading each scorecard's applied components
costs `O(scorecards)` requests, whereas asking each component for its scores
costs `O(components)` — three orders of magnitude apart.

### 3.2 `components`

Root stream. `ownerId`, `links`, `labels` and `eventSources` all come back
inside the same `searchComponents` sweep, so no per-component follow-up is
needed for the catalog itself.

```graphql
query components($cloudId: String!, $after: String) {
  compass {
    searchComponents(cloudId: $cloudId, query: {first: 100, after: $after}) {
      ... on CompassSearchComponentConnection {
        nodes { component {
          id name slug type description url ownerId
          labels { name }
          links { id type url name }
          eventSources { id eventType externalEventSourceId }
        } }
        pageInfo { hasNextPage endCursor }
      }
      ... on QueryError { message }
    }
  }
}
```

| Field | GraphQL type | Bronze | Note |
| --- | --- | --- | --- |
| `id` | `ID!` | String | `ari:cloud:compass:<cloudId>:component/<...>` |
| `name` | `String!` | String | |
| `slug` | `String` | String | |
| `type` | `CompassComponentType` | String | `APPLICATION` / `LIBRARY` / `SERVICE` / `OTHER` |
| `description` | `String` | Nullable(String) | |
| `url` | `URL` | Nullable(String) | component page in Compass |
| `ownerId` | `ID` | Nullable(String) | `ari:cloud:identity::team/<uuid>` — FK to `teams` |
| `labels[].name` | `String` | Array(String) | |
| `eventSources[]` | `EventSource` | JSON | `{id, eventType, externalEventSourceId}` — records which event feeds are wired |

Sync: full refresh. `unique_key = id`.

### 3.3 Links (a column, not a stream)

Links arrive with the catalog sweep and land as the `links` JSON column on
`components`. Each entry:

| Field | Type | Note |
| --- | --- | --- |
| `id` | `ID!` | link id |
| `type` | `CompassLinkType` | `REPOSITORY` / `PROJECT` / `DASHBOARD` / `CHAT_CHANNEL` / `ON_CALL` / `OTHER_LINK` |
| `url` | `URL!` | |
| `name` | `String` | |

dbt flattens the column with `ARRAY JOIN`, keyed `(component_id, id)`.

`REPOSITORY` links are the join to the git sources. The git connectors do not
persist `html_url` / `web_url` into staging, so the only cross-connector repo
key is `class_git_repositories.full_name` (`owner/repo`,
`group/subgroup/repo`, `workspace/repo`). The URL therefore has to be parsed:
host decides which `data_source` to match against, path becomes the candidate
`full_name`. Treat the parse as lossy and keep the raw URL in bronze.

### 3.4 Dependency edges (a column, not a stream)

Dependency edges are requested inside the catalog sweep and land as the
`relationships` JSON column on `components`:

```graphql
relationships(query: {first: 100}) {
  ... on CompassRelationshipConnection {
    nodes { relationshipType startNode { id } endNode { id } }
    pageInfo { hasNextPage }
  }
}
```

| Field | Type |
| --- | --- |
| `relationshipType` | `String!` (enum currently has a single member, `DEPENDS_ON`) |
| `startNode.id` | `ID` |
| `endNode.id` | `ID` |

Asking each component for its edges separately would cost one request per
component for data the catalog query already returns. The trade is that a
nested list cannot be paginated: `pageInfo.hasNextPage` is therefore carried
out as the `relationships_truncated` column, so a component with more edges
than the query asked for is a visible row rather than silent edge loss. Nothing
in the catalog is expected to approach that bound; if the column is ever true,
the edges want a substream of their own.

### 3.5 `scorecards`

A scorecard is a weighted checklist evaluated automatically against a
component: criteria assert that a field is filled, that an owner or a link of a
given type exists, or that a metric satisfies a comparator. Criterion weights
are normalised to a total of 100.

| Field | Type | Note |
| --- | --- | --- |
| `id` | `ID!` | |
| `name` | `String!` | |
| `description` | `String` | |
| `state` | enum | `PUBLISHED` / `DRAFT` |
| `importance` | enum | `REQUIRED` / `RECOMMENDED` / `USER_DEFINED` |
| `scoringStrategyType`, `scoreSystem` | | keep as raw strings |
| `changeMetadata` | object | `createdAt`, `createdBy`, `lastUserModificationAt`, `lastUserModificationBy` |
| `criterias[]` | interface | per criterion: `id: ID!`, `weight: Int!`, `name: String`, `__typename` (the check kind), plus kind-specific fields — `linkType` for link checks, `comparator` + `comparatorValue` + `metricDefinition.name` for metric checks |

Store `criterias[]` as JSON in bronze and flatten in silver: the criterion
subtypes are an open set (`HasOwner`, `HasDescription`, `HasLink`,
`HasMetricValue`, `HasCustom*Field`, ...) and a typed column per subtype would
churn.

`criterias[].description` is beta-gated — omit it.

Sync: full refresh. `unique_key = id`. **Dated snapshots required** — see
[History](#5-history).

### 3.6 `component_scorecard_scores`

```graphql
query scores($cloudId: ID!, $after: String) {
  compass {
    scorecards(cloudId: $cloudId) {
      ... on CompassScorecardConnection {
        nodes {
          id
          appliedToComponents(query: {first: 25, after: $after}) @optIn(to: "compass-beta") {
            ... on CompassScorecardAppliedToComponentsConnection {
              totalCount
              edges {
                node { id }
                score {
                  ... on CompassScorecardScore {
                    totalScore maxTotalScore
                    status { name lowerBound upperBound }
                    criteriaScores { criterionId score maxScore status dataSourceLastUpdated }
                  }
                  ... on QueryError { message }
                }
              }
              pageInfo { hasNextPage endCursor }
            }
            ... on QueryError { message }
          }
        }
        ... on QueryError { message }
      }
    }
  }
}
```

Note the score hangs off the **edge**, not the node.

| Field | Type | Note |
| --- | --- | --- |
| `component_id`, `scorecard_id` | `ID!` | |
| `totalScore` / `maxTotalScore` | `Int!` | normalised to 100 |
| `status.name` | `String` | `PASSING` / `NEEDS_ATTENTION` / `FAILING` |
| `criteriaScores[]` | array | `{criterionId, score, maxScore, status, dataSourceLastUpdated}` |
| `snapshot_date` | Date | stamped by the connector, part of the key |

`dataSourceLastUpdated` says when the metric underneath a criterion last
refreshed. It is the only way to tell "the criterion genuinely fails" from "the
metric feeding it went stale", so it must survive into silver.

Sync: full refresh, one snapshot per run. `unique_key = (component_id,
scorecard_id, snapshot_date)` — snapshots are **not** collapsed; the series is
the product value.

**Interpretation constraint for downstream consumers.** Metric-backed criteria
depend on external integrations (coverage, vulnerabilities, quality gates)
being wired per component. Where such an integration is absent the criterion
scores zero, which is indistinguishable at the score level from a genuinely
failing component. Consumers must therefore either restrict to criteria whose
inputs are Compass-intrinsic (owner / description / link presence), or filter
on `dataSourceLastUpdated` freshness. A raw average across all scorecards
measures integration coverage as much as engineering health.

### 3.7 `teams`

The Atlassian Teams directory is an organization-level entity. It is not a
Compass concept and not a Jira concept: Compass references it from
`component.ownerId`, and Jira surfaces it as a custom field of type
`atlassian-team`, both by the same team UUID. Do not plan team attribution off
the Jira field — how densely it is populated is a per-site configuration
choice, whereas Compass ownership is enforced by scorecards.

Enumeration (stable, no opt-in). The scope is an ARI built from the configured
cloud id, so no separate organization id has to be supplied:

```graphql
query insightTeams($scopeId: ID!, $cursor: String) {
  team {
    teamSearchV3(scopeId: $scopeId, first: 50, after: $cursor,
                 enablePagination: true, showEmptyTeams: true) {
      pageInfo { hasNextPage endCursor }
      nodes { team { id displayName description state organizationId memberCount isVerified } }
    }
  }
}
```

with `scopeId = ari:cloud:platform::site/<atlassian_cloud_id>`.

The search field does not populate `memberCount`; count `team_members` rows, or
resolve details per team via `teamV2`.

| Field | Type | Note |
| --- | --- | --- |
| `id` | `ID!` | `ari:cloud:identity::team/<uuid>` — the same UUID Compass and Jira both reference |
| `displayName` | `String` | |
| `description` | `String` | |
| `state` | `TeamStateV2` | |
| `organizationId` | `ID` | |
| `isVerified` | `Boolean` | |
| `memberCount` | `Int` | not returned by the search field |

`TeamV2.type` is deliberately not selected: it is an object needing its own
subselection, and nothing downstream asks what kind of team a team is.

Sync: full refresh. `unique_key = id`. Dated snapshots required.

`teamSearchV2` and `teamsTql` are the alternatives. `teamSearchV2` works but
requires an organization id the connector would otherwise have no way to
obtain; `teamsTql` needs its own opt-in *and* rejects a site-scoped ARI as
`scopeId`. Neither is worth the extra configuration.

**A team directory may be fed by an external HR system.** Where it is, team
names, membership and hierarchy are managed by that sync and are read-only in
Atlassian. Do not model provenance in this connector: it is a per-deployment
integration choice, not a property of the entity. Reconciling teams against an
HR-derived people set is a silver-layer concern.

### 3.8 `team_members`

Per team, paginated:

```graphql
members(first: 50, after: $after, state: [FULL_MEMBER, ALUMNI, REQUESTING_TO_JOIN]) {
  nodes { role state member { id name accountStatus } }
  pageInfo { hasNextPage endCursor }
}
```

| Field | Type | Note |
| --- | --- | --- |
| `team_id` | `ID!` | FK |
| `member.id` | `ID!` | `ari:cloud:identity::user/<accountId>` |
| `member.name` | `String` | |
| `member.accountStatus` | `String` | deactivated and closed accounts remain listed as members |
| `role` | enum | `REGULAR` / `ADMIN` |
| `state` | `TeamMembershipState` | `FULL_MEMBER` / `ALUMNI` / `REQUESTING_TO_JOIN` |

Sync: full refresh, partitioned by `team_id`. `unique_key = (team_id,
member_id)`. Dated snapshots required.

Two constraints for whoever writes the silver models:

1. **`User` has no `email` field on this API.** The join key to people is the
   account id. If the target people relation does not carry Atlassian account
   ids, this stream cannot be joined to it — resolve that before building on it.
2. **Membership is many-to-many.** A person can belong to several teams, so
   "a person's team" is a relation, not a function; any per-person aggregation
   through membership needs an explicit apportionment rule or it double-counts.
   Ownership does not have this problem: a component has exactly one owner.

### 3.9 `deployment_events`

The only incremental stream.

```graphql
query events($id: ID!, $after: String) {
  compass {
    component(id: $id) {
      ... on CompassComponent {
        events(query: {first: 50, after: $after, eventTypes: [DEPLOYMENT]}) {
          ... on CompassEventConnection {
            nodes {
              ... on CompassDeploymentEvent {
                displayName description url lastUpdated updateSequenceNumber
                deploymentProperties {
                  sequenceNumber state startedAt completedAt
                  environment { category displayName environmentId }
                  pipeline { pipelineId displayName url }
                }
              }
            }
            pageInfo { hasNextPage endCursor }
          }
          ... on QueryError { message }
        }
      }
    }
  }
}
```

| Field | Type | Bronze | Note |
| --- | --- | --- | --- |
| `component_id` | `ID!` | String | from the partition |
| `eventType` | enum | String | `DEPLOYMENT` for this stream |
| `displayName` | `String!` | String | |
| `description` | `String` | Nullable(String) | |
| `url` | `URL` | Nullable(String) | |
| `lastUpdated` | `DateTime!` | DateTime64 | **cursor** |
| `updateSequenceNumber` | `Long!` | UInt64 | monotonic per event; the dedup discriminator |
| `deploymentProperties.state` | enum | String | `PENDING` / `IN_PROGRESS` / `SUCCESSFUL` / `CANCELLED` / `FAILED` / `ROLLED_BACK` / `UNKNOWN` |
| `deploymentProperties.environment.category` | enum | String | `PRODUCTION` / `STAGING` / `TESTING` / `DEVELOPMENT` / `UNMAPPED` |
| `deploymentProperties.environment.displayName`, `.environmentId` | `String` | String | |
| `deploymentProperties.startedAt`, `.completedAt` | `DateTime` | Nullable(DateTime64) | duration; `completedAt` may stay null — see the `value_type` note below |
| `deploymentProperties.sequenceNumber` | `Long` | UInt64 | source-side ordinal |
| `deploymentProperties.pipeline` | object | JSON | `{pipelineId, displayName, url}` |

Sync: incremental on `lastUpdated`. `unique_key = (component_id,
update_sequence_number)`. One event is updated in place as it progresses, so the
ReplacingMergeTree version must be `lastUpdated` (or the sequence number), and
read-time dedup must pick the latest.

A nullable timestamp must be added **without** `value_type: string`. With it, a
Jinja `none` renders as the four-character text `"None"` and is stored as a
value, so a deployment that never completed would carry a fake timestamp rather
than a null. Without it the CDK literal-evaluates `"None"` back to a null, and a
real timestamp — which is not a Python literal — stays the string it was.

Three properties of this stream that the manifest and the silver models both
have to respect:

1. **No server-side date filter is usable.** `timeParameters {startFrom,
   endAt}` rejects every window tried — 7 days, 90 days, a year, with and
   without millisecond precision — with a complaint that `endAt` must be within
   one year of `startFrom`. `startFrom` alone returns nothing; `endAt` alone
   returns only events on that boundary date. Incremental must therefore be
   **client-side** on `lastUpdated` (`is_client_side_incremental`), i.e. pages
   are fetched and records filtered against state. Revisit if Atlassian fixes
   the filter — a server-side window would cut the request count sharply.
2. **The feed is a rolling window, roughly two weeks wide.** There is no
   deeper backfill: whatever is older than the window is gone. A missed sync
   loses events permanently, so this stream must run at least daily and its
   failures are not "catch up next run".
3. **`DEPLOYMENT` does not imply a release.** Any tool can publish events of
   this type through an external event source, and an integration may
   legitimately map build or artifact events onto it with a non-production
   `environment.category`. Both kinds can coexist in one feed, with the
   non-production ones dominating by volume. Deployment-frequency and lead-time metrics must
   therefore be keyed on `environment.category = PRODUCTION`; everything else
   is build or pre-production activity and must not be labelled as releases in
   any product surface. Check the category distribution before publishing any
   DORA-shaped metric from this stream.

Pagination is newest-first and the cursor behaves correctly.

## 4. Traversal contracts

Three different mechanisms coexist in this one API. Getting them confused is the
most likely source of silent data loss.

| Data | Traversal | Trap |
| --- | --- | --- |
| components, links, relationships, teams, members, events | `first` / `after` cursor, `pageInfo.hasNextPage` | none — behaves normally |
| scorecard applied-components | `query: {first, after}` (nested in an input object, not top-level args) | passing `first` as a sibling argument is a validation error |
| scorecard score history | `query: {filter: {startFrom, periodicity}}` — a **window ending at `startFrom`**; walk backwards by setting the next `startFrom` to (earliest date returned − 1 day) | the `after` cursor is inert here: it returns the same window forever with `hasNextPage: true`. A paginator that trusts `hasNextPage` loops indefinitely |

One further trap: in the **bulk** `teamsV2` form the nested `members` list comes
back truncated with `hasNextPage: true` on every team, including teams whose
members all fit. Per-team `teamV2(...).members(...)` paginates exactly and
reconciles with `memberCount`. Enumerate teams in bulk; fetch members per team.

**Every paginator path expression must be null-safe.** The cursor is read out of
the response by path, and a union that resolved to `QueryError` has no
connection under it — so a plain `response['data'][...]['pageInfo']` raises
`'None' has no attribute 'pageInfo'` and kills that partition. Guard each hop
(`(… or {}).get(…)`), and let the stream's error handler (§2.2) decide whether
the union error is fatal or skippable.

## 5. History

Compass and Teams emit **no change events and no field-level audit**. Scorecard
definitions expose only `changeMetadata` — a single "someone changed something"
timestamp with no diff and no version. Everything historical is therefore
either a Compass-served series or our own snapshots.

Bronze is append-only with a per-sync `_version`, so a full-refresh stream
already accumulates dated snapshots for free. The design decision is which
diffs to **materialise in silver**.

| Stream | Silver treatment | Why |
| --- | --- | --- |
| `components.ownerId` | **SCD intervals** `(component_id, owner_team_id, valid_from, valid_to)` | attribution axis: "team X owned component Y at time T". Without it, re-assigning a component silently rewrites all historical attribution |
| `component_links` (REPOSITORY) | **SCD intervals** | same argument one hop out: commits and PRs attribute to a component through the repo link, so the link must be time-bounded. Can start latest-only and be upgraded later — bronze snapshots accumulate regardless |
| `team_members` | **SCD intervals** | "person P was in team X at time T"; the same failure mode as ownership |
| `teams` | diff on `displayName`, `state` | renames must not retroactively rename history |
| `scorecards` | **dated snapshots of the definition** | weights renormalise to 100, so a score drop is ambiguous — a service degrading and a criterion being added look identical. Only a definition snapshot for the same date separates them |
| `component_scorecard_scores` | dated snapshots, never collapsed | the series *is* the deliverable |
| `components.relationships` | latest only | no historical metric depends on past dependency graphs |
| `deployment_events` | events, RMT by `update_sequence_number` | not applicable — already a log |

**Asymmetry to plan around:** score history is backfillable from the API (see
below), definition history is not. Definition snapshots therefore start on day
one of ingestion, and any backfilled score older than that has no matching
definition — those rows must be marked "criteria set unknown" rather than
silently compared against today's definition.

### 5.1 Optional one-off score backfill

`componentScorecardRelationship.scorecardScoreHistories` serves
`{date, totalScore}` per component x scorecard back to `appliedSince`, at
`DAILY` or `WEEKLY` periodicity, via the windowed traversal described in
[Traversal](#4-traversal-contracts). `criteriaScoreHistories` serves per-criterion
`{criterionId, scoreStatus, explanation}` — statuses only, no numbers.

Worth one bootstrap run so the product does not start from an empty series;
not worth running on a schedule, because the current score already arrives with
full per-criterion detail in `component_scorecard_scores` at no extra cost.
Both fields are beta-gated.

## 6. Volume

Per daily sync, as a function of catalog size:

| Stream | Requests |
| --- | --- |
| `components` (+ links, labels, event sources, dependency edges) | `ceil(N_components / 100)` |
| `scorecards` | 1 |
| `component_scorecard_scores` | `sum over scorecards of ceil(applied_components / 25)` — the field caps page size at 25 |
| `teams` | `ceil(N_teams / 50)` + 1 bulk resolve |
| `team_members` | `O(N_teams)` |
| `deployment_events` | `O(N_components)` — one page for a quiet component, several for a busy one |

Everything except `deployment_events` lands in the low hundreds of requests
for a catalog of a few thousand components. `deployment_events` is
`O(N_components)` with no server-side filter, so it dominates the sync by
roughly an order of magnitude and sets both the runtime and the rate-limit
exposure. It is the reason this connector is separate from a heavy tracker
connector rather than merged into one: the failure domains and the cadences do
not match.

## 7. Silver

No component or service-ownership concept exists in silver today; this
introduces one. Proposed families, following the established thin
`union_by_tag` + `ReplacingMergeTree(_version)` + `order_by=['unique_key']`
+ `delete+insert` shape:

- `class_component_catalog` — components with type, name, description
- `class_component_ownership` — the SCD ownership intervals
- `class_component_links` — links, with the parsed repo identity for the git join
- `class_component_dependencies` — the dependency edges
- `class_component_scorecards` — definitions (dated) and scores (dated)
- `class_component_events` — deployment events
- teams and membership: extend the existing people/org families rather than
  inventing a parallel org structure — decide with the identity owners before
  building, since a third representation of org structure is a liability

The repo join is `class_component_links` (parsed `full_name` + host-derived
`data_source`) against `class_git_repositories.full_name`. Expect partial
matches and keep the unmatched rows visible rather than dropping them.

## 8. Placement

| Item | Value |
| --- | --- |
| Path | `src/ingestion/connectors/dev-portal/compass/` |
| Category | `dev-portal` — **new**; no existing category covers a developer portal / service catalog. The choice also needs adding to the connector skill's category list |
| Type | `nocode` (declarative manifest) |
| Descriptor version | `1.0.0`, strict semver |
| Bronze namespace | `bronze_compass` |
| Schedule | daily; `deployment_events` sets the floor, since a missed run loses events |
| Secret fields | `atlassian_email`, `atlassian_api_token`, `atlassian_cloud_id` |
| `dbt_select` | `tag:compass+` |

Teams streams live in this connector rather than in their own: same host, same
credential, same cadence, same trivial volume. The connector boundary is a
sync/deploy/failure boundary, not a consumption boundary — silver models read
bronze tables, so a future consumer (for example a Jira model that wants the
Team field resolved) can join these tables regardless of which connector
produced them.

## 9. Open questions

1. **Category name.** `dev-portal` is proposed; it adds the first new source
   category in a while and touches the connector skill's fixed list.
2. **Teams in silver.** Extend the existing people/org families, or a separate
   `class_team_*` family? Needs the identity owners' call — three
   representations of org structure in one warehouse is a maintenance risk.
3. **Account-id join.** Does the target people relation carry Atlassian account
   ids? If not, `team_members` cannot be joined to people and the
   component → team → person chain stops at the team.
4. **`environment.category` distribution.** Decides whether
   `deployment_events` can support release metrics at all, or only build
   activity. Answer before any DORA-shaped product surface is built on it.
5. **Criterion id stability.** Does editing a scorecard criterion preserve its
   id? If not, "threshold retuned" is indistinguishable from "criterion
   replaced" in a snapshot diff. Verify on a throwaway scorecard rather than by
   mutating a live one.

## 10. Delivery checklist

Per the connector skill:

- [x] `descriptor.yaml` with strict semver, `type: nocode`, namespace, schedule, secret fields, `dbt_select`
- [x] `connector.yaml` — declarative manifest; named operations; `errors[]` response filters; per-stream traversal per [Traversal](#4-traversal-contracts)
- [x] `connectors-config.yaml` entry (bootstrap-db) — without it the bronze database is never created
- [x] connectors-ddl snapshot regenerated per the bootstrap-db README
- [x] per-stream mock tests — 100% stream coverage, including a `QueryError`-in-200 case and a `hasNextPage`-lies case per [Traversal](#4-traversal-contracts)
- [x] `scripts/ci/connector_wiring.py` green
- [x] bronze dbt conventions (RMT engine, `order_by=['unique_key']`) — the promotion model
- [ ] `class_people.sql` `depends_on` entry — not applicable while no stream feeds people; revisit with the account-id join in §9
- [ ] silver dbt conventions (read-time dedup, `delete+insert`) — deferred with silver itself (§7)
