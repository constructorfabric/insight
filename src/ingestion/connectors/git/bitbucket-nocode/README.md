# Bitbucket Cloud (git-cli-proxy) Connector

Declarative Bitbucket Cloud connector that extracts commit-level data through
the [git-cli-proxy](../../../../../docs/components/connectors/git/git-cli-proxy/specs/DESIGN.md)
instead of the vendor API. Bitbucket is where the per-commit API cost hurts
most: the CDK connector calls `diffstat/{sha}` once per commit against a
~1000 req/h budget, so a 50k-commit repository is a multi-day backfill. The
proxy serves the same rows from a bare clone.

Repository discovery still uses the Bitbucket API — one call per page of
repositories, not per commit.

Auth: an Atlassian API token used as HTTP basic `username:token`, both for the
API and (forwarded per request, never stored) for the clone the proxy performs.

## Prerequisites

1. A Bitbucket API token with `repository:read`, plus the account username or
   email it belongs to.
2. A reachable git-cli-proxy deployment and its bearer token. In-cluster the
   umbrella composes both (`insight-git-cli-proxy-config`); the proxy accepts
   traffic only from the namespaces its NetworkPolicy allows.

## K8s Secret

```yaml
apiVersion: v1
kind: Secret
metadata:
  name: insight-bitbucket-nocode-main
  labels:
    app.kubernetes.io/part-of: insight
  annotations:
    insight.cyberfabric.com/connector: bitbucket-nocode
    insight.cyberfabric.com/source-id: bitbucket-nocode-main
type: Opaque
stringData:
  bitbucket_username: "CHANGE_ME"
  bitbucket_token: "CHANGE_ME"
  bitbucket_workspaces: '["acme"]'
  git_proxy_url: "http://insight-git-cli-proxy:8085"
  git_proxy_token: "CHANGE_ME"
  bitbucket_start_date: "2020-01-01"
```

### Fields

| Field | Required | Description |
|-------|----------|-------------|
| `bitbucket_username` | Yes | Atlassian account username or email |
| `bitbucket_token` | Yes | API token with `repository:read` |
| `bitbucket_workspaces` | Yes | JSON array of workspace slugs |
| `git_proxy_url` | Yes | git-cli-proxy base URL. No default — a wrong value must fail `check`, not fall back |
| `git_proxy_token` | Yes | Bearer token the proxy requires on every `/v1` request |
| `bitbucket_api_base_url` | No | API base URL (default `https://api.bitbucket.org/2.0`) |
| `bitbucket_start_date` | No | Earliest date for the initial sync (default `2020-01-01`) |
| `bitbucket_page_size` | No | Bitbucket API page size, max 100 |
| `proxy_page_size` | No | Rows per proxy page (default 500) |
| `bitbucket_include_patch` | No | Store per-file diff text (default true) |
| `bitbucket_max_patch_bytes` | No | Truncation cap per file diff (default 1 MiB) |

### Automatically injected

| Field | Source |
|-------|--------|
| `insight_tenant_id` | `tenant_id` from tenant YAML |
| `insight_source_id` | `insight.cyberfabric.com/source-id` annotation |

### Local development

```bash
cp src/ingestion/secrets/connectors/bitbucket-nocode.yaml.example src/ingestion/secrets/connectors/bitbucket-nocode.yaml
# fill in real values, then:
kubectl apply -f src/ingestion/secrets/connectors/bitbucket-nocode.yaml
```

## Streams

| Stream | Upstream | Sync Mode | Cursor |
|--------|----------|-----------|--------|
| `repositories` | Bitbucket `/2.0/repositories/{workspace}` | incremental | `updated_on` |
| `commits` | proxy `/v1/commits` | incremental, per repository | `committed_date` |
| `commit_files` | proxy `/v1/file-changes` | incremental, per repository | `committed_date` |
| `repo_branches` | proxy `/v1/branches` | full refresh, per repository | — |

### How the streams fit together

`repositories` fans out over the configured workspaces (`ListPartitionRouter`)
and is the incremental **parent**: the commit streams route through
`SubstreamPartitionRouter` with `incremental_dependency: true`, so a sync visits
only repositories whose `updated_on` advanced. The CDK persists parent state
only when the child stream is incremental — `commits` and `commit_files` are.

The proxy routes on a **flat** `clone_url` field, but Bitbucket nests clone
links in an array (`links.clone[]`, one entry per protocol). Every repositories
stream therefore hoists the `https` entry to a top-level `clone_url` in a
transformation. The API link is used rather than a URL derived from
`full_name`: deriving would duplicate knowledge of the host layout.

The streams are otherwise independent: each carries its own cursor and asks the
proxy for its own window. They join downstream by `sha`.

`repo_branches` is full refresh — bronze keeps the latest state per branch, and
head-movement history is derived by the `snapshot` / `fields_history` dbt
macros. Its `unique_key` excludes `head_sha`, so the ReplacingMergeTree
collapses to current state and a head move is a tracked-column change.

### Bitbucket-specific behaviours

- **Pagination** is cursor-style: the response carries an absolute `next` URL,
  consumed via `RequestPath`.
- **`fields=`** trims the response to the used properties; the full repository
  object is large and most of it is unused here.
- **No server-side "updated after" filter** exists on `/repositories`, so the
  cursor filters client-side. The listing is requested `sort=updated_on`
  (ascending) so the cursor still advances monotonically across pages.

### Cold repositories

The first request for an uncached repository gets `429` + `Retry-After` while
the proxy clones it in the background; every proxy stream retries on `429`.
`409` (the pinned snapshot was superseded) and `413` (repository over the
proxy's size cap) fail the stream instead — retrying the same page token would
loop.

## Open question: partial clone support

The proxy clones with `--filter=blob:none`. Whether Bitbucket Cloud honours it
is **unverified** — it is open question #2 in the design and needs a live
credential to settle. Git degrades gracefully if the server does not support
filtering (it warns and performs a full clone), so the connector works either
way; what changes is the proxy's first-clone cost and disk profile, since the
`repack --filter=blob:none` purge still returns the entry to a blobless
skeleton afterwards. Measure before sizing the cache volume for Bitbucket.

## Silver Targets

Not wired yet — same position as `git/gitlab-nocode`. The silver models must
match the CDK `git/bitbucket-cloud` connector's column types exactly
(`union_by_tag` UNION ALLs the branches; one mismatched type raises
`Code: 386 NO_COMMON_TYPE` and breaks the shared class for every source). Until
they land this connector is bronze-only.

## Not ported

Pull requests and their children (comments, reviewers, commits, diffstat) stay
on the vendor API — that data does not exist in git. Porting them is a
mechanical translation of the CDK connector's streams and is tracked separately.
