# github

Declarative GitHub connector on the git-cli-proxy: commit-level extraction
(commits, per-file changes, branches) is served from a bare blobless clone by
the proxy instead of one vendor API call per commit; everything git cannot
carry (PRs, reviews, comments, issues, projects, CI, deployments) stays on
api.github.com.

Writes `data_source` `insight_github` into the `bronze_github` namespace, and
feeds the shared `class_git_*` silver models.

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
| commit_authors | proxy `/v1/authors` → `/repos/{r}/commits/{sha}` | last_committed_date authors parent, one lookup per author |

**org_members is deliberately absent**: the deployed `git/github-directory`
connector already syncs the org roster for identity resolution. Folding it in
here is a separate migration.

## Commit attribution

A commit carries a name and an e-mail; it never carries an account, because git
has no concept of one. Gold attributes activity to a person by e-mail, and the
roster can only claim an e-mail for a member who publishes one — so a member who
keeps their address private gets no commit attribution from the roster alone.

Two streams close that gap. `commit_authors` asks GitHub who each distinct
commit author is — the proxy enumerates them from the clone, one vendor lookup
per author — and reaches authors who never open a pull request.
`pull_request_commits` carries the same match inline on every PR commit,
including for addresses that are not public. `github__account_emails` collects
the pairs from both, adds the ones a noreply address names by construction, and
`github__identity_inputs` publishes them as e-mail claims against the account.

The claims feed the identity service's persons-seed, which decides what an
account is. No roster connector is required for attribution to work: an
account whose claimed e-mail matches a person the roster already knows — from
`github-directory`, BambooHR, Entra, or any other source — is linked to that
person and gets its binding minted right there, which is how a git-only
source joins an HR-rostered organization.

Three things to know before deploying:

- **When `github-directory` is deployed, both connectors must share one
  `insight_source_id`.** The seed and the resolver key an account on (source
  type, source id, login); under different ids the roster bindings and these
  claims describe different accounts, and a member with no published e-mail
  can end up as two persons.
- **An unmatched active account is minted as a new person.** An outside
  contributor, or a member committing under an address no roster carries,
  becomes a fresh person and shows up in the seed's `accounts_minted_new`
  counter. Merging a minted person into the right one (or excluding it) is the
  operator's manual-resolution workflow, and an operator-authored binding wins
  every later seed.
- **Only `value_type='email'` rows are emitted, never the `value_type='id'`
  binding.** Bindings are the seed's decision to make; emitting them here
  would assert membership this connector cannot know.

Commits GitHub cannot match at all (`author` comes back null — CI and service
identities) reach no account and stay unattributed.

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
