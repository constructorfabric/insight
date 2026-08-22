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
  name: insight-bitbucket-cloud-main
  labels:
    app.kubernetes.io/part-of: insight
  annotations:
    insight.cyberfabric.com/connector: bitbucket-cloud
    insight.cyberfabric.com/source-id: bitbucket-cloud-main
type: Opaque
stringData:
  bitbucket_username: "CHANGE_ME"
  bitbucket_token: "CHANGE_ME"
  bitbucket_workspaces: '["acme"]'
  bitbucket_start_date: "2026-01-01"
```

The proxy's address and bearer token are absent by design: the chart owns both
and reconcile injects them, so nothing here tracks the proxy's port or rotates
with its token.

`bitbucket_start_date` is the one bound: no stream fetches anything older, and a
repository nobody has touched since it is never listed, so never cloned.

### Fields

| Field | Required | Description |
|-------|----------|-------------|
| `bitbucket_username` | No | Atlassian account email/username. Set for personal API tokens (Basic `username:token`); leave empty for workspace/repository access tokens (Bearer). The clone username the proxy presents is derived from the same choice |
| `bitbucket_token` | Yes | API token with `repository:read` |
| `bitbucket_workspaces` | Yes | JSON array of workspace slugs |
| `bitbucket_api_base_url` | No | API base URL (default `https://api.bitbucket.org/2.0`) |
| `bitbucket_exclude_repositories` | No | JSON array of regular expressions matched against a repository slug; a match is never listed, cloned or walked. Matched with `search`, so anchor with `$` for "ends with" (e.g. `["\\.rospecs$"]`). Empty collects everything |
| `bitbucket_api_calls_per_hour` | No | Requests per hour spent against the Bitbucket API, as a string (default `"1000"`, the documented floor for repository data). Raise it where the token is granted more; proxy calls are not counted |
| `bitbucket_start_date` | Yes | Earliest date fetched, by every stream (YYYY-MM-DD); bounds the first-sync cost |

### Automatically injected

| Field | Source |
|-------|--------|
| `insight_tenant_id` | `tenant_id` from tenant YAML |
| `insight_source_id` | `insight.cyberfabric.com/source-id` annotation |

### Local development

```bash
cp src/ingestion/secrets/connectors/bitbucket-cloud.yaml.example src/ingestion/secrets/connectors/bitbucket-cloud.yaml
# fill in real values, then:
kubectl apply -f src/ingestion/secrets/connectors/bitbucket-cloud.yaml
```

## Streams

| Stream | Upstream | Sync Mode | Cursor |
|--------|----------|-----------|--------|
| `repositories` | Bitbucket `/2.0/repositories/{workspace}` | incremental | `updated_on` |
| `commits` | proxy `/v1/commits` | incremental, per repository | `committed_date` |
| `file_changes` | proxy `/v1/file-changes` | incremental, per repository | `committed_date` |
| `branches` | proxy `/v1/branches` | full refresh, per repository | — |
| `pull_requests` | Bitbucket `/pullrequests` (all states) | incremental | `updated_on` |
| `pull_request_comments` | `/pullrequests/{id}/comments` | windowed PR parent, full refresh per PR | — |
| `pull_request_commits` | `/pullrequests/{id}/commits` | windowed PR parent, full refresh per PR | — |
| `pull_request_diffstat` | `/pullrequests/{id}/diffstat` | windowed PR parent, full refresh per PR | — |
| `pull_request_activity` | `/pullrequests/{id}/activity` | windowed PR parent, full refresh per PR | — |
| `workspace_members` | `/workspaces/{w}/members` | full refresh | — |
| `pipelines` | `/repositories/{r}/pipelines` | newest-first data feed | `created_on` |
| `deployments` | `/repositories/{r}/deployments` | newest-first data feed | `created_on` |

### How the streams fit together

`repositories` fans out over the configured workspaces (`ListPartitionRouter`)
and is the incremental **parent**: the commit streams route through
`SubstreamPartitionRouter` with `incremental_dependency: true`, so a sync visits
only repositories whose `updated_on` advanced. The CDK persists parent state
only when the child stream is incremental — `commits` and `file_changes` are.

The proxy routes on a **flat** `clone_url` field, but Bitbucket nests clone
links in an array (`links.clone[]`, one entry per protocol). Every repositories
stream therefore hoists the `https` entry to a top-level `clone_url` in a
transformation. The API link is used rather than a URL derived from
`full_name`: deriving would duplicate knowledge of the host layout.

The streams are otherwise independent: each carries its own cursor and asks the
proxy for its own window. They join downstream by `sha`.

`branches` is full refresh — bronze keeps the latest state per branch, and
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

## The start-date bound

`bitbucket_start_date` means one thing — fetch nothing older — and every stream
has to enforce it in the REQUEST. Declaring it as a cursor's `start_datetime`
does not filter: the CDK only tracks state with it, so a stream that declares it
and nothing else still fetches everything. Two shared anchors carry the request
forms so an omission is visible:

| anchor | applies to | form |
|---|---|---|
| `repos_since_start` | every repository listing (12 of them) | `q=updated_on >= start_date`, server-side |
| `prs_since_start` | the four per-PR fan-out parents | `q=updated_on >= max(start_date, now - 30d)` |

Filtering repositories server-side is what bounds the clone cost: an untouched
repository is never returned, so the proxy never walks it. VERIFIED against the
live API — a workspace of 407 public repositories returns 47 for a cutoff six
weeks back, and no row below the cutoff.

One stream cannot comply. The deployments endpoint rejects `sort=created_on`
(400) and **accepts a `q` on `created_on` while silently ignoring it** — a
cutoff years in the future still returns every row. Its bound is therefore
applied to the records after the fetch (`is_client_side_incremental`): the
requests still walk a repository's whole deploy history, but nothing older than
the bound reaches bronze.

## Partial clone support: settled

Bitbucket Cloud honours `--filter=blob:none` (verified live — git-cli-proxy
PLAN §9.2). One sizing consequence remains: because patch text is stored, the
first backfill lazily pulls essentially every blob while paging history, so
the proxy cache should be sized for roughly full-clone weight per Bitbucket
repository during backfill; the post-serve purge returns entries to the
blobless skeleton afterwards.

### PR size

`pull_request_diffstat` is the only source of pull-request line counts:
Bitbucket has no GraphQL API and carries no diff totals on the pull request
itself, so the per-file diffstat rows are it. Grain therefore differs from the
GitHub and GitLab diff-stat streams, which get pull-request-level totals in one
query — silver sums these rows per pull request. A pull request whose branches
share no history has no computable diff and answers 400; that pull request is
skipped, the rest continue.

## Silver Targets

The staging models under `dbt/` carry `silver:class_git_*` tags and feed the
shared git classes. `union_by_tag` UNION ALLs every tagged branch positionally,
so each model's SELECT list matches the class column-for-column and type-for-type
— a mismatch raises `Code: 386 NO_COMMON_TYPE` and breaks the class for every
source that shares it.

| Stream | Staging model | Class |
|---|---|---|
| `repositories` | `bitbucket_cloud__repositories` | `class_git_repositories` |
| `branches` | `bitbucket_cloud__repository_branches` | `class_git_repository_branches` |
| `commits` | `bitbucket_cloud__commits` | `class_git_commits` |
| `file_changes` | `bitbucket_cloud__file_changes` | `class_git_file_changes` |
| `pull_requests` + `pull_request_diffstat` | `bitbucket_cloud__pull_requests` | `class_git_pull_requests` |
| `pull_requests` (participants) | `bitbucket_cloud__pull_requests_reviewers` | `class_git_pull_requests_reviewers` |
| `pull_request_comments` | `bitbucket_cloud__pull_requests_comments` | `class_git_pull_requests_comments` |
| `pull_request_commits` | `bitbucket_cloud__pull_requests_commits` | `class_git_pull_requests_commits` |

`pipelines`, `deployments` and `workspace_members` land in bronze only; no class
consumes them yet.

## Not in git

Pull requests and their children (comments, reviewers, commits, diffstat) come
from the vendor API — that data does not exist in a clone.
