# Jira connector — test scenarios

## Purpose

This document enumerates the states a test Jira tenant must reach so that dumped
responses can drive end-to-end coverage of the connector's pipeline:

```
Jira REST v3 + Agile API
        │  (airbyte)
        ▼
  bronze_jira.*
        │  (dbt tag:staging)
        ▼
  staging.jira_changelog_items
  staging.jira_issue_field_snapshot
        │  (rust jira-enrich)
        ▼
  silver.task_tracker_field_history
  silver.task_tracker_field_metadata
        │  (dbt other silver models)
        ▼
  silver.task_tracker_{comments,projects,sprints,users,worklogs}
```

Every scenario below has an issue key placeholder (`TEST-…`) that will be assigned
to a real issue in the test tenant once it's provisioned. The stub server then
replays the captured API responses, and integration tests assert the expected
shape of the downstream silver tables.

---

## 1. Coverage matrix

Each row in this matrix should be exercised by at least one issue in the tenant.

### 1.1 `event_kind` coverage (`silver.task_tracker_field_history`)

| id | scenario | expected in silver.fh |
|---|---|---|
| EK-01 | Fresh issue, no field ever edited after creation | 1 `synthetic_initial` per tracked field (no `changelog`) |
| EK-02 | Issue created, one field edited once | 1 `synthetic_initial` per field + 1 `changelog` for the edited field |
| EK-03 | Issue with ≥100 changelog entries on a mix of fields | correct `_seq` ordering, no gaps |
| EK-04 | Issue where a field appears in `snapshot` but never in changelog | `synthetic_initial` emitted; no `changelog` |
| EK-05 | Issue where a field appears in changelog but not in snapshot | `changelog` emitted with `value_id_type=none`; no `synthetic_initial` for that field |

### 1.2 `ValueIdType` coverage

`ValueIdType` is the `silver.task_tracker_field_history.value_id_type` enum.

| id | value_id_type | field(s) | scenario |
|---|---|---|---|
| VT-01 | `opaque_id` | `status` | set→transition→transition back (e.g. Todo → In Progress → Done → In Progress) |
| VT-02 | `opaque_id` | `priority` | null → Medium → High → null |
| VT-03 | `opaque_id` | `issuetype` | Task → Bug (affects same-project issuetype scheme) |
| VT-04 | `opaque_id` | `resolution` | null → Fixed → null (reopen) |
| VT-05 | `opaque_id` | `parent` | set parent; change parent; unset parent |
| VT-06 | `account_id` | `assignee` | unassigned → user A → user B → unassigned |
| VT-07 | `account_id` | `reporter` | changed by admin to different user |
| VT-08 | `string_literal` | `labels` | `[]` → `[a]` → `[a,b]` → `[b]` → `[]` |
| VT-09 | `string_literal` | `labels` | bulk-add of 10 labels in one edit |
| VT-10 | `string_literal` | Custom "Components"-like field | add + remove in the same edit (Jira may split into two `items[]`) |
| VT-11 | `path` | Cascading select custom field | pick L1 → pick L1.L2 → swap to L1'.L2' |
| VT-12 | `none` | `story_points`, `due_date`, `summary`, `description` | set → edit → clear |

### 1.3 Stream coverage

| id | stream | scenario |
|---|---|---|
| ST-01 | `jira_issue` | issue with every tracked field populated |
| ST-02 | `jira_issue` | issue with mostly null fields (only mandatory set) |
| ST-03 | `jira_issue` | issue with a key rename (move to different project → `ABC-1` → `XYZ-1`) |
| ST-04 | `jira_issue` | issue with unicode (emoji, CJK, RTL) in summary/description |
| ST-05 | `jira_issue` | issue with very long description (>100 KB) |
| ST-06 | `jira_issue_history` | 0 changelog entries |
| ST-07 | `jira_issue_history` | pagination boundary (exactly 100 entries) |
| ST-08 | `jira_issue_history` | `items[]` has same `fieldId` twice (Jira dupe) — dbt dedup must kick in |
| ST-09 | `jira_issue_history` | `items[]` has `fieldId=''` (phantom WorklogId/RemoteIssueLink event) — must be filtered in staging |
| ST-10 | `jira_statuses` | two projects with same status `name` but different `status_id` |
| ST-11 | `jira_priorities` | global priority scheme used by all projects |
| ST-12 | `jira_issuetypes` | project with team-managed (next-gen) issuetype scheme |
| ST-13 | `jira_resolutions` | at least two distinct resolutions used |
| ST-14 | `jira_user` | active user, deactivated user, service account/bot |
| ST-15 | `jira_user` | user referenced in historical changelog that was later removed from the directory |
| ST-16 | `jira_comments` | 0 / 1 / 50+ comments per issue |
| ST-17 | `jira_comments` | comment that was edited after creation |
| ST-18 | `jira_comments` | comment that was deleted (tombstone behavior) |
| ST-19 | `jira_worklogs` | 0 / 1 / 50+ worklogs per issue |
| ST-20 | `jira_worklogs` | worklog edited; worklog deleted |
| ST-21 | `jira_sprints` | issue in no sprint; in active sprint; in completed sprint; in multiple sprints simultaneously |
| ST-22 | `_boards` | at least one Scrum and one Kanban board |
| ST-23 | `_boards` | a broken/forbidden board id (expected to return 400/404 — `error_handler` IGNORE) |
| ST-24 | `jira_fields` | full custom field catalog: single-select, multi-select, text, number, date, user-picker, cascading, sprint, epic-link |
| ST-25 | `jira_projects` | normal project + archived project |
| ST-26 | `jira_projects` | project key containing digits/underscores (`PROJ_V2`, `ID3`) |

### 1.4 Multi-cardinality edges

| id | scenario |
|---|---|
| MC-01 | Issue moved between 3 sprints sequentially (Sprint field multi-cardinality: add/remove events) |
| MC-02 | Fix Versions: `[1.0]` → `[1.0, 1.1]` → `[1.1]` |
| MC-03 | Components: add + remove in same edit |
| MC-04 | Labels: try to add a label that's already present (should not produce a `delta_action=add` row) |
| MC-05 | Assignee of a user-picker multi custom field (rare): 0 → 1 → 2 → 1 → 0 |

### 1.5 Temporal / ordering edges

| id | scenario |
|---|---|
| TM-01 | Two edits to the **same** field within the same second (same `event_at`) — `_seq` must disambiguate ordering |
| TM-02 | Edits across a clock skew between Jira cloud and ingestion (issue where `created < first changelog_at` by milliseconds) |
| TM-03 | Issue created >5 years ago with an active changelog in the last week |
| TM-04 | Bulk edit applied to multiple issues at exactly the same second (shared changelog_id scheme) |

### 1.6 Failure-mode coverage (stub server returns these)

| id | scenario | expected behavior |
|---|---|---|
| FM-01 | One board returns 400 | connector continues, that board's sprints are skipped |
| FM-02 | One board returns 404 | same as FM-01 |
| FM-03 | Issue fetch returns 429 with `Retry-After: 5` | backoff + retry; pipeline eventually succeeds |
| FM-04 | Issue fetch returns 500 twice then 200 | two retries + success |
| FM-05 | `changelog.items` field is malformed JSON | `JSONExtractArrayRaw` returns `[]`, dbt run still succeeds, enrich emits only `synthetic_initial` for that issue |
| FM-06 | Auth token expired mid-sync (401) | connector surfaces error with actionable message; pipeline aborts (no partial write) |

### 1.7 Idempotency / duplication

| id | scenario |
|---|---|
| ID-01 | Run pipeline end-to-end twice in a row → `silver.task_tracker_field_history` count unchanged; `SELECT count() == SELECT count() FINAL` |
| ID-02 | Run with duplicate rows injected into bronze (Airbyte resumed from mid-stream) → ReplacingMergeTree dedup wins |
| ID-03 | Delete a row from silver manually, rerun → enrich re-emits the missing `synthetic_initial` |

### 1.8 Structural / schema edges

| id | scenario |
|---|---|
| SC-01 | Field response has explicit `"reporter": null` (JSON null, not missing key) — connector handles via `(...) or {}` guard |
| SC-02 | Issue returned without `fields.priority` at all (key missing) |
| SC-03 | Custom field with the same `id` changes semantics over time (`type: string` → `type: array`) — stream catalog still parses |
| SC-04 | Issue belongs to a project whose `key` contains a lowercase letter (Jira rejects these client-side but cloud tenants can have legacy) |
| SC-05 | Description containing HTML-ish content (`<img>`, `<script>`) and markdown |

---

## 2. Issue checklist (flat, for the test tenant)

When the tenant is provisioned, each of the following must be created in the test
account. Names reserve `TEST-<nn>` keys.

- [ ] `TEST-01` Covers EK-01, ST-02, ST-06
- [ ] `TEST-02` Covers EK-02, VT-01 (status transition once), ST-19 with 0 worklogs
- [ ] `TEST-03` Covers EK-03, ≥100 changelog entries across all field types
- [ ] `TEST-04` Covers EK-04 (has a field in snapshot never in changelog)
- [ ] `TEST-05` Covers EK-05 (field in changelog but absent from schema when snapshot taken)
- [ ] `TEST-06` Covers VT-02 (priority null → Medium → High → null)
- [ ] `TEST-07` Covers VT-03, VT-04 (issuetype + resolution churn)
- [ ] `TEST-08` Covers VT-05 (parent set/change/unset)
- [ ] `TEST-09` Covers VT-06 (assignee lifecycle)
- [ ] `TEST-10` Covers VT-07 (reporter change by admin)
- [ ] `TEST-11` Covers VT-08, VT-09 (labels combinations)
- [ ] `TEST-12` Covers VT-10 (component add+remove in one edit)
- [ ] `TEST-13` Covers VT-11 (cascading select)
- [ ] `TEST-14` Covers VT-12 (story_points, due_date, summary, description churn)
- [ ] `TEST-15` Covers ST-03 (project key rename)
- [ ] `TEST-16` Covers ST-04 (unicode)
- [ ] `TEST-17` Covers ST-05 (very long description)
- [ ] `TEST-18` Covers ST-07 (pagination at exactly 100)
- [ ] `TEST-19` Covers ST-08 (duplicate fieldId in items[])
- [ ] `TEST-20` Covers ST-09 (phantom fieldId="")
- [ ] `TEST-21` Covers MC-01 (issue through 3 sprints)
- [ ] `TEST-22` Covers MC-02, MC-03 (versions + components)
- [ ] `TEST-23` Covers MC-04 (re-add existing label)
- [ ] `TEST-24` Covers TM-01 (two same-second edits on one field)
- [ ] `TEST-25` Covers TM-03 (old issue + recent edits)
- [ ] `TEST-26` Covers SC-01, SC-02 (null / missing fields in response)

Tenant-level (not issue-level):

- [ ] Two projects with different status schemes sharing one status name (ST-10)
- [ ] One Scrum board + one Kanban board (ST-22)
- [ ] At least one archived project (ST-25)
- [ ] Active sprint + completed sprint + future sprint (ST-21)
- [ ] Custom fields: single-select, multi-select, text, number, date, user-picker, cascading, sprint, epic-link (ST-24)
- [ ] One deactivated user + one bot/service account (ST-14)
- [ ] One user who will be referenced in historical changelog, then removed from the org (ST-15)
- [ ] Deleted comment + deleted worklog (ST-18, ST-20)

---

## 3. Stub server architecture (companion spec)

The stub server is **out of scope for this document** but consumes its output.
Intended shape:

- Fixture capture: `./tools/jira-fixture-capture.sh --tenant <test-tenant>` hits real API once, writes responses to `tests/fixtures/jira/` organized by endpoint + query params.
- Stub: small HTTP server replays captured responses by URL. Unknown URLs return 404 so missing fixtures are loud.
- Failure modes (FM-01..FM-06): encoded as fixture metadata (`_meta.json` next to each captured response) — the stub reads this and synthesizes 400/404/429/500 per scenario.
- Integration harness (Kind + dbt + enrich) points Airbyte at `http://stub-jira:8080` and runs the full pipeline.

Each scenario id from §1 maps 1:1 to an assertion in the integration test suite,
so gaps between the fixture set and the assertions are mechanically visible.

---

## 4. Maintenance

- When a new field type / event kind / failure mode is added to the connector,
  add a row to the matrix and a checklist entry here **before** merging.
- CI should run at least one pipeline against the stub; a scenario that doesn't
  show up in the stub is equivalent to "untested."
- The scenario ids are stable and intended to be referenced from test names and
  commit messages (e.g. `test(EK-03): verify _seq monotonicity`).
