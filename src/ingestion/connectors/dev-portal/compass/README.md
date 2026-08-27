# Compass Connector

Extracts the Atlassian Compass service catalog — components, their owning
teams, repository links, dependency edges, scorecard results and deployment
events — plus the Atlassian Teams directory those owners come from. Everything
is read from one GraphQL endpoint with Basic auth (Atlassian account email +
API token).

Design rationale, traversal contracts and the history strategy live in
[SPEC.md](SPEC.md). Read it before changing a query — several fields in this API
behave in ways that a reasonable reading of the schema does not predict.

## Prerequisites

1. An Atlassian account with **Compass product access** on the site.
2. An API token for that account: id.atlassian.com → Security → API tokens.
   No OAuth application and no additional scopes are needed; the Teams streams
   work on the same credential.
3. The site's **cloud id**, which every Compass field requires:

   ```bash
   curl -s https://<your-site>.atlassian.net/_edge/tenant_info
   ```

   It is not discoverable through the GraphQL gateway, so it is configuration
   rather than something the connector can look up.

## K8s Secret

```yaml
apiVersion: v1
kind: Secret
metadata:
  name: insight-compass-main
  labels:
    app.kubernetes.io/part-of: insight
  annotations:
    insight.cyberfabric.com/connector: compass
    insight.cyberfabric.com/source-id: compass-main
type: Opaque
stringData:
  atlassian_email: "CHANGE_ME"
  atlassian_api_token: "CHANGE_ME"
  atlassian_cloud_id: "CHANGE_ME"
```

### Fields

| Field | Required | Description |
|-------|----------|-------------|
| `atlassian_email` | Yes | Account email owning the token; the Basic-auth username. |
| `atlassian_api_token` | Yes | Atlassian account API token. |
| `atlassian_cloud_id` | Yes | Site identifier (see Prerequisites). Also the basis of the site-scoped ARI the Teams streams pass as `scopeId`. |

> **Note on `username` / `password` spec fields.** This manifest uses
> `BasicHttpAuthenticator`, so importing it into the Airbyte Builder adds
> `username` and `password` to `spec.connection_specification`. Those are
> Builder artifacts mapped from the authenticator's config references — do NOT
> put them in the Secret. The real credentials are the `atlassian_*` fields.

### Automatically injected

| Field | Source |
|-------|--------|
| `insight_tenant_id` | `tenant_id` from tenant YAML |
| `insight_source_id` | `insight.cyberfabric.com/source-id` annotation |

### Local development

```bash
cp src/ingestion/secrets/connectors/compass.yaml.example src/ingestion/secrets/connectors/compass.yaml
# Fill in real values, then apply:
kubectl apply -f src/ingestion/secrets/connectors/compass.yaml
```

## Streams

| Stream | Description | Sync Mode |
|--------|-------------|-----------|
| `components` | The catalog: id, name, type, description, owning team, plus `labels`, `links`, `event_sources` and `relationships` nested in the same row | Full refresh |
| `scorecards` | Scorecard definitions with their criteria (weights, comparators, thresholds) as JSON | Full refresh |
| `component_scorecard_scores` | One row per component × scorecard: total, status band, per-criterion breakdown | Full refresh |
| `teams` | The Atlassian Teams directory | Full refresh |
| `team_members` | One row per team × person, with role and membership state | Full refresh |
| `deployment_events` | Compass events of type `DEPLOYMENT`, per component | Incremental (`last_updated`) |

Nested-in-row rather than separate streams: `links`, `labels`, `event_sources`
and `relationships` all arrive inside the catalog sweep, so splitting them into
their own streams would re-query the same data purely to reshape it. Flattening
belongs in dbt, where `ARRAY JOIN` costs nothing.

### Ownership join

`components.owner_team_id` is an ARI of the form
`ari:cloud:identity::team/<uuid>`; the UUID inside it is the same identifier
that `teams.team_id` carries and that Jira's `atlassian-team` custom field
stores. The three are one directory entity, joinable exactly with no name
matching.

`links` entries of type `REPOSITORY` are the join to the git sources. The git
connectors do not persist `html_url` / `web_url` into staging, so the only
cross-connector repo key is `class_git_repositories.full_name` — the link URL
has to be parsed (host selects which `data_source` to match, path becomes the
candidate `full_name`). Treat that parse as lossy and keep the raw URL.

### People join

The Teams API exposes **no `email` field** on the user type, so `team_members`
joins to people by the Atlassian account id embedded in `member_id`
(`ari:cloud:identity::user/<accountId>`), not by email as most other connectors
do. If the target people relation does not carry Atlassian account ids, this
join cannot be made and the component → team → person chain stops at the team.

## Caveats

These are properties of the source, not bugs to fix here. Anything built on
this data has to respect them.

- **A `DEPLOYMENT` event does not imply a release.** Any tool can publish
  events of that type through an external event source, and an integration may
  map build or artifact events onto it with a non-production
  `environment_category`. Key release metrics on
  `environment_category = 'PRODUCTION'`; everything else is build or
  pre-production activity and must not be presented as a release.
- **The event feed is a rolling window (~2 weeks) with no backfill.** A missed
  sync loses events permanently — that is why the schedule is daily and why
  failures here are not "catch up tomorrow".
- **Event rows are rewritten in place** as a deployment progresses, so
  `update_sequence_number` is the dedup discriminator and `completed_at` may
  never be filled for events whose terminal state never arrives.
- **Scorecard scores measure integration coverage as much as health.**
  Metric-backed criteria score zero wherever the integration feeding the metric
  is absent, which is indistinguishable from genuine failure at the score
  level. Use `criteria_scores[].dataSourceLastUpdated` to tell them apart, or
  restrict to criteria that Compass evaluates intrinsically (owner,
  description, link presence).
- **Team membership is many-to-many.** A person can belong to several teams, so
  per-person aggregation through membership needs an explicit apportionment
  rule or it double-counts. Component ownership has no such problem — a
  component has exactly one owner.
- **`teams.member_count` is not populated** by the search field this stream
  uses; count `team_members` rows instead.
- **A failed catalog query fails the sync; one unreadable component does not.**
  Most fields return a union, so a refusal arrives as a *successful* body with a
  `message` and no rows. Where an empty answer would be indistinguishable from
  "there is nothing" — the catalog, the scorecards, the team directory and its
  membership — the connector fails loudly. A single component that vanished
  between the catalog sweep and its event read is skipped instead, and the other
  components continue.
- **`component_scorecard_scores` rides an experimental field** and needs the
  `@optIn(to: "compass-beta")` directive. Every other stream is on stable
  fields, so if beta churn breaks it, it can be dropped without affecting the
  rest.

## Silver Targets

None yet — this connector is **bronze-only**. `dbt/` carries just the bronze→RMT
promotion, which makes later `FINAL` reads well-defined (load-bearing for
`deployment_events`, whose rows are replaced rather than appended).

Proposed silver class families are sketched in [SPEC.md](SPEC.md) §7 and
deliberately deferred: they introduce a service-ownership concept that does not
exist in silver today, and the teams half needs a decision from the identity
owners before a third representation of org structure lands in the warehouse.
