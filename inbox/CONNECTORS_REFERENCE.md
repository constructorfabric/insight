# Connector Reference: Data Sources for Constructor Insight


<!-- toc -->

- [How Data Flows](#how-data-flows)
- [Source 1: GitHub (Version Control)](#source-1-github-version-control)
  - [`github_repositories`](#githubrepositories)
  - [`github_branches`](#githubbranches)
  - [`github_commits`](#githubcommits)
  - [`github_commit_files` — Per-file line changes](#githubcommitfiles-per-file-line-changes)
  - [`github_pull_requests`](#githubpullrequests)
  - [`github_pull_request_reviews` — Formal review submissions](#githubpullrequestreviews-formal-review-submissions)
  - [`github_pull_request_comments`](#githubpullrequestcomments)
  - [`github_pull_request_commits`](#githubpullrequestcommits)
  - [`github_ticket_refs` — Ticket references extracted from PRs and commits](#githubticketrefs-ticket-references-extracted-from-prs-and-commits)
  - [`github_collection_runs` — Connector execution log](#githubcollectionruns-connector-execution-log)
- [Source 2: Bitbucket (Version Control)](#source-2-bitbucket-version-control)
- [Source 3: GitLab (Version Control)](#source-3-gitlab-version-control)
- [Source 4: YouTrack (Task Tracking)](#source-4-youtrack-task-tracking)
  - [`youtrack_issue` — Issue identifiers and timestamps](#youtrackissue-issue-identifiers-and-timestamps)
  - [`youtrack_issue_history` — Complete field change log](#youtrackissuehistory-complete-field-change-log)
  - [`youtrack_user` — User directory](#youtrackuser-user-directory)
- [Source 5: Jira (Task Tracking)](#source-5-jira-task-tracking)
  - [`jira_issue` — Issue identifiers and timestamps](#jiraissue-issue-identifiers-and-timestamps)
  - [`jira_issue_history` — Complete changelog (field change log)](#jiraissuehistory-complete-changelog-field-change-log)
  - [`jira_user` — User directory](#jirauser-user-directory)
- [Source 6: Microsoft 365 (Communication)](#source-6-microsoft-365-communication)
  - [`ms365_email_activity` — Email activity per user per date](#ms365emailactivity-email-activity-per-user-per-date)
  - [`ms365_teams_activity` — Teams activity per user per date](#ms365teamsactivity-teams-activity-per-user-per-date)
  - [`ms365_onedrive_activity` — OneDrive activity per user per date](#ms365onedriveactivity-onedrive-activity-per-user-per-date)
  - [`ms365_sharepoint_activity` — SharePoint activity per user per date](#ms365sharepointactivity-sharepoint-activity-per-user-per-date)
- [Source 7: Zulip (Chat)](#source-7-zulip-chat)
  - [`zulip_users` — User directory](#zulipusers-user-directory)
  - [`zulip_messages` — Aggregated message counts per sender](#zulipmessages-aggregated-message-counts-per-sender)
- [Source 8: Cursor (AI Dev Tool)](#source-8-cursor-ai-dev-tool)
  - [`cursor_daily_usage` — Daily aggregated usage per user](#cursordailyusage-daily-aggregated-usage-per-user)
  - [`cursor_events` — Individual AI invocation events](#cursorevents-individual-ai-invocation-events)
  - [`cursor_events_token_usage` — Token consumption per event (1:1 with cursor_events)](#cursoreventstokenusage-token-consumption-per-event-11-with-cursorevents)
- [Source 9: Windsurf (AI Dev Tool)](#source-9-windsurf-ai-dev-tool)
  - [`windsurf_daily_usage` — Daily aggregated usage per user](#windsurfdailyusage-daily-aggregated-usage-per-user)
  - [`windsurf_events` — Individual AI invocation events](#windsurfevents-individual-ai-invocation-events)
- [Unified Stream 1: `class_communication_metrics`](#unified-stream-1-classcommunicationevents)
- [Unified Stream 2: Task Tracker (Silver → Gold)](#unified-stream-2-task-tracker-silver-gold)
  - [Silver Step 1: `class_task_tracker_activities` — Unified event stream](#silver-step-1-classtasktrackeractivities-unified-event-stream)
  - [Silver Step 1 (parallel): `class_task_tracker_snapshot` — Current state (upsert)](#silver-step-1-parallel-classtasktrackersnapshot-current-state-upsert)
  - [Silver Step 2: `class_task_tracker` — Identity resolution + final Silver contract](#silver-step-2-classtasktracker-identity-resolution-final-silver-contract)
  - [Gold: Derived metrics (built on top of `class_task_tracker`)](#gold-derived-metrics-built-on-top-of-classtasktracker)
- [Source 10: BambooHR (HR)](#source-10-bamboohr-hr)
  - [`bamboohr_employees` — Employee records](#bamboohremployees-employee-records)
  - [`bamboohr_departments` — Department hierarchy](#bamboohrdepartments-department-hierarchy)
  - [`bamboohr_leave_requests` — Time off requests](#bamboohrleaverequests-time-off-requests)
- [Source 11: Workday (HR)](#source-11-workday-hr)
  - [`workday_workers` — Worker records (point-in-time)](#workdayworkers-worker-records-point-in-time)
  - [`workday_organizations` — Org units (departments, supervisory orgs, cost centers)](#workdayorganizations-org-units-departments-supervisory-orgs-cost-centers)
  - [`workday_leave` — Leave of absence and time off](#workdayleave-leave-of-absence-and-time-off)
- [Source 12: LDAP / Active Directory (Directory)](#source-12-ldap-active-directory-directory)
  - [`ldap_users` — User account directory](#ldapusers-user-account-directory)
  - [`ldap_group_members` — Group and OU membership](#ldapgroupmembers-group-and-ou-membership)
- [Source 13: Claude Admin — Anthropic API (AI Tool)](#source-13-claude-admin-anthropic-api-ai-tool)
  - [`claude_admin_messages_usage` — Daily token usage per API key per model](#claudeadminmessagesusage-daily-token-usage-per-api-key-per-model)
- [Source 14: Claude Admin — Anthropic Team / Seat Plans (AI Tool)](#source-14-claude-admin-anthropic-team-seat-plans-ai-tool)
  - [`claude_admin_users` — Seat assignment and status](#claudeadminusers-seat-assignment-and-status)
  - [`claude_admin_code_usage` — Daily Claude Code usage per user](#claudeadmincodeusage-daily-claude-code-usage-per-user)
- [Source 15: GitHub Copilot (AI Dev Tool)](#source-15-github-copilot-ai-dev-tool)
  - [`copilot_seats` — Seat assignment and last activity](#copilotseats-seat-assignment-and-last-activity)
  - [`copilot_usage` — Org-level daily usage totals](#copilotusage-org-level-daily-usage-totals)
  - [`copilot_usage_breakdown` — Daily breakdown by language and editor](#copilotusagebreakdown-daily-breakdown-by-language-and-editor)
- [Source 16: HubSpot (CRM)](#source-16-hubspot-crm)
  - [`hubspot_contacts` — Person records](#hubspotcontacts-person-records)
  - [`hubspot_companies` — Company / account records](#hubspotcompanies-company-account-records)
  - [`hubspot_deals` — Deal pipeline records](#hubspotdeals-deal-pipeline-records)
  - [`hubspot_activities` — Calls, emails, meetings, tasks](#hubspotactivities-calls-emails-meetings-tasks)
  - [`hubspot_owners` — HubSpot user directory (salespeople)](#hubspotowners-hubspot-user-directory-salespeople)
- [Source 17: Salesforce (CRM)](#source-17-salesforce-crm)
  - [`salesforce_contacts`](#salesforcecontacts)
  - [`salesforce_accounts` — Company / account records](#salesforceaccounts-company-account-records)
  - [`salesforce_opportunities` — Deal pipeline records](#salesforceopportunities-deal-pipeline-records)
  - [`salesforce_activities` — Tasks and Events](#salesforceactivities-tasks-and-events)
  - [`salesforce_users` — User directory](#salesforceusers-user-directory)
- [Source 18: OpenAI API (AI Tool)](#source-18-openai-api-ai-tool)
  - [`openai_api_daily_usage` — Daily token usage per API key per model](#openaiapidailyusage-daily-token-usage-per-api-key-per-model)
  - [`openai_api_requests` — Individual API request events](#openaiapirequests-individual-api-request-events)
- [Source 19: ChatGPT Team (AI Tool)](#source-19-chatgpt-team-ai-tool)
  - [`chatgpt_team_seats`](#chatgptteamseats)
  - [`chatgpt_team_activity` — Daily usage per user per model](#chatgptteamactivity-daily-usage-per-user-per-model)
- [Source 20: Allure TestOps (Quality / Testing)](#source-20-allure-testops-quality-testing)
  - [`allure_launches` — Test run / launch records](#allurelaunches-test-run-launch-records)
  - [`allure_test_results` — Individual test case results](#alluretestresults-individual-test-case-results)
  - [`allure_defects` — Defects linked to test failures](#alluredefects-defects-linked-to-test-failures)
- [All Tables at a Glance](#all-tables-at-a-glance)
- [Open Questions](#open-questions)
  - [OQ-1: Git deduplication across sources](#oq-1-git-deduplication-across-sources)
  - [OQ-2: Identity re-resolution strategy](#oq-2-identity-re-resolution-strategy)
  - [OQ-3: AI API tools — per-key user attribution](#oq-3-ai-api-tools-per-key-user-attribution)
  - [OQ-4: Zulip Bronze schema — extra fields in stream spec](#oq-4-zulip-bronze-schema-extra-fields-in-stream-spec)
  - [OQ-5: YouTrack Bronze schema — `insight_source_id` presence](#oq-5-youtrack-bronze-schema-sourceinstanceid-presence)
  - [OQ-6: `class_task_tracker_activities` — author/assignee field types](#oq-6-classtasktrackeractivities-authorassignee-field-types)
  - [OQ-7: Silver table naming convention — `class_` vs `_enriched`](#oq-7-silver-table-naming-convention-class-vs-enriched)
  - [OQ-8: Git Bronze schema — per-source tables vs unified table with `data_source`; extra tables](#oq-8-git-bronze-schema-per-source-tables-vs-unified-table-with-datasource-extra-tables)
  - [OQ-9: Git streams layer (`streams/raw_git/`) — relationship to Bronze](#oq-9-git-streams-layer-streamsrawgit-relationship-to-bronze)
  - [OQ-11: Pipeline architecture model — Medallion (Bronze/Silver/Gold) vs dbt-Mart](#oq-11-pipeline-architecture-model-medallion-bronzesilvergold-vs-dbt-mart)
  - [OQ-10: Git PR/commit field naming — `owner`/`repo_name` vs `project_key`/`repo_slug`](#oq-10-git-prcommit-field-naming-ownerreponame-vs-projectkeyreposlug)

<!-- /toc -->

> Version 2.13 — March 2026
> Based on: insight-spec PR #3 (Streams Proposal), PR #1 (GitHub/Bitbucket ETL), TASK_TRACKER_ANALYTICS.md (team meetings Jan–Feb 2026)

---

## How Data Flows

```
Source APIs (GitHub, Jira, M365, Cursor, BambooHR, ...)
    │
    ▼ Extract via connector
┌─────────────────────────────────────────────────┐
│  BRONZE — Raw tables per source                 │  {source}_{entity}
│  github_commits, youtrack_issue_history, ...    │  source-native schema + IDs
└─────────────────────────────────────────────────┘
    │                                    │
    ▼ Unify across sources               ▼ HR connectors also feed
┌───────────────────────────────┐   ┌───────────────────────────────┐
│  SILVER step 1                │   │  Identity Manager             │
│  class_{domain}               │   │  (PostgreSQL/MariaDB)         │
│  unified schema,              │   │  person_id ← email, username, │
│  source-native user IDs       │   │  employee_id, git login, ...  │
└───────────────────────────────┘   └───────────────────────────────┘
    │                                    │
    ▼ Resolve identities (separate job)  │ person_id lookup
┌─────────────────────────────────────────────────┐
│  SILVER step 2                                  │  class_{domain}
│  class_{domain}                                 │  canonical person_id replaces
│  same table name, identity-resolved rows        │  source-native user IDs
└─────────────────────────────────────────────────┘
    │
    ▼ Aggregate + derive (per-client config)
┌─────────────────────────────────────────────────┐
│  GOLD — Derived metrics                         │  domain-specific names
│  status_periods, throughput, wip_snapshots, ... │  no raw events
└─────────────────────────────────────────────────┘
```

**Naming convention:**
- Bronze: `{source}_{entity}` — e.g. `github_commits`, `youtrack_issue_history`
- Silver: `class_{domain}` — e.g. `class_commits`, `class_task_tracker`, `class_communication_metrics`
- Gold: domain-specific derived names — e.g. `status_periods`, `throughput`

Silver step 1 and step 2 share the same `class_` prefix — both represent unified, cross-source data. Step 2 adds canonical `person_id` via a separate identity resolution job.

---

## Source 1: GitHub (Version Control)

**Why multiple tables:** Git data is inherently relational — a commit has many files, a PR has many reviewers, comments, and commits. These are genuine 1:N relationships.

**API:** REST v3 + GraphQL v4. User identity: `login` (username) + numeric `id`. Email comes from the commit object, not the API user record. PRs have a formal review model with states (`APPROVED` / `CHANGES_REQUESTED` / `COMMENTED` / `DISMISSED`).

---

### `github_repositories`

| Field | Type | Description |
|-------|------|-------------|
| `owner` | String | Organization or user login |
| `repo_name` | String | Repository name |
| `full_name` | String | Full path, e.g. `org/repo` |
| `description` | String | Repository description |
| `is_private` | Int | 1 if private |
| `language` | String | Primary programming language |
| `size` | Int | Repository size in KB |
| `created_at` | DateTime | Repository creation date |
| `updated_at` | DateTime | Last update |
| `pushed_at` | DateTime | Date of most recent push |
| `default_branch` | String | Default branch name |
| `is_empty` | Int | 1 if no commits |
| `metadata` | String (JSON) | Full API response |

---

### `github_branches`

| Field | Type | Description |
|-------|------|-------------|
| `owner` | String | Repository owner |
| `repo_name` | String | Repository name |
| `branch_name` | String | Branch name |
| `is_default` | Int | 1 if default branch |
| `last_commit_hash` | String | Last collected commit — cursor for incremental sync |
| `last_commit_date` | DateTime | Date of last commit |
| `last_checked_at` | DateTime | When this branch was last checked |

---

### `github_commits`

| Field | Type | Description |
|-------|------|-------------|
| `owner` | String | Repository owner |
| `repo_name` | String | Repository name |
| `commit_hash` | String | Git SHA-1 (40 chars) |
| `branch` | String | Branch where commit was found |
| `author_name` | String | Commit author name |
| `author_email` | String | Author email — used for identity resolution |
| `author_login` | String | GitHub username of author (if matched) |
| `committer_name` | String | Committer name |
| `committer_email` | String | Committer email |
| `message` | String | Commit message |
| `date` | DateTime | Commit timestamp |
| `parents` | String (JSON) | Parent commit hashes — len > 1 = merge commit |
| `files_changed` | Int | Number of files modified |
| `lines_added` | Int | Total lines added |
| `lines_removed` | Int | Total lines removed |
| `is_merge_commit` | Int | 1 if merge commit |
| `language_breakdown` | String (JSON) | Lines per language, e.g. `{"TypeScript": 120}` |
| `ai_percentage` | Float | AI-generated code estimate (0.0–1.0) |
| `ai_thirdparty_flag` | Int | 1 if AI-detected third-party code |
| `scancode_thirdparty_flag` | Int | 1 if license scanner detected third-party |
| `metadata` | String (JSON) | Full API response |

---

### `github_commit_files` — Per-file line changes

| Field | Type | Description |
|-------|------|-------------|
| `owner` | String | Repository owner |
| `repo_name` | String | Repository name |
| `commit_hash` | String | Parent commit |
| `file_path` | String | Full file path |
| `file_extension` | String | File extension |
| `lines_added` | Int | Lines added in this file |
| `lines_removed` | Int | Lines removed in this file |
| `ai_thirdparty_flag` | Int | AI-detected third-party code |
| `scancode_thirdparty_flag` | Int | License scanner detected third-party |
| `scancode_metadata` | String (JSON) | License and copyright info |

---

### `github_pull_requests`

| Field | Type | Description |
|-------|------|-------------|
| `owner` | String | Repository owner |
| `repo_name` | String | Repository name |
| `pr_number` | Int | PR number (unique per repo) |
| `node_id` | String | GraphQL global node ID |
| `title` | String | PR title |
| `body` | String | PR description |
| `state` | String | `open` / `closed` / `merged` |
| `draft` | Int | 1 if draft PR |
| `author_login` | String | PR author GitHub login |
| `author_email` | String | Author email (from commit) |
| `head_branch` | String | Source branch |
| `base_branch` | String | Target branch |
| `created_at` | DateTime | PR creation time |
| `updated_at` | DateTime | Last update |
| `merged_at` | DateTime | Merge time (NULL if not merged) |
| `closed_at` | DateTime | Close time |
| `merged_by_login` | String | GitHub login of who merged |
| `merge_commit_hash` | String | Hash of merge commit |
| `files_changed` | Int | Files modified |
| `lines_added` | Int | Lines added |
| `lines_removed` | Int | Lines removed |
| `commit_count` | Int | Number of commits in PR |
| `comment_count` | Int | Number of general comments |
| `review_comment_count` | Int | Number of inline review comments |
| `duration_seconds` | Int | Time from creation to close |
| `ticket_refs` | String (JSON) | Extracted issue / ticket IDs |

---

### `github_pull_request_reviews` — Formal review submissions

| Field | Type | Description |
|-------|------|-------------|
| `owner` / `repo_name` | String | Repository reference |
| `pr_number` | Int | Parent PR |
| `review_id` | Int | Review unique ID |
| `reviewer_login` | String | Reviewer GitHub login |
| `reviewer_email` | String | Reviewer email |
| `state` | String | `APPROVED` / `CHANGES_REQUESTED` / `COMMENTED` / `DISMISSED` |
| `submitted_at` | DateTime | Review submission time |

GitHub's formal review model distinguishes review state from plain comments — Bitbucket and GitLab do not have an equivalent.

---

### `github_pull_request_comments`

| Field | Type | Description |
|-------|------|-------------|
| `owner` / `repo_name` | String | Repository reference |
| `pr_number` | Int | Parent PR |
| `comment_id` | Int | Comment unique ID |
| `comment_type` | String | `issue_comment` (general) / `review_comment` (inline on file) |
| `content` | String | Comment text (Markdown) |
| `author_login` | String | Comment author login |
| `author_email` | String | Author email |
| `created_at` / `updated_at` | DateTime | Timestamps |
| `file_path` | String | File path for inline comments (NULL for general) |
| `line_number` | Int | Line number for inline comments (NULL for general) |
| `in_reply_to_id` | Int | Parent comment ID for threaded replies |

---

### `github_pull_request_commits`

| Field | Type | Description |
|-------|------|-------------|
| `owner` / `repo_name` | String | Repository reference |
| `pr_number` | Int | Parent PR |
| `commit_hash` | String | Commit SHA |
| `commit_order` | Int | Order within PR (0-indexed) |

---

### `github_ticket_refs` — Ticket references extracted from PRs and commits

| Field | Type | Description |
|-------|------|-------------|
| `external_ticket_id` | String | Ticket ID, e.g. `PROJ-123` |
| `owner` / `repo_name` | String | Repository reference |
| `pr_number` | Int | Associated PR (NULL if from commit) |
| `commit_hash` | String | Associated commit (NULL if from PR) |

Links code activity back to task tracker items without requiring real-time joins.

---

### `github_collection_runs` — Connector execution log

| Field | Type | Description |
|-------|------|-------------|
| `run_id` | String | Unique run identifier |
| `started_at` / `completed_at` | DateTime | Run timing |
| `status` | String | `running` / `completed` / `failed` |
| `repos_processed` | Int | Repositories processed |
| `commits_collected` | Int | Commits collected |
| `prs_collected` | Int | PRs collected |
| `api_calls` | Int | API calls made |
| `errors` | Int | Errors encountered |
| `settings` | String (JSON) | Collection configuration |

Monitoring table — not an analytics source.

---

## Source 2: Bitbucket (Version Control)

**Same 9 data tables as GitHub** — repositories, branches, commits, commit files, pull requests, pull request reviewers, pull request comments, pull request commits, ticket refs — plus `bitbucket_collection_runs`. All with `bitbucket_` prefix.

**API:** REST v1/v2. Key structural differences from GitHub:

| Aspect | GitHub | Bitbucket |
|--------|--------|-----------|
| User identity | `login` (username) | `uuid` + `account_id` |
| Namespace field | `owner` | `workspace` |
| PR review model | Formal reviews with `state` (`APPROVED` / `CHANGES_REQUESTED` / etc.) | Simple reviewer list with `status` (`APPROVED` / `UNAPPROVED` / `NEEDS_WORK`) |
| Comment severity | — | `severity`: `NORMAL` / `BLOCKER` (blocking comments must be resolved before merge) |
| PR state values | `open` / `closed` / `merged` | `OPEN` / `MERGED` / `DECLINED` |
| Draft PRs | `draft` boolean | Not supported |
| Merged by | `merged_by_login` | Not returned by API |
| Review comments | `comment_type` distinguishes general vs inline | All comments are the same kind |

**Bitbucket-specific field differences:**

- `bitbucket_pull_requests`: `workspace` instead of `owner`; no `draft`, no `merged_by_login`
- `bitbucket_pull_request_reviewers`: `reviewer_uuid`, `reviewer_account_id`, `status` (`APPROVED` / `UNAPPROVED` / `NEEDS_WORK`) — no separate review state table
- `bitbucket_pull_request_comments`: adds `severity` (`NORMAL` / `BLOCKER`); no `comment_type`, no `in_reply_to_id`

---

## Source 3: GitLab (Version Control)

**Same logical structure** — repositories, branches, commits, merge requests, reviewers, comments, commits-in-MR, ticket refs, collection runs — all with `gitlab_` prefix.

**API:** REST v4. Key structural differences from GitHub:

| Aspect | GitHub | GitLab |
|--------|--------|--------|
| PR terminology | Pull Request | Merge Request |
| PR identifier | `pr_number` (per-repo) | `mr_iid` (per-project) + `mr_id` (global) |
| User identity | `login` | `username` + numeric `id` |
| Commit file stats | Inline in `github_commits` + `github_commit_files` | Separate `gitlab_num_stat` table; file paths in `gitlab_files` lookup |
| Review model | `github_pull_request_reviews` with state | `gitlab_mr_approvals` — approval only, no `CHANGES_REQUESTED` |
| MR state values | `open` / `closed` / `merged` | `opened` / `closed` / `merged` / `locked` |
| Draft MRs | `draft` boolean | `work_in_progress` boolean (legacy `WIP:` title prefix) |

**GitLab-specific tables** (replace or supplement GitHub equivalents):

- `gitlab_num_stat` — per-file line changes linked to commits (separate table, not inline in `gitlab_commits`)
- `gitlab_files` — file path lookup table used by `gitlab_num_stat`
- `gitlab_mr_approvals` — who approved an MR (replaces `github_pull_request_reviews`; approval only, no state transitions)

---

## Source 4: YouTrack (Task Tracking)

**Why multiple tables:** Issue and history are a genuine 1:N relationship — one issue has many field-change events. Merging would require either denormalization (repeat issue metadata on every history row) or loss of the history model entirely.

---

### `youtrack_issue` — Issue identifiers and timestamps

| Field | Type | Description |
|-------|------|-------------|
| `insight_source_id` | text | Connector instance identifier, e.g. `youtrack-acme-prod` |
| `youtrack_id` | text | YouTrack internal ID, e.g. `2-12345` |
| `id_readable` | text | Human-readable ID, e.g. `MON-123` |
| `created` | timestamp | Issue creation timestamp |
| `updated` | timestamp | Last update — cursor for incremental sync |

Intentionally minimal. All state lives in the history table.

---

### `youtrack_issue_history` — Complete field change log

Every state transition, reassignment, and field update is a separate row.

| Field | Type | Description |
|-------|------|-------------|
| `id_readable` | varchar | Human-readable issue ID |
| `issue_youtrack_id` | varchar | Parent issue's internal ID |
| `author_youtrack_id` | varchar | Who made the change |
| `activity_id` | varchar | Batch ID — multiple changes in one operation share this |
| `created_at` | timestamptz | When the change was made |
| `field_id` | varchar | Machine-readable field identifier |
| `field_name` | varchar | Human-readable field name, e.g. `State`, `Assignee` |
| `value` | jsonb | New field value after the change |
| `value_id` | varchar | Unique value change ID — for deduplication |

**`value` varies by field type:** string for State/Priority, object `{"name": "...", "id": "..."}` for user fields, array for tags.

---

### `youtrack_user` — User directory

| Field | Type | Description |
|-------|------|-------------|
| `youtrack_id` | varchar | YouTrack internal user ID |
| `email` | varchar | Email — primary key for cross-system identity resolution |
| `full_name` | varchar | Display name |
| `username` | varchar | Login username |

---

## Source 5: Jira (Task Tracking)

**Same model as YouTrack** — issue + full changelog history + user directory. Three tables with the same logical structure.

**Key API difference from YouTrack:** Jira changelog stores both the old (`from`) and new (`to`) values for each field change, and uses Atlassian account IDs (not internal numeric IDs) as user identifiers.

---

### `jira_issue` — Issue identifiers and timestamps

| Field | Type | Description |
|-------|------|-------------|
| `insight_source_id` | text | Connector instance identifier, e.g. `jira-team-alpha` |
| `jira_id` | text | Jira internal numeric ID, e.g. `10001` |
| `id_readable` | text | Human-readable key, e.g. `PROJ-123` |
| `project_key` | text | Project key, e.g. `PROJ` |
| `created` | timestamp | Issue creation timestamp |
| `updated` | timestamp | Last update — cursor for incremental sync |

Intentionally minimal. All state lives in the changelog table.

---

### `jira_issue_history` — Complete changelog (field change log)

Every state transition, reassignment, and field update is a separate row. Jira's changelog API returns one entry per operation; each entry may contain multiple field changes — each stored as a separate row.

| Field | Type | Description |
|-------|------|-------------|
| `id_readable` | varchar | Human-readable issue key, e.g. `PROJ-123` |
| `issue_jira_id` | varchar | Parent issue's internal numeric ID |
| `author_account_id` | varchar | Atlassian account ID of who made the change |
| `changelog_id` | varchar | Changelog entry ID — multiple field changes in one operation share this |
| `created_at` | timestamptz | When the change was made |
| `field_id` | varchar | Machine-readable field identifier |
| `field_name` | varchar | Human-readable field name, e.g. `status`, `assignee` |
| `value_from` | varchar | Previous raw value (ID or key) |
| `value_from_string` | varchar | Previous human-readable value, e.g. `In Progress` |
| `value_to` | varchar | New raw value after the change |
| `value_to_string` | varchar | New human-readable value, e.g. `Done` |

**`changelog_id` groups related changes:** when a user performs one action that updates multiple fields simultaneously, all resulting rows share the same `changelog_id`.

---

### `jira_user` — User directory

| Field | Type | Description |
|-------|------|-------------|
| `account_id` | varchar | Atlassian account ID — primary key |
| `email` | varchar | Email — primary key for cross-system identity resolution |
| `display_name` | varchar | Display name |
| `account_type` | varchar | `atlassian` / `app` / `customer` |
| `active` | boolean | Whether the account is active |

---

## Source 6: Microsoft 365 (Communication)

**Four separate Bronze tables** — one per report endpoint. M365 Graph API exposes Email, Teams, OneDrive, and SharePoint activity as separate endpoints. The connector stores each as its own table; all share the same identity key (`userPrincipalName + reportRefreshDate`).

**Field naming:** camelCase — original Graph API field names, no renaming at Bronze level.

**M365 Copilot** (`getMicrosoft365CopilotUsageUserDetail`) is a separate Graph API endpoint covering AI usage across Office apps. Not yet collected as a Bronze table — planned for future implementation.

> **Critical:** M365 Graph API returns only the last 7–30 days of activity. Data cannot be re-fetched once the window passes — loss is permanent.

---

### `ms365_email_activity` — Email activity per user per date

| Field | Type | Description |
|-------|------|-------------|
| `unique` | text | Primary key |
| `userPrincipalName` | text | User email (UPN) — identity key |
| `reportRefreshDate` | date | Report date |
| `reportPeriod` | text | Report period duration, e.g. `"7"` |
| `displayName` | text | User display name |
| `isDeleted` | boolean | Whether the account is deleted |
| `assignedProducts` | jsonb | M365 licenses assigned |
| `sendCount` | numeric | Emails sent |
| `receiveCount` | numeric | Emails received |
| `readCount` | numeric | Emails read |
| `meetingCreatedCount` | numeric | Meetings created via email |
| `meetingInteractedCount` | numeric | Meeting interactions via email |
| `lastActivityDate` | date | Last email activity (NULL if none) |

---

### `ms365_teams_activity` — Teams activity per user per date

| Field | Type | Description |
|-------|------|-------------|
| `unique` | text | Primary key |
| `userPrincipalName` | text | User email (UPN) — identity key |
| `userId` | text | Microsoft internal user ID |
| `reportRefreshDate` | date | Report date |
| `reportPeriod` | text | Report period duration |
| `tenantDisplayName` | text | M365 tenant name |
| `isDeleted` | boolean | Whether the account is deleted |
| `isLicensed` | boolean | Whether user has Teams license |
| `isExternal` | boolean | Whether user is an external guest |
| `hasOtherAction` | boolean | Whether user had other Teams actions not captured by specific metrics |
| `assignedProducts` | jsonb | M365 licenses assigned |
| `teamChatMessageCount` | numeric | Messages in team/group chats |
| `privateChatMessageCount` | numeric | Messages in private (1:1) chats |
| `postMessages` | numeric | Posts published in team channels |
| `replyMessages` | numeric | Replies to channel posts |
| `callCount` | numeric | Calls made |
| `meetingCount` | numeric | Total meetings count |
| `meetingsAttendedCount` | numeric | Meetings attended |
| `meetingsOrganizedCount` | numeric | Meetings organized |
| `adHocMeetingsAttendedCount` | numeric | Ad-hoc (unscheduled) meetings attended |
| `adHocMeetingsOrganizedCount` | numeric | Ad-hoc meetings organized |
| `scheduledOneTimeMeetingsAttendedCount` | numeric | One-time scheduled meetings attended |
| `scheduledOneTimeMeetingsOrganizedCount` | numeric | One-time scheduled meetings organized |
| `scheduledRecurringMeetingsAttendedCount` | numeric | Recurring meetings attended |
| `scheduledRecurringMeetingsOrganizedCount` | numeric | Recurring meetings organized |
| `audioDuration` | text | Total audio call duration |
| `videoDuration` | text | Total video duration |
| `screenShareDuration` | text | Total screen sharing duration |
| `urgentMessages` | numeric | Messages sent with urgent priority |
| `sharedChannelTenantDisplayNames` | text | Tenant names for shared channels |
| `lastActivityDate` | date | Last Teams activity |

**Total chat messages** = `teamChatMessageCount + privateChatMessageCount`. `postMessages` and `replyMessages` are channel activity (content publishing), not direct messaging — exclude from message counts.

---

### `ms365_onedrive_activity` — OneDrive activity per user per date

| Field | Type | Description |
|-------|------|-------------|
| `unique` | text | Primary key |
| `userPrincipalName` | text | User email (UPN) — identity key |
| `reportRefreshDate` | date | Report date |
| `reportPeriod` | text | Report period duration |
| `isDeleted` | boolean | Whether the account is deleted |
| `assignedProducts` | jsonb | M365 licenses assigned |
| `viewedOrEditedFileCount` | numeric | Files viewed or edited |
| `syncedFileCount` | numeric | Files synced via desktop client |
| `sharedInternallyFileCount` | numeric | Files shared with internal users |
| `sharedExternallyFileCount` | numeric | Files shared externally |
| `lastActivityDate` | date | Last OneDrive activity |

---

### `ms365_sharepoint_activity` — SharePoint activity per user per date

| Field | Type | Description |
|-------|------|-------------|
| `unique` | text | Primary key |
| `userPrincipalName` | text | User email (UPN) — identity key |
| `reportRefreshDate` | date | Report date |
| `reportPeriod` | text | Report period duration |
| `isDeleted` | boolean | Whether the account is deleted |
| `assignedProducts` | jsonb | M365 licenses assigned |
| `viewedOrEditedFileCount` | numeric | SharePoint files viewed or edited |
| `visitedPageCount` | numeric | SharePoint pages visited |
| `syncedFileCount` | numeric | Files synced from SharePoint |
| `sharedInternallyFileCount` | numeric | Files shared internally |
| `sharedExternallyFileCount` | numeric | Files shared externally |
| `lastActivityDate` | date | Last SharePoint activity |

**What feeds downstream:** `ms365_email_activity` and `ms365_teams_activity` → `class_communication_metrics` (Silver step 1). OneDrive and SharePoint tables are collected but not yet mapped to a unified stream — available for future use without re-fetching.

---

## Source 7: Zulip (Chat)

**Why two tables:** Users and messages are a 1:N relationship. `zulip_users` is the identity anchor; `zulip_messages` holds aggregated counts. Could not be merged without repeating user metadata on every message record.

---

### `zulip_users` — User directory

| Field | Type | Description |
|-------|------|-------------|
| `id` | bigint | Zulip user ID — primary key |
| `email` | text | Email — used for cross-system identity resolution |
| `full_name` | text | Display name |
| `role` | numeric | 100 owner / 200 admin / 400 member / 600 guest |
| `is_active` | boolean | Whether account is active |
| `uuid` | text | Universally unique identifier |

---

### `zulip_messages` — Aggregated message counts per sender

| Field | Type | Description |
|-------|------|-------------|
| `sender_id` | bigint | Sender's Zulip user ID |
| `count` | numeric | Number of messages in this record |
| `created_at` | timestamptz | Message timestamp / aggregation period |

Aggregated counts — individual message content is not collected.

---

## Source 8: Cursor (AI Dev Tool)

**Why three tables:** `cursor_daily_usage` and `cursor_events` are different granularities — aggregated daily vs individual invocations. `cursor_events_token_usage` is 1:1 with `cursor_events` and could be merged into it, but is kept separate to match the API response structure and allow NULL-free storage when token data is absent.

---

### `cursor_daily_usage` — Daily aggregated usage per user

| Field | Type | Description |
|-------|------|-------------|
| `unique` | text | Primary key |
| `day` | text | Day label |
| `date` | bigint | Unix timestamp in milliseconds |
| `email` | text | User email |
| `userId` | text | Cursor platform user ID |
| `isActive` | boolean | Whether user had any activity this day |
| `chatRequests` | numeric | AI chat interactions |
| `cmdkUsages` | numeric | Cmd+K (inline edit) usages |
| `composerRequests` | numeric | Composer feature requests |
| `agentRequests` | numeric | Agent mode requests |
| `bugbotUsages` | numeric | Bug bot usages |
| `totalTabsShown` | numeric | Tab completion suggestions shown |
| `totalTabsAccepted` | numeric | Tab completions accepted |
| `totalAccepts` | numeric | All AI suggestions accepted |
| `totalApplies` | numeric | Code applications (apply to file) |
| `totalRejects` | numeric | Suggestions rejected |
| `totalLinesAdded` | numeric | Total lines of code added |
| `totalLinesDeleted` | numeric | Total lines deleted |
| `acceptedLinesAdded` | numeric | Lines added from accepted AI suggestions |
| `acceptedLinesDeleted` | numeric | Lines deleted from accepted AI suggestions |
| `mostUsedModel` | text | Most used AI model that day, e.g. `claude-3.5-sonnet` |
| `tabMostUsedExtension` | text | File extension with most tab completions |
| `applyMostUsedExtension` | text | File extension with most applies |
| `clientVersion` | text | Cursor IDE version |
| `subscriptionIncludedReqs` | numeric | Requests covered by subscription |
| `usageBasedReqs` | numeric | Requests on usage-based billing |
| `apiKeyReqs` | numeric | Requests using API key |

---

### `cursor_events` — Individual AI invocation events

| Field | Type | Description |
|-------|------|-------------|
| `unique` | text | Primary key |
| `userEmail` | text | User email |
| `timestamp` | timestamptz | Event timestamp |
| `kind` | text | Event type: `chat`, `completion`, `agent`, `cmd-k`, etc. |
| `model` | text | AI model used, e.g. `gpt-4o`, `claude-3.5-sonnet` |
| `maxMode` | boolean | Whether max mode was enabled |
| `isChargeable` | boolean | Whether event incurs billing |
| `requestsCosts` | numeric | Request cost in credits |
| `cursorTokenFee` | numeric | Cursor platform fee |
| `isTokenBasedCall` | boolean | Billed by tokens vs per-request |
| `isHeadless` | boolean | Triggered without UI (automated) |

---

### `cursor_events_token_usage` — Token consumption per event (1:1 with cursor_events)

| Field | Type | Description |
|-------|------|-------------|
| `event_unique` | text | Parent event reference (`cursor_events.unique`) |
| `inputTokens` | numeric | Tokens in the prompt |
| `outputTokens` | numeric | Tokens in the model response |
| `cacheReadTokens` | numeric | Tokens served from prompt cache |
| `cacheWriteTokens` | numeric | Tokens written to cache |
| `totalCents` | numeric | Total cost in cents |
| `discountPercentOff` | numeric | Discount applied |

All fields nullable — not all events have token-level detail.

---

## Source 9: Windsurf (AI Dev Tool)

**Same logical model as Cursor** — daily aggregates + individual invocation events. Key differences: Windsurf's primary AI feature is **Cascade** (chat + agent in one surface, no separate "composer"); completions are called **Supercomplete** in addition to standard tab completions. Token usage is included inline in the events table (no separate 1:1 table).

---

### `windsurf_daily_usage` — Daily aggregated usage per user

| Field | Type | Description |
|-------|------|-------------|
| `email` | text | User email |
| `user_id` | text | Windsurf / Codeium platform user ID |
| `date` | date | Activity date |
| `is_active` | boolean | Whether user had any activity this day |
| `completions_shown` | numeric | AI completion suggestions shown |
| `completions_accepted` | numeric | Suggestions accepted (tab) |
| `supercomplete_shown` | numeric | Supercomplete suggestions shown |
| `supercomplete_accepted` | numeric | Supercomplete suggestions accepted |
| `lines_accepted` | numeric | Lines of code accepted from AI suggestions |
| `cascade_chat_requests` | numeric | Cascade chat interactions |
| `cascade_agent_requests` | numeric | Cascade agent (multi-step) interactions |
| `cascade_write_actions` | numeric | File write operations performed by Cascade agent |
| `most_used_model` | text | Most used AI model that day, e.g. `claude-3.5-sonnet` |
| `client_version` | text | Windsurf IDE version |
| `subscription_included_reqs` | numeric | Requests covered by subscription |
| `usage_based_reqs` | numeric | Requests on usage-based billing |

---

### `windsurf_events` — Individual AI invocation events

| Field | Type | Description |
|-------|------|-------------|
| `user_email` | text | User email |
| `event_id` | text | Unique event identifier |
| `timestamp` | timestamptz | Event timestamp |
| `kind` | text | Event type: `completion`, `supercomplete`, `cascade_chat`, `cascade_agent`, etc. |
| `model` | text | AI model used, e.g. `claude-3.5-sonnet`, `gpt-4o` |
| `is_chargeable` | boolean | Whether event incurs billing |
| `request_cost` | numeric | Request cost in credits |
| `is_token_based_call` | boolean | Billed by tokens vs per-request |
| `input_tokens` | numeric | Tokens in the prompt (nullable) |
| `output_tokens` | numeric | Tokens in the model response (nullable) |
| `cache_read_tokens` | numeric | Tokens served from prompt cache (nullable) |
| `total_cents` | numeric | Total cost in cents (nullable) |

Token fields are nullable — not all events have token-level detail.

---

## Unified Stream 1: `class_communication_metrics`

**Sources:** `ms365_email_activity` + `ms365_teams_activity` + `zulip_messages`

One row per user per day per channel type. OneDrive and SharePoint fields from `m365_raw` are not included — the stream covers only communication activity.

**Two-step Silver pipeline** — same pattern as task tracker:
- **Step 1:** Unify M365 + Zulip (+ future: Slack) into a single schema; `user_email` as identity key.
- **Step 2:** Separate identity resolution job replaces `user_email` with canonical `person_id` from Identity Manager. Required for cross-domain joins (communication + commits + tasks via single `person_id`).

| Field | Type | Description |
|-------|------|-------------|
| `ingestion_date` | timestamp | When ingested — cursor for incremental downstream sync |
| `source_id` | text | Composite key tracing to source record: `{source}:{id}:{channel}` for MS365; `{source}:{id}` for Zulip (no channel part — one Zulip record maps to one channel) |
| `event_date` | date | Date the communication occurred |
| `source` | text | `ms365_teams`, `ms365_email`, `zulip` |
| `user_email` | text | Lowercase email — populated in step 1, retained for traceability |
| `person_id` | text | Canonical person_id from Identity Manager — populated in step 2 (NULL until resolved) |
| `user_display_name` | text | Display name (where available) |
| `channel` | text | Communication channel type |
| `direction` | text | `outbound`, `inbound`, `engagement`, `NULL` (direction not applicable) |
| `count` | numeric | Number of events |
| `metadata` | jsonb | Source-specific extras (durations, urgentMessages, etc.) |

**Channel mapping:**

| source | channel | direction | Source table | Source field |
|--------|---------|-----------|--------------|--------------|
| `ms365_teams` | `chat_group` | outbound | `ms365_teams_activity` | `teamChatMessageCount` |
| `ms365_teams` | `chat_private` | outbound | `ms365_teams_activity` | `privateChatMessageCount` |
| `ms365_teams` | `channel_post` | outbound | `ms365_teams_activity` | `postMessages` |
| `ms365_teams` | `channel_reply` | outbound | `ms365_teams_activity` | `replyMessages` |
| `ms365_teams` | `call` | outbound | `ms365_teams_activity` | `callCount` |
| `ms365_teams` | `meeting` | engagement | `ms365_teams_activity` | `meetingsAttendedCount` |
| `ms365_email` | `email_sent` | outbound | `ms365_email_activity` | `sendCount` |
| `ms365_email` | `email_received` | inbound | `ms365_email_activity` | `receiveCount` |
| `ms365_email` | `email_read` | engagement | `ms365_email_activity` | `readCount` |
| `zulip` | `chat` | outbound | `zulip_messages` | `count` |

> Total chat messages across platforms: `channel IN ('chat_group', 'chat_private', 'chat') AND direction = 'outbound'`. Channel posts/replies are content publishing, not messaging — exclude from message counts.

---

## Unified Stream 2: Task Tracker (Silver → Gold)

**Sources:** YouTrack + Jira

Two-step Silver pipeline. The connector writes `class_task_tracker_activities` and `class_task_tracker_snapshot` simultaneously during data collection. `class_task_tracker` is built on top as the final Silver contract after identity resolution.

---

### Silver Step 1: `class_task_tracker_activities` — Unified event stream

Append-only event log built from Bronze changelogs. Each row = one field-change event + full snapshot of universal fields at that moment.

**Key principles:**
- Only universal fields as columns — fields guaranteed to exist in any task tracker for any client. Everything optional goes into `fields_map`.
- No text values in rows — only refs and numbers. Text values (`"In Progress"`, `"Bug"`) live in ClickHouse Dictionaries (`status_dict`, `type_dict`). Critical for table size at scale.
- Status and type are not normalised — each client and project has its own names. Interpretation (which status counts as "in_progress") is the responsibility of the Gold layer via configuration.

| Field | Type | Description |
|-------|------|-------------|
| `id` | UInt64 | Unique event identifier |
| `source` | String | `youtrack` / `jira` / `github` |
| `insight_source_id` | String | Connector instance, e.g. `youtrack-acme-prod`, `jira-team-alpha` |
| `activity_ref` | String | Source activity ID — deduplication + link back to Bronze |
| `task_id` | String | Human-readable ID, e.g. `MON-123`, `PROJ-42` |
| `issue_ref` | String | Internal tracker ID |
| `event_date` | Date | Event date |
| `event_author_raw` | UInt64 | Author ID in source system (numeric, not text) |
| `assignee_raw` | UInt64 NULL | Assignee ID in source system (numeric, not text) |
| `type_ref` | UInt32 | Issue type ID → `type_dict` dictionary |
| `state_ref` | UInt32 | Status ID → `status_dict` dictionary |
| `changed_field` | String NULL | Which field changed in this event (`status`, `assignee`, ...) |
| `changed_from` | String NULL | Previous value (ref/id, not text) |
| `changed_to` | String NULL | New value (ref/id, not text) |
| `created_date` | Date | When the issue was created |
| `done_date` | Date NULL | Last transition to a final status |
| `parent_issue_ref` | String NULL | Parent issue ID (single parent — enforced by automation) |
| `title_version` | UInt32 | Title change counter |
| `description_version` | UInt32 | Description change counter |
| `fields_map` | Map(String, String) | Whitelisted extra fields per `insight_source_id`; values are IDs/numbers, not text |
| `collected_at` | DateTime64(3) | Collection timestamp |
| `_version` | UInt64 | ReplacingMergeTree version |

**`fields_map` examples** (whitelist configured per `insight_source_id`):

| Key | Example value | Universal? |
|-----|---------------|-----------|
| `sprint` | `"42"` (sprint ID) | No — Scrum only |
| `priority` | `"3"` (priority ID) | Most teams |
| `story_points` | `"5"` | No — estimation teams only |
| `reporter` | `"1234"` (user ID) | Most teams |
| `fix_version` | `"101"` (version ID) | No |
| `labels` | `"5,12"` (label IDs) | Partial |

Gold layer extracts needed fields from `fields_map` via client configuration — not hardcoded columns.

---

### Silver Step 1 (parallel): `class_task_tracker_snapshot` — Current state (upsert)

Same schema as `class_task_tracker_activities`, but:

- **Unique key**: `(insight_source_id, issue_ref)` — one row per issue
- **Engine**: ReplacingMergeTree by `_version`
- **Not append-only**: updated on every change

Written by the connector simultaneously with `class_task_tracker_activities`. The snapshot is never reconstructed post-factum from activity history — that would require replaying the full event tree.

Enables instant current-state queries without full-table scans: "how many issues are open for person X", "what is in the roadmap right now", "how many bugs opened after yesterday's release".

---

### Silver Step 2: `class_task_tracker` — Identity resolution + final Silver contract

Replaces source-specific identifiers (`author_youtrack_id`, `author_account_id`) with canonical `person_id` from Identity Manager. Enables joining tasks with git commits, communications, and HR data via a single unified identifier.

| Field | Type | Description |
|-------|------|-------------|
| `task_id` | String | Human-readable ID, e.g. `MON-123`, `PROJ-42` |
| `issue_ref` | String | Internal tracker ID |
| `source` | String | `youtrack` / `jira` / `github` |
| `insight_source_id` | String | Connector instance identifier |
| `type` | String | Issue type (not normalised — stored as-is) |
| `state` | String | Current status (not normalised — stored as-is) |
| `event_date` | Date | Event date |
| `event_author_person_id` | String | Canonical person_id from Identity Manager — who made the transition |
| `assignee_person_id` | String NULL | Canonical person_id — who is assigned at this moment |
| `changed_field` | String NULL | Which field changed (`status`, `assignee`, ...) |
| `changed_from` | String NULL | Previous value |
| `changed_to` | String NULL | New value |
| `parent_issue_ref` | String NULL | Parent issue for hierarchy |
| `created_date` | Date | Issue creation date |
| `done_date` | Date NULL | Last transition to a final status |
| `title_version` | UInt32 | Title change counter |
| `description_version` | UInt32 | Description change counter |
| `fields_map` | Map(String, String) | Whitelisted extra fields |
| `ingestion_at` | DateTime64(3) | Ingestion timestamp — for incremental downstream sync |
| `deleted` | Boolean | Soft delete flag |
| `_version` | UInt64 | ReplacingMergeTree version |

**Three distinct "who" fields — critical distinction:**

| Field | Who | Example |
|-------|-----|---------|
| `event_author_person_id` | Who made the state transition | QA engineer who moved the issue to Done |
| `assignee_person_id` | Who is assigned to the issue at this moment | Developer who implemented the feature |
| `fields_map['reporter']` | Who created the issue | QA who filed the bug |

Tasks link to git commits via `task_id` matched against commit messages (extracted ticket pattern).

---

### Gold: Derived metrics (built on top of `class_task_tracker`)

Gold does not store raw events — only computed metrics. Requires per-`insight_source_id` status category configuration:

```yaml
insight_source_id: jira-acme-prod
status_categories:
  in_progress: ["In Progress", "In Development"]
  testing:     ["In Review", "To Verify", "QA"]
  done:        ["Done", "Closed", "Resolved"]
sprint_field:       "sprint"         # key in fields_map
story_points_field: "story_points"   # key in fields_map
```

| Gold Table | Description |
|------------|-------------|
| `status_periods` | `(task_id, status, entered_at, exited_at)` — cycle time per state |
| `lifecycle_summary` | `created_at → started_at → testing_started_at → done_at` — lead time |
| `throughput` | COUNT(done) per week / sprint — delivery rate |
| `wip_snapshots` | Issues per status per day — CFD, bottleneck analysis |

From `status_periods`: cycle time = `exited_at - entered_at` per status; lead time = first `in_progress` to `done`; WIP = `WHERE entered_at < now AND exited_at IS NULL`; throughput = `COUNT(done)` per week.

---

## Source 10: BambooHR (HR)

**SMB-focused HR system.** Returns current-state records — no effective dating, no versioning. Simple REST API. Flat employee record with job title as a plain string; time-off categories are freeform.

**Primary use in Insight:** identity resolution (canonical email + manager chain), org hierarchy for team-level aggregation, leave history for burnout risk signals.

---

### `bamboohr_employees` — Employee records

| Field | Type | Description |
|-------|------|-------------|
| `employee_id` | text | BambooHR internal numeric ID |
| `email` | text | Work email — primary key for cross-system identity resolution |
| `full_name` | text | Display name |
| `first_name` | text | First name |
| `last_name` | text | Last name |
| `department` | text | Department name |
| `department_id` | text | Department ID |
| `job_title` | text | Job title (freeform string) |
| `employment_type` | text | `Full-Time` / `Part-Time` / `Contractor` |
| `status` | text | `Active` / `Terminated` |
| `manager_id` | text | Manager's BambooHR employee ID |
| `manager_email` | text | Manager's email — used to build org hierarchy |
| `location` | text | Office location or `Remote` |
| `hire_date` | date | Employment start date |
| `termination_date` | date | Employment end date (NULL if active) |

---

### `bamboohr_departments` — Department hierarchy

| Field | Type | Description |
|-------|------|-------------|
| `department_id` | text | BambooHR department ID |
| `name` | text | Department name |
| `parent_department_id` | text | Parent department ID (NULL for root) |

---

### `bamboohr_leave_requests` — Time off requests

| Field | Type | Description |
|-------|------|-------------|
| `request_id` | text | BambooHR request ID |
| `employee_id` | text | Employee's BambooHR ID |
| `employee_email` | text | Employee email |
| `leave_type` | text | `Vacation` / `Sick` / `Parental` / `Unpaid` / etc. (freeform) |
| `start_date` | date | Leave start |
| `end_date` | date | Leave end |
| `duration_days` | numeric | Working days absent |
| `status` | text | `approved` / `pending` / `cancelled` |
| `created_at` | timestamptz | When the request was submitted |

---

## Source 11: Workday (HR)

**Enterprise HCM.** Structurally different from BambooHR in several key ways:

- **Effective dating** — records are point-in-time snapshots; org changes produce new versioned rows rather than overwriting the current state. This enables historical org structure queries.
- **Worker type** — explicit distinction between `Employee` and `Contingent_Worker` (contractors, consultants).
- **Positions** — a job slot (`position_id`) is a separate entity from the person filling it; one position can be vacant or change hands.
- **Supervisory Organization** — the actual management hierarchy unit (`supervisory_org`), separate from cost center and department.

---

### `workday_workers` — Worker records (point-in-time)

| Field | Type | Description |
|-------|------|-------------|
| `worker_id` | text | Workday internal worker ID |
| `email` | text | Work email — primary key for cross-system identity resolution |
| `full_name` | text | Display name |
| `first_name` | text | First name |
| `last_name` | text | Last name |
| `worker_type` | text | `Employee` / `Contingent_Worker` |
| `employment_status` | text | `Active` / `Terminated` / `Leave` |
| `job_title` | text | Business title |
| `job_profile` | text | Standardized job profile name |
| `position_id` | text | Position (job slot) identifier |
| `supervisory_org_id` | text | Supervisory organization ID — defines the reporting chain |
| `supervisory_org_name` | text | Supervisory organization name |
| `department` | text | Department name |
| `cost_center_id` | text | Cost center ID |
| `cost_center_name` | text | Cost center name |
| `manager_id` | text | Manager's Workday worker ID |
| `manager_email` | text | Manager's email |
| `location` | text | Office or `Remote` |
| `hire_date` | date | Employment start date |
| `termination_date` | date | Employment end date (NULL if active) |
| `effective_date` | date | Date from which this record version is valid |

---

### `workday_organizations` — Org units (departments, supervisory orgs, cost centers)

| Field | Type | Description |
|-------|------|-------------|
| `org_id` | text | Workday org unit ID |
| `org_type` | text | `Supervisory` / `Department` / `CostCenter` / `Company` |
| `name` | text | Org unit name |
| `parent_org_id` | text | Parent org unit ID (NULL for root) |
| `head_worker_id` | text | Org head's Workday worker ID |
| `effective_date` | date | Date from which this org version is valid |

---

### `workday_leave` — Leave of absence and time off

| Field | Type | Description |
|-------|------|-------------|
| `leave_id` | text | Workday leave request ID |
| `worker_id` | text | Worker's Workday ID |
| `worker_email` | text | Worker email |
| `leave_category` | text | `Leave_of_Absence` / `Time_Off` |
| `leave_type` | text | e.g. `Vacation`, `Sick`, `Parental`, `FMLA` (policy-defined) |
| `start_date` | date | Leave start |
| `end_date` | date | Leave end |
| `duration_days` | numeric | Working days absent |
| `status` | text | `Approved` / `Pending` / `Cancelled` |
| `created_at` | timestamptz | When the request was submitted |

---

## Source 12: LDAP / Active Directory (Directory)

**Different model from HR systems.** LDAP is a hierarchical directory protocol — the primary record is a distinguished name (`dn`), not a numeric employee ID. It is the authoritative source for account status (enabled/disabled), group membership, and manager relationships in Microsoft environments.

**Primary use in Insight:** identity resolution (linking login accounts to email addresses), account lifecycle (join/leave detection via `account_disabled`), group membership for team segmentation.

---

### `ldap_users` — User account directory

| Field | Type | Description |
|-------|------|-------------|
| `dn` | text | Distinguished name — unique identifier, e.g. `CN=John Smith,OU=Engineering,DC=corp,DC=example,DC=com` |
| `sam_account_name` | text | Windows login name (Active Directory) / `uid` in OpenLDAP |
| `email` | text | `mail` attribute — primary key for cross-system identity resolution |
| `full_name` | text | `cn` (common name) |
| `first_name` | text | `givenName` |
| `last_name` | text | `sn` (surname) |
| `department` | text | `department` attribute |
| `title` | text | `title` attribute |
| `manager_dn` | text | `manager` attribute — DN of the manager's LDAP record |
| `ou` | text | Organizational unit path |
| `account_disabled` | boolean | Whether the account is disabled |
| `last_logon` | timestamptz | Last successful login (Active Directory only) |
| `created_at` | timestamptz | `whenCreated` — account provisioning date |
| `updated_at` | timestamptz | `whenChanged` — last directory update |

---

### `ldap_group_members` — Group and OU membership

| Field | Type | Description |
|-------|------|-------------|
| `group_dn` | text | Distinguished name of the group |
| `group_name` | text | `cn` of the group |
| `group_type` | text | `security` / `distribution` / `ou` |
| `member_dn` | text | DN of the member (user or nested group) |
| `member_email` | text | Resolved email of the member (NULL for nested groups) |
| `is_nested_group` | boolean | True if `member_dn` is itself a group |

Group membership is many-to-many. A user in `Engineering > Backend` appears in both the sub-group and all parent groups. `is_nested_group` allows flattening the hierarchy downstream.

---

## Source 13: Claude Admin — Anthropic API (AI Tool)

**Direct Anthropic API usage** — tracks token consumption and costs for teams calling the Claude API programmatically (internal tooling, automations, AI-powered features). Different from Cursor/Windsurf: there is no IDE context, no completions model. The unit of analysis is a token bucket per dimensional combination (model × api_key × workspace × service_tier × context_window) per day, not a developer session.

**API source:** Anthropic Admin API (`/v1/organizations/usage_report/messages`, `/v1/organizations/cost_report`). Returns daily aggregated usage groupable by up to 5 dimensions. Per-request detail is not exposed by this endpoint — only daily rollups.

Bronze tables (served by the `claude-admin` connector — see Source 14): `claude_admin_messages_usage` and `claude_admin_cost_report`.

---

### `claude_admin_messages_usage` — Daily token usage per API key per model

| Field | Type | Description |
|-------|------|-------------|
| `date` | date | Usage date |
| `api_key_id` | text | API key identifier (name or last-4 alias from Anthropic Console) |
| `model` | text | Model ID, e.g. `claude-opus-4-6`, `claude-sonnet-4-6`, `claude-haiku-4-5` |
| `request_count` | numeric | Number of API requests |
| `input_tokens` | numeric | Input tokens consumed |
| `output_tokens` | numeric | Output tokens generated |
| `cache_read_tokens` | numeric | Tokens served from prompt cache |
| `cache_write_tokens` | numeric | Tokens written to prompt cache |
| `total_cost_cents` | numeric | Total cost in cents |

Granularity: one row per `(date, api_key_id, model)`. No user attribution at this level — user breakdown requires the events table.

---

### `claude_admin_messages_usage` — per-request events (not available)

Individual API request events were hypothesized in early drafts but are **not** available from the current Anthropic Admin API. Only daily aggregates (above) are supported. The `X-Anthropic-User-Id` request header can be set by callers, but it does not surface in Admin API responses.

| Field | Type | Description |
|-------|------|-------------|
| `request_id` | text | Unique request ID from Anthropic response headers |
| `timestamp` | timestamptz | Request timestamp |
| `api_key_id` | text | API key used |
| `user_id` | text | Value of `X-Anthropic-User-Id` header — maps to internal user identifier (nullable) |
| `model` | text | Model ID |
| `input_tokens` | numeric | Input tokens |
| `output_tokens` | numeric | Output tokens |
| `cache_read_tokens` | numeric | Cache read tokens |
| `cache_write_tokens` | numeric | Cache write tokens |
| `cost_cents` | numeric | Request cost in cents |
| `stop_reason` | text | Why generation stopped: `end_turn` / `max_tokens` / `stop_sequence` / `tool_use` |
| `application` | text | Internal application tag — identifies which product or service made the call (set by the caller) |

`application` is a convention, not an Anthropic API field — callers must set it themselves (e.g. via `X-Anthropic-User-Id` or a custom header pattern).

---

## Source 14: Claude Admin — Anthropic Team / Seat Plans (AI Tool)

**Per-seat subscription** for claude.ai — covers Claude Code CLI usage. Fundamentally different from the programmatic API (Source 13):

| Aspect | Programmatic API (Source 13) | Team / Seat Plan (Source 14) |
|--------|------------------------------|-------------------------------|
| Billing | Pay-per-token | Fixed per-seat/month |
| Access | `api.anthropic.com` | Claude Code CLI (via Admin API) |
| Usage data | Token counts + costs | Tokens, tool actions, session counts (no per-request cost) |
| Clients | Programmatic only | `claude_code` (web/mobile not exposed by the Admin API) |

**Claude Code** is the only client surfaced by the Admin API's `/usage_report/claude_code` endpoint. Web/mobile activity for seat-based plans is covered by the separate `claude-enterprise` connector (Enterprise Analytics API).

**API source:** Anthropic Admin API — user management and Claude Code usage endpoints for Team/Enterprise accounts. Unified with Source 13 under a single Insight connector: `claude-admin` (8 Bronze streams; see `src/ingestion/connectors/ai/claude-admin/`).

---

### `claude_admin_users` — Seat assignment and status

| Field | Type | Description |
|-------|------|-------------|
| `user_id` | text | Anthropic platform user ID |
| `email` | text | User email — primary key for cross-system identity resolution |
| `role` | text | `owner` / `admin` / `member` |
| `status` | text | `active` / `inactive` / `pending` |
| `added_at` | timestamptz | When the seat was assigned |
| `last_active_at` | timestamptz | Last recorded activity across all clients |

---

### `claude_admin_code_usage` — Daily Claude Code usage per user

| Field | Type | Description |
|-------|------|-------------|
| `actor_identifier` | text | User email (when `actor_type='user'`) or API key name (when `actor_type='api_actor'`) |
| `actor_type` | text | `user` / `api_actor` |
| `date` | date | Activity date |
| `terminal_type` | text | Terminal/client type (e.g., `Apple_Terminal`, `rider`, `unknown`) |
| `session_count` | numeric | Number of distinct sessions |
| `lines_added` | numeric | Lines of code added |
| `lines_removed` | numeric | Lines of code removed |
| `tool_use_accepted` | numeric | Accepted tool actions (sum across edit/write/multi_edit/notebook tools) |
| `tool_use_rejected` | numeric | Rejected tool actions |
| `core_metrics_json` | text | Full `core_metrics` object (JSON) — sessions, LOC, commits, PRs |
| `tool_actions_json` | text | Full `tool_actions` object (JSON) — per-tool accepted/rejected counts |
| `model_breakdown_json` | text | Full `model_breakdown` array (JSON) — per-model tokens + estimated cost |

**`client = 'claude_code'` signals:** Claude Code sessions tend to have high `tool_use_count`, long multi-turn conversations, and large `cache_write_tokens` (system prompt + file context caching). These patterns distinguish developer AI tool usage from general knowledge work.

No `cost_cents` field — under a Team subscription the per-token cost is not meaningful; the cost is the seat fee.

---

## Source 15: GitHub Copilot (AI Dev Tool)

**Organisation-managed subscription** — Copilot Business ($19/user/month) or Copilot Enterprise ($39/user/month). Accessed via GitHub API (`/orgs/{org}/copilot/*`).

**Key structural difference from Cursor/Windsurf:** The GitHub Copilot API does not expose per-user daily usage. It provides:
- Per-seat last-activity info (`last_activity_at`, `last_activity_editor`)
- Org-level daily aggregates with breakdown by **language × editor**

No per-user token counts or per-user daily metrics exist in the standard API — only seat-level activity timestamps.

**Three tables:** seats, org-level daily totals, and per-language/editor breakdown.

---

### `copilot_seats` — Seat assignment and last activity

| Field | Type | Description |
|-------|------|-------------|
| `user_login` | text | GitHub login of the seat holder |
| `user_email` | text | Email (from linked GitHub account) — for identity resolution |
| `plan_type` | text | `business` / `enterprise` |
| `pending_cancellation_date` | date | If seat is scheduled for cancellation (NULL otherwise) |
| `last_activity_at` | timestamptz | Last recorded Copilot activity across all editors |
| `last_activity_editor` | text | Editor used in last activity, e.g. `vscode`, `jetbrains` |
| `created_at` | timestamptz | When the seat was assigned |
| `updated_at` | timestamptz | Last seat record update |

---

### `copilot_usage` — Org-level daily usage totals

| Field | Type | Description |
|-------|------|-------------|
| `date` | date | Usage date |
| `total_suggestions_count` | numeric | Code completion suggestions shown |
| `total_acceptances_count` | numeric | Suggestions accepted (tab) |
| `total_lines_suggested` | numeric | Lines of code suggested |
| `total_lines_accepted` | numeric | Lines of code accepted |
| `total_active_users` | numeric | Users with at least one completion interaction |
| `total_chat_turns` | numeric | Copilot Chat interactions (IDE + github.com) |
| `total_chat_acceptances` | numeric | Code blocks accepted from chat |
| `total_active_chat_users` | numeric | Users who used Copilot Chat |

Org-level only — no per-user breakdown at this table.

---

### `copilot_usage_breakdown` — Daily breakdown by language and editor

| Field | Type | Description |
|-------|------|-------------|
| `date` | date | Usage date |
| `language` | text | Programming language, e.g. `python`, `typescript`, `go` |
| `editor` | text | Editor, e.g. `vscode`, `jetbrains`, `neovim`, `vim`, `xcode` |
| `suggestions_count` | numeric | Suggestions shown for this language × editor |
| `acceptances_count` | numeric | Suggestions accepted |
| `lines_suggested` | numeric | Lines suggested |
| `lines_accepted` | numeric | Lines accepted |
| `active_users` | numeric | Active users for this language × editor combination |

One row per `(date, language, editor)`. Enables analysis of adoption by editor and language coverage without per-user resolution.

---

## Source 16: HubSpot (CRM)

**Sales CRM** — tracks customer contacts, company accounts, deal pipeline, and sales activities. Primary use in Insight: linking commercial activity (deals, calls, meetings) to team members for workload and performance analytics.

**API:** HubSpot REST API v3. Objects are modular — contacts, companies, deals, and activities are separate endpoints joined by associations.

---

### `hubspot_contacts` — Person records

| Field | Type | Description |
|-------|------|-------------|
| `contact_id` | text | HubSpot internal contact ID |
| `email` | text | Primary email — identity resolution key |
| `first_name` | text | First name |
| `last_name` | text | Last name |
| `job_title` | text | Job title |
| `company_id` | text | Associated company ID |
| `owner_id` | text | HubSpot owner (salesperson) ID |
| `lifecycle_stage` | text | `subscriber` / `lead` / `opportunity` / `customer` / etc. |
| `created_at` | timestamptz | Record creation |
| `updated_at` | timestamptz | Last update — cursor for incremental sync |

---

### `hubspot_companies` — Company / account records

| Field | Type | Description |
|-------|------|-------------|
| `company_id` | text | HubSpot internal company ID |
| `name` | text | Company name |
| `domain` | text | Website domain |
| `industry` | text | Industry classification |
| `owner_id` | text | Account owner ID |
| `created_at` | timestamptz | Record creation |
| `updated_at` | timestamptz | Last update |

---

### `hubspot_deals` — Deal pipeline records

| Field | Type | Description |
|-------|------|-------------|
| `deal_id` | text | HubSpot internal deal ID |
| `deal_name` | text | Deal name |
| `pipeline` | text | Pipeline name |
| `stage` | text | Current deal stage, e.g. `appointmentscheduled` / `closedwon` / `closedlost` |
| `amount` | numeric | Deal amount |
| `close_date` | date | Expected or actual close date |
| `owner_id` | text | Deal owner (salesperson) ID |
| `company_id` | text | Associated company |
| `contact_id` | text | Associated primary contact |
| `created_at` | timestamptz | Deal creation |
| `updated_at` | timestamptz | Last update |

---

### `hubspot_activities` — Calls, emails, meetings, tasks

| Field | Type | Description |
|-------|------|-------------|
| `activity_id` | text | HubSpot engagement ID |
| `activity_type` | text | `call` / `email` / `meeting` / `task` / `note` |
| `owner_id` | text | Activity owner (who performed it) |
| `contact_id` | text | Associated contact (nullable) |
| `deal_id` | text | Associated deal (nullable) |
| `timestamp` | timestamptz | When the activity occurred |
| `duration_seconds` | numeric | Duration (calls and meetings) |
| `outcome` | text | Call outcome or meeting status (source-specific values) |
| `created_at` | timestamptz | Record creation |

---

### `hubspot_owners` — HubSpot user directory (salespeople)

| Field | Type | Description |
|-------|------|-------------|
| `owner_id` | text | HubSpot owner ID |
| `email` | text | Owner email — identity resolution key |
| `first_name` | text | First name |
| `last_name` | text | Last name |
| `archived` | boolean | Whether the owner account is deactivated |

---

## Source 17: Salesforce (CRM)

**Enterprise CRM.** Same logical domain as HubSpot (contacts, accounts, opportunities, activities, users) but enterprise-grade with a significantly different data model.

**API:** Salesforce REST API + SOQL query language. Key structural differences from HubSpot:

| Aspect | HubSpot | Salesforce |
|--------|---------|-----------|
| Companies | Companies | Accounts |
| Deals | Deals | Opportunities |
| Activities | Engagements (unified) | Tasks + Events (separate objects) |
| User ID | `owner_id` (numeric) | `OwnerId` (18-char Salesforce ID) |
| Custom fields | Portal properties | Custom `__c` fields (schema-driven) |
| History | Separate history objects | `FieldHistory` tracking per object |

---

### `salesforce_contacts`

| Field | Type | Description |
|-------|------|-------------|
| `contact_id` | text | Salesforce 18-char ID |
| `email` | text | Primary email — identity resolution key |
| `first_name` | text | First name |
| `last_name` | text | Last name |
| `title` | text | Job title |
| `account_id` | text | Associated Account (company) ID |
| `owner_id` | text | Record owner (salesperson) Salesforce ID |
| `lead_source` | text | Lead origin |
| `created_date` | timestamptz | Record creation |
| `last_modified_date` | timestamptz | Last update — cursor for incremental sync |

---

### `salesforce_accounts` — Company / account records

| Field | Type | Description |
|-------|------|-------------|
| `account_id` | text | Salesforce 18-char ID |
| `name` | text | Account name |
| `website` | text | Website URL |
| `industry` | text | Industry |
| `type` | text | `Customer` / `Partner` / `Prospect` / etc. |
| `owner_id` | text | Account owner ID |
| `parent_account_id` | text | Parent account for hierarchies (NULL for root) |
| `created_date` | timestamptz | Record creation |
| `last_modified_date` | timestamptz | Last update |

---

### `salesforce_opportunities` — Deal pipeline records

| Field | Type | Description |
|-------|------|-------------|
| `opportunity_id` | text | Salesforce 18-char ID |
| `name` | text | Opportunity name |
| `stage_name` | text | Current stage, e.g. `Prospecting` / `Closed Won` / `Closed Lost` |
| `amount` | numeric | Opportunity amount |
| `close_date` | date | Expected or actual close date |
| `probability` | numeric | Win probability (0–100) |
| `owner_id` | text | Opportunity owner ID |
| `account_id` | text | Associated account |
| `lead_source` | text | Lead origin |
| `is_closed` | boolean | Whether the opportunity is closed |
| `is_won` | boolean | Whether the outcome was a win |
| `created_date` | timestamptz | Record creation |
| `last_modified_date` | timestamptz | Last update |

---

### `salesforce_activities` — Tasks and Events

| Field | Type | Description |
|-------|------|-------------|
| `activity_id` | text | Salesforce 18-char ID |
| `activity_type` | text | `Task` / `Event` |
| `subject` | text | Activity subject / title |
| `owner_id` | text | Activity owner |
| `who_id` | text | Contact or Lead associated |
| `what_id` | text | Related object (Opportunity, Account, etc.) |
| `activity_date` | date | Due date (Task) or start date (Event) |
| `duration_minutes` | numeric | Duration in minutes (Events only) |
| `status` | text | Task status: `Not Started` / `Completed` / etc. |
| `call_type` | text | `Inbound` / `Outbound` (calls only) |
| `call_duration_seconds` | numeric | Call duration (calls only) |
| `created_date` | timestamptz | Record creation |

---

### `salesforce_users` — User directory

| Field | Type | Description |
|-------|------|-------------|
| `user_id` | text | Salesforce 18-char user ID |
| `email` | text | Email — identity resolution key |
| `first_name` | text | First name |
| `last_name` | text | Last name |
| `title` | text | Job title |
| `department` | text | Department |
| `profile` | text | Salesforce profile (permission level) |
| `is_active` | boolean | Whether the user account is active |

---

## Source 18: OpenAI API (AI Tool)

**Direct OpenAI API usage** — tracks token consumption and costs for teams calling the OpenAI API programmatically. Same model as Source 13 (Claude API): daily aggregates + per-request events.

**API:** OpenAI Usage API (`/v1/usage`). Returns aggregated usage per day, groupable by model and API key.

---

### `openai_api_daily_usage` — Daily token usage per API key per model

| Field | Type | Description |
|-------|------|-------------|
| `date` | date | Usage date |
| `api_key_id` | text | API key identifier (name or last-4 alias) |
| `model` | text | Model ID, e.g. `gpt-4o`, `gpt-4o-mini`, `o1`, `o3-mini` |
| `request_count` | numeric | Number of API requests |
| `input_tokens` | numeric | Input (prompt) tokens consumed |
| `output_tokens` | numeric | Output (completion) tokens generated |
| `cached_tokens` | numeric | Tokens served from prompt cache |
| `reasoning_tokens` | numeric | Internal reasoning tokens (o1/o3 models only; billed but not in output) |
| `total_cost_cents` | numeric | Total cost in cents |

`reasoning_tokens` is specific to OpenAI's reasoning models (`o1`, `o3`) — they consume tokens internally before producing a response; these are billed but not visible in output.

---

### `openai_api_requests` — Individual API request events

| Field | Type | Description |
|-------|------|-------------|
| `request_id` | text | Unique request ID from response headers |
| `timestamp` | timestamptz | Request timestamp |
| `api_key_id` | text | API key used |
| `user_id` | text | Value of `user` field in the request body — caller-defined identifier (nullable) |
| `model` | text | Model ID |
| `input_tokens` | numeric | Input tokens |
| `output_tokens` | numeric | Output tokens |
| `cached_tokens` | numeric | Cached tokens |
| `reasoning_tokens` | numeric | Reasoning tokens (o1/o3 only, nullable) |
| `cost_cents` | numeric | Request cost in cents |
| `finish_reason` | text | Why generation stopped: `stop` / `length` / `tool_calls` / `content_filter` |
| `application` | text | Internal application tag (caller-set convention) |

---

## Source 19: ChatGPT Team (AI Tool)

**Per-seat subscription** for chatgpt.com — covers ChatGPT Team ($25/user/month) and ChatGPT Enterprise. Same two-table model as Source 14 (Claude Team): seats + daily activity.

| Aspect | OpenAI API (Source 18) | ChatGPT Team (Source 19) |
|--------|------------------------|--------------------------|
| Billing | Pay-per-token | Fixed per-seat/month |
| Access | `api.openai.com` | `chatgpt.com` + desktop app |
| Clients | Programmatic only | `web`, `desktop`, `mobile` |

**API source:** OpenAI Admin API — workspace user management and usage reports for Team/Enterprise accounts.

---

### `chatgpt_team_seats`

| Field | Type | Description |
|-------|------|-------------|
| `user_id` | text | OpenAI platform user ID |
| `email` | text | User email — identity resolution key |
| `role` | text | `owner` / `admin` / `member` |
| `status` | text | `active` / `inactive` / `pending` |
| `added_at` | timestamptz | When the seat was assigned |
| `last_active_at` | timestamptz | Last recorded activity |

---

### `chatgpt_team_activity` — Daily usage per user per model

| Field | Type | Description |
|-------|------|-------------|
| `user_id` | text | OpenAI platform user ID |
| `email` | text | User email |
| `date` | date | Activity date |
| `client` | text | `web` / `desktop` / `mobile` |
| `model` | text | Model used, e.g. `gpt-4o`, `o1`, `o3-mini` |
| `conversation_count` | numeric | Number of distinct conversations |
| `message_count` | numeric | Messages sent |
| `input_tokens` | numeric | Input tokens consumed |
| `output_tokens` | numeric | Output tokens generated |
| `reasoning_tokens` | numeric | Reasoning tokens (o1/o3 models only) |

No `cost_cents` — flat subscription.

---

## Source 20: Allure TestOps (Quality / Testing)

**Test management and reporting platform.** Tracks test launches (CI/CD runs), individual test results, and defects linked to failures. Primary use in Insight: delivery quality metrics — pass rates, flaky tests, defect accumulation trends linked to commit and sprint activity.

**API:** Allure TestOps REST API. Key entities: Projects → Launches → Test Results → Defects.

---

### `allure_launches` — Test run / launch records

| Field | Type | Description |
|-------|------|-------------|
| `launch_id` | bigint | Allure internal launch ID |
| `project_id` | bigint | Project this launch belongs to |
| `name` | text | Launch name, e.g. `Regression Suite - main` |
| `status` | text | `passed` / `failed` / `broken` / `unknown` |
| `created_date` | timestamptz | Launch start time |
| `closed_date` | timestamptz | Launch end time (NULL if running) |
| `duration_seconds` | numeric | Total run duration |
| `passed_count` | numeric | Tests passed |
| `failed_count` | numeric | Tests failed |
| `broken_count` | numeric | Tests broken (infrastructure/setup failures) |
| `skipped_count` | numeric | Tests skipped |
| `total_count` | numeric | Total tests in launch |
| `tags` | jsonb | Launch tags (environment, branch, build number, etc.) |

---

### `allure_test_results` — Individual test case results

| Field | Type | Description |
|-------|------|-------------|
| `result_id` | bigint | Allure test result ID |
| `launch_id` | bigint | Parent launch |
| `test_case_id` | bigint | Test case definition ID (stable across runs) |
| `test_name` | text | Test case name |
| `full_path` | text | Suite / class / method path |
| `status` | text | `passed` / `failed` / `broken` / `skipped` |
| `duration_seconds` | numeric | Test execution duration |
| `start_time` | timestamptz | Test start |
| `stop_time` | timestamptz | Test stop |
| `flaky` | boolean | Marked as flaky (inconsistent results across runs) |
| `message` | text | Failure message (NULL if passed) |
| `trace` | text | Stack trace (NULL if passed) |

---

### `allure_defects` — Defects linked to test failures

| Field | Type | Description |
|-------|------|-------------|
| `defect_id` | bigint | Allure defect ID |
| `project_id` | bigint | Project |
| `name` | text | Defect name / title |
| `status` | text | `open` / `resolved` |
| `created_date` | timestamptz | When the defect was first detected |
| `closed_date` | timestamptz | When resolved (NULL if open) |
| `external_issue_id` | text | Linked ticket in YouTrack / Jira (e.g. `PROJ-123`) |
| `result_count` | numeric | Number of test results linked to this defect |

`external_issue_id` enables joining Allure defects with `class_task_tracker` — linking quality failures to delivery timeline.

---

## All Tables at a Glance

| Source | Raw Tables | Notes |
|--------|-----------|-------|
| **GitHub** | `github_repositories`, `github_branches`, `github_commits`, `github_commit_files`, `github_pull_requests`, `github_pull_request_reviews`, `github_pull_request_comments`, `github_pull_request_commits`, `github_ticket_refs`, `github_collection_runs` | REST v3 + GraphQL v4; formal review states |
| **Bitbucket** | `bitbucket_repositories`, `bitbucket_branches`, `bitbucket_commits`, `bitbucket_commit_files`, `bitbucket_pull_requests`, `bitbucket_pull_request_reviewers`, `bitbucket_pull_request_comments`, `bitbucket_pull_request_commits`, `bitbucket_ticket_refs`, `bitbucket_collection_runs` | REST v1/v2; uuid identity; comment severity field |
| **GitLab** | same structure with `gitlab_` prefix + `gitlab_num_stat`, `gitlab_files`, `gitlab_mr_approvals` | Merge Requests; effective-dated file stats; approval model |
| **YouTrack** | `youtrack_issue`, `youtrack_issue_history`, `youtrack_user` | Full field change history; `insight_source_id` in issue table |
| **Jira** | `jira_issue`, `jira_issue_history`, `jira_user` | Same model as YouTrack; changelog has explicit from/to values; `insight_source_id` in issue table |
| **M365** | `ms365_email_activity`, `ms365_teams_activity`, `ms365_onedrive_activity`, `ms365_sharepoint_activity` | 4 separate endpoint tables; camelCase field names; M365 Copilot not yet collected |
| **Zulip** | `zulip_messages`, `zulip_users` | Aggregated counts, no message content |
| **Cursor** | `cursor_daily_usage`, `cursor_events`, `cursor_events_token_usage` | Daily aggregates + per-event detail |
| **Windsurf** | `windsurf_daily_usage`, `windsurf_events` | Same model as Cursor; token usage inline in events table |
| **BambooHR** | `bamboohr_employees`, `bamboohr_departments`, `bamboohr_leave_requests` | SMB HR; current-state records only |
| **Workday** | `workday_workers`, `workday_organizations`, `workday_leave` | Enterprise HCM; effective-dated, worker_type, supervisory orgs |
| **LDAP / AD** | `ldap_users`, `ldap_group_members` | Directory protocol; account status + group membership |
| **Claude Admin** | `claude_admin_users`, `claude_admin_messages_usage`, `claude_admin_cost_report`, `claude_admin_code_usage`, `claude_admin_api_keys`, `claude_admin_workspaces`, `claude_admin_workspace_members`, `claude_admin_invites` | Anthropic Admin API (merged from former claude-api + claude-team); token usage + costs + seats + Claude Code usage |
| **GitHub Copilot** | `copilot_seats`, `copilot_usage`, `copilot_usage_breakdown` | Org-level aggregates only; no per-user daily metrics in API |
| **HubSpot** | `hubspot_contacts`, `hubspot_companies`, `hubspot_deals`, `hubspot_activities`, `hubspot_owners` | CRM; contacts + pipeline + sales activities |
| **Salesforce** | `salesforce_contacts`, `salesforce_accounts`, `salesforce_opportunities`, `salesforce_activities`, `salesforce_users` | Enterprise CRM; Tasks + Events separate; 18-char IDs |
| **OpenAI API** | `openai_api_daily_usage`, `openai_api_requests` | Pay-per-token; `reasoning_tokens` for o1/o3 models |
| **ChatGPT Team** | `chatgpt_team_seats`, `chatgpt_team_activity` | Per-seat subscription; web + desktop + mobile |
| **Allure TestOps** | `allure_launches`, `allure_test_results`, `allure_defects` | Test runs + per-test results + defects linked to external tickets |

| Stream / Silver Table | Sources | Purpose |
|----------------------|---------|---------|
| `class_communication_metrics` | M365 (Email + Teams) + Zulip | Cross-platform communication load |
| `class_task_tracker_activities` | YouTrack + Jira | Silver step 1 — unified append-only event stream, source-native IDs |
| `class_task_tracker_snapshot` | YouTrack + Jira | Silver step 1 (parallel) — current state per issue, upsert |
| `class_task_tracker` | YouTrack + Jira | Silver step 2 — identity-resolved event stream; canonical `person_id` |

---

---

## Open Questions

The following architectural decisions are unresolved and require team alignment before implementation.

### OQ-1: Git deduplication across sources

A company may mirror the same repository from GitHub to Bitbucket (or GitLab). The same `commit_hash` will arrive from two separate Bronze sources.

- Does `class_commits` deduplicate by `commit_hash` globally, regardless of source?
- If yes: which source wins when metadata differs (e.g. author email present in one, absent in another)?
- If no: both rows exist in Silver with different `source` values — aggregations must `COUNT(DISTINCT commit_hash)`.

### OQ-2: Identity re-resolution strategy

When Identity Manager merges two previously separate `person_id` values (or splits one), Silver step 2 tables (`class_task_tracker`, `class_communication_metrics`, `class_commits`, ...) become stale.

- Does the identity resolution job do a full rewrite of affected Silver rows on each run?
- Or does Identity Manager maintain a version/history so Gold can query point-in-time person assignments?
- What is the acceptable lag between an identity change and Gold reflecting it?

### OQ-3: AI API tools — per-key user attribution

`claude_admin_messages_usage` and `openai_api_daily_usage` aggregate by `api_key_id`, not by person. Per-request user attribution requires the caller to pass `X-Anthropic-User-Id` / `user` field — optional conventions, not enforced.

- Should `class_ai_api_usage` carry a nullable `person_id` (resolved only when the header is present)?
- Or is per-key usage tracked separately from per-person IDE tool usage (`class_ai_dev_usage`), with no attempt to unify them at Silver?
- How does cost attribution work when one API key is shared across a team?

### OQ-4: Zulip Bronze schema — extra fields in stream spec

`streams/raw_zulip/zulip_users.md` (PR #3) contains a `recipient_id` (bigint) field not described in this Reference. `streams/raw_zulip/zulip_messages.md` contains a `uniq` (text, PRIMARY KEY) field also absent here.

- Are these fields present in the actual connector output and should be added to the Reference?
- Or are they implementation artifacts that should be removed from the stream spec?

### OQ-5: YouTrack Bronze schema — `insight_source_id` presence

This Reference lists `insight_source_id` as the first field in both `youtrack_issue` and `youtrack_issue_history` (required for multi-instance support). However, `streams/raw_youtrack/youtrack_issue.md` and `streams/raw_youtrack/youtrack_issue_history.md` (PR #3) do not include this field.

- Is `insight_source_id` populated by the current connector or only planned?
- If not yet implemented, should it be added to the stream spec before production use?

### OQ-6: `class_task_tracker_activities` — author/assignee field types

This Reference defines `event_author_raw` (UInt64) and `assignee_raw` (UInt64 NULL) — numeric source-system IDs resolved to `person_id` in Silver step 2. `streams/stream_task_tracker/task_tracker_activities.md` (PR #3) instead uses `event_author` (text) and `assignee` (text) — human-readable names stored directly.

- Which representation is canonical for Silver step 1: numeric IDs (requiring a lookup step) or text names (denormalized, no lookup needed)?
- The numeric ID approach requires ClickHouse Dictionaries (`status_dict`, `type_dict`) — is that infrastructure confirmed?

### OQ-7: Silver table naming convention — `class_` vs `_enriched`

This Reference uses `class_{domain}` naming for Silver tables (e.g. `class_commits`, `class_task_tracker`, `class_communication_metrics`). `IDENTITY_RESOLUTION_V3.md` (PR #2) uses `{entity}_enriched` (e.g. `commits_enriched`, `tasks_enriched`).

- Which naming convention is adopted going forward?
- All downstream references (Gold queries, identity resolution jobs) must use a single consistent scheme.

### OQ-8: Git Bronze schema — per-source tables vs unified table with `data_source`; extra tables

This Reference defines separate Bronze tables per source: `github_commits`, `bitbucket_commits`, `gitlab_commits`, etc. `connectors/git/schema.dbml` and `connectors/git/tables/*.md` (PR #1) define a single unified table per entity — `git_commits`, `git_pull_requests`, etc. — with a `data_source` field (`"insight_github"` / `"insight_bitbucket_server"` / `"insight_gitlab"`) as discriminator. Both use ClickHouse / ReplacingMergeTree.

- Which approach is canonical: per-source tables (this Reference) or unified table + `data_source` (PR #1)?
- Per-source tables make source-specific fields explicit but require UNION for cross-source queries. Unified table simplifies queries but mixes source-specific nullable fields.
- All downstream Silver jobs and this Reference must align on one approach.

Additionally, `schema.dbml` (PR #1) contains two entities absent from this Reference:
- `git_pr_participants` — Bitbucket-specific table listing all PR participants (authors, reviewers, commenters); no GitHub equivalent
- `git_prs_with_metadata` — materialized view / derived table joining PRs with metadata

Should these be documented in this Reference, and if so, under which source(s)?

### OQ-9: Git streams layer (`streams/raw_git/`) — relationship to Bronze

`streams/raw_git/*.md` (PR #3) defines a PostgreSQL schema (`git.commit`, `git.author`, `git.repo`, `git.num_stat`, `git.file`, `git.loc`, `git.branch`) with auto-increment IDs and FK relationships. This is structurally incompatible with both schema.dbml (ClickHouse) and this Reference.

- Is this a separate intermediate layer between the connector and Bronze, or a legacy/GitLab-only design?
- If intermediate: this layer is undocumented in the overall pipeline diagram and needs to be added.
- If legacy: these files should be marked as superseded or removed to avoid confusion.

### OQ-11: Pipeline architecture model — Medallion (Bronze/Silver/Gold) vs dbt-Mart

`CONNECTORS_ARCHITECTURE.md` (PR #2) and this Reference describe fundamentally different architectural models. Specific conflicts:

| Aspect | This Reference | CONNECTORS_ARCHITECTURE.md (PR #2) |
|--------|---------------|-------------------------------------|
| Bronze naming | `{source}_{entity}` — e.g. `github_commits`, `youtrack_issue_history` | `raw_{source}_{entity}` — e.g. `raw_gitlab_commits`, `raw_youtrack_issues` |
| Pipeline stages | Bronze → Silver (`class_*`) → Gold | Raw Tables → dbt (Staging → Intermediate → Mart) → Materialized Views |
| Gold layer | `status_periods`, `throughput`, `wip_snapshots` | `mart_dev_activity`, `mart_task_flow`, `mart_communication`, `mart_people` |
| Identity Resolution | Identity Manager in PostgreSQL / MariaDB | Stored in ClickHouse Dictionaries |
| Unifier Layer | Not mentioned | Explicit layer between connectors and storage — YAML schemas with semantic metadata propagating to Semantic Dictionary |
| Orchestration | Not mentioned | AirByte / Dagster |
| Additional sources | Not described | Linear (task tracking), Slack (communication), MCP / AI Assistant (ai_tools) |

- Which model is adopted for implementation?
- If Medallion: `CONNECTORS_ARCHITECTURE.md` needs to be updated — remove `raw_` prefixes, replace mart layer with Gold naming.
- If dbt-Mart: this Reference needs a full revision — rename tables, add Unifier Layer, move Identity Resolution to ClickHouse Dictionaries.
- Are Linear, Slack, and MCP planned sources or already implemented? If planned, add to this Reference.

### OQ-10: Git PR/commit field naming — `owner`/`repo_name` vs `project_key`/`repo_slug`

This Reference uses `owner` + `repo_name` for GitHub-style namespace fields. `connectors/git/tables/*.md` (PR #1) uses `project_key` + `repo_slug` as a unified cross-source naming. Additional mismatches: `ticket_refs` (Reference) vs `jira_tickets` (PR #1); `author_login`, `merged_by_login`, `draft`, `review_comment_count` present in Reference but absent in PR #1 tables.

- Which field names are canonical for the unified Bronze schema?
- GitHub-specific fields (`draft`, `merged_by_login`, `review_comment_count`) — are they stored as NULL for Bitbucket/GitLab rows, or excluded?

---

*Based on insight-spec repository, Streams Proposal (PR #3) and GitHub/Bitbucket ETL Design (PR #1), February 2026. Jira raw table schema designed by analogy with YouTrack (same three-table model, Jira API field names). Task Tracker Silver/Gold architecture updated March 2026 per team meeting notes and TASK_TRACKER_ANALYTICS.md research.*
