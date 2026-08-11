# Connector readiness

Per-connector maturity and the next action each one needs. This is an engineering
assessment of the code in this repository — it says nothing about any deployment.

Keep it current when a connector's state changes; a stale readiness table is worse
than none.

## Vocabulary

**Readiness** — how far the connector has been proven, not how complete it looks.

| Level | Meaning |
|---|---|
| `unverified` | Ships and is wired, but no one has confirmed the data it produces is right. |
| `vendor-api-verified` | Exercised against the vendor's real API. Correctness of the resulting silver rows is still unconfirmed. |
| `partial` | Known to be incomplete — a named gap, recorded in the Notes column. |
| `rewrite-needed` | The current implementation is the wrong shape; extending it costs more than replacing it. |
| `unassessed` | Nobody has looked yet. |

**Next step** — the single action that moves it forward.

| Action | Meaning |
|---|---|
| `VALIDATE` | Prove the output is correct end to end, then move it to a higher level. |
| `FINISH` | Implementation is incomplete; complete it. |
| `REWRITE` | Replace the implementation. |
| `REVIEW` | Decide whether it should exist at all before spending anything else on it. |

## Matrix

| Connector | Priority | Readiness | Next step | Notes |
|---|---|---|---|---|
| jira | **P1** | unverified | VALIDATE | Sole producer of all nine `class_task_*` classes |
| bamboohr | **P1** | rewrite-needed | REWRITE | Does not retrieve the full data set — scope the gap before rewriting |
| bitbucket-cloud | **P1** | unassessed | VALIDATE | |
| chatgpt-team | P2 | unverified | VALIDATE | |
| claude-enterprise | P2 | unassessed | VALIDATE | Sole producer of four `class_ai_assistant_usage` measures |
| claude-team | P2 | unassessed | VALIDATE | Sole producer of `class_ai_overage` |
| confluence | P2 | unverified | VALIDATE | |
| gitlab | P2 | partial | VALIDATE | Commit and lines-of-code filters still to be decided |
| m365 | P2 | unverified | VALIDATE | Sole producer of `class_collab_email_activity` and both document-activity branches |
| ms-entra | P2 | unverified | VALIDATE | Roughly four fifths of the surface covered |
| outline | P2 | unverified | VALIDATE | |
| slack | P2 | unverified | VALIDATE | |
| zendesk | P2 | vendor-api-verified | VALIDATE | Sole producer of `class_support_activity` and both support dimensions |
| zoom | P2 | unverified | VALIDATE | |
| github-v2 | P3 | unassessed | REVIEW | |
| hubspot | P3 | unverified | VALIDATE | Sole producer of all five `class_crm_*` classes |
| zulip-proxy | P3 | unverified | VALIDATE | |
| active-directory | unset | unassessed | REVIEW | |
| cursor | unset | unassessed | VALIDATE | Sole producer of `tool_use_offered` / `tool_use_accepted` |
| github-directory | unset | unassessed | — | Added recently; not yet assessed |

## What to validate first

Priority says how much the connector matters. It does not say how much a defect in it
would cost. That comes from how many other sources feed the same silver class: where a
connector is the **only** contributor to a class, a wrong number reaches gold with
nothing to contradict it, and no cross-source reconciliation will reveal it.

Ordering below is that blast radius crossed with the priority above.

**1 — sole producer and P1.** A defect is both unbacked and high-stakes.

- **jira** — the only contributor to all nine `class_task_*` classes. Everything in Task Delivery rests on it alone.
- **bamboohr** — the only contributor to `class_hr_events` and `class_hr_working_hours`, and one of three for `class_people`. It is also `rewrite-needed`, so validate only enough to scope the rewrite.

**2 — sole producer, lower priority.** Unbacked, so still worth doing before anything with a peer.

- **hubspot** — all five `class_crm_*` classes.
- **m365** — `class_collab_email_activity` and both `class_collab_document_activity` branches.
- **zendesk** — `class_support_activity`, `dim_support_agent`, `dim_support_ticket`. Already vendor-api-verified, so what remains is confirming the silver rows.
- **cursor** — the only source of `tool_use_offered` / `tool_use_accepted`, which are the inputs to `ai.accepted_edit_actions` and `ai.tool_acceptance_rate`.
- **claude-team** — `class_ai_overage`.
- **claude-enterprise** — `action_count`, `conversation_count` and the non-chat surfaces of `class_ai_assistant_usage`.

**3 — has a peer, so reconciliation can catch it.** Two sources feeding one class can be
compared against each other; disagreement is itself a signal.

- **git**: bitbucket-cloud, github-v2, gitlab — three contributors to every `class_git_*` class. P1 bitbucket-cloud leads on priority, but a defect here is the most likely to be caught.
- **wiki**: confluence, outline — two contributors to all three `class_wiki_*` classes.
- **chat**: slack, zulip-proxy — peers of m365 on `class_collab_chat_activity`.
- **meetings**: zoom — peer of m365 on `class_collab_meeting_activity`.
- **identity**: ms-entra, active-directory — peers of bamboohr on `class_people` and `class_person_attribute_claims`.
- **AI dev usage**: chatgpt-team — one of four on `class_ai_dev_usage`.

**4 — decide before spending.** `REVIEW` means the question is whether to keep it, so
validating first is wasted effort.

- **github-v2**, **active-directory**, **github-directory**.

## Caveats

- `github-directory` is not yet assessed; its row is a placeholder.
- The bamboohr gap is recorded from a truncated note — confirm the exact scope before starting the rewrite.
- Priorities marked `unset` have not been assigned, not deliberately deprioritised.
