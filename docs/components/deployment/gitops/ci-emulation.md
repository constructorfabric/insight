---
status: proposed
date: 2026-08-09
---

# Emulating the test-stand workflow locally

`deploy/gitops/scripts/emulate-ci-deploy.sh` runs, from a laptop, the same three stages that `.github/workflows/deploy-test-stand.yml` runs after the umbrella chart is published: **deploy → seed → smoke**. This page explains what the emulation is worth — which parts are byte-identical to CI, which parts cannot be and why — and gives the exact sequence an engineer runs to gain confidence in the whole path *before the workflow is ever enabled*, including a read-only rehearsal against the live stand.

It is written for the person who is about to turn that workflow on for the first time, and for the person who has to answer "does it reproduce by hand?" the first time it goes red.

## Table of Contents

- [1. Why a local harness at all](#1-why-a-local-harness-at-all)
- [2. The emulation model](#2-the-emulation-model)
  - [2.1 Byte-identical to CI](#21-byte-identical-to-ci)
  - [2.2 Necessarily different](#22-necessarily-different)
  - [2.3 Present locally, absent in CI — on purpose](#23-present-locally-absent-in-ci--on-purpose)
  - [2.4 How drift is detected rather than assumed](#24-how-drift-is-detected-rather-than-assumed)
- [3. Prerequisites](#3-prerequisites)
- [4. The rehearsal sequence](#4-the-rehearsal-sequence)
  - [Step 0 — tooling](#step-0--tooling)
  - [Step 1 — read the plan without a cluster](#step-1--read-the-plan-without-a-cluster)
  - [Step 2 — pin the API server fingerprint](#step-2--pin-the-api-server-fingerprint)
  - [Step 3 — read-only rehearsal against the live stand](#step-3--read-only-rehearsal-against-the-live-stand)
  - [Step 4 — rehearse as the CI credential](#step-4--rehearse-as-the-ci-credential)
  - [Step 5 — the first real run, one stage at a time](#step-5--the-first-real-run-one-stage-at-a-time)
  - [Step 6 — only now, enable the workflow](#step-6--only-now-enable-the-workflow)
- [5. Reading a failure](#5-reading-a-failure)
- [6. Safety rules that are not negotiable](#6-safety-rules-that-are-not-negotiable)
- [7. Known gaps and things to reconcile](#7-known-gaps-and-things-to-reconcile)
- [Traceability](#traceability)

## 1. Why a local harness at all

The test-stand workflow fires on a merge to `main` and on nothing else. That makes it the worst possible place to discover that the gitops environment is wrong: the feedback loop is *merge, wait, read a redacted public log, guess, merge again*, and every iteration leaves a red X on somebody's merge commit and a published stand in an unknown state.

Two failure modes make that loop especially expensive here:

- **Most of what can go wrong is configuration, not code.** A missing Secret key, a context name that does not match the inventory, a ServiceAccount without `patch` on deployments, an edge route the Gateway will refuse the moment it is written. None of it needs a chart to be published to be discoverable, and all of it is discoverable in seconds from a laptop.
- **The stand is published.** Failures are not private. A deploy that half-lands is visible to whoever looks at the stand next, and the run log that explains it is public forever.

So the harness exists to move every one of those discoveries to *before* the merge. Its design constraint follows directly: it must not invent a deploy path. A local script that did the same thing a slightly different way would give a green run that proves nothing about the red one.

## 2. The emulation model

### 2.1 Byte-identical to CI

The following commands are the same commands, in the same order, with the same flags. The harness prints each one before it runs it, so the terminal output doubles as the proof.

| Stage | Command | Workflow step |
|---|---|---|
| deploy | `helm upgrade --install <release> oci://ghcr.io/constructorfabric/charts/insight --version <v> --namespace <ns> --values deploy/gitops/environments/test-stand/values.yaml --set-string authenticator.oidc.clientSecret=… --wait --timeout 10m --history-max 10` | `Stage 1/3 — upgrade the release` |
| deploy | `helm list -n <ns> --deployed --failed --pending --uninstalling --filter '^<release>$' -o json`, then assert `status == deployed` and `chart == insight-<v>` | `Stage 1/3 — verify the release…` |
| deploy | `kubectl -n <ns> rollout restart <the three envFrom services>` and `rollout status --timeout=5m` on the same list | `Stage 1/3 — restart what the chart cannot know to restart` |
| deploy | `kubectl -n <ns> get httproute <two routes>` and fail unless every one is `Accepted=True` — a post-condition on the routes the upgrade has just written, not a check on somebody else's object | `Stage 1/3 — confirm the edge still routes` |
| seed | `./src/ingestion/tools/seed/seed-stand.sh -n <ns> --context <ctx> --days 730` — with `--email <addr>` appended **only** when an explicit address was configured on that side (see [§2.2](#22-necessarily-different)); the common case on both sides passes no address at all | `Stage 2/3 — seed the stand` |
| seed | slice the last column-0 `{ … }` block out of the raw seed log, parse it, write it as the stand manifest | `Stage 2/3 — capture the seed manifest` |
| smoke | `uv run --project tests --frozen pytest tests/stand/smoke --stand-manifest <path>`, with `SMOKE_BASE_URL` set and nothing else aiming it | `Stage 3/3 — smoke the stand…` |
| any failure | `bash .github/workflows/scripts/stand-diagnostics.sh <namespace> <release>` | `Failure diagnostics (curated, redacted)` |

Three properties of that list are load-bearing and worth stating rather than leaving implied:

- **The verification half of stage 1 is not belt-and-braces.** `helm upgrade` succeeding and *the release being the chart this run asked for* are different questions, and a resumed run, a hand-deploy that raced CI, or a mistyped version input answers the first `yes` and the second `no`.
- **`make deploy` is deliberately not used, in either place.** `deploy-insight` chains `apply-app-secrets`, which hard-requires a sealed manifest this stand has no controller for and then rewrites the chart's config Secrets from a key name this stand does not use; and it hardcodes `--atomic` after `$(HELM_UPGRADE_FLAGS)`, so no existing knob turns the rollback off. `make diff`, `make status` and `make rollback` *do* work for this environment unchanged, which is why `make diff` is the deploy stage's read-only form.
- **Smoke never runs after a failed seed.** In the workflow that is a property of steps without `if:`. In the harness it is a property of each stage function exiting the script rather than returning. Same guarantee, two mechanisms — which is itself something to keep in mind if either file is restructured.

### 2.2 Necessarily different

These differences cannot be removed. Each one is a place where a green local run does *not* prove the CI run will be green, so read this list as the residual risk the rehearsal leaves behind.

| Aspect | CI | Local | Why it cannot be closed |
|---|---|---|---|
| **Runner OS** | `ubuntu-latest`, GNU coreutils | your laptop, often BSD/macOS coreutils | The harness sticks to spellings both accept (`base64 --decode`, POSIX `awk`, no GNU-only `grep` flags), but a laptop is not a runner and never will be. A stage that passes locally and fails in CI on a text-processing difference is the one class of bug this harness cannot rule out. |
| **Credential source** | the `insight-test-stand` GitHub environment: a base64 kubeconfig in `TEST_STAND_KUBECONFIG`, decoded to `$RUNNER_TEMP` under `umask 077` | a kubeconfig file you name with `--kubeconfig` | Admin kubeconfigs stay human-only by design; CI gets a namespace-scoped ServiceAccount. The two credentials have different rights, which is exactly what `--as-user` exists to rehearse. |
| **Identity on the cluster** | the namespace-scoped `ci-deployer` ServiceAccount | whoever your kubeconfig is, usually far more privileged | A rehearsal run as yourself can succeed at a step CI will be refused. Run [Step 4](#step-4--rehearse-as-the-ci-credential). |
| **Checkout** | the merge commit on `main`, fresh clone, `persist-credentials: false`, tree guaranteed clean | your working tree, with whatever you are mid-way through | `make diff` depends on `sync-clean`, which fails on any file `git status --porcelain` reports. That is not the harness being fussy: the real deploy has the same prerequisite. It is also why the harness refuses a `--kubeconfig` that lives inside the repository. |
| **Console redaction** | every stage is piped through `redact-stand-log.py`; the raw stream is teed to `$RUNNER_TEMP` | not redacted | CI's console is public forever; yours is not, and redacting the one copy you are debugging from removes the detail you are debugging. To see what CI *would* have published, re-run a stage piping through `python3 .github/workflows/scripts/redact-stand-log.py`. |
| **Seed manifest transport** | `$RUNNER_TEMP`, discarded with the runner, never uploaded | `deploy/gitops/.deploy/ci-emulation/seed-manifest.json`, gitignored, persists | Locally it has to persist so stage 3 can be re-run without re-seeding. It carries persona addresses, UUIDs, the tenant and in-cluster service URLs — treat it as run-internal and never attach it to anything. |
| **Chart version** | the `publish-chart` job's output — the version that was just published | `--chart-version`, defaulting to the committed `deploy/gitops/.insight-version` | `.insight-version` is written back to the branch by the publishing job, so a checkout always reads a version from *before* the run you are emulating — and it can be far behind the stand rather than one release behind. **Always pass `--chart-version` explicitly.** Forgetting it under `--apply` is not "emulating an older version": it proposes whatever that file says, which may be a downgrade of a published stand, and the harness's own post-upgrade check compares the release against the version it *requested*, so a downgrade reports success. |
| **Stand address** | the `TEST_STAND_BASE_URL` repository variable | derived from `authenticator.oidc.redirectUri` in the committed values file, or `--base-url` | Same address, different route to it. If the two ever disagree, the committed values file is the one the deployed authenticator will actually redirect to. |
| **Seed persona address** | unset by default; the `TEST_STAND_SEED_EMAIL` environment secret is an override | unset by default; `--seed-email` (or `TEST_STAND_SEED_EMAIL` in your shell) is an override | **This row used to be a real difference and is now only a difference in where an override would come from.** Both sides pass nothing and let `seed-stand.sh` read the dev-lead address out of the realm the stand applied, so both seed the same person by construction rather than by two configurations agreeing. The old local default — the seeder's committed canonical dev-lead address — was removed rather than reworded: its justification was "the local default addresses the same person the roster describes", and that sentence was false on the live stand, which is how the drift stayed invisible. A harness that resolves an address when CI does not would rehearse the old behaviour while CI ran the new one, which is the one failure this file exists to prevent. |
| **Step budgets** | per-step `timeout-minutes` (deploy 14, seed 65, smoke 5) | none | helm's own `--timeout 10m` and the seed Job's `activeDeadlineSeconds` are identical in both, and those are the ones that produce a clean diagnosable failure. The GitHub step budget only exists to stop a wedged runner, and a laptop has you instead. |
| **Concurrency** | `group: test-stand-deploy`, `cancel-in-progress: false` | nothing | Two people running the harness at once against the same stand will interleave. Say so in chat before you use `--apply`. |

### 2.3 Present locally, absent in CI — on purpose

The harness does four things the workflow does not. All four are read-only, all four are labelled in the output, and none of them changes what the stages run.

1. **The cluster guard.** `--kubeconfig` and `--expect-cluster` are both mandatory in every mode. Before anything runs it asserts that the kubeconfig's current-context equals the committed inventory's `kubeContext`, that the cluster entry that context points at is named exactly `--expect-cluster`, and — when `--expect-api-server-sha256` is given — that the sha256 of the cluster's API server URL matches. CI needs less because its credential arrives from a branch-restricted environment; a laptop has every kubeconfig you have ever been given.
2. **Read-only stage forms.** `make diff` for deploy, `seed-stand.sh --dry-run` for seed, `pytest --collect-only` plus one unauthenticated `GET /` for smoke. The seed dry run is the most valuable of the three: it performs the *same* cluster discovery the real run performs, so it is simultaneously a rehearsal and an RBAC probe. That now includes reading the realm ConfigMap for the dev-lead address — one more object under a verb the probe already exercises, and the point at which a stand whose realm cannot answer that question says so before anything is applied. It also means the dry run can now *fail* where it previously always printed a manifest: a stand with no discoverable address stops here naming `--email`. That is the rehearsal doing its job, not a regression in it.
3. **Prerequisite checks.** Presence of the `client-secret` key in the `insight-oidc` Secret (presence only — the value is never read into the shell), and the `Accepted` status of the two edge routes. The Secret is an object the upgrade depends on and does not own. The routes are the opposite — the release renders them from `gateway.route` / `keycloak.route`, so reading their status *before* an upgrade tells you whether the Gateway accepted the last one, which is the cheapest available prediction of whether it will accept the next one. Either way, each check converts a failure that would otherwise cost a full deploy into a message in the first ten seconds.
4. **The parity report.** Described next.

### 2.4 How drift is detected rather than assumed

Two files that must run the same commands will not stay that way by good intentions. Every harness run greps the workflow for the literal command fragments it rehearses and prints `found` or `ABSENT` for each, plus named `BUG` lines for divergences already known to break a run:

- the workflow passing `--window-days`, which `seed-stand.sh` does not accept and refuses;
- the workflow setting `INSIGHT_STAND_BASE_URL` without `SMOKE_BASE_URL`, which the smoke conftest treats as "not aimed" and refuses;
- the workflow setting `INSIGHT_STAND_PERSONA_PASSWORD` without `SMOKE_PERSONA_PASSWORD`, which `tests/stand/smoke/login.py` never reads;
- the workflow still *requiring* `secrets.TEST_STAND_SEED_EMAIL` in its preflight, which would fail the job on a secret nothing reads;
- the workflow passing `--email "$SEED_EMAIL"` unconditionally, which means the realm is never consulted.

The last two are worth reading as a pair, because they are not equally bad. The first is loud and harmless: a preflight that demands a retired secret fails immediately and names it. The second is the dangerous one — a workflow that supplies an address the realm may never have carried produces a Job that succeeds, a release that stays `deployed`, and one persona who authenticates and resolves to nobody. A harness that discovered while CI supplied would be rehearsing the fixed path against the broken one, which is the single thing this report exists to prevent.

One anchor that looks obvious and is not: **`--email` is deliberately not in the literal list.** It is conditional on both sides now, so the literal appears in each file's text whether or not any given run passes it — an anchor asserting its presence would prove nothing, and one asserting its absence would fire on the override machinery itself. Assert the *disposition*, as the two checks above do, rather than the fragment.

Each `BUG` check is an assertion about the workflow's *text*, so it goes quiet the moment CI is fixed. The report is never fatal — a grep cannot prove two paths are identical, and a harness that refused to run because of one would just get bypassed. It is a prompt to go and diff the printed commands against the workflow by eye, which is the only check that actually proves anything.

The workflow spells `VALUES_FILE`, `CHART_REF` and the rest out in full rather than composing them from `ENV_NAME` partly so those literals stay greppable. If you shorten them, update the anchors in the harness in the same change.

## 3. Prerequisites

- `helm`, `kubectl`, `yq`, `jq`, `make`, `uv`, `curl`. `make -C deploy/gitops doctor` checks the first five with version floors and installation hints.
- A kubeconfig for the stand, **outside the repository working tree**, whose current-context is named exactly what `deploy/gitops/environments/test-stand/inventory.yaml` says. Rename yours if it is not:
  ```bash
  kubectl --kubeconfig <file> config rename-context <yours> <the inventory's kubeContext>
  kubectl --kubeconfig <file> config use-context <the inventory's kubeContext>
  ```
- A clean working tree if you intend to run the deploy stage, because `make diff` inherits `sync-clean`.
- For the smoke stage in `--apply` mode, the `SMOKE_*` credentials in your environment. The harness never defaults, bridges or prints them; the suite resolves them itself and fails naming the one that is missing. On this stand that is `SMOKE_BASE_URL`, `SMOKE_LOGIN_MODE=password` and `SMOKE_PERSONA_PASSWORD` — whose value is the realm generator's `DEV_PASSWORD` constant (`insight_seed.keycloak_realm`), shared by every user the seeded realm carries, so it comes out of the checkout rather than out of a password manager.

## 4. The rehearsal sequence

This is the order to run things in. Steps 1–4 change nothing anywhere and can be run today, against the live stand, with no coordination.

### Step 0 — tooling

```bash
make -C deploy/gitops doctor
uv sync --project tests --frozen
```

### Step 1 — read the plan without a cluster

```bash
./deploy/gitops/scripts/emulate-ci-deploy.sh --print-commands
```

Nothing runs — not even the cluster guard. You get every stage command in both its apply and dry-run form, quoted so it can be pasted back into a shell. Open `.github/workflows/deploy-test-stand.yml` beside it and read the two together. This is the step that actually establishes that the local path and the CI path are the same path; everything after it is checking the environment, not the equivalence.

### Step 2 — pin the API server fingerprint

Context and cluster names are local aliases. A kubeconfig for a *different* cluster whose entries happen to carry the same names passes every name check. The fingerprint does not:

```bash
kubectl --kubeconfig <file> config view --minify \
  -o 'jsonpath={.clusters[0].cluster.server}' | shasum -a 256
```

Keep the digest wherever the team keeps stand facts and pass it as `--expect-api-server-sha256` from now on. A digest is safe to write down; the URL it is a digest of is not — this repository is public and a cluster API endpoint is not ours to publish.

### Step 3 — read-only rehearsal against the live stand

Run the stages individually, in order, and read the output of each before moving on.

```bash
KC=<path to the stand kubeconfig, outside this repo>
CL=<the cluster entry name>
SHA=<the digest from step 2>

./deploy/gitops/scripts/emulate-ci-deploy.sh \
  --kubeconfig "$KC" --expect-cluster "$CL" --expect-api-server-sha256 "$SHA" \
  --stage deploy --chart-version <the version you want to rehearse>
```

Renders the chart offline through the committed values file, diffs it against the last render, and then reports on the OIDC client secret and the two edge routes. It never contacts the cluster for the render itself, so a failure here is a repository problem, not a stand problem.

```bash
./deploy/gitops/scripts/emulate-ci-deploy.sh \
  --kubeconfig "$KC" --expect-cluster "$CL" --expect-api-server-sha256 "$SHA" \
  --stage seed
```

Runs the seeder's own `--dry-run`. It discovers the tenant, the datastore coordinates, the seed image, the IdP source type and the dev-lead persona address from the cluster, and prints the Job it *would* apply. This is the single highest-value read-only step: if discovery works, the real seed's inputs are correct, and if it does not, the message names the flag that fixes it. Read the resolved dev-lead address in that output — `dev_user=<address> (from ConfigMap …)` — against the realm you expect the stand to be serving. The parenthesis is the part that matters: an address alone cannot be checked by eye, but `(from ConfigMap …)` versus `(from --email)` says whether the seed is following the realm or overriding it, which is the one line that tells you the roster about to be written and the roster the IdP will authenticate are the same roster. The harness states the same thing in its plan output before it runs anything, because the absence of a flag is the one thing a printed command cannot show.

```bash
./deploy/gitops/scripts/emulate-ci-deploy.sh \
  --kubeconfig "$KC" --expect-cluster "$CL" --expect-api-server-sha256 "$SHA" \
  --stage smoke
```

Confirms the public URL answers, then collects the suite. Collection needs a seed manifest, which a dry run cannot produce — so on a first pass this step stops with a message saying exactly that. That is the expected outcome, not a failure; it becomes meaningful after Step 5's seed.

### Step 4 — rehearse as the CI credential

Everything above ran as you. CI runs as a namespace-scoped ServiceAccount, which is the thing that will actually be refused:

```bash
./deploy/gitops/scripts/emulate-ci-deploy.sh \
  --kubeconfig "$KC" --expect-cluster "$CL" --expect-api-server-sha256 "$SHA" \
  --stage seed --as-user system:serviceaccount:<namespace>:ci-deployer
```

The RBAC probe prints `yes`/`no` per verb for the five permissions the three stages need, with the reason each one is needed. Every `no` is a `helm upgrade` or a seed that will fail on a merge. Note that the probe uses impersonation and therefore reports what the ServiceAccount *may* do; the stages themselves still run as your kubeconfig.

### Step 5 — the first real run, one stage at a time

Only now, and only with a deliberate choice to change a published stand. Say so wherever the team coordinates first — the workflow's concurrency group has no local equivalent, and two people applying at once will interleave.

```bash
./deploy/gitops/scripts/emulate-ci-deploy.sh \
  --kubeconfig "$KC" --expect-cluster "$CL" --expect-api-server-sha256 "$SHA" \
  --chart-version <version> \
  --stage deploy --apply --i-know-this-deploys yes-deploy-test-stand
```

Then `--stage seed --apply …`, which writes the manifest into the artefact directory, and then `--stage smoke --apply …`, which reads it. Running them separately the first time is worth the extra typing: each stage's failure mode is different, and a combined `--stage all` run makes the second failure harder to attribute than the first.

`--apply` without the token is refused. The token is deliberately the same string the Makefile's protected-environment `CONFIRM=` uses, so there is one string to remember; and because it names the environment, a command line recalled from history for a different stand fails closed instead of deploying.

Once each stage has passed individually, run the whole thing end to end — that is the run that most closely resembles what CI will do:

```bash
./deploy/gitops/scripts/emulate-ci-deploy.sh \
  --kubeconfig "$KC" --expect-cluster "$CL" --expect-api-server-sha256 "$SHA" \
  --chart-version <version> \
  --apply --i-know-this-deploys yes-deploy-test-stand
```

### Step 6 — only now, enable the workflow

By this point you know that the values file renders, that the release upgrades and reports the chart it was asked for, that the three envFrom services restart cleanly, that the routes are Accepted, that the seeder can discover everything it needs, that the manifest capture produces a document the suite can read, and that the CI ServiceAccount holds every permission the path uses. What is left for the first CI run to discover is confined to [§2.2](#22-necessarily-different): the runner's OS, the environment's secrets, and the checkout.

Populate the `insight-test-stand` GitHub environment, restrict it to `main`, and merge the workflow. Re-run `--print-commands` and read the parity report after any later edit to either file.

## 5. Reading a failure

The harness runs the workflow's own `stand-diagnostics.sh` on any stage failure, with the same positional interface (`<namespace> <release>`) and the same ambient-kubeconfig assumption. You therefore see exactly the evidence a red CI run would publish — no more, which is the point. It emits GitHub workflow commands (`::group::`, `::error::`); on a laptop those appear as literal text, and that is left alone deliberately, because a second output mode would make the local and the CI evidence hard to compare.

Nothing is rolled back. That is the intended disposition and the reason the upgrade runs with `--wait` and deliberately without `--atomic`: a rollback deletes the pods that hold the reason, leaving a red run and a healthy-looking stand, which is the one combination nobody can diagnose. Recovery is the next deploy, or:

```bash
make -C deploy/gitops status   ENV=test-stand
make -C deploy/gitops rollback ENV=test-stand
```

Both work for this environment unchanged, and both assert the current context against the inventory first.

If the harness produced output you want to share, remember it names your cluster, your context and your kubeconfig path, and is not redacted. Pipe the stage through `python3 .github/workflows/scripts/redact-stand-log.py` to see the CI-safe form, and scrub the rest by hand.

## 6. Safety rules that are not negotiable

- **Read-only is the default, and it is a real default.** No stage mutates anything without both `--apply` and the typed token.
- **The guard runs in every mode**, including dry runs — because the seed dry run does reach the cluster.
- **Nothing secret is printed.** The OIDC client secret lives in a variable for the length of one `helm upgrade` and is never echoed; the printed command shows the substitution expression instead. `set -x` is not used anywhere in the script, for that reason.
- **The seed manifest is run-internal.** It stays under the gitignored artefact directory. It is never an artifact, never an attachment, never a paste.
- **The kubeconfig lives outside the working tree.** Enforced, not advised: `make diff` fails on any file git reports, so a kubeconfig next to the values file breaks the stage it is supposed to help you rehearse.

## 7. Known gaps and things to reconcile

- **The environment's hand-deploy runbook and the workflow have drifted apart at least once.** `deploy/gitops/environments/test-stand/README.md` and the workflow are two descriptions of the same sequence, maintained separately. When they disagree, the workflow is what runs on a merge and the harness follows the workflow. Treat a disagreement as a bug in the README, and fix it in the same change.
- **The Makefile still hardcodes `--atomic`.** It does not affect this path today, because neither the workflow nor the harness uses `make deploy`. It does mean `make deploy ENV=test-stand` remains the wrong tool for this stand, and it is worth the one-line `ATOMIC ?= --atomic` change so that stops being true.
- **A scripted password login depends on the stand's realm — and on the stand having been seeded.** The realm half is satisfied here: the test stand serves realm `insight`, generated from the seed roster, with a local password user per persona, so `SMOKE_LOGIN_MODE=password` is the mode this harness rehearses. The generic warning still stands for any *other* stand — a realm that federates to an external provider holds no local users and serves no password form, and no amount of test code works around that; the suite fails saying so rather than skipping. The half Steps 1–4 genuinely cannot rehearse is the seeding: a login resolves against the `identity.persons` rows the seeder writes and fails closed, so an unseeded stand authenticates a persona and then denies them. That failure looks like a broken login and is a missing seed.
- **`.insight-version` is not the version CI just published, and may be behind the stand.** It is written back to the branch at the end of the publishing job, so any checkout reads an earlier one — far enough behind, defaulting to it is a downgrade the harness's own verification will call a success (it compares the release against the version it asked for). Pass `--chart-version` explicitly, always, and check it against `helm list -n insight` before `--apply`.
- **The harness has no concurrency control.** CI coalesces runs on `test-stand-deploy` with `cancel-in-progress: false`; two engineers with `--apply` have nothing but each other.
- **Discovering the dev-lead address makes the seed stage depend on the deploy stage in a new way.** The address comes out of the realm ConfigMap, which the bring-up outside this repository writes and the chart's config Job applies. Seeding a stand whose realm was never applied used to work and now fails, naming `--email` — which is the correct disposition, because seeding such a stand produced rows no login could ever resolve to. It is still a new ordering constraint, and it bites a human running `--stage seed` against a half-brought-up stand rather than CI, whose stages are a prefix and therefore already ordered.
- **Nothing yet asserts that the two projections agree.** Discovery removes the *cause* of the observed drift — one address, one writer — but adds no check that the realm and the seeded rows describe the same roster. If they diverge for some other reason (a realm regenerated from a different roster, a manifest left over from an older seed), the smoke gate still catches it only as a login failure. `tests/lib/insight_stand/personas.py` already contains the comparison and skips it on a cluster for want of a realm document; the applied realm is now a readable cluster object, so making that check live is a cheap follow-up rather than an open idea.

## Traceability

- Harness: `deploy/gitops/scripts/emulate-ci-deploy.sh`
- Workflow: `.github/workflows/deploy-test-stand.yml`
- Environment: `deploy/gitops/environments/test-stand/` (and its `README.md` for the hand-deploy runbook)
- Diagnostics: `.github/workflows/scripts/stand-diagnostics.sh`, `.github/workflows/scripts/redact-stand-log.py`
- Seeder: `src/ingestion/tools/seed/seed-stand.sh`, `src/ingestion/tools/seed/PROFILE.md`
- Smoke suite: `tests/stand/smoke/`
