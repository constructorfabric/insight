# Work plan — git-cli-proxy

> Status: working document. Companion to [DESIGN.md](DESIGN.md) (the contract)
> and [issue #2224](https://github.com/constructorfabric/insight/issues/2224)
> (the concept). This file records what was decided, what shipped, and what is
> left — including the reasoning behind decisions that look arbitrary in a diff.
>
> Delivery: [PR #2237](https://github.com/constructorfabric/insight/pull/2237)
> (design, merged) and
> [PR #2288](https://github.com/constructorfabric/insight/pull/2288) (the
> proxy). The nocode connectors were split into
> [PR #2366](https://github.com/constructorfabric/insight/pull/2366) — see
> §8.1 — so several sections here span both.
>
> Last updated 2026-08-10: hardening, the review pass and the reliability
> pass are in #2288 (§8, §9.3), including full-clone promotion (§7.9) and the
> default-branch rework (§9.5). Next, in order: vendor-API streams (§9.4),
> rebase resilience beyond the lookback (§9.6), silver dbt models (§9.7),
> patch_id spec (§9.8).

## 1. Problem and premise

The CDK git connectors (github-v2, gitlab, bitbucket-cloud) extract
commit-level data through vendor APIs, and the heaviest streams cost **one API
call per commit**:

- GitHub `file_changes`: `GET /repos/{o}/{r}/commits/{sha}`
- Bitbucket `file_changes`: `diffstat/{sha}`

Against Bitbucket's ~1000 req/h budget a 50k-commit repository is a ~50-hour
backfill. Everything those streams emit is derivable from a git clone in
minutes.

**Premise**: move commit/file/branch extraction to a clone-based service, keep
everything that genuinely only exists in the vendor API (repositories, PRs,
reviews) on the vendor API.

Validation of the premise (per stream, what a clone can serve):

| Stream | Today | From a clone? |
|---|---|---|
| `commits` | API, paged per branch | yes — `git log` |
| `file_changes` | **1 API call per commit** | yes — `git log --numstat` |
| `repository_branches` | API | yes — `git for-each-ref` |
| `commit_branch_reachability` | API (bitbucket) | yes — `git branch --contains` |
| `repositories` (metadata) | API | no, but cheap: one call per page |
| `pull_requests` + children | API, per PR | no — PRs exist only in the API |

## 2. Architecture

One vendor-agnostic service plus one thin nocode connector per vendor:

```
nocode connector ──┬─→ vendor API   (repositories, PRs and their children)
                   └─→ git-cli-proxy (commits, file changes, branches)
                                  └─→ origin (git fetch over https)
```

`url_base` is a per-stream requester property in declarative CDK, so mixing two
upstreams in one manifest needs no special machinery.

Division of labour:

| Concern | Owner |
|---|---|
| Sync cursors, incremental state | Airbyte state (connector) |
| Repository discovery, PR data | Vendor API |
| Commit/file/branch extraction | git-cli-proxy |
| Envelope fields (`unique_key`, `tenant_id`, …) | Connector transformations |
| Git credentials | Connector config → per-request headers; never stored by the proxy |
| Disk | Proxy-managed LRU cache, rebuildable at any time |

**Core invariant**: the proxy may lose any repository at any moment and
re-clone it. What it may never do is let a reader observe a partially deleted
one.

## 3. Decisions that are not obvious from the code

### 3.1 Freshness: fetch-if-stale, no prefetch

A repo is fetched only when the local copy is older than `X-Max-Staleness`
(default from config). Rejected alternative: a `/v1/prefetch` endpoint warmed
by an Argo pre-step — the only possible caller would be a workflow step holding
git credentials, i.e. extra coupling for a wall-clock optimisation, not
correctness.

Because one sync's streams hit the same repo within minutes, the first stream
pays the fetch and the rest read the same snapshot — which is also the
consistency property we want.

### 3.2 Bounded wait, then 429

Preparation (clone or fetch) runs as a **background task** that owns the entry's
write lock. The request joins it and waits at most 15 s:

- fast fetch → completes inline, caller gets data;
- cold clone → caller gets `429` + `Retry-After: 30`, **the clone keeps
  running**.

This reconciles single-flight with the 429 contract: no request hangs for the
length of a clone, and a client giving up never cancels the work (`kill_on_drop`
would otherwise abort git mid-clone).

### 3.3 Cache key is a hash, never a name

Directory name = `sha256(tenant_id \0 source_id \0 clone_url)`. Repository names
are untrusted vendor data: building paths from them invites traversal and
case-insensitive filesystem collisions. Identical clone URLs under two sources
are two isolated entries **by design** — access rights differ per source.

### 3.4 Credentials live only in the child process env

`remote.origin.url` is stored clean. The token is injected per invocation via
`GIT_CONFIG_KEY_n` = `http.extraheader` (Basic) — invisible in `ps`, absent
from disk, absent from git error messages. Consequence: **token rotation never
requires a re-clone**; the cloned objects do not depend on the credential, only
the transport does.

Every git invocation gets the credentials, not just the explicit fetch — see
§7.1.

### 3.5 Warm reads require proof of origin access

Each entry stores a one-way fingerprint (sha256) of the credentials that last
proved origin access. A warm read is served only to a caller whose credentials
match; a mismatch forces a fetch, where the vendor itself is the authority.

Why: the proxy has ONE deployment-wide bearer token, and `X-Tenant-Id` /
`X-Source-Id` are caller-supplied. Without this, anyone holding the proxy token
could name another tenant's cache entry and read it. Those headers are cache
**partition inputs**, never authorization claims.

### 3.6 Blob lifecycle: skeleton on disk, blobs in transit

1. Clone `--bare --filter=blob:none` (commits + trees only, ~10–20% of full
   size). The origin becomes a *promisor remote*.
2. Enumerate the window's changed paths — needs **no blobs**.
3. Batch-prefetch the referenced blob OIDs (`git fetch origin <oid>…`).
4. Compute numstat / renames / patches locally.
5. Purge: `git repack -a -d --filter=blob:none --no-write-bitmap-index`.

Steady-state disk is proportional to *metadata*; peak disk to *one commit
window*. This is what makes a bounded cache viable for large histories.

### 3.7 Disk is self-managed

`DISK_BUDGET_BYTES` (required, no default, must sit below the volume size),
high/low watermarks with hysteresis (85% → reclaim to 65%), two-tier LRU
reclaim: blob purge first (keeps the repo warm — the next sync fetches instead
of re-cloning), whole-entry eviction only when purging cannot free enough.
Reclaim runs **before** a clone takes disk. `MAX_REPO_BYTES` aborts an
oversized clone and answers `413` (permanent: retrying just burns the budget).

Entries with live readers are never touched.

No `statfs` backstop: the workspace forbids `unsafe_code`, and the volume is
dedicated to this cache, so measuring our own tree is equivalent.

### 3.8 One writer per volume

In-process locks, the LRU state and the single-flight map live in one process →
exactly one pod may bind the cache volume. RWO PVC + `replicas: 1` +
`strategy: Recreate` are ONE decision, not three. Future scale = shard cache
keys across pods with their own volumes, never a shared volume.

### 3.9 Pagination: ascending and pinned to a snapshot

All list endpoints order ascending and return an opaque `next_page_token`
carrying the last position **plus the snapshot generation**.

- A request **with** a token never fetches: it is served from that generation,
  whatever the staleness window says. Only the first page of a walk can trigger
  a fetch.
- A superseded generation fails `409` rather than splicing two histories — a
  fetch between pages can make an older commit reachable, which a
  position-only cursor would silently skip.

Ordering keys: commits and file changes by `(committed_date, sha)`, branches by
`(name)` — hence the token's key is a generic two-part ordering key.

## 4. patch_id — what it is and why it exists

`/v1/commits` carries a `patch_id` field on every commit.

**What**: the output of `git patch-id --stable` over the commit's full diff — a
canonical hash of the *change itself*, computed in the batch form
`git log --patch <shas> | git patch-id --stable`, which emits
`<patch-id> <commit-sha>` pairs.

**Properties** (this is why git has it):

- insensitive to hunk order and whitespace/context noise;
- identical for the same change applied on different bases — so a cherry-pick,
  a rebase, and the original commit share a `patch_id` while their SHAs differ;
- merge commits have no single diff and therefore no `patch_id` (absent from
  the map).

**Why the proxy computes it rather than the consumer**:

1. It answers "is this the same work counted twice?" — the duplicate /
   cherry-pick question that motivated storing diffs in the first place.
2. It is computed from the **untruncated** diff, before `max_patch_bytes`
   applies. Duplicate detection therefore never depends on whether the patch
   text was stored, truncated, or omitted (`include_patch=false`).
3. Doing it in the proxy costs nothing extra: the blobs are already local for
   numstat.

**Downstream use**: group by `(tenant, source, repository, patch_id)` to
collapse cherry-picked or rebased work; keep `sha` as the identity of the
commit object itself. The repository component is load-bearing: `patch_id` is
derived from diff CONTENT alone, so unrelated repositories collide on it
routinely — a vendored file, a licence header, an identical one-line fix — and
one source contains many repositories. Which repository identity to group on
must be settled with §9.8: a clone URL is what the connector's keys carry but
is not canonical (host aliases, a `.git` suffix, http vs ssh all vary for one
repository), so either the normalization is specified or the vendor id is used.
A cross-repository collision test belongs with those rules. Which instance of a group a metric should count depends on the lens —
the selection rules (last-of-group for default-branch reports, first-of-group
for all-branches views) are §9.8 and belong in the git domain spec, not in
individual dashboards.

## 5. Patch text: stored by default

`/v1/file-changes` returns per-file unified diff text with
`include_patch=true` **by default**, capped by `max_patch_bytes` (1 MiB), and
every row flagged with `patch_truncated`.

Rationale for storing rather than counting once: LOC may need recomputing later
under filters the connector does not know today (ignore blank lines, ignore
comments, ignore vendored paths), and "what else might we need" is answered by
keeping the source material. The truncation flag exists so a later recount can
tell an incomplete diff from a complete one instead of silently under-counting.

Cost, accepted knowingly: `patch` is the dominant bronze volume in git data.
`include_patch=false` is available when that trade flips.

## 6. Branch head history

Bronze keeps only the **latest** state per branch; head-movement history is
derived downstream by the same dbt machinery user profiles use:

```
bronze branches (append-only → RMT)          unique_key EXCLUDES head_sha
  → <vendor>__branches_snapshot   {{ snapshot(check_cols=['head_sha']) }}
  → <vendor>__branches_head_history {{ fields_history(...) }}  → (old, new, at)
```

Two facts that make this work:

- The bronze `unique_key` must be `(tenant, source, repo, branch_name)` —
  **without** `head_sha`. With the sha in the key, RMT never collapses and no
  transition is ever detected.
- `snapshot()` reads its source `FINAL` **by design** (ADR-0001): pre-merge
  duplicates must not become versions. So there is no race against RMT
  collapse — history granularity equals sync cadence.

Physical limit worth stating: per-push granularity is **not** obtainable from
git. A clone sees refs only at fetch time; a head that moved and moved back
between fetches is unobservable, and the proxy's reflog is not a source (same
discretisation, and it dies with LRU eviction). Per-push history is vendor
webhook/events territory.

Branch deletion is likewise not an event here — the row simply stops updating
(same property as profile snapshots).

## 7. Facts discovered by running things

These cost real debugging time; they are contract, not trivia.

### 7.1 A partial clone lazily fetches — from every command

Any git command on a promisor-backed repo can trigger a lazy fetch of a missing
object. Reader steps that ran without credentials therefore hit
`could not read Username` → classified as an origin auth rejection. **All** git
invocations that touch objects must carry credentials.

### 7.2 `-M` reads blob content

Rename detection compares file contents, so putting `-M` on the OID-enumeration
step made the step that exists to *prevent* lazy fetches trigger them. Keep
`-M` on numstat/patches, off enumeration.

### 7.3 `diff-tree` with several revisions diffs *between* them

The multi-commit form is `git log --no-walk <shas>`, not `git diff-tree <shas>`.

### 7.4 Raw output abbreviates OIDs

`--raw` prints abbreviated OIDs unless `--no-abbrev` is given, and abbreviated
OIDs cannot be fetched. `--full-index` affects only patch headers, not raw
lines.

### 7.5 `repack --filter` fails with bitmaps enabled

`--filter` splits objects across packs while bitmap writing assumes a single
pack. With `repack.writeBitmaps` on, the purge fails and the blobs stay on
disk — `--no-write-bitmap-index` is mandatory, not cosmetic.

### 7.6 Helm parses YAML numbers as float64

A 46e9 byte budget renders as `4.6170898432e+10` and the service refuses to
deserialize it into `u64`. Pass byte values through `| int64`.

### 7.7 The Builder strict validator resolves no `$ref`

Shared `definitions.linked` authenticators/url_base are rejected, so the
manifests are fully inlined. Duplication is the price of Builder compatibility.

### 7.8 Operational traps

- A stale proxy process on the same port answers requests and makes a fixed
  build look broken. Check for a listener before concluding anything.
- `.env.local` values can carry inline comments (`TOKEN=glpat-… # admin`);
  a naive `cut -d= -f2-` yields a 34-char "token" and every request fails
  authentication.
- `cargo clippy` runs at `-D warnings` with pedantic, and the pre-commit hook
  reformats files (ruff/yamlfmt) and aborts the commit — re-add and re-commit.

### 7.9 GitLab fork-network pools refuse explicit-OID wants

Found by running the connectors against a self-hosted GitLab. On a repository
deduplicated into a fork-network object pool, Gitaly serves a plain clone fine
but refuses SOME blobs as explicit promisor wants: the batch prefetch dies with
`did not send all necessary objects` on every retry, permanently. A blobless
clone is therefore **unreliable against pooled GitLab repos** — this is a
property of the repository, not a transient fault. The proxy heals it once:
drop the filter, `git fetch --refetch`, delete `*.promisor` markers, repack,
mark the entry `full_clone` in `meta.json`. Full-clone entries are exempt from
the blob-purge reclaim tier (the purged blobs could not be fetched back).
First pages retry transparently after the promotion; continuations get the
standard `409` (the promotion bumps the generation).

### 7.10 `membership=true` is useless for a stats account

A read-only service account (`read_api` + `read_repository`) can be a member
of no project while still seeing many, so `/projects?membership=true` returns
zero rows and the connector silently syncs nothing. Discovery therefore scopes
by an explicit required `gitlab_groups` list via
`/groups/{g}/projects?include_subgroups=true`, which works by group
**visibility** (and honours `order_by=last_activity_at` + `last_activity_after`,
verified live). Mirrors `bitbucket_workspaces`.

### 7.11 Bitbucket git-over-https auth is NOT the API auth

`email:api_token` authenticates on `api.bitbucket.org` but is 401 on the git
endpoint; `x-token-auth` is 401 too (that username is reserved for
workspace/repo access tokens). What works for personal API tokens on the clone
endpoint: the account's short username, or the reserved
`x-bitbucket-api-token-auth`. The connector defaults to the latter and exposes
`bitbucket_git_username` for the access-token case.

### 7.12 Spec `default:` values never reach the runtime config

They are Builder-UI hints only. `config['bitbucket_api_base_url']` rendered as
an empty `url_base` because the key was absent — every config read in a
manifest needs `config.get('key', <literal default>)`.

### 7.13 `partition_router` placement is load-bearing and unvalidated

It belongs INSIDE `retriever`. At stream level the CDK silently ignores it —
the Bitbucket substream parents listed `/repositories/` with an empty
workspace and got `410 Gone`. Neither the strict validator nor `discover`
catches this; only a live `read` does.

### 7.14 `#magic___^_^___line` inside folded scalars is content

The Airbyte Builder's line-break marker survives copy-paste into `>-` blocks
as literal text — it polluted `unique_key` values in both manifests. Grep for
it after any Builder round-trip.

## 8. What shipped

| Phase | Content | Where |
|---|---|---|
| 1 | Service skeleton: gears-rust host, minimal system-gear set, static bearer auth on `/v1`, fail-fast config, Dockerfile, CI registration (workspace, `components.py`, manifest lists in every backend Dockerfile) | `src/backend/services/git-cli-proxy` |
| 2 | Git engine: hashed cache layout, hermetic git runner, per-repo RW-lock + single-flight, blobless clone, fetch-if-stale | `src/…/engine/{key,meta,runner,store}.rs` |
| 3 | Extraction: commits (incl. `patch_id`, `is_in_default_branch`), file changes (statuses from `--raw` merged with `--numstat` counts, renames, patches), branches, windowed blob prefetch + purge | `src/…/engine/read/*` |
| 4 | HTTP API v1 with the error contract and snapshot-pinned pagination | `src/…/api/*` |
| 5 | Disk budget, watermarks, two-tier LRU reclaim, per-repo cap → 413 | `src/…/engine/disk.rs` + store |
| 6 | Helm subchart (first PVC chart in the repo), umbrella registration, config Secret, image build jobs, render-contract workflow | `src/…/git-cli-proxy/helm`, `charts/insight`, `.github/workflows` |
| 7 | Nocode connectors for GitLab and Bitbucket Cloud — **moved out of this change**, see §8.1 | separate change |
| 8 | Hardening: repo origins restricted to http(s) at the boundary, in-flight refresh joins re-prove credentials, page tokens bound to their cache entry, continuations stop narrowing `--since`, `/v1/file-changes` row/byte caps with commit-boundary cursoring, patch buffer capped while reading, `run_piped` drains producer stderr, per-write-unique `meta.json` tmp names, full-clone promotion for origins refusing promisor wants (§7.9), `branch_names` → `is_in_default_branch`, canonical problem+json errors, `OperationBuilder` + committed OpenAPI + drift gate, `proxyToken` supplied-or-fail, disk-budget render guard, `global.storageClass`, `git_cli_proxy` wired into the `changes`/bump/publish CI graph | proxy + `charts/insight` + `.github/workflows` |
| 9 | Design alignment: `since` applied as a predicate rather than git's traversal cutoff, `statvfs` as the second free-space view, admission able to refuse (`429`), and the §4.3 metrics implemented. The enumeration walk was split from the window read — keys only (~100 B/commit) for the whole-history pass, full headers for the page's own commits — so per-page memory is bounded by the page, not by history × message size | `src/…/engine/{read/commits,disk,store,metrics}.rs`, `src/…/api/*` |

| 10 | Reliability: the blob purge made to actually free disk (`repack` never filters a promisor pack, and every pack in a blobless clone is one), each served window re-measuring its entry so the reclaim planner sees the truth, `max_repo_bytes` enforced by killing a clone mid-transfer rather than measuring afterwards, separate read/prefetch/heavy timeouts so a stalled read cannot hold an entry for half an hour, one metrics layer outside the bearer check with a real response size, problem+json on the two rejection paths that escaped it, and `--raw`/`--numstat` read under `-z` so a filename reaches the row as git has it on disk | `src/…/engine/{store,disk,runner,read/*}.rs`, `src/…/api/*` |

| 11 | Review follow-through: page tokens carry the clone's incarnation so a cursor cannot survive an eviction and re-clone, object ids accepted at SHA-256 length, commit fields separated by a byte git cannot put inside an ident, admission reserving the headroom it grants, metadata that cannot be published invalidating its entry, copy detection requested so `copied` can actually be emitted, `sha` prefixes bounded above, and a served window ordered by instant | `src/…/engine/{store,meta,page,read/commits}.rs`, `src/…/api/*` |

Quality: 182 Rust tests, clippy clean (pedantic, `-D warnings`), 18 Helm
render-contract assertions, connector wiring guard green, the committed
OpenAPI document matching its drift gate.

A review of DESIGN against the implementation drove phase 9. Four sections
promised behaviour the service did not have — §4.2's `since` predicate, §3.6's
second free-space view and its admission refusal, and every metric in §4.3 —
and the rollout notes below already told an operator to watch two of those
metrics. The design was right in each case and the code followed it; the
reverse edits (an error envelope the platform mandates, the host owning
`/healthz`, config names the gears host fixes) are recorded in DESIGN itself.

### 8.1 The connectors moved to their own change

The GitLab and Bitbucket nocode connectors were split out. Only the proxy can
be verified here — it carries hermetic tests, a committed API contract and a
drift gate, whereas nothing in CI validates a declarative connector manifest,
and the connectors' hardest defects were found against live vendor instances
rather than in tests. §7.9–§7.14 and §9.1–§9.2 below record findings from
those live runs; the fixes for §7.10–§7.14 now live in the connector change,
and §7.9's fix is in this one.

## 9. Remaining work, in order

### 9.1 Live verification of both connectors — DONE ONCE, NEEDS RE-RUN

Both connectors were driven end-to-end through
`airbyte/source-declarative-manifest:7.23.6` against a local proxy and real
vendor instances, all four streams each, with every stream returning records
and no errors. What each stream must produce is asserted by the connector
change's own checks; the counts are environment-specific and are not recorded
here.

The 429 path was forced explicitly, since a small repository clones inside the
15 s inline wait: SIGSTOP the cloning git child → `429` + `Retry-After: 30`,
SIGCONT → the retried request serves. The GitLab run also exercised full-clone
promotion live — pooled repositories (§7.9) healed automatically and served
afterwards.

The run was not a formality: it surfaced six real defects, all fixed
(§7.9–§7.14 plus the `record['id']` → `uuid` repository key). Those fixes are
re-derived in the connector change but have NOT been re-observed against a
vendor since; that re-run is a precondition for taking it out of draft.

### 9.2 Settle the Bitbucket partial-clone question — ✅ SETTLED

Bitbucket Cloud honours `--filter=blob:none`: a pristine clone of a test
repository produced a promisor pack with the blobs absent, measurably smaller
than the full clone. PVC caveat recorded in
DESIGN §8: with `include_patch=true` (the default) the first backfill lazily
pulls essentially every blob, so size the Bitbucket cache for ~full-clone
weight; the skeleton pays off only after the blob-purge reclaim tier runs.
Opposite finding on GitLab: filter honoured, but pooled repos need the §7.9
promotion.

### 9.3 Work through the PR #2288 review comments — ✅ DONE

29 open threads (5 human, 24 CodeRabbit; 4 critical). Every one that is a
defect in this change is fixed in phase 8's row above, each with a test that
fails against the previous behaviour.

Three were answered rather than actioned:

- **Rust 1.97 / Python 3.13.** The repo standard is `rust:1.95-bookworm`
  (four sibling Dockerfiles, `rust-version = "1.95.0"` at the workspace, four
  CI pins) and Python 3.12 (every workflow). 1.97 appears only in the local
  cargo-watch dev image, which ships nothing. Changing either is a
  cross-cutting bump, not this change.
- **Tenant spoofing.** `X-Tenant-Id` is an unauthenticated cache-partition
  input. The reviewer asked for a tech-debt issue and proper interservice JWT
  plus gateway auth, not an in-PR fix — the API is deliberately not behind the
  gateway (§6). Tracked separately; §3.7 states the boundary.
- **Handler→engine row assembly.** The row assembly is now a pure free
  function with its own unit tests, which is what made the response caps
  testable. Moving it into the engine layer buys nothing further.

### 9.4 Finish the vendor-API streams — NOT STARTED (contract collected)

Not portable to git, so they stay on the vendor API. Manifest additions are
not written yet; what follows is the porting contract extracted from the CDK
connectors so the port needs no re-discovery.

**GitLab** (source of truth:
`src/ingestion/connectors/git/gitlab/source_gitlab/streams/merge_request*.py`
and the `.schema.json` files next to them):

| Stream | Path | Cursor | Key parts | Notes |
|---|---|---|---|---|
| `merge_requests` | `/projects/{project_id}/merge_requests` | `updated_at` (`order_by=updated_at&sort=asc`, inject `updated_after`) | `project_id:iid` | substream of `repositories` (parent_key `id` → partition `project_id`, `incremental_dependency: true`) |
| `merge_request_notes` | `…/merge_requests/{mr_iid}/notes` | `mr_updated_at` — the PARENT MR's `updated_at`, stamped from the slice | `project_id:note_id` | substream of `merge_requests`; carry `updated_at`+`project_id` via ParentStreamConfig `extra_fields` |
| `merge_request_approvals` | `…/merge_requests/{mr_iid}/approvals` | `mr_updated_at` (same) | `project_id:mr_iid` | SINGLE-OBJECT response (`field_path: []`); CDK skips 402 (premium-only) and 404 — port as response_filter IGNORE |
| `merge_request_commits` | `…/merge_requests/{mr_iid}/commits` | `mr_updated_at` (same) | `project_id:mr_iid:sha` | links MRs to commits already extracted by the proxy |

Record shaping, identical across all four: flatten nested objects to
`author_id`/`author_username`, `merged_by_*`, `resolved_by_id`,
`milestone_id`, `position_new_path`/`position_new_line`; encode
`assignee_ids`/`reviewer_ids`/`labels`/`approved_by` as JSON strings
(`tojson`); trim `title` to 1024 and `description`/`body` to 16384 chars with
`*_truncated` boolean flags (`value_type: boolean` on AddFields — bare Jinja
renders strings); RemoveFields the bulky originals. Envelope =
`tenant:source:<key parts>`, same as every existing stream.

**Bitbucket** (source of truth:
`src/ingestion/connectors/git/bitbucket-cloud/source_bitbucket_cloud/streams/pr_*.py`).
The list endpoint is `/repositories/{ws}/{slug}/pullrequests` with
`sort=updated_on`, `fields=` trimming, explicit `state=` params (the default
hides everything but OPEN), and `q=updated_on>="<floor>"`. Children per PR:
`…/pullrequests/{id}/comments`, `/commits`, `/diffstat`, `/activity`.
Reviewers do NOT need a child call — they ride on the PR object, BUT the LIST
endpoint omits `participants` unless `fields=+values.participants` is asked
for explicitly (`closed_by` IS in the list; `participated_on` maps to
`reviewed_at`).

An honesty note for the port: the CDK's `pr_base.py` is a THREE-phase
selection (updated_on floor → OPEN re-sweep → terminal-state reconcile by
`id>` cursor) built to catch state transitions the plain floor query misses.
Declarative YAML cannot express that; the port covers phase 1 only, and the
gap is mitigated the same way as rebase loss — a `lookback_window` on the
cursor (§9.6). If that proves insufficient for PR state accuracy, PR streams
stay on the CDK connector; do not try to fake the reconcile in Jinja.

Structural cost: each child stream must inline its FULL parent chain (§7.7) —
notes/approvals embed a copy of `merge_requests` which itself embeds a copy of
`repositories`. Expect the manifests to roughly triple. This is mechanical,
not a smell.

**The jira/confluence lesson applies here directly** (see
`project_jira_confluence_substream_hang`): fat substream parents killed those
connectors twice — a `fields=*all` parent blew a 226 MB requests-cache, and
the concurrent CDK self-deadlocks at `default_concurrency: 1` once a sync has
~10k partitions. For these streams that means: (a) every inlined parent copy
selects the MINIMAL field set it needs (`fields=` on Bitbucket, the trimmed
inline schema on GitLab) — never the full payload; (b) keep
`default_concurrency: 4` (already set in both manifests — do not "simplify"
it to 1); (c) MR/PR children multiply partitions (one per MR, not per repo) —
check the ~10k per-partition state cap (§9.10) against MR counts, not repo
counts.

Residual rate-limit exposure: Bitbucket PR children are per-PR calls. Measure
whether PR-only traffic fits the ~1000 req/h budget once commits move to the
proxy.

### 9.5 Rework commit branch membership → default-branch only — ✅ DONE

Decision from PR review discussion: consumers do NOT need every branch that
contains a commit — they need to know whether the commit is in the DEFAULT
branch. The current `/v1/commits` contract (`branch_names: [string]`,
computed as one `rev-list` per branch intersected with the page) must be
reworked to `is_in_default_branch: bool`:

- proxy: resolve the default branch once per request (`HEAD` symref — the
  same fact `/v1/branches` already exposes as `is_default`), run ONE
  `rev-list` over it, intersect with the page. Drops the per-branch loop
  entirely — cost stops scaling with branch count, which on a repository with
  many branches was the dominant term.
- both nocode manifests: replace `branch_names` in the `commits` schema.
- DESIGN.md §4.2 field table + this plan's §3/§8 mentions.

Landed as described. The enumeration still walks `--all`: commits on
non-default branches are still extracted (the patch_id selection rules in §9.8
need them); only the membership computation narrowed. `fetch` additionally
runs `git remote set-head origin --auto`, because a fetch does not update the
mirrored `HEAD` and a default-branch rename at origin would otherwise leave
the new boolean wrong for every row until the entry was evicted.

**The loss mode is worse than the field it replaced, and is documented rather
than hidden.** `branch_names` degraded gracefully — a stale row was merely
incomplete. `is_in_default_branch` asserts a boolean that flips exactly once
in the normal lifecycle, and is emitted on the wrong side of that flip: a
commit first seen on a feature branch is emitted `false`, and merging it later
does not change its committed date, so a date-cursored sync never revisits it.
Squash-merge is fine (the merge is a new commit, emitted `true`);
fast-forward and true merge leave the original commits permanently wrong.
Downstream can reconstruct reachability from `parent_hashes` + `is_merge` with
no proxy change, and §9.6's lookback corrects anything merged inside the
window. See DESIGN §4.2.

### 9.6 Rebase resilience for date-cursored enumeration — MITIGATED, NOT SOLVED

`git log --since=<cursor>` is a traversal cutoff. After a rebase the branch
head is rewritten; replacement commits whose committer dates fall BEFORE the
cursor are silently lost forever, because no later sync ever looks behind the
cursor again.

Mitigation, shipped in the connector change — a **lookback window**: always
enumerate from `cursor − window` instead of `cursor`. The declarative CDK has
this as a first-class knob (`DatetimeBasedCursor.lookback_window`, ISO-8601
duration); bronze dedups the re-read rows, so the only cost is re-serving one
window of commits per sync. It is a config field with a `P1M` default, not a
constant.

**It is a heuristic and must not be read as a completeness guarantee.** A
rewrite that moves commits below `cursor − window` still loses them
permanently, and nothing detects that it happened. The window buys coverage
proportional to its length, not correctness. The correctness boundary is
reachability, not dates: the adaptive mechanism below is what would actually
close this, and it is unbuilt.

Recorded hypothesis for an **adaptive mechanism** (design exploration, not
committed work): store the last known branch head sha in the cursor state;
the proxy checks whether that sha is still reachable from the branch — if it
is not, a rebase happened, and the enumeration for that repo must deepen
(e.g. re-walk from the fork point or from `cursor − N·window`) instead of
trusting the date floor. This needs a proxy affordance (a "was <sha>
reachable" check or a `known_head=` request parameter that widens the walk on
divergence) plus connector-side state; write it up properly before building.

### 9.7 Silver dbt models (blocking for dashboards)

Both connectors are bronze-only until these land, so they contribute nothing to
the UI.

Hard constraint: column types must match the existing CDK connectors' models
**exactly**. `union_by_tag` UNION ALLs the branches, so one mismatched type
raises `Code: 386 NO_COMMON_TYPE` and breaks the shared class for *every*
source. Check before writing:

```
grep -rn '<col>' src/ingestion/connectors/*/*/dbt/*__<class>.sql
```

Include the branch-head history chain from §6. Write these AFTER the
default-branch rework (§9.5) — the commits schema changes — and encode the
patch_id selection rules (§9.8) where the models feed commit-counting metrics.

### 9.8 patch_id selection rules — spec work, then metrics

`patch_id` groups a cherry-pick, a rebase and the original commit (§4). Raw
counting therefore double-counts the same work. The selection rule depends on
the lens, and it must be WRITTEN INTO the git domain spec (extend
`docs/components/connectors/git/` — the silver/metrics contract) so every
metric applies it consistently rather than each dashboard reinventing it:

- **Default-branch-only reports** (what landed): within a group of commits
  sharing a `patch_id`, count only the LAST one (latest `committed_date`) —
  that is the instance that actually reached the default branch; the earlier
  ones are its drafts on feature branches.
- **All-branches views** (when the work was done): count only the FIRST one
  (earliest `committed_date`) — later instances are mechanical replays of the
  same change, not new work.

Both rules need `is_in_default_branch` (§9.5) and the full multi-branch
enumeration the proxy already does. Both also group by
`(tenant, source, repository, patch_id)` — never without the repository (§4):
`patch_id` is content-derived, so unrelated repositories collide on it, and
the spec section must settle which repository identity is canonical and carry
a cross-repository collision test. Deliverable: a spec section + the silver
models (§9.7) exposing the group key so metrics can apply either rule.

### 9.9 Identity fields git does not carry

The CDK commit schemas carry `author_login` / `author_id` /
`committer_login` / `committer_id` — vendor identities absent from git, which
knows only name and email. Confirm identity resolution keys on email; if a
login is genuinely required, plan a cheap enrichment stream rather than
re-introducing per-commit API calls. Same for `protected` on branches (API-only:
either drop it or keep `repository_branches` on the vendor API, which is cheap).

### 9.10 Per-partition state limits

The declarative CDK caps tracked partitions (~10k) before degrading to a global
cursor. Validate against the largest tenant's repository count before relying
on per-repo cursors at scale — and note that the MR/PR child streams (§9.4)
create one partition per merge request, not per repository, which reaches the
cap far sooner.

### 9.11 Rollout

- Enable `gitCliProxy.deploy=true` in the target environment's values and set
  `networkPolicy.allowedNamespaceLabels` to the namespace running Airbyte
  connector jobs — an empty allow-list denies all ingress by design.
- Size `persistence.size` from the worst-case FULL-CLONE working set, not
  from skeleton sizes: with `include_patch=true` the first backfill lazily
  pulls essentially every blob, and an entry promoted to `full_clone` (§7.9)
  is exempt from blob purging, so only whole-entry eviction reclaims it. Allow
  the sum of the repositories expected warm at once, plus the largest single
  repository again as fetch/repack scratch, plus 15%. Keep
  `cache.diskBudgetBytes` at 85–90% of the volume — the chart now refuses to
  render outside 50–90%, and refuses a `cache.maxRepoBytes` above the budget.
- Set `gitCliProxy.proxyToken` in the environment's values (the render fails
  without it — the token has a second holder in the connectors, so a generated
  one could not be re-derived there and would rotate on every GitOps reconcile)
  and provision the connector Secrets carrying the same `git_proxy_token`;
  reconcile then creates the Airbyte sources.
- Watch `git_proxy_evictions_total{tier}` and
  `git_proxy_admission_rejects_total`: a sustained rise means the budget is too
  small for the working set — that, not utilisation, is the alert condition (an
  LRU cache is full by design). A non-zero admission-reject rate is the
  stronger signal: it means a request was refused outright because nothing
  could be reclaimed. `git_proxy_disk_used_bytes` against
  `git_proxy_disk_budget_bytes` shows how much headroom is left.

### 9.12 Enumeration cache — measured, and rejected

Applying `since` as a predicate means every page enumerates whole history.
Snapshot pinning makes that walk immutable per `(entry, generation)`, so
caching it looked like the obvious follow-up. Measured on a synthetic
50k-commit repository before building it: the keys-only walk is ~0.25 s and
~5 MB; the per-page window work around it (blob prefetch over the network,
patch and numstat reads) dwarfs it. In steady state an incremental sync with
the default lookback yields about one page, so a cache saves nothing; on a
backfill it saves tens of seconds once, against a clone and blob transfer
measured in minutes. What a cache costs is a stateful component in a
deliberately stateless service (DD-GP-02) with real coherence obligations —
it must be invalidated on eviction, re-clone and promotion, or it serves a
previous incarnation's history.

Not built. What WAS kept from the same investigation is the walk/window
split in phase 9: the whole-history pass now reads ~100 bytes per commit
instead of full messages, which bounds per-page memory regardless of any
cache. Revisit only if paging shows up slow against a measured repository,
and bring these numbers.

## 10. Deliberately out of scope

- Prefetch/warm-up endpoint and an Argo pre-step (§3.1).
- Shared object stores between forks (`alternates`) — isolation beats dedup;
  it would also break the "different repos never contend" property.
- Multi-replica proxy / shared volumes (§3.8).
- Per-push branch history (§6) — not obtainable from git.
