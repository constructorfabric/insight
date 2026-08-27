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
| project_fields | GraphQL `ProjectV2.fields`, per board | full refresh, one row per (field, collection day) |
| project_items | GraphQL `ProjectV2.items`, per board | card `updatedAt`, server-side `updated:>=DATE` (day granularity) |
| issue_fields | GraphQL `organization.issueFields` | full refresh |
| issue_types | GraphQL `organization.issueTypes` | full refresh |
| workflow_runs | `/repos/{r}/actions/runs` | created_at, weekly step windows |
| deployments | `/repos/{r}/deployments` | created_at, newest-first data feed |
| deployment_statuses | `/deployments/{id}/statuses` | windowed deployments parent |
| pull_request_timeline_events | GraphQL `pullRequest.timelineItems` | windowed PR parent, per PR |
| issue_timeline_events | GraphQL `issue.timelineItems` | updated_at issues parent, per issue |
| commit_authors | proxy `/v1/authors` → `/repos/{r}/commits/{sha}` | last_committed_date authors parent, one lookup per author |

**org_members is deliberately absent**: the deployed `git/github-directory`
connector already syncs the org roster for identity resolution. Folding it in
here is a separate migration.

**The two catalogue streams answer for identity, not volume.** A timeline event
names the native issue field it changed by node id, and the issue payload names
its type by display name only. Without `issue_fields` and `issue_types` neither
identifier resolves to anything an operator can read, and a rename silently
orphans whatever was bound to the old name. Both are organization-scoped, so
they cost one paginated query per configured organization per sync.

**Projects V2 boards cost one sweep per board, not per issue.** Both board
streams partition over the organization's boards, closed ones included: a
closed board holds history and never changes again. `project_items` must not
ride the issue cursor — moving a card does not bump the issue's `updated_at`,
so board movement on an otherwise untouched issue is invisible from the issue
side.

**Two things about the card filter.** `items(query:)` filters server-side on
the card's update date, and its granularity is a DAY — a full ISO timestamp
returns zero rows. Worse, an unparseable qualifier returns zero rows with **no**
GraphQL error, so a typo reads as "nothing changed": a green sync, an empty
stream, and no way to tell it from a quiet week. The cursor's `datetime_format`
is therefore `%Y-%m-%d`, so a timestamp cannot reach the query string, and
`assert_boards_yield_cards` catches the outcome if one ever does. One day is
re-read on every sync by design: the row key carries the day the card changed,
so a re-read rewrites the same row.

**Why a collection day sits in the board keys.** GitHub keeps no history of a
board field, an option rename, or a non-status card value. `updatedAt` says
when a value was last touched and never what it was before, so a succession of
snapshots is the only record — the day in the key makes each sync's view its
own row while a re-collection inside one day replaces it. Status history is the
exception: it comes from the issue timeline and is retroactively recoverable.

**Board status is not the issue's state.** An issue is open or closed; a board
column is a separate field, one per board, and the two are bound separately.
Nothing in the connector or in silver merges them.

**Re-syncing an existing deployment.** `issues` gained the hoisted
`issue_field_values_json` column and `issue_timeline_events` gained the field
and type identifiers. Both are cursored, so an item nobody touches again is
never re-read and would keep the old, emptier shape forever — clear the state
of those two streams once the new descriptor is live. The catalogue streams
have no state and need nothing.

`issue_timeline_events` now also gains `project_id`, `project_number` and
`was_automated`, and collects the two board-membership event types. Historical
events carry those columns only after a re-walk — issue timelines are retained
indefinitely, so clearing that stream's cursor recovers the whole board status
history. Card field values cannot be backfilled: the snapshot is all there is.

## What `github_start_date` bounds

One bound, one direction: every stream fetches everything from the start date to
now, and nothing before it. It is a floor on age, not a window size — a stream
that stops short of it discards data inside the range that was asked for, and
one that reaches behind it spends requests on data nobody wants.

Two consequences worth stating, because both are easy to get wrong:

- **A repository whose last push predates the start date is not walked at all.**
  It has no commit, file change or author inside the window, so the clone the
  proxy streams would spend on it buys nothing.
- **A pull request's reviews, commits and lifecycle events reach as far back as
  the pull requests themselves do**, as do a deployment's statuses. Each of
  those streams carries its own cursor and sets `incremental_dependency`, which
  is what makes that affordable: the first sync walks the whole window once,
  and later syncs resume from stored state instead of re-reading it.

Sub-resources of an item inside the window are fetched whole — every review of
an in-window pull request, including one submitted before the start date. The
item is what the bound applies to.

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
  `insight_source_id` — `github-main` in the shipped examples.** The seed and
  the resolver key an account on (source type, source id, member id); under
  different source ids the roster bindings and these claims describe different
  accounts, and a member with no published e-mail can end up as two persons.
  The id comes from the Secret's `insight.cyberfabric.com/source-id`
  annotation, so this is a deployment decision, not a code one.
- **An unmatched active account is not minted a person — it is queued.** This
  connector states no account id (see the last point), so the seed has nothing
  to write a binding from: minting here would create a person no account can
  ever belong to. An outside contributor, or a member committing under an
  address no roster carries, is instead counted under
  `accounts_skipped_no_source_id` and surfaced on the operator's review queue
  as `no_source_id`. Binding it to the right person (or excluding it as a bot)
  is the manual-resolution workflow, and an operator-authored binding wins
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
- GraphQL errors arrive as HTTP 200. GitHub types an exhausted budget both
  `RATE_LIMIT` and `RATE_LIMITED`, so the throttle predicates match either and
  fall back to the message; anything else fails with GitHub's own message.
- REST and GraphQL are metered as two separate hourly budgets. `api_budget`
  paces each one against its own `X-RateLimit-*` headers and waits out a
  window it exhausts. This is not something the error handlers could do:
  `DefaultErrorHandler` caps total backoff at 600 seconds and the manifest
  exposes no field to raise it, so a reset further out than that would
  exhaust the retries and fail the stream.

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
`class_git_item_events` for the lifecycle streams. Issues feed the
`class_task_*` classes, and a board's status column reaches them as its own
field, one per board. Card field values, board field definitions, CI and
deployments stay bronze-only. Column types must match the other git connectors
exactly: the classes union positionally.
