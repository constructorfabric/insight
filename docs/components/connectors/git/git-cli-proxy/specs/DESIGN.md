# DESIGN — Git CLI Proxy

> Status: draft for review
> Scope: implementation design for the git-cli-proxy service and the consumption contract for nocode git connectors
> Concept: [constructorfabric/insight#2224](https://github.com/constructorfabric/insight/issues/2224)

## 1. Purpose & Scope

The CDK git connectors (github-v2, gitlab, bitbucket-cloud) extract commit-level
data through vendor APIs, and the heaviest streams cost **one API call per
commit** (`file_changes`: GitHub `GET /repos/{o}/{r}/commits/{sha}`, Bitbucket
`diffstat/{sha}`). Every backfill and every large sync therefore runs into
vendor rate limits; the worst case (Bitbucket, ~1000 req/h) turns a 50k-commit
repository into a ~50-hour backfill.

Everything those streams emit is derivable from a git clone in minutes. This
document designs:

1. **git-cli-proxy** — a single vendor-agnostic HTTP service that maintains a
   disk-bounded cache of bare repository clones and serves commit-level data
   (commits, per-file changes, branches) as paginated JSON.
2. **The consumption contract** — how a per-vendor **nocode (declarative
   low-code CDK)** connector mixes vendor-API streams and proxy streams in one
   manifest.

Out of scope: PR/review streams (they stay on vendor APIs — that data does not
exist in git), the silver/gold dbt layer (unchanged), and the deferred work
tracked in PLAN §9.

### 1.1 Division of labor

| Concern | Owner |
|---|---|
| Sync cursors, incremental state | Airbyte state (connector) |
| Repository discovery, PR data | Vendor API (connector streams) |
| Commit/file/branch extraction | git-cli-proxy |
| Envelope fields (`unique_key`, `tenant_id`, `source_id`, `data_source`, `collected_at`) | Connector transformations (`AddFields`) |
| Git credentials | Connector config → per-request headers; never stored by the proxy |
| Disk | Proxy-managed LRU cache, rebuildable at any time |

The proxy is **stateless with respect to sync correctness**: it may lose any
repository at any moment and re-clone on demand. What it may never do is
partially delete a repository that is in use (§3.5).

## 2. Architecture Overview

```
                       ┌────────────────────────────┐
                       │   nocode connector (per    │
                       │   vendor: github/gitlab/   │
                       │   bitbucket)               │
                       └─────┬───────────────┬──────┘
        vendor API           │               │        git-cli-proxy API
  repositories, pull_        │               │   commits, file_changes,
  requests, pr_* streams     │               │   branches
                             ▼               ▼
                  ┌──────────────┐   ┌──────────────────────┐
                  │  vendor API  │   │     git-cli-proxy    │
                  │ (api.github… │   │  bare blobless clones│
                  └──────────────┘   │  on PVC, LRU-evicted │
                                     └──────────┬───────────┘
                                                │ git fetch (https)
                                                ▼
                                     ┌──────────────────────┐
                                     │  origin (github.com, │
                                     │  gitlab, bitbucket)  │
                                     └──────────────────────┘
```

One proxy deployment serves all vendors, all tenants, all sources. Vendor
differences are confined to (a) the clone URL the connector passes and (b) the
username convention for HTTP basic auth against origin (§3.7).

## 3. Proxy Service Design

### 3.1 Repository cache

**Clone form.** `git clone --bare --filter=blob:none --no-tags <url>` —
commits and trees only, no working tree, no blobs (~10–20% of full repo size).
The origin remote becomes a *promisor remote*: any missing blob can be fetched
later by OID. `--no-tags` is not a size optimisation: a tag is a ref like any
other, so a tag pointing into a deleted branch keeps that branch's commits
reachable and the enumeration keeps emitting them long after the branch is
gone.

**Cache key.** `(tenant_id, source_id, clone_url)`. The on-disk directory name
is the SHA-256 of the key — repository names or URL fragments are **never**
used to build paths (path traversal via hostile repo names; case-insensitive
filesystem collisions). Identical clone URLs under two sources are two
isolated cache entries by design: access rights differ per source, and a
shared entry would let a source with a revoked token keep reading data fetched
with someone else's token.

**Layout.**

```
<data_dir>/repos/<sha256(tenant_id \0 source_id \0 clone_url)>/
    repo.git/        # bare, blobless
    meta.json        # clone_url, tenant_id, source_id, sizes,
                     # last_fetched_at, last_accessed_at, generation
```

`meta.json` is debugging aid + LRU/freshness bookkeeping. Losing it is
equivalent to evicting the entry.

**Remote URL hygiene.** `remote.origin.url` is stored **clean** — no
credentials ever appear in it (§3.7). This is what makes token rotation free:
cloned objects do not depend on the credential, only the transport does.

### 3.2 Freshness: fetch-if-stale

There is **no prefetch/warm-up mechanism**. Freshness is a local, per-request
rule:

1. Repo not on disk → enqueue clone, join it, wait up to `INLINE_WAIT`;
   serve if it finishes, else `429 Retry-After` (§4.4) while it continues.
2. Repo on disk, `now − last_fetched_at > max_staleness` → run
   `git fetch --prune` (single-flight), then serve.
3. Otherwise → serve immediately from the local clone.

`max_staleness` comes from the `X-Max-Staleness` request header, defaulting to
`DEFAULT_MAX_STALENESS_SECONDS`. Because one sync's streams (`commits`,
`file_changes`, `branches`) hit the same repo within minutes, the first stream
pays the fetch and the rest read the same snapshot — which is also the
consistency property we want (all streams of a sync observe one origin state).

A no-op fetch (nothing changed) is one ref-advertisement round trip —
milliseconds — so even a staleness-window miss costs almost nothing.

**Bounded wait, then `429`.** Preparation runs as a background task that owns
the entry's write lock; the request joins it and waits at most
`INLINE_WAIT` (15 s). A fast fetch therefore completes inline and the caller
gets data; a cold clone exceeds the wait and the caller gets
`429` + `Retry-After: 30`, while the clone keeps running to completion. This is
what reconciles single-flight with the `429` contract (§4.4): waiters never
hang for the length of a clone, and a client giving up never cancels the work.
Single-flight guarantees concurrent requests for one repo trigger at most one
clone/fetch regardless of which of them time out.

**Credential continuity.** Each entry records a one-way fingerprint of the
credentials that last proved origin access (`meta.json`). A warm read is served
only to a caller whose credentials match; a mismatch forces a fetch. Rotation
costs one fetch, never a re-clone. The cache key alone is never treated as an
authorization claim (§3.7).

This is continuity, **not** current authorization: it proves the caller
presents the same credential that last succeeded, never that the credential is
still valid at origin. A revoked or scope-reduced credential keeps reading
cached data for up to `max_staleness` — and **without bound** for as long as a
caller keeps paginating, because §4.1 guarantees a request carrying a page
token never fetches. Revocation is therefore not enforced by this service;
where that matters, the control is the freshness window and the lifetime of a
page sequence, both of which an operator sets.

### 3.3 Blob lifecycle

Blobs exist on disk only transiently, for the commit window being served:

1. **Enumerate** new commits with tree-only data:
   `git log --no-walk --raw --no-abbrev` — filenames and A/M/D/R statuses need
   **no blobs**. Two flag details are load-bearing: `diff-tree` with several
   revisions diffs *between* them instead of per commit (so the multi-commit
   form is `log --no-walk`), and raw output abbreviates OIDs unless
   `--no-abbrev` is given — abbreviated OIDs cannot be fetched (`--full-index`
   affects only patch headers).
2. **Prefetch in batch** the blob OIDs referenced by the window's diffs
   (both sides of each changed path; the all-zero OID marks an absent side and
   is skipped): one `git fetch origin <oid>...` against the promisor remote —
   never rely on git's implicit one-blob-per-roundtrip lazy fetching.
3. **Compute** `--numstat` with `--raw` (counts from numstat, exact statuses
   from the tree diff), rename detection (`-M`), and patches locally.
4. **Purge**: return the repo to its skeleton size. Run after serving a
   window when the repo has grown past its skeleton baseline, and during
   eviction (§3.6). Three details are load-bearing, and without any one of
   them the purge frees nothing at all:
   - `repack` repacks promisor packs *separately* and never applies
     `--filter` to them. In a blobless clone every pack is a promisor pack —
     the clone's own and one per lazy fetch — so the `*.promisor` markers come
     off before the repack and go back on after. They must go back on: the
     objects did come from origin, and git has to keep tolerating the ones the
     purge just dropped.
   - `--filter` alone writes the filtered-out objects to a second pack beside
     the first. `--filter-to=<dir>` puts them somewhere the purge can delete.
   - `--no-write-bitmap-index` is mandatory: the filter splits objects across
     packs while bitmap writing assumes a single pack, so with
     `repack.writeBitmaps` enabled the repack fails outright.

   The purge takes the entry's write lock without waiting for it. A reader
   holding the entry is served, not repacked under; the eviction path (§3.6)
   is the backstop. Whether or not a purge follows, the check re-measures the
   entry and writes the result to `meta.json` — a prefetch that grows an entry
   and reports nothing leaves the reclaim planner believing every entry is
   skeleton-sized, so it never plans the cheap tier and evicts warm
   repositories instead. Re-measurement is throttled per entry, or a
   two-hundred-page walk pays a full directory walk per page.

Blob prefetch is always required: numstat, `patch_id` computation, and patch
text all need blob content. `include_patch=false` (§4.2) only skips patch
text assembly and transfer — bronze retention policy, not a disk optimization
on the proxy side.

Initial backfills of large repositories are processed in **windows** (page-
sized commit ranges): prefetch → serve → purge, keeping peak disk usage
bounded regardless of repository history size.

**Promotion out of the partial clone.** Some origins serve a blobless clone
and then refuse explicit promisor wants for individual objects — a GitLab
fork-network object pool behaves this way — so every batch prefetch fails, on
every retry, permanently. That is a property of the repository, not a transient
fault, so the entry is healed once: drop the filter, refetch, remove the
`*.promisor` markers, repack, and record `full_clone` in `meta.json`.

A promoted entry leaves the blobless regime for good. Its blobs cannot be
fetched back, so it is exempt from the purge tier and only whole-entry eviction
reclaims it — `reclaimable_by_purge` is `size − skeleton`, and for a promoted
entry the full clone *is* the skeleton. Sizing must account for this (§6). A
first page retries transparently after promotion; a continuation gets the
standard `409`, because promotion bumps the generation its cursor is pinned to.

### 3.4 Git invocation rules

Every git subprocess invocation MUST:

- carry an explicit `--git-dir` (equivalently `-C <path>` for bare dirs) —
  process cwd is treated as undefined; the service never calls `chdir`;
- receive a **clean environment**: inherited `GIT_DIR`/`GIT_WORK_TREE` are
  stripped; `GIT_CONFIG_GLOBAL=/dev/null`, `GIT_CONFIG_SYSTEM=/dev/null`
  (no shared config between tenants); `GIT_TERMINAL_PROMPT=0` (fail, never
  prompt);
- receive credentials via environment only (§3.7);
- run with `GIT_TRACE*` disabled in production (curl traces dump HTTP
  headers).

The image needs a git that supports `repack --filter` together with
`--filter-to` (§3.3), which Debian bookworm's own git does not. Rather than
trusting a version number, the service runs that exact invocation against a
throwaway repository at boot and refuses to start if git rejects it: a git
without the purge is a failed deploy, not disk pressure discovered weeks
later.

### 3.5 Concurrency model

Cross-repo requests are independent (bare repos share nothing on disk; no
`alternates` in v1 — deferred, PLAN §9). Resource pressure is capped by a
global semaphore on heavy operations (clone/fetch/repack), sized in config.

Within one repo, a keyed read-write lock plus a reader refcount:

| Operation | Lock |
|---|---|
| Read (`log`, `diff-tree`, `for-each-ref`) | shared with other readers only |
| `fetch` / initial clone | fully exclusive against readers, single-flight for waiters. A fetch updates many refs, so a concurrent `--branches` walk could otherwise straddle two ref sets and break the one-snapshot-per-sync property of §3.2 |
| `repack -a -d --filter` (deletes packs) | fully exclusive: only at zero reader refcount |
| Eviction (delete directory) | fully exclusive: only at zero refcount, never for a just-requested repo |

Every git invocation runs under one of three time budgets, and the ordering
between them is the contract, not the exact numbers:

| Budget | Covers | Why it is separate |
|---|---|---|
| read (5 min) | `log`, `for-each-ref`, `rev-list`, `patch-id` | Holds the entry's READ lock. A stalled read — most often a lazy promisor fetch behind what looks like local plumbing — blocks fetch and eviction for its whole budget while other streams retry past the connector's own ceiling and fail the sync. |
| prefetch (10 min) | the per-page blob prefetch | Network, but bounded by one page. |
| heavy (30 min) | clone, fetch, repack, promotion | Whole-repository work, and genuinely slow. |

Initial clone lands in a temp directory and is atomically renamed into place.

**Invariant:** the proxy may lose a whole repo and re-clone it; it must never
let a reader observe a partially deleted one.

### 3.6 Disk self-management

The proxy owns its disk budget; kubelet-level enforcement (pod eviction) is
the last-resort backstop, not the mechanism.

- **Budget**: `disk_budget_bytes` — required, no default (fail-fast). Must
  be set 10–15% below the volume size to leave headroom for transient packs
  and git temp files (avoiding ENOSPC mid-pack-write).
- **Accounting**: per-repo size recomputed after every clone/fetch/repack,
  stored in `meta.json`; total = sum. That sum is a LOWER bound — it sees only
  published entries, never a clone still staging under `tmp/`, nor another
  writer on the mount. `statvfs` supplies the second view, and effective free
  space is the minimum of the two, so effective usage is the maximum. Both
  feed the same watermark.
- **Watermarks with hysteresis**: crossing the high watermark (e.g. 85% of
  budget) triggers eviction down to the low watermark (e.g. 65%).
- **Two-tier eviction**, cheap to expensive, in LRU order by
  `last_accessed_at`:
  1. blob purge (`repack --filter=blob:none`) — repo stays warm;
  2. full directory delete.
  Pinned repos (refcount > 0) and just-requested repos are skipped
  (anti-thrashing).
- **Admission control**: before clone/fetch, check headroom; evict
  synchronously if needed; if nothing can be evicted respond `429`.
- **Reservation**: admission decides and reserves in one step, holding the
  reservation for as long as the operation runs. Checking current usage alone
  answers "is the cache full right now", which several cold clones can all be
  told no to before any of them writes a byte — and they then overrun the
  budget together, surfacing as a git or I/O failure where the caller should
  have seen a `429`. Each operation reserves the distance between the entry's
  current size and the per-repo cap, which is the most it can still add.
- **Per-repo cap**: `max_repo_bytes` — a clone, fetch or promotion runs
  under a watcher that measures the tree it is filling and KILLS the child on
  breach. Measuring afterwards is too late: the disk the cap exists to protect
  has already been spent, and a repository an order of magnitude over the cap
  can fill the volume before anything objects. The watcher polls, so the cap
  can be overshot by one interval's worth of transfer; the post-hoc check
  catches that remainder. Either way the caller gets a **permanent**
  (non-retryable) error. Protects the shared cache from a single oversized
  monorepo.

Degradation mode: warm-repo reads always keep working; only cold clones queue
behind `429`. Disk-full is a normal steady state for an LRU cache — the alert
condition is a sustained rise in eviction/429 rates (budget too small for the
working set), not utilization itself.

### 3.7 Credential handling

Git credentials travel **connector → proxy → git subprocess env** and exist
nowhere else:

- The connector sends `X-Git-Username` / `X-Git-Token` on every request
  (values from connector config, i.e. the Airbyte source's secrets).
- The proxy passes them to git via environment only — either a credential
  helper reading env vars:

  ```
  git -c credential.helper='!f(){ printf "username=%s\npassword=%s\n" "$GIT_USER" "$GIT_TOKEN"; }; f' fetch …
  ```

  or the argv-invisible env-config form (`GIT_CONFIG_COUNT` /
  `GIT_CONFIG_KEY_0=http.extraheader` / `GIT_CONFIG_VALUE_0=…`, git ≥ 2.31).
  Both keep tokens out of `ps`, out of on-disk config, and out of git error
  messages (the stored remote URL is clean).
- **Rotation**: since no credential is persisted, the next request simply
  carries the new token; a rotated token never requires re-cloning.
- Username conventions per vendor (the only vendor-specific knowledge in the
  transport): GitHub — any username (e.g. `x-access-token`) + PAT; GitLab —
  `oauth2` + token; Bitbucket Cloud — `x-token-auth` + token. The connector
  supplies the username; the proxy does not hardcode vendor rules.
- No request header is logged anywhere in this service or its host, so there
  is nothing to redact at the log boundary. The controls that do exist are
  structural: `GitCredentials` and the gear config both carry `Debug` impls
  that print the token as `<redacted>`, credentials reach git through
  `GIT_CONFIG_KEY_n` and never through argv, and the stored remote URL holds
  no userinfo. Should an access log ever be added, it must be added with a
  header deny-list, not after one.
- **`X-Tenant-Id` / `X-Source-Id` are cache partition inputs, not
  authorization claims.** One deployment-wide bearer token authenticates *the
  caller class*, not a tenant, so those headers alone must never grant access
  to a warm entry: the credential continuity check of §3.2 is what binds a
  caller to an entry. A caller that cannot satisfy origin auth for a repository cannot
  read its cached data, whatever tenant/source it names.
- Proxy-level authentication (service-to-service): static bearer token via
  `Authorization`, provisioned by deployment; independent from git
  credentials.

## 4. HTTP API (v1)

### 4.1 Conventions

- Repo identity: `repo=<clone_url>` query parameter (URL-encoded). Only
  `http://` and `https://` origins are accepted. git reads the URL as a
  transport selector — `ext::` runs a shell command, a bare path reads the
  pod's own filesystem — so the value is parsed at the boundary and anything
  else is `400`. Embedded credentials are refused too: they would override the
  header the service injects and reach git's stderr.
- `sha=<id>[,<id>…]`: optional explicit selection on `/v1/commits` and
  `/v1/file-changes`, taking full ids or hex prefixes between 7 characters and
  the length of an object id. A prefix selects every commit it matches; it is
  not resolved against the repository and is not required to be unique. A
  debugging and incident-review affordance — the sync path pages by cursor
  instead.
- Required headers: `X-Tenant-Id`, `X-Source-Id`, `X-Git-Username`,
  `X-Git-Token`, `Authorization` (proxy bearer). Optional:
  `X-Max-Staleness: <seconds>`.
- Pagination: list endpoints order rows **ascending by a two-part key** and
  return an opaque `next_page_token` encoding the last emitted pair, the
  **snapshot generation** it came from, and the **incarnation** of the clone
  that produced it. Both are needed: the generation restarts at `1` for every
  clone, so an entry evicted and re-cloned between two pages would otherwise
  hand the continuation a matching generation over a different history. The key is `(committed_date, sha)` for
  `/v1/commits` and `/v1/file-changes`. `/v1/branches` is the exception: a
  branch has no date to walk by, so it orders by `(name, "")` — the second
  component is the empty string, since branch names are unique within a
  snapshot and nothing is left to break a tie on. Ascending
  order makes pagination deterministic and is friendly to Airbyte cursor
  checkpointing. `page_size` default 1000, max 10000. The order is by
  INSTANT: `%cI` carries the committer's own UTC offset, so two commits from
  different time zones do not compare correctly as text, and both the walk and
  the served window normalise before ordering.
- **Snapshot pinning.** The generation and incarnation in the token bind a
  page sequence to one ref snapshot: a request that carries a token **never
  refreshes refs** (it is served from that generation, whatever the staleness
  window says), and only the first page of a walk can trigger a fetch. A
  continuation may still ask origin for BLOBS it does not have locally
  (§3.3) — those are requests for exact object ids and move no ref, so the
  snapshot the page is sliced from cannot change underneath it. If the pinned generation is
  gone (a fetch by another caller, an eviction, a repack), the request fails
  `409 snapshot_changed` naming the live generation rather than silently
  splicing two snapshots — which would let a commit that became reachable
  mid-walk be skipped. The consumer restarts the stream slice from its own
  cursor; bronze is append-only, so replayed rows dedup downstream.
  INVARIANT: the token selects a position, never a repository or a tenant.
  It carries a binding to the cache entry it was minted from, so a cursor
  cannot continue a *different* repository that happens to be at the same
  generation; a mismatch is `400`, not `409`, because resuming from a stored
  cursor cannot fix it. The binding grants nothing — access is decided by the
  credential fingerprint on every request, token or not.
- Responses: `{ "items": [ … ], "next_page_token": "…" | null }`.
- The proxy returns **pure git data**. Envelope fields are the connector's
  job (§5.4).

### 4.2 Endpoints

#### `GET /v1/commits`

`?repo&since&page_size&page_token`

All branches (`--branches`, never `--all`: a tag pinning a deleted branch
would otherwise keep resurrecting its commits), deduplicated by sha, merge
commits included. `since`
filters on `committed_date ≥ since` over everything currently reachable —
after a force-push this re-emits rewritten commits, which is the correct
append-only bronze behavior (dedup is downstream RMT's job).

| Field | Type | Source |
|---|---|---|
| `sha` | string | `git log` |
| `message` | string | subject + body |
| `authored_date`, `committed_date` | ISO-8601 | author/committer dates |
| `author_name`, `author_email` | string | |
| `committer_name`, `committer_email` | string | |
| `parent_hashes` | string[] | |
| `is_merge` | bool | `len(parents) > 1` |
| `additions`, `deletions`, `changed_files` | int | numstat pass (shared with file-changes) |
| `is_in_default_branch` | bool | one `rev-list` over the `HEAD` symref, intersected with the page |
| `patch_id` | string \| null | `git patch-id --stable` over the commit's full diff, null for a merge commit, which has no single diff — canonical, whitespace- and hunk-order-insensitive hash for duplicate/cherry-pick detection. Always computed from the untruncated diff, so dedup never depends on stored patch text. |

`is_in_default_branch` is evaluated **at emit time**, against the snapshot
the page was served from. A commit first seen on a feature branch is emitted
`false`, and merging it later does not change its committed date, so a
date-cursored incremental sync never revisits it and the bronze row keeps
`false` permanently. Under squash-merge the merge is a new commit and is
emitted `true`, so the field is right for what the default branch actually
contains; under fast-forward or a true merge the original commits are the ones
that matter, and they stay wrong.

Consumers needing *present-tense* reachability must derive it downstream from
`parent_hashes` and `is_merge`, which this endpoint already emits — no proxy
change is required for that. The connector's `lookback_window` also re-reads a
trailing window each sync, which corrects anything merged inside it. Re-emitting
or tombstoning superseded rows is deliberately out of scope.

The endpoint no longer reports every branch containing a commit. That form cost
one `git branch --contains` per commit in the page — up to ten thousand
subprocess spawns for one request, each walking reachability across all refs —
and it answered a question no consumer asked. Per-branch reachability is
therefore not available here; the enumeration itself still walks every branch,
so commits off the default branch are still extracted.

#### `GET /v1/file-changes`

`?repo&since&include_patch=true&max_patch_bytes=1048576&page_size&page_token`

One row per file × commit, **non-merge commits only** (parity with current
connectors), rename detection on (`-M`).

`include_patch` defaults to **true**: patch text is stored so that LOC can be
recomputed post-factum under additional filters (blank lines, comments, …)
without re-extraction. `max_patch_bytes` is a safety cap for pathological
diffs (lockfiles, vendored trees); truncation is flagged per row so consumers
never silently under-count. Commit-level dedup does not depend on patch text
(see `patch_id` on `/v1/commits`).

| Field | Type | Notes |
|---|---|---|
| `sha`, `committed_date` | | join keys to commits |
| `filename` | string | |
| `previous_filename` | string \| null | renames |
| `status` | string | `added` \| `modified` \| `removed` \| `renamed` \| `copied` \| `type_changed` — git's raw statuses. Rename AND copy detection are both requested (`-M -C`); with `-M` alone git never emits a copy and every copy arrives as an addition |
| `additions`, `deletions`, `changes` | int \| null | null for binary |
| `is_binary` | bool | |
| `patch` | string \| null | per-file unified diff; truncated at `max_patch_bytes` |
| `patch_truncated` | bool | true when `patch` was cut at the cap |

`page_size` bounds COMMITS, and a commit fans out to one row per changed file,
each carrying patch text — so the response carries its own bounds as well: it
stops at a commit boundary once it would exceed the row or total-patch-byte
cap, and the cursor names the last commit emitted in full. A page can
therefore return fewer commits than `page_size`, and a caller must page until
`next_page_token` is null rather than until a short page. A commit whose own
rows exceed a cap is emitted whole and over budget: refusing it would leave
the walk unable to advance past it, making the repository permanently
unsyncable. `max_patch_bytes` is capped at 8 MiB.

#### `GET /v1/branches`

`?repo&page_size&page_token` — full refresh, but paginated: branch counts are
unbounded, so the response must be too. The walk orders by **name** ascending
(there is no date cursor here), which is why the page token carries a generic
two-part ordering key rather than a date and a sha.

| Field | Type |
|---|---|
| `name` | string |
| `head_sha` | string |
| `head_committed_date` | ISO-8601 |
| `is_default` | bool (origin `HEAD` symref) |

#### `GET /healthz`

Served by the `api-gateway` host gear, not by this service — the platform owns
that route for every gear, and one service overriding it would diverge from
the rest. Liveness is process health, readiness is HTTP serving; neither fails
on disk pressure, because warm reads keep working when the cache is full. The
disk figures are gauges (§4.3), not a health payload.

### 4.3 Prometheus metrics

`git_proxy_disk_used_bytes`, `git_proxy_disk_budget_bytes`,
`git_proxy_repos`, `git_proxy_evictions_total{tier=blob|full}`,
`git_proxy_admission_rejects_total`, `git_proxy_cold_clones_total`,
`git_proxy_fetches_total{result=noop|updated|error}`,
`git_proxy_request_duration_seconds{endpoint,status}`,
`git_proxy_response_size_bytes{endpoint}`.

`git_proxy_repos` carries no `_total` suffix: it is a gauge, and the exporter
appends that suffix only to counters. `endpoint` is the matched route
template, never a request path, so the label set stays bounded by the route
table. Requests refused by the bearer check are recorded too — a token
rotation gone wrong shows up as `status="401"`, not as silence.

### 4.4 Error semantics

| Status | Meaning | Connector behavior |
|---|---|---|
| `400` | missing identity/credential header, missing or unusable `repo`, malformed page token, malformed `sha`, non-numeric `X-Max-Staleness`, or a query string that does not parse | permanent — a config or wiring bug |
| `401` | proxy bearer token missing or wrong (`PROXY_TOKEN_REJECTED`), or origin rejected the supplied git credentials (`ORIGIN_CREDENTIALS_REJECTED`) | fail the sync (config error) |
| `404` | repo not found at origin | fail the slice; parent record is stale |
| `409` | the pinned snapshot is gone (§4.1), including when a promotion (§3.3) or a fetch bumped the generation the cursor pinned | restart the stream slice from its cursor |
| `413` | repo exceeds `max_repo_bytes` | permanent — do not retry |
| `429` + `Retry-After` | preparation did not finish within `INLINE_WAIT`, admission was rejected, or a caller presenting unproven credentials lost the re-proof race to a concurrent caller | retry with backoff (declarative error handler) |
| `5xx` | internal/transient | retry with backoff |

Error bodies are RFC 9457 `application/problem+json`, produced by the
platform's canonical-error types (`DNA REST/API.md §7`) rather than a shape of
this service's own: no error type of ours crosses the API boundary.
Caller-actionable failures name the offending field in
`context.field_violations`; internal failures carry a generic `detail` and the
diagnosis goes to the log only — the crate `serde`-skips it, so paths and cache
internals cannot reach the wire.

Two deviations are deliberate. `413` has no canonical category, so the envelope
is canonical and only the status is overridden; and the catalogue carries the
retry hint for `429` in the body, while the connectors' backoff reads the
`Retry-After` header, so it is set explicitly. Both predate the catalogue and
are the connector contract.

The `429` path is the **only** cold-start mechanism: connector-side retry
absorbs clone latency with zero orchestration changes (no Argo pre-steps, no
prefetch endpoint).

## 5. Nocode Connector Consumption

> The connectors themselves ship separately
> ([PR #2366](https://github.com/constructorfabric/insight/pull/2366)); this
> section is the proxy-side contract they consume, not their manifest. Where
> the two could drift — spec fields, the error-handler policy — the connector
> change is authoritative and this section states only what the proxy
> guarantees.

Each vendor gets one declarative connector whose manifest mixes two base URLs.
Per-stream `url_base` is standard declarative CDK (precedent in-repo: the
figma manifest declares `url_base` per retriever).

### 5.1 Spec (config) additions

```yaml
required: [ …, git_proxy_url, git_proxy_token ]
properties:
  git_proxy_url:
    type: string
    description: Base URL of the git-cli-proxy service (no default; fail-fast)
  git_proxy_token:
    type: string
    airbyte_secret: true
```

No defaults for either — missing config must fail check, not silently point
somewhere.

### 5.2 Stream topology

| Stream | Base URL | Mode | Notes |
|---|---|---|---|
| `repositories` | vendor API | incremental | cursor on the vendor's "last pushed" field: GitHub `pushed_at`, GitLab `last_activity_at`, Bitbucket `updated_on` |
| `pull_requests` + children | vendor API | as today | data does not exist in git |
| `commits` | proxy | incremental, substream of `repositories` | cursor `committed_date`, per-partition |
| `file_changes` | proxy | incremental, substream of `repositories` | cursor `committed_date`, per-partition |
| `repository_branches` | proxy | full refresh, substream of `repositories` | latest state in bronze; head history via dbt snapshot/fields_history (§5.6); or stay on vendor API if `protected` must be kept (§8) |

The parent link uses `SubstreamPartitionRouter` with
`incremental_dependency: true`: because `repositories` is itself incremental,
a sync only visits repos whose "last pushed" advanced since the previous sync.
(Known CDK constraint: parent state persists only when the child stream is
incremental — `commits`/`file_changes` are, so the contract holds; `branches`
rides along in the same sync.)

### 5.3 Proxy-facing requester (shared definition)

```yaml
definitions:
  proxy_requester:
    type: HttpRequester
    url_base: "{{ config['git_proxy_url'] }}"
    authenticator:
      type: BearerAuthenticator
      api_token: "{{ config['git_proxy_token'] }}"
    request_headers:
      X-Tenant-Id: "{{ config['insight_tenant_id'] }}"
      X-Source-Id: "{{ config['insight_source_id'] }}"
      # Vendor-specific: GitHub any username (e.g. x-access-token), GitLab
      # `oauth2`, Bitbucket Cloud `x-token-auth`. This definition is per-vendor
      # (one connector = one vendor), so the constant is correct here — do NOT
      # lift it into a shared cross-vendor definition.
      X-Git-Username: "oauth2"
      X-Git-Token: "{{ config['api_token'] }}" # the same PAT the vendor streams use
    error_handler:
      type: DefaultErrorHandler
      backoff_strategies:
        - type: WaitTimeFromHeader
          header: Retry-After
      response_filters:
        - type: HttpResponseFilter
          http_codes: [429]
          action: RETRY
        - type: HttpResponseFilter
          http_codes: [413]
          action: FAIL          # oversized repo — permanent
```

`max_retries` / total retry budget must be generous enough to cover a cold
blobless clone of the largest expected repo (minutes, not seconds).

### 5.4 Stream sketch: `commits`

```yaml
streams:
  commits:
    type: DeclarativeStream
    retriever:
      type: SimpleRetriever
      requester:
        $ref: "#/definitions/proxy_requester"
        path: /v1/commits
        request_parameters:
          repo: "{{ stream_partition.repo_clone_url }}"
          since: "{{ stream_interval.start_time }}"
      partition_router:
        type: SubstreamPartitionRouter
        parent_stream_configs:
          - stream: "#/streams/repositories"
            parent_key: clone_url
            partition_field: repo_clone_url
            incremental_dependency: true
      paginator:
        type: DefaultPaginator
        pagination_strategy:
          type: CursorPagination
          cursor_value: "{{ response['next_page_token'] }}"
          stop_condition: "{{ response['next_page_token'] is none }}"
        page_token_option: { type: RequestOption, inject_into: request_parameter, field_name: page_token }
      record_selector:
        extractor: { type: DpathExtractor, field_path: [items] }
    incremental_sync:
      type: DatetimeBasedCursor
      cursor_field: committed_date
      datetime_format: "%Y-%m-%dT%H:%M:%SZ"
      start_datetime: "{{ config['start_date'] }}"
    transformations:
      - type: AddFields
        fields:
          - { path: [tenant_id],   value: "{{ config['insight_tenant_id'] }}" }
          - { path: [source_id],   value: "{{ config['insight_source_id'] }}" }
          - { path: [data_source], value: "insight_github" }
          - { path: [collected_at], value: "{{ now_utc().strftime('%Y-%m-%dT%H:%M:%SZ') }}" }
          # Repository identity is REQUIRED in the key: forks share commit
          # SHAs, so tenant+source+sha alone lets RMT collapse commits from
          # different repositories into one row. Prefer the vendor repository
          # id from the parent record over a URL.
          - { path: [unique_key],  value: "{{ [config['insight_tenant_id'], config['insight_source_id'], stream_partition.repo_id, record['sha']] | join(':') }}" }
```

(Illustrative, not literal — exact field set per vendor mirrors the current
bronze schemas; `file_changes` adds `filename` after the repository id and sha.)

Add `409 snapshot_changed` handling to the paginated streams: the pinned
snapshot is gone (§4.1), so the slice restarts from its stored cursor rather
than retrying the dead token.

### 5.5 Ordering and state

- Proxy rows arrive ascending by `committed_date` → the CDK cursor checkpoints
  monotonically; an interrupted sync resumes without gaps.
- Per-partition cursors: state size grows with repo count. The declarative CDK
  caps tracked partitions (~10k) before degrading to a global cursor — must be
  validated against the largest orgs (§8).
- Force-push: the proxy re-emits rewritten commits (they satisfy
  `committed_date ≥ since`); bronze stays append-only; RMT dedup by
  `unique_key` downstream absorbs re-emission.

### 5.6 Branch head history (downstream, dbt)

Bronze keeps only the **latest state** per branch; head-movement history is
produced by the same dbt machinery used for user profiles:

```
bronze branches (append-only → RMT)                    unique_key excludes head_sha
  → <vendor>__branches_snapshot.sql      {{ snapshot(...) }}        SCD2 versions
  → <vendor>__branches_head_history.sql  {{ fields_history(...) }}  (old, new, at) per head change
```

- The bronze `unique_key` MUST be `(tenant, source, repo, branch_name)` —
  **without** `head_sha` — so RMT collapses to current state and a head move
  is a tracked-column change, not a new key.
- `snapshot()` `check_cols`: `['head_sha']` (optionally
  `head_committed_date`). The macro reads the source `FINAL` by design
  (ADR-0001): pre-merge duplicates never become versions, so there is **no
  race against RMT collapse** — history granularity equals sync cadence,
  which is also the physical observability limit (intermediate heads between
  fetches are unobservable in git; per-push granularity is vendor
  webhook/events territory, out of scope).
- Branch deletion is not an event in this scheme (the row just stops
  updating) — same property as profile snapshots; add liveness tracking later
  if "branch deleted" must become a transition.

## 6. Deployment

- **One Deployment, one replica, one RWO PVC.** In-process locks, refcounts
  and LRU state require a single writer per volume. Horizontal scale (if ever
  needed) = shard cache keys across pods with their own volumes — never a
  shared volume.
- **Volume**: PVC preferred (cache survives restarts; no re-clone storm after
  deploys). `emptyDir` + `sizeLimit` is acceptable but its enforcement is pod
  eviction — the app budget must sit safely below either way. The chart
  enforces that: a `cache.diskBudgetBytes` outside 50–90% of
  `persistence.size`, or a `cache.maxRepoBytes` above the budget, fails the
  render rather than the pod.
- **Sizing** is a *full-clone* calculation, not a skeleton one. The blobless
  skeleton is the steady state only after the purge tier has run; with
  `include_patch=true` (the connector default) the first backfill lazily pulls
  essentially every blob, and an entry promoted to `full_clone` (§3.3) is
  exempt from blob purging entirely — `reclaimable_by_purge` is
  `size − skeleton`, and for a promoted entry the full clone *is* the
  skeleton, so purging frees nothing and only whole-entry eviction reclaims
  it. Size for:

  ```
  persistence.size ≥ Σ(full-clone bytes of repositories expected warm at once)
                   + max(single-repo full-clone bytes)   # fetch/repack scratch
                   + 15%
  ```

  with `cache.maxRepoBytes` above the largest repository intended to sync (a
  repository over it is refused `413` permanently, §4.4) and
  `cache.diskBudgetBytes` at 85–90% of the volume.
- **Config.** The gears-rust host reads `gears.git-cli-proxy.config`, with env
  overrides `APP__gears__git_cli_proxy__config__<field>`. Required, no
  defaults, validated at boot: `data_dir`, `disk_budget_bytes`,
  `max_repo_bytes`, `default_max_staleness_seconds`, `heavy_ops_concurrency`,
  `proxy_token` (secret, never rendered into the ConfigMap). Optional:
  `ca_cert_path` for an origin behind a private CA. One field exists solely for
  the test harness — `allow_file_repos`, which admits `file://` origins and is
  hard-coded `false` in the chart; no deployment may enable it.
- **Probes**: liveness = process health; readiness = HTTP serving (not disk
  pressure).
- **Network**: cluster-internal Service only, no ingress exposure; egress to
  git origins over https. Every `/v1` request carries `X-Git-Token` and the
  proxy bearer token, so the connector→proxy hop must be restricted at two
  levels: a `NetworkPolicy` admitting ingress only from the Airbyte job
  namespace (the sole consumer — no other service and no user calls this API),
  and a `git_proxy_url` whose scheme is validated (`^https?://`) so a missing
  or malformed scheme fails `check` instead of rendering an empty `url_base`.
  Plaintext in-cluster is the intended default and the shipped example: the hop
  is pod-to-pod, the chart issues no certificate for it, and the NetworkPolicy
  — not transport encryption — is what restricts who may call the API. A
  deployment that terminates TLS in front of the Service simply configures an
  `https` URL; nothing here prevents that.
- **Image**: minimal base + a git that passes the boot purge probe (§3.4).

## 7. Design Decisions

### DD-GP-01: Clone-based extraction behind a service, not inside connectors

Shelling out to git inside each CDK connector would avoid the new service but
duplicate clone/cache/locking logic per vendor and give every connector pod a
disk profile. One proxy implements the git logic once for all current and
future vendors; connectors shrink to declarative YAML.

### DD-GP-02: Stateless proxy; cursors in Airbyte, creds in headers

Any proxy-side sync state would create a second source of truth to reconcile
after evictions/restores. With cursors in Airbyte state and credentials per
request, the entire proxy disk is a cache with a trivial recovery story:
delete = slow next sync, never a wrong one.

### DD-GP-03: fetch-if-stale + 429/Retry-After; no prefetch

A prefetch endpoint would need an orchestration caller (Argo pre-step) holding
git credentials — extra coupling for a wall-clock optimization only. The
staleness rule is order-independent across streams, idempotent, and the
declarative error handler already implements the waiting side. Revisit only if
cold-clone latency demonstrably breaks sync deadlines.

### DD-GP-04: Cache directories named by key hash

Repo names are untrusted vendor data; hashing `(tenant, source, clone_url)`
eliminates path traversal, case-collision, and encoding classes entirely, and
enforces tenant isolation structurally. `meta.json` restores human
readability.

### DD-GP-05: Blobless clones with windowed blob transit

Full clones make disk proportional to total repo content; blobless skeletons +
batch prefetch + `repack --filter=blob:none` make steady-state disk
proportional to *metadata* and peak disk proportional to *one commit window*.
Fallback for origins without partial-clone support (§8): full bare clone +
identical repack purge — same steady state, heavier first download.

### DD-GP-06: Ascending `(committed_date, sha)` pagination, pinned to a generation

Descending or insertion-ordered pagination breaks when the cache entry is
evicted/re-cloned mid-pagination. Ascending order over an immutable-ish
history is deterministic across cache lifecycle events and lets Airbyte
checkpoint safely at any page boundary.

Ordering alone is not enough: a fetch between two pages can make an older
commit reachable, which a position-only cursor would skip. The token therefore
carries the snapshot generation, continuation requests never fetch, and a
superseded generation fails `409` instead of splicing two histories (§4.1).

### DD-GP-07: Credential continuity instead of trusting caller-supplied identity

One deployment-wide bearer token cannot express "this caller may read this
tenant's repository", and `X-Tenant-Id`/`X-Source-Id` are caller-supplied.
Rather than adding a second authorization system, the cache entry remembers a
fingerprint of the credentials that proved origin access and refuses to serve a
warm read to anyone else (forcing a fetch, where the vendor itself is the
authority). Access to cached data therefore requires access to the origin —
the property that matters — with no key distribution beyond the git PAT the
connector already holds.

The bound is continuity, not liveness: it proves the caller presents the same
credential that last succeeded, never that the credential is still valid. See
§3.2 for the exposure window a revoked credential retains.

## 8. Open Questions / Pre-build Validation

1. **Identity fields.** Current commit schemas carry `author_login` /
   `author_id` / `committer_login` / `committer_id` — vendor identities absent
   from git (only name/email exist). Confirm identity resolution keys on
   email; otherwise design a cheap enrichment stream. Similarly `protected`
   on branches is API-only: either drop it or keep `repository_branches` on
   the vendor API (it is cheap — one call per page of branches).
2. ~~**Bitbucket Cloud partial clone.**~~ ANSWERED: Bitbucket Cloud honours
   `--filter=blob:none` — a pristine clone produces a promisor pack with the
   blobs absent. The DD-GP-05 fallback stays for origins that do not, and
   §3.3's promotion covers origins that accept the filter but then refuse the
   objects.
3. **Per-partition state limits.** Validate the declarative CDK's partition
   cap (~10k) against the largest tenant's repo count.
4. ~~**`incremental_dependency` semantics**~~ ANSWERED: verified end to end
   against the declarative runtime — parent state persists only via an
   incremental child, which is why `repo_branches` (full refresh) does not
   carry it while the two commit streams do.
5. **Bitbucket PR pressure.** PR child streams remain per-PR API calls;
   measure whether PR-only traffic fits Bitbucket limits after commits move to
   the proxy.

Items 1, 3 and 5 are connector-side and close in PR #2366 or later; nothing in
this service depends on them.

## 9. Traceability

- Concept & discussion: [constructorfabric/insight#2224](https://github.com/constructorfabric/insight/issues/2224)
- Replaces the API-heavy streams of: `src/ingestion/connectors/git/github-v2`,
  `src/ingestion/connectors/git/gitlab`,
  `src/ingestion/connectors/git/bitbucket-cloud`
- Silver contract unchanged: [git connector spec](../../README.md)
- Thin-extractor & envelope contracts: ADR-0002, ADR-0004
  (`docs/domain/connector`, `docs/domain/ingestion-data-flow`)
