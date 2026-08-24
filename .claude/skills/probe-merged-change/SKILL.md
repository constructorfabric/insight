---
name: probe-merged-change
description: >-
  Run an exploratory quality pass over a change that has ALREADY merged in
  constructorfabric/insight — research the diff, smoke it on a stand carrying that exact build,
  then execute a test plan across the five quality vectors (Efficiency, Reliability, Performance,
  Security, Versatility) and hand each surviving finding to file-bug-insight. Use for "test what
  just merged", "exploratory pass on #N", "check this change across the quality vectors",
  "we shipped X, go break it", or a QA sweep of a release candidate's new feature. NOT for
  planning tests before implementation (scope-feature-tests), NOT for writing the Testing section
  into an issue body (quality-vector-tests), NOT for validating a whole stand
  (insight-stand-validate), and NOT for confirming a known defect is fixed (verify-fix).
allowed-tools: Bash, Read, Write, Edit, Grep, Glob, Skill
---

# Probe a merged change

A change is in `main`. Nobody has tried to break it yet. Your job is to find what the author's own tests could not, and to leave behind reports someone can act on.

**The change is not on trial for existing — it is on trial for its claims.** The author already believes it works; they ran their suite. What has not happened is somebody reading the merge description as a set of assertions and driving each one until it gives.

## Where this sits

The other four testing skills each stop short of this:

| Skill | Owns | Not this because |
|---|---|---|
| `scope-feature-tests` | reasoning out coverage before implementation | the code does not exist yet |
| `quality-vector-tests` | authoring the Testing section into the issue | it formats scenarios, it does not run them |
| `insight-stand-validate` | validating a whole stand | scoped to an instance, not to a diff |
| `verify-fix` | confirming a known defect is fixed | the defect is already named |

Take vector semantics from `quality-vector-tests/references/vector-mapping.md` rather than re-deriving them. Take the axes catalogue from `scope-feature-tests` — exact-limit probing, tenant isolation, contract compatibility and URL-state lifecycle are already enumerated there. This skill owns execution order, the probes below, and the judgement about what is worth filing.

## Order

1. **Research** — read the diff and the merge description.
2. **Smoke** — three commands on a stand carrying that build.
3. **Plan** — vector by vector, against what you read.
4. **Execute**, collecting as you go.
5. **File** through `file-bug-insight`.

### Smoke before you plan

Three checks decide whether a plan is worth writing: the migration applied, one round trip stores and reads back what you sent, and one gated read refuses the wrong caller. If any of those fails, stop — but classify before you file. Re-run the failed check on a stand you have confirmed healthy and seeded, still on that same build. What reproduces there is the report; what does not is `layer: stand`, and the pass has learned nothing about the change yet.

Run this on a stand carrying the **exact build the change shipped in**, not on `main` built locally. Pin the images to the build tag from the release commit and reuse an existing compose project so the named volumes survive: the migration then runs against a populated database, which is the case that actually breaks. A fresh instance only proves the migration works on empty.

## Read the change's own claims as testable assertions

A merge description states things the author believes. Each is a check. Quote it, then try to break it.

- **A documented workaround is a claim.** One change said "narrowing the period reaches the rest" of a capped listing. The narrowest window the endpoint accepted was a single day, so once a day held more than the cap the workaround reached nothing — 65 rows unreachable by any request. That was the strongest finding of the pass and it came from reading one sentence sceptically.
- **A stated limit is a claim.** Drive it to its edge and past it.
- **A "verified end to end" line is a claim.** Repeat the run described and check it says what the author says it says.
- **A deferred item is not a claim.** The change that lists what it does not do yet has told you the truth. Do not file it as a defect.

## Probe for the third state at every limit

Boundary testing usually asks for the last accepted value and the first refused one. That framing has two states and misses the one that hurts: **accepted, reported success, silently modified**.

For a cap on something stored, send a value far past it and then read the record back. A 100,000-character message returned `204` and kept 4,000 of it. Nothing in the response, the interface or the log said so.

Where the cap is not a write — a page size, a rate limit, a result set — the same question lands on the response instead: does the payload, its metadata or the next request admit that anything was withheld? A truncated list that looks identical to a complete one is the same defect wearing different clothes.

Where a write is silently clipped, check the client too. A field with no `maxLength` and no counter means nothing warned before, and a success message means nothing warned after.

The same question generalises: for any input the system narrows — truncates, rounds, coerces, de-duplicates, drops an unknown field — does anything tell the person who sent it?

## Look for findings that only exist in combination

Two limitations can each be defensible and together be a defect.

Rate limiting absent is a capacity note. A listing capped at 200 with no paging is a documented limit. Together, any signed-in caller makes everyone else's rows unreachable in about three seconds.

Before writing anything up, put the individual findings side by side and ask what one caller can do holding all of them at once. File the combination as its own report when the combined consequence is worse than the sum — and say plainly what each half contributes.

## Check what the client sends against what the server declares

Cheap, mechanical, and routinely wrong in a change that touched both sides.

Capture one real request body from the interface, then compare its keys against the request type in the published OpenAPI and against the stored columns. One change had the SPA sending `app_name` and `app_version` on every submission while the server declared neither and dropped both — so no report could be tied to the build it came from.

Read the same seam in the other direction: fields the response promises that the interface never renders.

## Collecting

`drive-ui` owns getting an authenticated browser and the evidence set; `playwright-cli` owns the commands. Two things belong here.

**Mock the response rather than writing the content.** Hostile input — XSS payloads, absurd lengths, empty and error states — goes through the real render path via `playwright-cli route` without a row landing in a shared stand's database.

**Volume proof needs a stand you can throw away.** Floods, caps and rate limits need writes nobody else is reading. Expect a mass write to a shared stand to be refused. Where a finding has a volume half and a shape half, prove each where it belongs and say which came from where.

## What not to file

The pass will surface more than it should report.

- **Pre-existing platform behaviour.** Before filing an envelope or framework oddity, run the same probe against an endpoint the change did not touch. A bare `422` from the body deserialiser reproduced everywhere; it was not this change's doing and went in the report as a note, not an issue.
- **Environment artifacts.** The `layer: stand` rule in `file-bug-insight` applies unchanged.
- **Deferred work the change names.**
- **Design decisions you disagree with.** A choice the author documented and defended is a conversation, not a bug. Where it carries a risk they did not weigh — the person on the other side of it, usually — say so once, in the report, and let them decide.

Report the passes too. Tenant isolation holding, injection landing as literal text, latency inside budget — a reader who sees only findings cannot tell a thorough pass from a shallow one.
