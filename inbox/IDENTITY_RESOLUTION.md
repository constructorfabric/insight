# Identity Resolution

## Overview

The Monitor platform collects user data from multiple independent sources (Git, Zulip, GitLab, Constructor, BambooHR, HubSpot, YouTrack). A single employee may have different usernames, emails, and accounts across these systems. The **identity resolution pipeline** unifies these scattered records into a single profile per person.

## Core Concept

Each source record is decomposed into **(token, rid)** pairs, where:

- **token** — a value that can identify a person: username, email, work_email, etc. Each source may produce any number of tokens.
- **rid** — a deterministic identifier for the source account: `rid = cityHash64(source, source_id)`.

Each pair `(token, rid)` forms a **segment endpoint**. Two records belong to the same person if they share a token — like two segments sharing an endpoint. The algorithm transitively groups all records connected through shared tokens (full value match, not partial).

## Data Sources

Each source normalizes raw data into `(source, source_id, token)` tuples. A single source record may produce multiple tokens:

| Source | Token fields | Notes |
|---|---|---|
| Git | author, email | Lowercased |
| Zulip | full_name, email | |
| GitLab | username, email, commit_email, public_email | Multiple emails per user |
| Constructor | username, full_name, email | Handle + full name if different |
| BambooHR | first_name + last_name, work_email | Dots replaced with spaces in names |
| HubSpot | first_name + last_name, email | From users + owners tables |
| YouTrack | username, email | |

## Walkthrough Example

The following example traces one person — Andrei Sokolov — through the entire pipeline. He appears across three source systems under different usernames and emails.

### Source Data

**BambooHR:**

| id | first_name | last_name | work_email |
|---|---|---|---|
| b1 | Andrei | Sokolov | Andrei.Sokolov@acme.io |

**Git commits:**

| hash | author | email |
|---|---|---|
| c1 | sokol | sokol91@gmail.com |
| c2 | sokol | sokol@example.org |
| c3 | sokol | a.sokolov@gmail.com |

**YouTrack:**

| id | username | email |
|---|---|---|
| y1 | Andrey Sokolov | a.sokolov@acme.com |

### Step 1 — Unified References

Each source record is decomposed into `(source, source_id, token, rid, meta)` tuples. `rid = cityHash64(source, source_id)` — all tokens from the same source account share the same `rid`. Each `(token, rid)` pair is a segment that the algorithm will use for grouping:

| source | source_id | token | rid | meta |
|---|---|---|---|---|
| bamboo | b1 | andrei sokolov | h1 | |
| bamboo | b1 | andrei.sokolov@acme.io | h1 | |
| git | c1 | sokol | h2 | |
| git | c1 | sokol91@gmail.com | h2 | |
| git | c2 | sokol | h3 | |
| git | c2 | sokol@example.org | h3 | |
| git | c3 | sokol | h4 | |
| git | c3 | a.sokolov@gmail.com | h4 | |
| youtrack | y1 | andrey sokolov | h5 | |
| youtrack | y1 | a.sokolov@acme.com | h5 | |

At this point the algorithm can already group some records. Running min-propagation on this data produces **3 groups**:

| Group | Members | Connected by |
|---|---|---|
| 1 | h2, h3, h4 | shared token `sokol` |
| 2 | h1 | isolated — no shared tokens |
| 3 | h5 | isolated — no shared tokens |

Bamboo (h1) and YouTrack (h5) remain separate because their tokens don't overlap with anything else. The enrichment steps below will bridge these gaps.

### Step 2 — Manual Identity Pairs

Injects **synthetic bridge records** from a seed table. Each pair forces two accounts into the same identity group by creating records with a shared `rid`.

This step does not apply to our example, but a pair like:

```text
p.jones@example.net  <->  pjones@example.net
```

would create synthetic records that bridge two otherwise unrelated accounts.

Manual pairs are a **last resort**. The goal is to design algorithmic rules (name aliases, domain aliases) that resolve identities automatically. Manual pairs are only added when no general rule can cover the case.

### Step 3 — First Name Aliases

Uses a seed table. Aliases are **unidirectional**: only records whose username contains a whole word matching the **left column** (`first_name`) get a synthetic record with that word replaced by the **right column** (`alias`). Word boundaries are spaces, start of string, and end of string — not substring matching.

This is important because "Alexey" may call himself "Alex", but "Alex" could be either "Alexey" or "Alexander".

Alias seed data (excerpt):

| first_name | alias |
|---|---|
| andrei | andrey |
| andrey | andrei |

Applied to our example — the bamboo record has username "andrei sokolov" which contains whole word "andrei", and the youtrack record has "andrey sokolov" which contains "andrey":

| source | source_id | token | rid | meta |
|---|---|---|---|---|
| bamboo | b1 | andrey sokolov | h1 | alias: a1 |
| youtrack | y1 | andrei sokolov | h5 | alias: a2 |

Now `h1` and `h5` share the token "andrey sokolov" (and also "andrei sokolov") — they are connected.

### Step 3b — Email Domain Aliases

Uses a seed table. If a record's email is on a domain from the list, synthetic records are generated for **all other domains** in the list.

Domain alias seed data (excerpt):

| domain |
|---|
| gmail.com |
| acme.io |
| acme.com |

Applied to our example — every email on a listed domain gets variants for the other domains:

| source | source_id | token | rid | meta |
|---|---|---|---|---|
| bamboo | b1 | andrei.sokolov@gmail.com | h1 | domain: d1 |
| bamboo | b1 | andrei.sokolov@acme.com | h1 | domain: d3 |
| git | c1 | sokol91@acme.io | h2 | domain: d2 |
| git | c1 | sokol91@acme.com | h2 | domain: d3 |
| git | c3 | a.sokolov@acme.io | h4 | domain: d2 |
| git | c3 | a.sokolov@acme.com | h4 | domain: d3 |
| youtrack | y1 | a.sokolov@acme.io | h5 | domain: d2 |
| youtrack | y1 | a.sokolov@gmail.com | h5 | domain: d1 |

Now `h4` (git c3) and `h5` (youtrack y1) share the token "a.sokolov@gmail.com" — another connection.

### Full Token Table After Enrichment

Combining all real and synthetic records:

| source | source_id | token | rid | meta |
|---|---|---|---|---|
| bamboo | b1 | andrei sokolov | h1 | |
| bamboo | b1 | andrei.sokolov@acme.io | h1 | |
| git | c1 | sokol | h2 | |
| git | c1 | sokol91@gmail.com | h2 | |
| git | c2 | sokol | h3 | |
| git | c2 | sokol@example.org | h3 | |
| git | c3 | sokol | h4 | |
| git | c3 | a.sokolov@gmail.com | h4 | |
| youtrack | y1 | andrey sokolov | h5 | |
| youtrack | y1 | a.sokolov@acme.com | h5 | |
| bamboo | b1 | andrey sokolov | h1 | alias: a1 |
| youtrack | y1 | andrei sokolov | h5 | alias: a2 |
| bamboo | b1 | andrei.sokolov@gmail.com | h1 | domain: d1 |
| bamboo | b1 | andrei.sokolov@acme.com | h1 | domain: d3 |
| git | c1 | sokol91@acme.io | h2 | domain: d2 |
| git | c1 | sokol91@acme.com | h2 | domain: d3 |
| git | c3 | a.sokolov@acme.io | h4 | domain: d2 |
| git | c3 | a.sokolov@acme.com | h4 | domain: d3 |
| youtrack | y1 | a.sokolov@acme.io | h5 | domain: d2 |
| youtrack | y1 | a.sokolov@gmail.com | h5 | domain: d1 |

### Step 4 — Identity Resolution (Min-Propagation)

The algorithm groups records by shared tokens. Each `(token, rid)` pair is a segment — two rids end up in the same group if they share any token.

**Connections found:**

| Shared token | Connects |
|---|---|
| `sokol` | h2, h3, h4 |
| `andrey sokolov` | h1 (alias), h5 |
| `andrei sokolov` | h1, h5 (alias) |
| `a.sokolov@gmail.com` | h4, h5 (domain) |

**Transitive closure:**

```text
h2 ←— "sokol" —→ h3
h2 ←— "sokol" —→ h4
h4 ←— "a.sokolov@gmail.com" —→ h5
h5 ←— "andrey sokolov" —→ h1

Result: h1, h2, h3, h4, h5 → profile_group_id = 1
```

All five source accounts are unified into one profile.

This step runs in two parallel tracks:

1. **Full resolution** — applies the algorithm to ALL records (real + synthetic).
2. **Natural resolution** — applies it to ONLY real records (no aliases, no domain variants).

The **augmented groups** step compares both results and keeps only synthetic records that actually **bridged** distinct natural groups. This prevents synthetic records from inflating groups when they don't contribute new connections.

### Step 5 — Final Groups

Strips all synthetic records. Outputs only real records with their final `profile_group_id`:

| source | source_id | rid | profile_group_id |
|---|---|---|---|
| bamboo | b1 | h1 | 1 |
| git | c1 | h2 | 1 |
| git | c2 | h3 | 1 |
| git | c3 | h4 | 1 |
| youtrack | y1 | h5 | 1 |

### Profile Link

The master profile table. Maps each `profile_group_id` to source-specific IDs:

- `bamboo_id`
- `gitlab_id`
- `platform_id` (Constructor)
- `hubspot_owner_id`
- `zulip_id`
- `git_author_id`

### Profile Emails

An index table mapping every known email to its `profile_link_id`.

## The Min-Propagation Algorithm

### How It Works

The input is a table of `(token, rid)` pairs. The algorithm finds connected components — groups of rids that are transitively linked through shared tokens:

1. **Initialize** — assign each `rid` its own value as its group ID.
2. **Iterate** (default 20 passes):
   - For each token, find the **minimum** group ID among all rids sharing that exact token.
   - Propagate that minimum as the new group ID to every rid associated with that token.
   - On the next pass, rids carry a potentially lower group ID, which propagates further through other shared tokens.
3. **Converge** — after enough iterations, all transitively connected rids share the same minimum group ID.
4. **Rank** — `dense_rank()` converts raw group IDs into sequential `profile_group_id` values.

Matching is always on **full token values** — token "sokol" matches token "sokol", not a substring of "sokol123".

### Blacklist

A seed table contains generic tokens (e.g., "admin", "test", "bot", "root") that are excluded from matching to prevent false positives. Usernames with 3 or fewer characters are also excluded.

## Adding a New Source

1. Create a source model that outputs `(source, source_id, token)` tuples — one row per token (username, email, etc.).
2. Add it to the union in the unified references step.
3. Run the pipeline.

## Ideas: Finding Missing Links

This section collects approaches for discovering identity connections that the current pipeline misses.

### Fuzzy Last Name Search in Email Local Parts

**Problem:** A person's email local part may contain a misspelled or abbreviated last name that exact matching will never catch. For example, `asokolov@gmail.com` (missing letter) won't match the username "andrei sokolov" — there is no shared token.

**Idea:** Extract last names (or rare name parts) from all sources and use them as search queries against email local parts across all unresolved records. Rank results by character-level similarity:

- The email local part is split into **words** by non-letter characters (dots, digits, underscores, etc.). Each word is compared independently against the search term.
- Higher score when more characters of the search term appear in the candidate word (in order).
- Higher score when the lengths of the search term and the candidate word are closer to each other.
- Common first names (Alexey, Nikolay, Sergey, etc.) should be excluded from search — they appear across many unrelated accounts and produce noise. Only relatively unique tokens (typically last names) are useful as search terms.

**Example:**

Search term: `sokolov` (extracted last name)

Candidates are email local parts split into words by non-letter characters. The best-matching word determines the score:

| candidate (raw) | words | best match | matched letters | length diff | score |
|---|---|---|---|---|---|
| a.sokolov | a, sokolov | sokolov | 7/7 | 0 | high |
| asokolov | asokolov | asokolov | 7/7 | +1 | high |
| sokolov123 | sokolov | sokolov | 7/7 | 0 | high |
| soklov | soklov | soklov | 3/7 | -1 | low |
| sokolova | sokolova | sokolova | 7/7 | +1 | high |
| ivanov | ivanov | ivanov | 2/7 | -1 | low |

This kind of search cannot be used for automatic merging — the false positive rate is too high. Instead, it should produce a **review list** of candidate pairs for manual verification. Confirmed pairs are then added to the `identity_pairs` seed table.

**Scope:** This search is only useful across records that are NOT already in the same profile group. Records already grouped together don't need further linking.

## Technical Notes

- All models target **ClickHouse** (uses `arrayJoin`, `cityHash64`, `lowerUTF8`).
- `rid = cityHash64(source, source_id)` — deterministic hash identifying each source account.
- The 20-pass default is sufficient for transitive chains up to 20 hops (in practice, chains are much shorter).
