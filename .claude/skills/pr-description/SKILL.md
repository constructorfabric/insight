---
name: pr-description
description: "Write the body of a pull request in constructorfabric/insight — the shape a reviewer can decide from in under 30 seconds: bold inline labels for Why / What changed / Out of scope / Verified, What changed as a list of changes, each with what it is for, one fact per sentence in the active voice, a bold-led line per decision the diff cannot show, screenshots for anything visual, and a word cap per block so a three-line fix does not read as a document. Use whenever a PR body is being written or rewritten: 'write the PR description', 'open a PR for this branch', 'the description is too long', 'update the PR body', or right before `gh pr create` and before marking a draft ready. Also use when reviewing someone's description for what it is missing. The repo is PUBLIC and AGENTS.md bans production-derived information from PR bodies, so every number in a description comes from a local stand, a fixture, or a test run. It owns the prose only — `file-bug-insight` owns issues, `drive-ui` owns capturing the screenshots this skill asks for."
disable-model-invocation: false
user-invocable: true
allowed-tools: Bash, Read, Write, Edit, Glob, Grep, Skill
---

# PR descriptions

A description exists so a reviewer reaches *"I understand the diff, I can decide"* in under 30 seconds. The diff already shows what changed. The body carries what the diff cannot: why, what was decided, and what was checked.

## Shape

```
Closes #NNN.

**Draft — <the blocker>.** <Only on a draft. What stops it, in one or two
sentences.>

**Why.** <One sentence, two at most. ≤ 40 words. The symptom or the gap — what
a reader would notice. The cause belongs in What changed.>

**What changed.**
- **<The problem this change answers.>** <A sentence for the change and its
  lever, then a sentence for what it is for. ≤ 30 words per bullet.>

**<Decision.>** <≤ 30 words: why, and how reversible. One per fork, none if the
branch holds no fork.>

<Evidence — full-size before/after screenshots plus a crop table for anything
visual; otherwise one measured number.>

**Out of scope.** <≤ 30 words. Deliberately not in this branch, and where it is
tracked.>

**Verified.** <≤ 30 words. What actually ran, and anything that didn't.>
```

**Labels are bold inline lead-ins, not headings.** `## Why` renders as an H2 with a rule under it, and four of those turn a three-line fix into a document. The label runs into its own sentence, so each block is one short paragraph. What changed is the one block that holds a list.

**One fact per sentence, in the active voice.** A sentence that joins two facts with a comma, `and` or `so` hides the second one. Split it. Name who acts and make it the subject: *"The legend takes the table's own inset"*, not *"The table's inset is applied to the legend"*.

`Why`, `What changed` and `Verified` are always present. `Out of scope` appears only when something real belongs in it — never a "None" row. Evidence is required whenever the change is visible or measurable.

**Title.** Conventional commit, naming an outcome: `fix(frontend): line the members-grid legend up with the person column`. Not a file, not a ticket restatement.

## The parts

1. **`Closes #NNN.` first, alone.** A defect branch closes its issue; for a defect with no issue yet, `file-bug-insight` files one first. A small docs or chore PR that tracks nothing simply drops the line — never invent a number. A draft names its blocker next, in a bold-led line, before **Why**.
2. **Why — the symptom, in one sentence.** What a reader would notice, not what the issue asked for. The cause is not the symptom: it goes next to the fix that answers it.
3. **What changed — a list, one change per bullet.** Each bullet opens with the problem it answers, in bold, so the reader pairs every fix with what broke. Two sentences follow: the change, then what it is for. Name the actual lever when it is small; a reviewer who learns that reads the diff in seconds. *"**The legend sat off the grid.** The legend takes the table's own inset, `px-3`. That lines it up with the person column."* Say what an internal name does, or explain it in the same bullet: a reader outside the code does not know a Makefile target or a fixture by name. A bullet names a change in behaviour or mechanism, never a file or a commit — CI already publishes both. A change that runs as a sequence stays one bullet, with its steps numbered under it in the order they run.
4. **Decisions, bold-led, after the list.** A decision is a fork the reviewer would otherwise guess at: something deliberately left in, deliberately taken out, or changed on purpose beyond the issue. Choice in bold, then the reason, then how reversible it is. *"**Manage now opens on Metric catalog — a deliberate behaviour change.** It previously opened on nothing while a built page sat one click away. Easy to carve back out if that card was load-bearing."*
5. **Evidence, measured.** UI change → a full-size before/after showing it in its real surroundings, then a `| Before | After |` crop table when the detail is small: the full shot proves it in context, the crop proves the pixels. `drive-ui` captures both. Anything countable → the count: `connectors 27 → 20`, `the row now has 23px to spare`, `zero dangling ref() across 206 models`. A number nobody measured is not evidence.
6. **Out of scope, when it exists.** A known thing this branch deliberately does not do, and where it is tracked. Omit the label when there is nothing.
7. **Verified.** Only what actually ran on this branch. Name the gap when there is one: *"`dbt parse` did not run: this machine has no dbt. The ref-graph check stands in for it."* CI reports its own jobs; don't transcribe them, and don't claim a job that is still pending.

## Size

| Change | Budget | Headings |
|---|---|---|
| fix, chore, docs, dependency bump | ≤ 150 words | none — bold labels only |
| feature, behaviour change | ≤ 300 words | at most one `##` heading, from the optional list |
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
| What changed as one paragraph of several changes | A list, one change per bullet. |
| A bullet that names a change and stops | Add the sentence that says what it is for. |
| A bullet that opens with the fix | Open with the problem it answers, in bold. |
| A draft whose blocker sits at the bottom | Name the blocker right after the `Closes` line. |
| The steps of one change listed as separate changes | One bullet for the change, its steps numbered under it. |
| Two facts in one sentence — "X moved, so Y now fails less" | Two sentences. |
| Passive voice — "the token is checked", "the step was rewritten" | Name the actor as the subject: "the gateway checks the token". |
| Prose describing pixels | Screenshot. |
| An `##` heading on a five-line body | Bold labels. Headings are for the optional sections. |
| **Out of scope** reading "None" or "N/A" | Delete the label. |
