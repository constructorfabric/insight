---
name: drive-ui
description: "Drive the Insight web UI in a real browser to see, verify, explore, or capture evidence from any stand — a local compose or kind install, or a shared remote one. Use this whenever the task means LOOKING at the dashboard rather than reading its code: 'check the IC page', 'is that chart still broken', 'screenshot the metrics drilldown', 'reproduce it in the UI', 'grab evidence for a bug', 'open the stand and look at X', 'exercise every option in this screen', 'find the download limit', or any UI defect you are about to file. Read it BEFORE launching a browser at any remote stand, because the Entra-plus-passkey ones cannot be logged into from a browser you launched, and the wrong acquisition move costs the user a login they cannot complete. Also read it before reporting a wrong number as a UI defect: the data that decides it is captured here. The `playwright-cli` skill owns the commands and this skill owns getting an authenticated browser, exploring what is on screen, and capturing evidence someone can act on — it hands the issue itself to `file-bug-insight`, and `scope-feature-tests` plans coverage on paper where this drives the stand in front of you."
disable-model-invocation: false
user-invocable: true
allowed-tools: Bash, Read, Write, Edit, Glob, Grep, Skill
---

# Driving the Insight UI

Three things about the Insight UI go wrong expensively: **getting a browser that is already authenticated**, **covering a screen full of options without either missing a case or clicking forever**, and **capturing evidence a reader can act on**. This skill covers all three, for local and remote stands alike. The `playwright-cli` skill owns the command surface — snapshots, refs, clicks, screenshots, tracing — so read it for *how* to drive.

The SPA lives in this repo at `src/frontend/` — source, Dockerfile and Helm chart together. So when you need to read the UI's code rather than run it, look there.

## Pick the stand, then get a browser

The right acquisition move depends entirely on the identity provider, so start there — not with `open`.

| Stand | Auth | Move |
|---|---|---|
| local compose, vite frontend (`http://localhost:3000`) | Keycloak, username + password | `playwright-cli open` |
| seeded compose test stand | Keycloak, username + password | `playwright-cli open --persistent` on the **gateway** origin, never the published frontend port |
| local k8s / kind (`http://localhost:8080` after a port-forward) | Keycloak, username + password | `playwright-cli open --persistent` |
| a shared remote stand behind Keycloak | username + password | `playwright-cli open --persistent` |
| a remote stand behind Microsoft Entra | **passkey** | Attach to the user's own Chrome — never launch |

### Local: the login is cheap, the person is not

`./dev-compose.sh up` runs auth via Keycloak — sign in as `DEV_USER_EMAIL` (`dev@company.nonpresent` by default) with password `insight-dev`. The UI is at `http://localhost:3000` and the gateway at `http://localhost:8080`. For a kind install there is no Vite origin; port-forward instead (`kubectl port-forward svc/insight-gateway 8080:80`) and use `http://localhost:8080`.

What actually blocks you is identity, not auth. The login email has to resolve to a seeded person row or the app rejects the session — a `403` / "person not found" at login, or a `401 urn:insight:error:caller_unresolved` from identity on `/api` calls. The fix is a seed, not a browser move:

```sh
./dev-compose.sh seed identity     # makes DEV_USER_EMAIL resolve to a person
```

Read that symptom as "the stand isn't ready", not "the product is broken" — filing it would be a `layer: stand` finding, which `file-bug-insight` will bounce.

The Keycloak login is a real username-and-password form, so `--persistent` is worth it — the session survives between runs.

The seeded test stand is different from ordinary development compose. Drive it through the origin `test_stand_origin` in `dev-compose.sh` computes: read `GATEWAY_PORT` from the `.env.compose.test-stand` belonging to the worktree that launched the stack, then open `http://localhost:<port>`. Do not open the published `insight-front` port. That nginx serves the SPA and nothing else — `src/frontend/nginx/default.conf.template` has no `/api` or `/auth` location, so both fall through `try_files $uri $uri/ /index.html` and answer HTML. A browser there parses the login chain as a page and loops, which reads as a broken frontend and is not one. The gateway is the single origin that fronts SPA, auth and API together.

### Behind a passkey: attach, never launch

A passkey is bound to the user's real Chrome profile and its platform authenticator. A browser you launch — Playwright's bundled Chromium *or* system Chrome with a fresh profile — cannot complete that login. Asking the user to "just sign in" in your window sends them to a prompt that cannot succeed, and you find out only when the wait times out. So borrow the browser they are already signed into.

```sh
playwright-cli attach --cdp=chrome        # preferred: one checkbox, no restart
playwright-cli attach --extension=chrome  # when CDP is blocked or the user prefers it
export PLAYWRIGHT_CLI_SESSION=chrome      # every later command now targets that session
```

**Attach names the session after the channel, not `default`.** `--cdp=chrome` creates a session called `chrome`, so an unqualified `goto` or `snapshot` afterwards talks to a different browser — or none. Either export `PLAYWRIGHT_CLI_SESSION` as above, or prefix each command (`playwright-cli -s=chrome snapshot`).

If CDP fails with `Could not connect to chrome: DevToolsActivePort file not found`, the user has to open `chrome://inspect/#remote-debugging` and tick **"Allow remote debugging for this browser instance"**. Open the page for them if you like — `open -a "Google Chrome" "chrome://inspect/#remote-debugging"` — but the click is theirs. Their windows, tabs and session all survive, which is why this beats every alternative.

**Never suggest restarting Chrome with `--remote-debugging-port`.** Chrome 136+ refuses remote debugging on the *default* profile, so the restart either fails or hands you an empty logged-out profile — which is how you end up asking for a passkey you cannot get.

**Detach, don't close.** While attached you are holding the user's own browser, and `playwright-cli close` closes *their* windows. Use `playwright-cli -s=<session> detach`. `playwright-cli list` tells you which sessions you launched yourself; those are the only ones safe to kill.

Bank the session once you have it, so later runs skip auth entirely — but keep it out of `$EVIDENCE`. That directory gets attached to a public issue, and a saved state file is live session cookies:

```sh
mkdir -p ~/.playwright-auth && chmod 700 ~/.playwright-auth
playwright-cli -s=chrome state-save ~/.playwright-auth/stand-state.json   # state-load next time
```

Re-save when it goes stale — the symptom is landing back on a login page — and delete it when the investigation ends.

## Know when the page is actually ready

Gate on **the app having rendered**, not on a token appearing in storage. The session lives behind the NGINX BFF, so an `oidc.user` entry in `sessionStorage` may never exist even on a perfectly good session. Polling for one is an invisible hang: the browser is fine, the user is logged in, and you wait for something that is not coming.

Wait for a real element instead — snapshot and look for a control the page cannot render without:

```sh
playwright-cli goto "http://localhost:3000/ic/<url-encoded-email>/personal"
playwright-cli snapshot --depth=6
```

When a wait does time out, **screenshot before giving up**. A picture of the actual page — "Sign-in failed", an empty state, a 502 — turns a silent hang into a diagnosis in one look. The same reflex applies to any recipe from memory or an old note: routes and the auth stack have changed inside a month before. Check it against the running build, and say so when it turns out stale.

## Rule out your own driving before believing a negative

A negative observation is the easiest false finding to produce, because the way you drove the page can manufacture it. Before "the control does nothing", "the column never changed", "no request fired" or "the list is short" becomes a claim, reproduce it from a fresh `goto` on the shortest path to the symptom. Four instruments account for most of these:

- **The page you have been clicking in holds a cache.** The SPA keeps query results, so a control whose answer is already cached fires no request and re-renders nothing. On a page you have been driving, "no request fired" measures the cache, not the control.
- **A selector reaches the whole page, not the container you meant.** A control matching by role or label can sit outside the dialog you are testing, so its state says nothing about that dialog. Check containment before attributing behaviour to it.
- **A rendered list is a window onto a longer one.** Record tables virtualise, and the API behind them pages with a cursor. Take the count from the record label or by following the cursor to the end, never from the rows in the DOM.
- **A wait that did not time out still proves nothing** about a control that renders late. Give a slow surface its full time, and screenshot before calling anything missing.

When a negative survives that pass, say which instrument you ruled out. When it does not, the finding was yours and never reaches the report.

## Routes worth knowing

- `/ic/<url-encoded-email>/personal` — IC dashboard, personal view (`@` encodes as `%40`)
- `/ic/<url-encoded-email>/team` — team view for the same person
- `/metrics` — metrics catalog

Group cards expose `aria-label="Open <group title> details"`, which makes them stable targets:

```sh
playwright-cli click "getByRole('button', { name: 'Open Git output details' })"
```

## Capture what is behind the number

A wrong number on screen may or may not come from the frontend, and you do not have to decide which. Two commands capture the evidence that decides it, and skipping them is how a gold-view defect gets filed against the SPA:

```sh
playwright-cli console      # client-side errors behind a broken or empty widget
playwright-cli requests     # then `request <n>` for the failing call's status and body
```

Attach both to the report: the value on screen, and the value the API actually returned, pasted rather than characterised. Those two facts side by side are what a reader needs — leave what they imply to whoever picks the issue up.

## Explore a configurable surface

A screen with option controls — a report builder, an export menu, a filtered drilldown — has more combinations than anyone can click, so decide what to click before opening it. `playwright-cli snapshot` gives you the rendered control set; the code behind it in `src/frontend/` gives you the closed sets and the limits the request is checked against. Those are the axes. `scope-feature-tests` owns deciding which of them are worth writing tests for; this is what to do with a browser once one is in front of you.

Exercise every member of a short, closed axis, and combine axes pairwise rather than building an unbounded Cartesian product — a full cross of five controls is a day of clicking and nobody reads the result.

For every numeric, date, row, request or file limit, drive the pair: the largest value it accepts and the smallest it refuses. Add empty and single-item cases where those are genuinely different states. A distant invalid value proves only that rejection exists — it hides an off-by-one between what the UI allows and what the API accepts, which is where these mismatches actually live.

Enter URL-backed state both ways: navigate with the controls, then load the equivalent URL cold and reload it, and walk Back/Forward when the feature claims its state is shareable. A validator passing in isolation does not prove the router runs it before the page renders, and this SPA has two paths — `validatePortalSearch` (`src/frontend/src/lib/portal/portal-search.ts`) silently drops what it cannot parse, while `assertDateRange` (`src/frontend/src/api/period-to-date-range.ts`) throws into `AppErrorBoundary` and replaces the whole shell. The same bad link can degrade quietly or blank the app depending on which one it reaches, so try it rather than reasoning about it.

When the surface builds a downloadable artifact, open the artifact. Parse every format it offers and compare headers, row count, missing-versus-zero cells and a representative value against the preview or the API response — `playwright-cli requests` gives you that response to compare against. A download event and a non-zero byte count tell you the transport worked, which was never the part in doubt.

## Capture evidence someone can act on

Write artifacts **outside this repo** — nothing here is gitignored for scratch output, so leftovers surface in someone's `git status` and ride along on the next `git add -A`:

```sh
EVIDENCE="$(mktemp -d)"     # or a path under the session scratchpad dir
```

For a UI defect, three artifacts answer the three questions a reader has:

1. **The offending element, tight** — `playwright-cli screenshot <ref>` on the widget, not the whole page. Answers "where do I look?"
2. **A contrast shot** — the same widget in a state that is correct, or a sibling that behaves. Answers "how do I know it's wrong?"
3. **The page snapshot** — `playwright-cli snapshot --filename="$EVIDENCE/<case>.yml"`, plus `--boxes` when the complaint is about position or overlap. Answers "what was actually on screen?"

Annotate before capturing rather than describing the element in prose afterwards. Then hand the issue to `file-bug-insight` — it owns attaching the images and has a working upload path, so don't tell the user to drag them in.

### Scrub the frame, not just the widget

A screenshot of a populated stand carries more than the thing you meant to capture. Person names, a scope picker, team metric values and other people's free text all render, and the issue tracker is public.

Two moves, in order. Reproduce on a screen carrying no person data where the defect allows it — a config surface beats a populated table, though a metric name, group title or description there can be author-written too, so it is a better starting point rather than a safe one. Where the defect needs the populated screen, redact before uploading rather than cropping the layout apart, since the layout is usually part of what the reader needs to see.

Read the whole frame every time, and treat any free text in it as personal until you have checked. The offending element is rarely the sensitive part.

### Mock the response instead of writing the content

To see how a surface renders hostile or extreme content, mock the response rather than creating it. `playwright-cli route "**/api/..." --body=...` puts XSS payloads, absurd lengths, empty states and error states through the real render path without a row landing in a shared stand's database. `unroute` afterwards.

This is the difference between proving that four payloads render as escaped text and planting scripts in a stand colleagues read.

## When you can't get a browser

The user may not have time to flip a toggle or sign in, and that is a normal outcome rather than a blocker. Say plainly what you could not verify, and stop there. Reading the frontend source to work out what the page *would* have shown produces a conclusion, not an observation — and this skill exists to produce observations. "Not visually confirmed" is a complete and useful answer; a claim that implies visual confirmation it never got is neither honest nor useful.
