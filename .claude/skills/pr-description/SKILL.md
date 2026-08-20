---
name: pr-description
description: "Write the body of a pull request in constructorfabric/insight — the shape a reviewer can decide from in under 30 seconds: bold inline labels for Why / What changed / Out of scope / Verified, a bold-led line per decision the diff cannot show, screenshots for anything visual, and a word cap per block so a three-line fix does not read as a document. Use whenever a PR body is being written or rewritten: 'write the PR description', 'open a PR for this branch', 'the description is too long', 'update the PR body', or right before `gh pr create` and before marking a draft ready. Also use when reviewing someone's description for what it is missing. The repo is PUBLIC and AGENTS.md bans production-derived information from PR bodies, so every number in a description comes from a local stand, a fixture, or a test run. It owns the prose only — `file-bug-insight` owns issues, `drive-ui` owns capturing the screenshots this skill asks for."
disable-model-invocation: false
user-invocable: true
allowed-tools: Bash, Read, Write, Edit, Glob, Grep, Skill
---

# PR descriptions

A description exists so a reviewer reaches *"I understand the diff, I can decide"* in under 30 seconds. The diff already shows what changed. The body carries what the diff cannot: why, what was decided, and what was checked.

## Shape

```
Closes #NNN.

**Why.** <One sentence, two at most. ≤ 40 words. The symptom or the gap — what
a reader would notice. The cause belongs in What changed.>

**What changed.** <One sentence, two at most. ≤ 50 words. The change and the
mechanism.>

**<Decision.>** <≤ 30 words: why, and how reversible. One per fork, none if the
branch holds no fork.>

<Evidence — full-size before/after screenshots plus a crop table for anything
visual; otherwise one measured number.>

**Out of scope.** <≤ 30 words. Deliberately not in this branch, and where it is
tracked.>

**Verified.** <≤ 30 words. What was actually run, and anything that wasn't.>
```

**Labels are bold inline lead-ins, not headings.** `## Why` renders as an H2 with a rule under it, and four of those turn a three-line fix into a document. The label runs into its own sentence, so each block is one short paragraph and the whole body reads as five lines.

`Why`, `What changed` and `Verified` are always present. `Out of scope` appears only when something real belongs in it — never a "None" row. Evidence is required whenever the change is visible or measurable.

**Title.** Conventional commit, naming an outcome: `fix(frontend): line the members-grid legend up with the person column`. Not a file, not a ticket restatement.

## The parts

1. **`Closes #NNN.` first, alone.** A defect branch closes its issue; for a defect with no issue yet, `file-bug-insight` files one first. A small docs or chore PR that tracks nothing simply drops the line — never invent a number.
2. **Why — the symptom, in one sentence.** What a reader would notice, not what the issue asked for. The cause is not the symptom: it goes next to the fix that answers it.
3. **What changed — the mechanism, in one sentence.** Name the actual lever when it is small (`the legend now takes the table's own inset — px-3`); a reviewer who learns that reads the diff in seconds. No file list, no bullet per commit — CI already publishes both.
4. **Decisions, bold-led, inside What changed.** A decision is a fork the reviewer would otherwise guess at: something deliberately left in, deliberately taken out, or changed on purpose beyond the issue. Choice in bold, then the reason, then how reversible it is. *"**Manage now opens on Metric catalog — a deliberate behaviour change.** It previously opened on nothing while a built page sat one click away. Easy to carve back out if that card was load-bearing."*
5. **Evidence, measured.** UI change → a full-size before/after showing it in its real surroundings, then a `| Before | After |` crop table when the detail is small: the full shot proves it in context, the crop proves the pixels. `drive-ui` captures both. Anything countable → the count: `connectors 27 → 20`, `the row now has 23px to spare`, `zero dangling ref() across 206 models`. A number nobody measured is not evidence.
6. **Out of scope, when it exists.** A known thing this branch deliberately does not do, and where it is tracked. Omit the label when there is nothing.
7. **Verified.** Only what actually ran on this branch. Name the gap when there is one: *"`dbt parse` was not run — dbt is unavailable here, so the ref-graph check stands in for it."* CI reports its own jobs; don't transcribe them, and don't claim a job that is still pending.

## Size

| Change | Budget | Headings |
|---|---|---|
| fix, chore, docs, dependency bump | ≤ 100 words | none — bold labels only |
| feature, behaviour change | ≤ 220 words | at most one `##` heading, from the optional list |
| anything longer | needs a reason you can state | link the issue or the spec instead of inlining it |

Word budgets count prose; images, links and code identifiers don't. The per-block caps hold at every size — a feature spends its larger budget on an optional heading, never on a fatter **Why**.

## Optional headings

Feature-tier only, one at most, and only when the change raises the question:

- `## How it works` — the mechanism is non-obvious from the entry point.
- `## Worth knowing before you read the diff` — invariants and traps that make the diff readable: units, timezones, which database the DDL lives in, what a term means here.
- `## Open after this` — known gaps, numbered, each one something a reader could pick up.
- `## Left in place on purpose` — what a reviewer will expect to see removed and won't.

## Public repo, no production detail

[AGENTS.md](../../../AGENTS.md) bans production-derived information from every GitHub-visible surface, and a PR body is one of them. Numbers come from a local stand, a synthetic fixture, or a test run — never from a deployed environment, not even aggregated, anonymised or rounded. Say *"stored values may differ in case"*, never *"half the stored rows differ in case"*.

Screenshots follow the same rule: seeded local data only, never a real install.

## Images

An agent cannot upload to GitHub's `user-attachments` CDN — that endpoint is browser-only. Push the file to an assets branch and link its raw URL, as [#2580](https://github.com/constructorfabric/insight/pull/2580) and [#2591](https://github.com/constructorfabric/insight/pull/2591) do:

```bash
git switch --orphan pr-<n>-assets && git rm -rq --cached . 2>/dev/null
cp /path/to/shot.png . && git add shot.png
git commit -qm "chore(assets): screenshots for #<n>" && git push -u upstream pr-<n>-assets
# link: https://raw.githubusercontent.com/constructorfabric/insight/pr-<n>-assets/shot.png
```

When the user pasted the images into the issue, reuse those URLs instead — they already resolve.

## Workflow

1. Read the branch: `git log upstream/main..HEAD --oneline`, `gh pr diff <n>`, and the linked issue.
2. Ask the user for what only they know — the reason behind a fork, whether a screenshot exists. Don't infer intent from a diff.
3. Draft to the shape above, show it, and wait.
4. Apply: `gh pr edit <n> --body-file <file>`, or `gh pr create --body-file <file>` for a new one.

Bot blocks (CodeRabbit release notes, coverage tables) append themselves below the body — leave them alone; re-posting a body does not disturb them.

## Common mistakes

| Mistake | Fix |
|---|---|
| Work log — "first I tried X, then reverted to Y" | Describe the branch as it stands. Exception: the commit history is unreadable without it. |
| Restating the file list CI already shows | Delete. The diff is the file list. |
| Body drifts from the branch — "as of {date}", "supersedes the earlier PR" | Edit in place. The body states current reality; git history carries the evolution. |
| A table long enough to scroll | Tables are for skimming. Enumerations stay in the scan output. |
| "Lint and typecheck green" while the job is still pending | Run it locally, or cut the claim. |
| A decision buried mid-paragraph | Own line, bold lead, its own reason. |
| Prose describing pixels | Screenshot. |
| An `##` heading on a five-line body | Bold labels. Headings are for the optional sections. |
| **Out of scope** reading "None" or "N/A" | Delete the label. |
