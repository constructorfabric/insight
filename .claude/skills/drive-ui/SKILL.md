---
name: drive-ui
description: "Drive the Insight web UI in a real browser to see, verify, or capture evidence from any stand — a local compose or kind install, or a shared remote one. Use this whenever the task means LOOKING at the dashboard rather than reading its code: 'check the IC page', 'is that chart still broken', 'screenshot the metrics drilldown', 'reproduce it in the UI', 'grab evidence for a bug', 'open the stand and look at X', or any UI defect you are about to file. Read it BEFORE launching a browser at any remote stand, because the Entra-plus-passkey ones cannot be logged into from a browser you launched, and the wrong acquisition move costs the user a login they cannot complete. Also read it before reporting a wrong number as a UI defect — the data that decides it is captured here, and this skill collects observations rather than drawing conclusions from them. The `playwright-cli` skill owns the commands; this skill owns getting an authenticated browser and capturing evidence someone can act on, and hands the issue itself to `file-bug-insight`."
disable-model-invocation: false
user-invocable: true
allowed-tools: Bash, Read, Write, Edit, Glob, Grep, Skill
---

# Driving the Insight UI

Two things about the Insight UI go wrong expensively, and neither is about clicking: **getting a browser that is already authenticated**, and **capturing evidence a reader can act on**. This skill covers both, for local and remote stands alike. The `playwright-cli` skill owns the command surface — snapshots, refs, clicks, screenshots, tracing — so read it for *how* to drive.

The SPA itself is not in this repo (`src/frontend/` holds only the Helm chart); it lives in `insight-front`. So when you need to read the UI's code rather than run it, look there.

## Pick the stand, then get a browser

The right acquisition move depends entirely on the identity provider, so start there — not with `open`.

| Stand | Auth | Move |
|---|---|---|
| local compose (`http://localhost:3000`) | FakeIdP bypass, or Keycloak | `playwright-cli open` — no login step in fakeidp mode |
| local k8s / kind (`http://localhost:8080` after a port-forward) | Keycloak, username + password | `playwright-cli open --persistent` |
| a shared remote stand behind Keycloak | username + password | `playwright-cli open --persistent` |
| a remote stand behind Microsoft Entra | **passkey** | Attach to the user's own Chrome — never launch |

### Local: the login is free, the person is not

`./dev-compose.sh up` defaults to `AUTH_MODE=fakeidp`, which is a bypass — no credentials to type. The UI is at `http://localhost:3000` and the gateway at `http://localhost:8080`. For a kind install there is no Vite origin; port-forward instead (`kubectl port-forward svc/insight-gateway 8080:80`) and use `http://localhost:8080`.

What actually blocks you is identity, not auth. FakeIdP logs in as `DEV_USER_EMAIL` (`dev@company.nonpresent` by default), and that email has to resolve to a seeded person row or the app rejects the session — a `403` / "person not found" at login, or a `401 urn:insight:error:caller_unresolved` from identity on `/api` calls. The fix is a seed, not a browser move:

```sh
./dev-compose.sh seed identity     # makes DEV_USER_EMAIL resolve to a person
```

Read that symptom as "the stand isn't ready", not "the product is broken" — filing it would be a `layer: stand` finding, which `file-bug-insight` will bounce.

With `AUTH_MODE=keycloak` you get a real username-and-password form, and `--persistent` is worth it so the session survives between runs.

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

Attach both to the report: the value on screen, and the value the API actually returned, pasted rather than characterised. Those two facts side by side are what a reader needs — leave what they imply to whoever picks the issue up. `metric-parity` collects the same question further down the medallion when you can reach it.

## Capture evidence someone can act on

Write artifacts **outside this repo** — nothing here is gitignored for scratch output, so leftovers surface in someone's `git status` and ride along on the next `git add -A`:

```sh
EVIDENCE="$(mktemp -d)"     # or a path under the session scratchpad dir
```

For a UI defect, three artifacts answer the three questions a reader has:

1. **The offending element, tight** — `playwright-cli screenshot <ref>` on the widget, not the whole page. Answers "where do I look?"
2. **A contrast shot** — the same widget in a state that is correct, or a sibling that behaves. Answers "how do I know it's wrong?"
3. **The page snapshot** — `playwright-cli snapshot --filename="$EVIDENCE/<case>.yml"`, plus `--boxes` when the complaint is about position or overlap. Answers "what was actually on screen?"

Annotate before capturing rather than describing the element in prose afterwards. Then hand the issue to `file-bug-insight` — and be straight about the constraint it will repeat: GitHub has no API for uploading images to an issue, so the user drags the PNGs in themselves.

## When you can't get a browser

The user may not have time to flip a toggle or sign in, and that is a normal outcome rather than a blocker. Say plainly what you could not verify, and stop there. Reading the frontend source to work out what the page *would* have shown produces a conclusion, not an observation — and this skill exists to produce observations. "Not visually confirmed" is a complete and useful answer; a claim that implies visual confirmation it never got is neither honest nor useful.
