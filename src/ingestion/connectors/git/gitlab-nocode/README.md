# GitLab (git-cli-proxy) Connector

Declarative GitLab connector that extracts commit-level data through the
[git-cli-proxy](../../../../../docs/components/connectors/git/git-cli-proxy/specs/DESIGN.md)
instead of the vendor API, removing the one-API-call-per-commit cost that drives
the CDK git connectors into rate limits. Repository discovery still uses the
GitLab API — that is one call per page of projects, not per commit.

Auth: GitLab personal/project access token (`PRIVATE-TOKEN` header) for the API;
the same token is forwarded to the proxy per request as HTTP basic credentials
for the clone, and a separate bearer token authenticates the proxy itself.

## Prerequisites

1. A GitLab access token with `read_api` (project discovery) and
   `read_repository` (the clone the proxy performs on its behalf). Personal,
   project or group tokens all work.
2. A reachable git-cli-proxy deployment and its bearer token. In-cluster the
   umbrella composes both (`insight-git-cli-proxy-config`); the proxy accepts
   traffic only from the namespaces its NetworkPolicy allows.

## K8s Secret

```yaml
apiVersion: v1
kind: Secret
metadata:
  name: insight-gitlab-nocode-main
  labels:
    app.kubernetes.io/part-of: insight
  annotations:
    insight.cyberfabric.com/connector: gitlab-nocode
    insight.cyberfabric.com/source-id: gitlab-nocode-main
type: Opaque
stringData:
  gitlab_url: "https://gitlab.example.com"
  gitlab_token: "CHANGE_ME"
  git_proxy_url: "http://insight-git-cli-proxy:8085"
  git_proxy_token: "CHANGE_ME"
  gitlab_start_date: "2020-01-01"
```

### Fields

| Field | Required | Description |
|-------|----------|-------------|
| `gitlab_url` | Yes | Base URL, no trailing slash, no `/api` suffix |
| `gitlab_token` | Yes | Access token; needs `read_api` + `read_repository` |
| `git_proxy_url` | Yes | git-cli-proxy base URL. No default — a wrong value must fail `check`, not fall back |
| `git_proxy_token` | Yes | Bearer token the proxy requires on every `/v1` request |
| `gitlab_start_date` | No | Earliest date for the initial sync (default `2020-01-01`) |
| `gitlab_page_size` | No | GitLab API page size, max 100 |
| `proxy_page_size` | No | Rows per proxy page (default 500) |
| `gitlab_include_patch` | No | Store per-file diff text (default true) |
| `gitlab_max_patch_bytes` | No | Truncation cap per file diff (default 1 MiB) |

### Automatically injected

| Field | Source |
|-------|--------|
| `insight_tenant_id` | `tenant_id` from tenant YAML |
| `insight_source_id` | `insight.cyberfabric.com/source-id` annotation |

### Local development

```bash
cp src/ingestion/secrets/connectors/gitlab-nocode.yaml.example src/ingestion/secrets/connectors/gitlab-nocode.yaml
# fill in real values, then:
kubectl apply -f src/ingestion/secrets/connectors/gitlab-nocode.yaml
```

## Streams

| Stream | Upstream | Sync Mode | Cursor |
|--------|----------|-----------|--------|
| `repositories` | GitLab `/projects` | incremental | `last_activity_at` |
| `commits` | proxy `/v1/commits` | incremental, per repository | `committed_date` |
| `commit_files` | proxy `/v1/file-changes` | incremental, per repository | `committed_date` |
| `repo_branches` | proxy `/v1/branches` | full refresh, per repository | — |

### How the incremental streams fit together

`repositories` is the incremental **parent**: the commit streams route through
`SubstreamPartitionRouter` with `incremental_dependency: true`, so a sync visits
only repositories whose `last_activity_at` advanced. The CDK persists parent
state only when the child stream is incremental — `commits` and `commit_files`
are, which is what makes the dependency hold.

The streams are otherwise **independent**: each carries its own cursor and each
asks the proxy for its own window. Nothing is shared between them at runtime
(unlike the CDK connector, where `file_changes` reads a temp file written by
`commits` in the same process). They join downstream by `sha`.

`repo_branches` is full refresh: bronze keeps the latest state per branch, and
head-movement history is derived by the `snapshot` / `fields_history` dbt macros
— the same pattern user profiles use. Its `unique_key` therefore excludes
`head_sha`, so the ReplacingMergeTree collapses to current state and a head move
is a tracked-column change.

### Cold repositories

The first request for an uncached repository gets `429` + `Retry-After` while
the proxy clones it in the background; every proxy stream retries on `429`.
`409` (the pinned snapshot was superseded) and `413` (repository over the
proxy's size cap) fail the stream instead — retrying the same page token would
loop.

## Silver Targets

Not wired yet. The silver models map to the same `class_*` tables the CDK
`git/gitlab` connector feeds, and their column types must match its models
exactly (`union_by_tag` UNION ALLs the branches; one mismatched type raises
`Code: 386 NO_COMMON_TYPE` and breaks the shared class for every source). Until
those models land this connector is bronze-only and contributes nothing to the
dashboards.

## Not ported

Merge requests, notes and approvals stay on the vendor API — that data does not
exist in git. Porting them is a mechanical translation of the CDK connector's
streams and is tracked separately from the proxy work.
