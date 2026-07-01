# silver/git

Git silver models. Two layers:

- **Class union** (`class_git_*`): per-source staging (GitHub, Bitbucket Cloud)
  unioned into source-neutral row-level tables. Contract relied on by every
  downstream git query.
- **Facts** (`fct_git_*`): row-level enrichment on top of class_git_*
  (one row in = one row out, adds `person_key`, `week`, derived flags).
- **Metrics** (`mtr_git_*`): aggregate tables ready for dashboards.

## Models

### Class union

| Model | Grain |
|---|---|
| `class_git_repositories` | one row per repo |
| `class_git_repository_branches` | one row per branch |
| `class_git_commits` | one row per commit |
| `class_git_file_changes` | one row per file change |
| `class_git_pull_requests` | one row per PR |
| `class_git_pull_requests_reviewers` | one row per reviewer action |
| `class_git_pull_requests_comments` | one row per PR comment |
| `class_git_pull_requests_commits` | one row per PR ↔ commit link |

### Facts

| Model | Grain | Adds |
|---|---|---|
| `fct_git_pr` | one row per PR | `person_key`, `state_norm`, `cycle_time_h` |
| `fct_git_commit` | one row per commit | `person_key`, `week` |
| `fct_git_file_change` | one row per file change | `person_key` (via commit), `week`, `file_category` (code\|spec\|config) |
| `fct_git_review` | one row per review | reviewer `person_key` |

### Metrics

| Model | Grain | Metrics |
|---|---|---|
| `mtr_git_person_totals` | `(tenant_id, person_key)` | `prs_created`, `prs_merged`, `avg_pr_cycle_time_h`, `commits`, `loc`, `clean_loc` |
| `mtr_git_person_weekly` | `(tenant_id, person_key, week)` | `commits`, `prs_merged`, `code_loc`, `spec_lines`, `config_loc` |

## Identity

`person_key` is the stable identifier exposed by every fact/metric. Current
implementation uses `lower(author_email)` with fallback to `author_name` when
email is empty (Bitbucket Cloud PR authors have no email).

Once identity resolution is wired end-to-end via `class_people.person_id`, the
facts will resolve `person_key → person_id` without changing downstream
metrics.

### Reviewer namespace (separate)

`fct_git_review.person_key = lower(reviewer_name)` — GitHub login or
Bitbucket display_name. This is a **different namespace** from the
email-based key on `fct_git_pr` / `fct_git_commit`, so the same human will
not match across reviews and commits until an identity bridge resolves
login ↔ email.

Consequence for now: `reviews_given` is **not** included in
`mtr_git_person_totals`. It returns once the bridge lands. `fct_git_review`
remains available for reviewer-scoped analysis.

## Running

```bash
# full chain from any git source to metrics (traverses via ref edges)
dbt run --select tag:github+         # or tag:bitbucket-cloud+
# all silver models (class + fct + mtr across every domain)
dbt run --select tag:silver
```

## Caveats

- `cycle_time_h` for Bitbucket Cloud uses `pr.closed_on` which is a
  staging-level heuristic (set to `updated_on` on terminal states). Accurate
  to within sync cadence for typical PRs; improves when the Bitbucket Cloud
  `pr_activity` stream lands.
- `mtr_git_person_weekly` buckets every metric by **commit-date week**,
  including `prs_merged` (week is the PR's merge-commit week when
  `merge_commit_hash` resolves, else `closed_on` week). All CTEs share the
  same week semantic so the final join never splits a person across rows.
- `file_category` regex matches common test/spec/config path patterns.
- PR `state` values are source-case-preserved (`MERGED`, `OPEN`). Facts expose
  `state_norm = lower(state)` for downstream use.
- `pickup_time` and `rework_ratio` are excluded here because Bitbucket Cloud
  staging currently lacks the signals they need (`reviewed_at` NULL,
  `commit_order` zero). Both return once the Bitbucket Cloud stream-expansion
  PR lands.
