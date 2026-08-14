# github

Declarative GitHub connector on the git-cli-proxy: commit-level extraction
(commits, per-file changes, branches) is served from a bare blobless clone by
the proxy instead of one vendor API call per commit; everything git cannot
carry (PRs, reviews, comments, issues, projects, CI, deployments) stays on
api.github.com.

Supersedes the retired `github-v2` CDK connector: same `data_source`
(`insight_github`), same bronze namespace (`bronze_github`), same silver
staging models.

## Streams

| Stream | Source | Incremental |
|---|---|---|
| repositories | GitHub `/orgs/{org}/repos` | full refresh |
| commits | proxy `/v1/commits` | committed_date, per pushed repo |
| file_changes | proxy `/v1/file-changes` | committed_date, per pushed repo |
| branches | proxy `/v1/branches` | full refresh, per pushed repo |
| pull_requests | `/repos/{r}/pulls` (list) | updated_at, newest-first data feed |
| pull_request_reviews | `/pulls/{n}/reviews` | windowed PR parent, full refresh per PR |
| pull_request_commits | `/pulls/{n}/commits` | windowed PR parent, full refresh per PR |
| pull_request_diff_stats | GraphQL `repository.pullRequests` nodes | updated_at, newest-first data feed |
| pull_request_comments | `/repos/{r}/issues/comments` | updated_at, server-side `since` |
| pull_request_review_comments | `/repos/{r}/pulls/comments` | updated_at, server-side `since` |
| issues | `/repos/{r}/issues` | updated_at, server-side `since`; PRs filtered out |
| projects_v2 | GraphQL `organization.projectsV2` | full refresh |
| workflow_runs | `/repos/{r}/actions/runs` | created_at, weekly step windows |
| deployments | `/repos/{r}/deployments` | created_at, newest-first data feed |
| deployment_statuses | `/deployments/{id}/statuses` | windowed deployments parent |
| pull_request_timeline_events | GraphQL `pullRequest.timelineItems` | windowed PR parent, per PR |
| issue_timeline_events | GraphQL `issue.timelineItems` | updated_at issues parent, per issue |

**org_members is deliberately absent**: the deployed `git/github-directory`
connector already syncs the org roster for identity resolution. Folding it in
here is a separate migration.

## Error policy

- GitHub reports secondary rate limits as **403**, so throttle predicates
  (body message, `x-ratelimit-remaining: 0`) run before any 403 handling;
  what remains of 403 — plus 404/410 — on repo-scoped streams skips that
  repository and the sync continues. 401 fails fast as a config error.
- The proxy answers 429 + Retry-After while a repository is being cloned in
  the background; proxy requesters retry generously (a cold clone of a large
  repository runs many Retry-After cycles). 404/413 skip the repository;
  409 (superseded snapshot) fails the attempt — the rerun resumes from the
  stored cursor.
- GraphQL errors arrive as HTTP 200: rate-limit-typed errors back off,
  anything else fails with GitHub's own message.

## Known NULL columns

`pull_requests.merged_by` is on the detail response (`GET /pulls/{n}`) only and
is not fetched. Pull-request size is not on the pull-request row either: it
comes from `pull_request_diff_stats`, which reads the GraphQL list node.

## Cost shape

Steady state ≈ 6 calls/repo + 1 call per PR updated in the child window +
1 call per deployment created in the deploy window + O(changed rows). First
workflow_runs backfill goes back at most 90 days — GitHub's run retention —
paged in weekly windows because `created` queries cap at 1000 results per
window.

### PR size

`pull_request_diff_stats` carries per-pull-request `additions`, `deletions` and
`changed_files`. REST has these on the pull-request DETAIL response only, which
would cost one call per pull request; the GraphQL list node carries all three,
so a page of 100 pull requests costs a single request. The connection accepts
no `updatedAfter` argument, so the stream is a newest-first data feed like
`pull_requests`.

## Silver Targets

The nine staging models under `dbt/` feed the `class_git_*` classes, plus
`class_git_item_events` for the lifecycle streams. Issues, projects, CI and
deployments stay bronze-only. Column types must match the other git connectors
exactly: the classes union positionally.
