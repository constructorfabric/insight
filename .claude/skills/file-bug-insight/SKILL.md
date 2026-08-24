---
name: file-bug-insight
description: "File an Insight defect as a GitHub issue in constructorfabric/insight — triage against existing issues, gather evidence, collect what the reproduction produced, draft a report that reads in under a minute, confirm, create, and verify the metadata landed. It reports OBSERVATIONS ONLY — no investigation, no root-cause analysis, no naming the file or layer to fix; that is the assignee's job. Use whenever the user asks to file/report/raise/log a bug, ticket, defect or issue, and trigger PROACTIVELY once an investigation has converged on 'this is broken and should be recorded' — don't wait for the words 'file a bug'. Also fires on 'log this', 'report it', 'this is broken, make a ticket', 'turn this into an issue', 'we should file two bugs for X and Y'. The repo is PUBLIC, so the default flow is draft → confirm → create and the body must be scrubbed of internal detail. Prefer this over the general `file-bug` skill for anything in the Insight product — dashboards, metrics, connectors, dbt, ClickHouse, identity, the Helm install — since it carries the medallion evidence walk, the reproduction-data discipline and the live board IDs; the general skill is for a Constructor *platform* defect that belongs in YouTrack."
disable-model-invocation: false
user-invocable: true
allowed-tools: Bash, Read, Write, Edit, Glob, Grep, Skill, Agent, AskUserQuestion
---

# File an Insight bug

Turn an observed defect into an issue someone else can act on in under a minute, carrying the data the reproduction produced.

**Report what you saw, not why it happens.** Reproduce, collect, attach. Diagnosis — which file, which layer, which expression — belongs to whoever picks the issue up: they have the context to be right, and a confident wrong cause in the body sends them down it before they start. An observation you can defend beats an explanation you cannot.

**The tracker is always `constructorfabric/insight` on GitHub** — inside this repo there is no routing decision to make. (A Constructor *platform* bug — APS, Learn, Proctor, a platform stand's auth or navigation — goes to YouTrack instead; that routing lives in the general `file-bug` skill, not here.)

**The repo is public.** PRs close issues with `Closes #N`, so an issue here is outward-facing. Two consequences: draft → confirm → create is the default flow, and the body gets scrubbed before it goes anywhere.

## Coming from the QA fleet

A finding may arrive already carrying a `verdict`, an `existing_issue` and a `layer`. That shape is defined in `.claude/skills/explore-ui/references/finding-contract.md` — read it when the file is there, and don't stall when it isn't: the fields are self-describing, and a defect you found yourself has to clear the same four gates anyway.

- **`verdict` must be `CONFIRMED`.** An `UNVERIFIED` finding is a hypothesis, and filing one spends a reader's attention on a maybe. Reproduce it yourself first, or hand it to the `qa-finding-refuter` agent where the fleet is installed.
- **`existing_issue` must have been searched.** If it names an issue, comment on that issue instead of filing. If the match is *closed*, say so in the comment — a regression is more urgent than a new bug.
- **`layer: stand` is not a product bug.** A `join_use_nulls` view mismatch, a stale `schema_status` cache, an unseeded connector, a tenant mismatch — these are environment faults, and the reproduction shows it: the same check passes on a correctly populated instance. Record that and stop.
- **A missing `layer` is not a blocker.** You do not have to know where the fix lands in order to file. Attach what you observed at each layer you could reach and leave the conclusion open.

## Companion skills

Each of these owns a slice of the work. Some are still being built out here, so check that one exists before relying on it, and fall back to the hand-run commands in this skill rather than stalling.

| Skill | Owns | Reach for it when |
|---|---|---|
| `playwright-cli` | the browser command surface — snapshots, refs, clicks, screenshots, console, network | exploring a stand or reproducing any UI defect |
| `drive-ui` | getting an *authenticated* browser on any stand — the Keycloak realm login and the `DEV_USER_EMAIL` seed locally, a passkey attach on a remote one — plus the routes and the evidence set | any UI defect, local or remote |
| `metric-parity` | the full bronze → silver → gold walk | collecting the same query at every layer |
| `release-verify` | install and seed health | settling "product bug, or empty instance?" |
| `probe-merged-change` | the exploratory pass over a change that just merged | findings arrive here from that pass, already reproduced |

One collection step belongs here rather than in `drive-ui`: whenever a value on screen looks wrong, capture the browser console and the API response behind it (`playwright-cli console`, then `requests` and `request <n>`) and attach both. Whether the wrong number arrived from the API or was rendered wrong is the single most useful fact in the report — and it is an observation, not a diagnosis, as long as you paste what the response actually contained.

## Triage — before you gather

Three checks that routinely change the plan.

**Search first.** Never file a blind duplicate:

```sh
gh issue list --repo constructorfabric/insight --state open   --search "<key phrase>" --limit 50
gh issue list --repo constructorfabric/insight --state closed --search "<key phrase>" --limit 50
```

**Search the two states separately.** In a combined `--state all` search, closed issues rank after every open one, so they fall off the end of the window: "threshold" returns 30 open and 0 closed at `--limit 30`, and raising the limit only postpones the problem — enough open matches still crowd them out. Two queries guarantee you see both. A closed match is the *more* urgent finding, since it means a regression.

Search more than once with different vocabulary — the metric key, the field name, the group title, the error code, the user-visible label. Same defect → add your evidence to the existing issue. A genuinely different symptom → file new and cross-link with a one-line `related to #N` (a bare link, not a "how this differs" writeup — that reads as noise).

**Product bug, or environment artifact?** A metric that is empty because nothing was seeded or synced is not a product defect. File only what would still be wrong on a correctly populated instance. The cheapest check is the bottom of the medallion: no bronze rows for that connector and window means a seed or sync gap, so stop. (`release-verify` sweeps this for the whole install where it exists.)

**One bug or several?** One issue per distinct reproduction. Two symptoms that need different steps to trigger are two issues; the same symptom reached by two paths is one issue with both paths in Steps. Where you cannot tell, file one and say what else you saw — merging beats splitting a single defect across two threads.

## Gather evidence — never write from memory

Collect first, write second. The evidence must let someone else reproduce this.

**Artifacts do not go in this repo.** Nothing in this tree is gitignored for scratch output — `scratch/`, `tmp/`, `artifacts/` are merely untracked, so a screenshot or a body file left behind surfaces in someone's `git status` and rides along on the next `git add -A`. Write evidence and the issue body to the session scratchpad directory your environment names, or to a fresh `mktemp -d`; that is what the `--body-file` path below assumes. (`../insight-workspace/scratch/` also works when that checkout sits alongside this one.)

- **Data / metric bugs** — run the same question at all three layers and record all three answers, even the ones that look normal. A reader who sees rows at bronze and silver but not at gold learns more from those three counts than from any sentence you could write about them. Empty **bronze** is the one case that ends the report: nothing was synced or seeded, so there is no product defect to file.
  ```sh
  CH=(docker exec insight-clickhouse clickhouse-client -u insight --password "${CLICKHOUSE_PASSWORD:-insight-local}")
  "${CH[@]}" -q "SELECT … FROM insight.<gold> WHERE …"                      # gold — served
  "${CH[@]}" -q "SELECT … FROM silver.class_<domain>_<entity> WHERE …"      # silver — dedup / identity
  "${CH[@]}" -q "SELECT … FROM bronze_<connector>.<table> WHERE …"          # bronze — raw ingest
  ```
  Every layer needs its database prefix: the client connects to `insight`, so an unqualified `class_*` resolves to the wrong database. The password is required — compose sets `CLICKHOUSE_PASSWORD` with `insight-local` as the default.
  For a remote stand: `../insight-workspace/scripts/ch.sh query --target <target> "<sql>"` (`ch.sh` lists its targets). Those three queries *are* the three-layer walk; `metric-parity` automates it where it exists.
- **UI bugs** — reproduce it in a browser first (`drive-ui` owns the stand and the browser; `playwright-cli` owns the commands), then lead with a tight annotated shot of the broken widget plus a contrast shot of something that renders correctly. The stand URL belongs in your commands, never in the issue.
- **Pipeline / config bugs with no UI** — the failure signal itself: the exact error and stack, or a row-count contrast that runs the code's own filter (returns 0) against the unfiltered count (>0). **If the failure is silent** — completes "successfully" with zero effect — say so explicitly. That is the key symptom.
- **What the metric is *supposed* to do** lives in the model under `src/ingestion/` and the definition registry in `src/backend/services/analytics/`. Read the intent before calling behaviour wrong.

**Volume and destructive proof belongs on a stand you can throw away.** A cap, a flood, a rate limit, a migration against a warm database — each needs writes nobody else is reading. Stand one up rather than reaching for a shared instance, and expect a mass write to a shared stand to be refused outright.

Split deliberately when a finding has two halves. Prove the volume behaviour where you can write freely, and confirm the half that needs no volume — a response carrying no paging field, a parameter being ignored — where the change is actually deployed. Say in the report which half came from where, naming the state and never the environment.

## Type and priority

- **Issue Type = `Bug`** — the native type (`--type "Bug"`), never a `bug` label.
- **Priority is the Insight #40 project *field*, not a label.** Never add `priority:*`. Options: `Blocker` (blocks the next installable release), `High` (meaningful demo features), `Medium` (default). Suggest a level and confirm it.
- **Don't label.** Component, team, release and planning labels are applied during grooming by the people who own that call, and a wrong one routes the bug to the wrong team. Describe the symptom; the owning team is identified during grooming.

## Body template — four headings

```markdown
## Summary
<ONE sentence: what is broken in product terms, and its consequence. Nothing else — no repro
detail, no history, no scope. A reader triaging a list often reads only this line.>

## Steps to Reproduce
1. <UI path, or the fastest isolated check — one query or command>
2. <what to observe>
3. <what the call or query returned — the status, the visible result. The verbatim error text goes
   under Additional information, so this stays a list of actions and outcomes.>

**Expected:** <one line>
**Actual:** <one line — the failure at that step, NOT a restatement of Summary>

<A runnable proof, or a matched comparison with one variable changed. If a field being *absent*
(bug) versus *present-but-null* (no data) is the distinguishing signal, say so — that one line
stops a reviewer waving off a real defect as missing data. If it only reproduces from a given
state, name the STATE ("a freshly migrated database"), never the environment.>

## Additional information
<The data the reproduction produced, and nothing you inferred from it. Whatever you ran and what
it returned: counts at each layer, the API status and response body, the log or dbt error, the
same check on a state where it works. Label each one with what produced it. If a value looks
wrong, give the value you saw and the value the spec or the UI led you to expect — not a theory
about where it went wrong.>

## Notes        ← optional, one line (e.g. `related to #N`)
```

**No `## Root Cause` heading.** It used to be in this template, and removing it is the point of the observations-only rule: a cause written by someone who reproduced the bug but did not write the code reads as authoritative, and the assignee spends their first hour ruling it out. What you observed goes under Additional information; what it means is theirs to decide.

**No `## Impact` heading.** It restates the Summary in longer words. Affected instances or states go next to the evidence in Steps; a knock-on effect is one line in Notes. Wanting the heading back means the Summary sentence is not carrying its weight.

Additive when it sharpens the report: an **Examples** table (observed → expected) for a rule, threshold, sign or mapping bug. No fix checklist and no code links — a list of sites to change is a diagnosis, and naming one wrong is worse than naming none.

## Write plainly

One idea per sentence. Short declarative lines a tired on-call reader parses on the first pass. If a sentence has more than one comma-joined clause plus a dash-aside, split it. State what happens, then why.

- ✗ *"Deploy-side, not migration-side: the hook is skipped/lost on a successful fresh install while Helm reports success, leaving the gold layer unbuilt (install-time logs were unavailable — the Job leaves no trace because it never ran)."*
- ✓ *"On a fresh install the gold layer stays unbuilt. Helm reports success. No hook Job or Pod exists afterwards. Running the migration script by hand builds the layer."*

The second version is also the shape this skill asks for: four things observed, no claim about which side owns the fix.

**Use words the reader already has.** The issue is read by whoever is on triage, not only by someone who has just been in that code. Two habits lose that reader:

- **Shorthand you picked up while debugging.** "Token mint failed", "the reconcile cannot mint", "the hook is lost" — each is a compressed insider phrase. Write what actually happens: *"cannot get an access token"*, *"the token request to the Airbyte API fails"*, *"the Job is never created"*.
- **A verb doing a noun's job.** If a phrase needs a paragraph to unpack, it is not saving anyone time. Prefer the longer plain phrase over the shorter clever one.

Keep the product's own vocabulary where you *observed* it — table, view and column names inside an error, the API route you called, the metric key on the tile, the medallion layer names. That is what makes the report greppable. The rule is about phrases you invented, not terms the system printed.

- ✗ *"Token mint fails, so no connector is provisioned."*
- ✓ *"The pipeline cannot get an access token from Airbyte, so no connector is created. The request to `POST /api/v1/applications/token` returns 404."*

The check that catches this: read the Summary as if you had not spent the last hour in the system. If any phrase you invented would send that reader to ask "what is that?", replace it — terms the system printed stay.

**Say each fact once.** Every fact lives in exactly one section. Repetition teaches the reader to skim, and skimming is how the one load-bearing line gets missed.

**Title = the plain, user-visible symptom.** No metric IDs, table or column symbols, or migration names — those live in the body. Don't append the diagnosis as a trailing clause, don't reach for filler adverbs, and don't use a qualifier the reader can't resolve from the title alone ("after a database migration" — which one?).

- ✗ *"YouTrack sync is reported failed and its transforms are skipped even though the data synced successfully"* → ✓ *"YouTrack sync is reported failed and its transforms never run"*
- ✗ *"A connector sync fails **outright** when the previous sync is still running"* → ✓ drop `outright`; "fails" already says it.

**No prescribed fix and no acceptance criteria** — that is the assignee's call. Describe an expected result in plain language and, where a prototype exists, point at it as the source of truth; don't specify exact colours or pixel values.

## Worked example

A real filed bug, condensed. Read it for calibration on how little text a complete report needs.

> **Adding a threshold to a metric makes its threshold list fail permanently**
>
> ## Summary
> Once a metric has its first threshold, every read of that metric's thresholds fails, so thresholds can no longer be viewed, edited or removed.
>
> ## Steps to Reproduce
> 1. Create a metric, then `POST /v1/metrics/{id}/thresholds` with any valid body.
> 2. Read them back: `GET /v1/metrics/{id}/thresholds`.
> 3. Both calls return 500 `application/problem+json` (server log below).
>
> **Expected:** 201 with the created threshold, then 200 with the list.
> **Actual:** 500 on the create and on every later read of that metric's thresholds.
>
> Reproduces on a freshly migrated database with no other data.
>
> ## Additional information
> - Every later read of that metric's thresholds returns the same 500: the create's read-back, the list, an update and a delete.
> - The row is in the table — `SELECT field_name, operator, value FROM thresholds` returns it with `value = 1.000000`.
> - The admin threshold endpoints (`metric_threshold`) accept and return the same shape on the same instance, with no error.
> - Server log, verbatim, at the moment of the failed read:
>   ```
>   failed to list thresholds error=Query Error: error occurred while decoding column
>   "value": mismatched types; Rust type `core::option::Option<f64>` (as SQL type
>   `DOUBLE`) is not compatible with SQL type `DECIMAL`
>   ```
>
> ## Notes
> Found by the endpoint contract suite; the affected tests are currently skipped against this issue.

Three things that example gets right, and they are the ones reports usually miss. The title is a symptom a user could have reported. The "row is in the table" line is load-bearing — without it a triager reads a 500 as a flaky write and moves on. And every line under Additional information is something that was *run and observed*: the decode error is pasted, not paraphrased, and the working admin endpoint is offered as a contrast the assignee can use — not as a theory about what differs.

## Scrub the body

Keep **out**: internal hostnames of any kind, the phrase "dev stand", cluster and kube context names, workspace paths (`wiki/…`, `scratch/…`), JWTs, tokens, credentials, and exact data values tied to a real person (genericize `14,753` → "~14.7k"; use `jane.doe@corp.com`).

Keep **in**: the repo's own code references — file paths, view, table and column names, API routes. Those *are* the product and are what make the bug actionable.

## Confirm before creating

Show the title, the type, the priority you propose and the rendered body, then wait — unless the user said "just create it". Creation is not a draft: the repo is public and watchers are notified the moment the issue exists, so a wrong title or an unscrubbed line is already out. This is also where the priority gets settled, since it is your suggestion until the user picks one.

## Create, board, priority, images

Write the scrubbed body to a file **outside this repo** — never inline a multi-line body.

```sh
BODY="$(mktemp -d)/bug-body.md"   # or a path under the session scratchpad dir

# 1. Create — native Type=Bug, NO labels (grooming applies those), NO bug label
gh issue create --repo constructorfabric/insight \
  --type "Bug" --title "<title>" \
  --body-file "$BODY"

# 2. Add to the Insight board — idempotent; auto-add is unreliable
gh project item-add 40 --owner constructorfabric --url <issue-url>

# 3. Set the Priority FIELD. Parse with jq, NOT python — issue bodies carry control chars
ITEM=$(gh project item-list 40 --owner constructorfabric --limit 800 --format json \
  | jq -r --argjson n <ISSUE_NUMBER> '.items[] | select((.content.number // -1)==$n) | .id')
gh project item-edit --project-id PVT_kwDOERGOus4Ba9e9 --id "$ITEM" \
  --field-id PVTSSF_lADOERGOus4Ba9e9zhVxXAs \
  --single-select-option-id <Blocker=79628723 | High=0a877460 | Medium=da944a9c>
```

Verify those IDs with `gh project field-list 40 --owner constructorfabric` if an edit fails.

**Images.** There is no *documented* API, but the web UI's upload endpoint accepts a plain `gh auth token` (verified 2026-08). Upload **before** creating the issue and embed the returned URL in the body, so the asset attaches with the initial render:

```sh
# Upload one PNG; prints the asset URL to embed. The token rides stdin (-H @-),
# never curl's argv, so it can't show up in a process listing.
REPO=constructorfabric/insight   # the repo the issue will live in — swap when reusing elsewhere
REPO_ID=$(gh api "repos/$REPO" --jq .id)
ASSET=$(gh auth token | sed 's/^/Authorization: Bearer /' \
  | curl -sf --connect-timeout 5 --max-time 120 -X POST -H @- -H "Accept: application/json" \
      --data-binary "@<shot>.png" \
      "https://uploads.github.com/user-attachments/assets?name=<shot>.png&content_type=image/png&repository_id=$REPO_ID" \
  | jq -er '.url // empty')
[ -n "$ASSET" ] || echo "upload failed — use the manual fallback" >&2
# → https://github.com/user-attachments/assets/<uuid>; write ![<what it shows>]($ASSET)
#   into $BODY where the evidence belongs — the embed doesn't happen by itself
```

Three caveats. The endpoint is **undocumented** — if the POST fails (empty `$ASSET`), fall back to the old flow: create the issue, then tell the user to drag the PNGs into the description box (don't imply they attached automatically). Always pass the target repo's `repository_id` — the POST 404s without it, and asset visibility is scoped to that repo. And the screenshot is public the moment the issue is: scrub it like the body (no internal hostnames in the URL bar, no tokens in a visible console). Unattached uploads 404 anonymously until the issue references them — that's normal, not a failure. For a data or pipeline bug the inline query proof is usually the evidence and no screenshot is needed.

## Verify what landed

```sh
gh issue view <n> --repo constructorfabric/insight --json title,labels,body,url
gh api repos/constructorfabric/insight/issues/<n> --jq '.type.name'          # → "Bug"
```

Confirm: Type is `Bug`; no `bug` or `priority:` label; the body renders and is grep-clean of internal detail. The `item-add` and `item-edit` calls above already report whether the board and Priority field took, so don't re-read them. Report the URL with a one-line summary.

Don't self-assign, and don't post a status comment unless asked. On this board, moving an issue is a separate decision — *To Verify* means development is done and awaiting validation, *Done* means QA verified it, and publishing that claim is the user's call.
