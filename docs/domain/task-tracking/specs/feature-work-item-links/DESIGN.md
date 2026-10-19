# Technical Design — Links Between Work Items

Collecting the links GitHub and Jira record between issues, and between an
issue and a pull request, so that the links in force over any interval can be
read back.

<!-- toc -->

- [1. What a link is here](#1-what-a-link-is-here)
- [2. Two sources, because one is not enough](#2-two-sources-because-one-is-not-enough)
- [3. Storage: intervals, not a current set](#3-storage-intervals-not-a-current-set)
- [4. Reading it](#4-reading-it)
- [5. Edges that are bounds rather than facts](#5-edges-that-are-bounds-rather-than-facts)
- [6. Both directions are stored](#6-both-directions-are-stored)
- [7. Jira](#7-jira)
- [8. Deferred](#8-deferred)

<!-- tocstop -->

## 1. What a link is here

Six kinds, in one vocabulary:

| `link_type` | Meaning | Target |
|---|---|---|
| `parent` | the item's parent | issue |
| `sub_issue` | the item is the parent of the target | issue |
| `blocked_by` | the target blocks the item | issue |
| `blocking` | the item blocks the target | issue |
| `duplicate_of` | the item was marked a duplicate of the target | issue or pull request |
| `connected` | a manual connection | issue or pull request |
| `closed_by_pr` | the target pull request closes the item | pull request |

A target carries its own repository, because a link may cross repositories and
an issue number alone identifies nothing.

## 2. Two sources, because one is not enough

**The timeline** emits twelve link event types, each a pure delta: it names the
link that appeared or disappeared and never the set that resulted. Hierarchy
and dependency links always emit a matching pair, so folding adds against
removes reconstructs the set exactly — a claim checked against real data, where
seven additions and two removals reconciled with the count the vendor reports.

**The snapshot** — `closedByPullRequestsReferences` and the hierarchy
connections on the issue — states what is true now and carries no history.

Neither alone is enough, and the split is not a preference:

- Hierarchy and dependency links fold exactly, so the timeline rules them.
- A pull request closing an issue does NOT reliably reach the timeline. It may
  arrive as a `ConnectedEvent`, as a `CrossReferencedEvent` that reports
  `willCloseTarget: false`, or not at all, while the connection states it
  plainly. So that kind is observed, never folded.

No link type is ever built from both sources, because their precision differs
and a consumer that averaged them would be comparing a second to a sync.

## 3. Storage: intervals, not a current set

`silver.class_task_links` holds one row per link OCCURRENCE:

| column | meaning |
|---|---|
| `valid_from` | when the link appeared |
| `valid_to` | when it went away; NULL while it is still there |
| `valid_from_known` | 1 when the vendor stated the moment; 0 when it is a lower bound |
| `evidence` | `event` (folded from add/remove) or `observation` (bounded by first and last seen) |

A link removed and re-added later is two rows. `valid_from` is therefore part
of the key — as the identity of an occurrence, not as a version of one row,
which is the thing `union_by_tag` forbids (ADR-0001, ADR-0004).

Intervals are built by pairing each addition with the nearest removal that
follows it, in ClickHouse an `ASOF LEFT JOIN`. One property of that join drives
a guard: an ASOF join that matches nothing yields the column type's DEFAULT
rather than NULL, so an unclosed interval would read as closed at the epoch —
a real date, which every range query would believe.
`assert_link_intervals_are_sane` refuses that shape, along with an interval
that ends before it begins.

## 4. Reading it

The links in force over a window:

```sql
SELECT * FROM silver.class_task_links FINAL
WHERE valid_from < :to AND (valid_to IS NULL OR valid_to > :from)
```

The links as of an instant is the same predicate with one point. The links now
is `valid_to IS NULL`.

## 5. Edges that are bounds rather than facts

`valid_from_known = 0` marks a lower bound, and there are two ways to get one:

- **A removal with no addition.** The link was created before anything
  collected. Dropping the row would report a link that never existed;
  inventing a start would report a date nobody observed. The interval is left
  open at the bottom and flagged.
- **An observed link.** First-seen is not first-true — the link may predate the
  first snapshot that carried it.

Filtering these out silently drops the oldest links, which are exactly the ones
a long window asks about.

## 6. Both directions are stored

One action writes two events: a parent records `SubIssueAdded`, its child
records `ParentIssueAdded`, at the same instant. Both are kept, so a link
appears once from each side.

Deduplicating to a canonical direction would make the parent's history depend
on whether the child was collected, and the two ends can live in different
repositories — one of which the token may not be able to read at all.

## 7. Jira

`jira__task_links` folds the changelog. A link added or removed is one
`issuelinks` item naming the linked issue by the key it had then, with a
sentence ("This issue blocks PROJ-2", or "This work item blocks PROJ-2" in
newer entries) that is the only place its type survives;
a parent change is one `parent` item naming the old and the new parent by id
and key. Every interval is therefore `evidence = event`.

- **An occurrence is a link of one type to an issue.** "blocks PROJ-2" and
  "relates to PROJ-2" open and close on their own. The occurrence is keyed by
  the link type's immutable id and this side's direction, never by its phrase:
  an administrator can reword a type, and the changelog keeps the phrase of the
  day it was written.
- **Phrase to type.** The sentence's subject is dropped and its tail, the
  phrase, is resolved in order: to the link held between the same two issues
  that states that phrase; to the one held link no phrase states, when it is the
  only phrase there still added; to a type the phrase has been seen to mean
  that is already known between the two issues; to the type it means most
  often. A phrase that resolves to none keeps the occurrence apart by itself.
- **Mapping.** The Blocks type is `blocking` outward and `blocked_by` inward,
  and Duplicate outward is `duplicate_of`, by the current name of the resolved
  type, then by Jira's default wording for a phrase resolved to no type. Every
  other type — Relates, Cloners, a custom type — is `connected`, and so is
  Duplicate inward: the vocabulary has no inverse of `duplicate_of`.
- **Held before collection.** A link whose first event removed it, or that the
  snapshot holds and no event reaches, opens at the issue's creation with
  `valid_from_known = 0` — the earliest it can have existed.
- **Both directions.** Jira logs a link on both issues, so each side is read
  from its own changelog. It logs a parent change on the child alone, so the
  parent's `sub_issue` row is mirrored from it.
- **Immutable target ids.** The changelog's key resolves to the linked issue's
  id through `jira__issue_key_ids`: the issues' own rows, the `Key` items of
  moved issues, the links snapshots hold, and parent changes. A key that
  resolves to nothing — a project the connector cannot read, an issue deleted
  before it was read — leaves `target_id` NULL and keeps `target_readable`; the
  key still keeps the occurrence apart.

A link that disappears with no event — the linked issue deleted — stays open,
because the changelog is the only evidence an interval reads. The producer is
rebuilt whole, so `class_task_links` reconciles Jira's rows against it.

## 8. Deferred

- **Cross-references.** `CrossReferencedEvent` is a mention, not a link: it has
  no removal counterpart, so it is a point in time and not an interval. It
  belongs in an event relation, not this one.
- **The pull-request side of a connection.** `closingIssuesReferences` states
  the same link from the other end; collecting it would duplicate what the
  issue side already carries.
- **Jira's Epic Link.** Instances that predate the `parent` field record an
  epic's children in an app field; only `parent` changes are read.
- **A consumer.** Nothing in gold reads this yet.
