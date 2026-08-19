# Worked example — scoping tests for a metric consolidation (non-port)

This is the second shape the skill has to handle: not a port, but a **refactor/consolidation** —
[constructorfabric/insight#1538](https://github.com/constructorfabric/insight/issues/1538)
(parent #1515). It collapses today's connector-scoped metric rows (`slack_messages_sent`,
`zulip_messages_sent`, `m365_teams_chats`, …) into four modality counters (`messages_sent`,
`emails_sent`, `meeting_hours`, `files_shared`), each carrying a per-source **breakdown**.

There's no old implementation in another language, so "parity vs the old code" doesn't apply.
The insight is that a consolidation still has a differential — **against the outputs it retires** —
and that's the strongest gate available. This example exists to keep the skill from collapsing
into port-only thinking.

## What reading the code changed (the assumption-correction move, 4×)

The issue reads like a clean spec. The code contradicted it in four load-bearing ways, and each
correction reshaped the scope:

1. **`coalesce(…,0)` vs honest-NULL.** The issue fills absent sources with fake zeros. The house
   rule (`20260423120000_bullet-views-honest-nulls.sql`, and #1515's own principle) is
   honest-NULL — an un-ingested source must be **absent/NULL**, so the FE renders "ComingSoon",
   not a fake 0 bar. → became its own test group + an acceptance gate.
2. **Wrong field mapping.** The issue derives `meeting_hours` from
   `bronze_zoom.participants WHERE status='in_meeting'`; there is no `status` column. The real
   pipeline reads silver `class_collab_meeting_activity` as `greatest(audio,video,screen)/3600`
   for both sources. → corrected the map before writing any check.
3. **Breakdown is a net-new shape, and the parts still exist as sibling keys.** Today the parts
   are separate `metric_key`s that already ship. That's the gift: the consolidation's correctness
   is provable as a **differential** — new `messages_sent` must equal
   `slack_messages_sent + zulip_messages_sent + m365_teams_chats` on the same seed. → became the
   headline gate.
4. **Not cross-vendor comparable.** `messages_sent`→`total_chat_messages`, which silver documents
   as "intentionally NOT a clean cross-vendor metric." → tests must not assert cross-vendor
   absolute equality, only the per-source parts.

Grounding also caught scaffolding reality the issue glosses: **Slack has zero e2e scaffolding**
(needs a new `bronze_slack` template), so "one validation test per source" is more work than it reads.

## The scope that resulted (abridged)

> Test scope for collab modality counters + breakdown (#1538, parent #1515). Goal: on the same
> seeded data, each new counter reproduces the retired sibling keys it subsumes, and
> `metric_value == Σ breakdown[].value` — daily and after roll-up.
>
> **Axis = metric × part × silver-field.** Corrected framing: honest-NULL not fake-zero;
> `meeting_hours` from silver `greatest(audio,video,screen)`; parts already exist as sibling keys
> (differential targets); `messages_sent` not cross-vendor comparable.
>
> **Out of scope:** the other 13 metrics in #1515; threshold calibration; fuzzy/alias identity;
> documented connector limits (Zoom QoS, Slack parity).
>
> **1. Metric math & the sum-invariant, per part** — each part maps to the right silver field;
> `value = Σ parts`; no double-count. (matrix: metric × part → silver field → retired key)
> **2. Differential vs retired keys (headline gate)** — same seed → new counter == Σ retired
> siblings; each part == its old key; zero diff.
> **3. Honest-NULL & fail-safe** — absent source ⇒ no fake-0 part; all parts absent ⇒ value NULL
> (ComingSoon); blank email dropped, not attributed.
> **4. Identity, peer & period roll-up** — unify only on `lower(email)`; **breakdown array sums
> over week/month/quarter and the invariant still holds** (new SQL — no existing machinery merges
> a tuple array over N days); org percentiles attach; no cross-tenant leakage.
> **5. Consumer contract & cutover** — new keys registered; FE breakdown actually renders (dead
> today, `breakdown={[]}`); retired keys pulled from FE key-maps *and* the downstream trend view
> that hard-codes them; existing collab e2e migrated; new `bronze_slack` scaffolding added;
> analytics restarted after reseed.
>
> ## Acceptance
> - [ ] All groups pass
> - [ ] Differential: new counter == Σ retired sibling keys, each part == its retired key, zero diff
> - [ ] Sum-invariant holds daily **and** after roll-up
> - [ ] Absent source ⇒ NULL, never fake-0; per-part test exists for every matrix cell

## What to notice

- The **differential survives the loss of an old implementation** — it just retargets from
  "the old service" to "the sibling keys this replaces." When a feature consolidates or rewrites
  existing outputs, always ask *what does it retire?* and gate on reproducing that.
- A consolidation adds its own invariants: **whole == Σ parts**, and it must **keep holding after
  roll-up**. Those beat any hand-written case list.
- The corrections weren't nitpicks — the honest-NULL and field-mapping catches would each have
  produced a test suite that asserts the wrong thing. That's the skill earning its keep.
