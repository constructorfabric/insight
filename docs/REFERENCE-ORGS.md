# Reference Organisations

Defines the organisation sizes Insight is tested against, how much data each one holds, and the
rules for building a fixture that matches. It exists so that a performance or soak fixture can be
built to scale, latency budgets are comparable run to run, and a cost-of-ownership figure has a
denominator.

Figures are derived from measuring **two production installations** with identical SQL and are
stated here as project standards. Companion documents: [`TESTING.md`](TESTING.md) (where these
fixtures run) and [`product/VISION.md`](product/VISION.md) §6.4.2 (the company-size tiers).
Tracked in #2215 (reference organisations) and #2216 (recommended resource requirements).

---

## 1. Scope and purpose

The quality framework names one measurement fixture — *a reference organisation of ~1,000 users
with a typical connector mix* — shared by the **Efficiency** vector (#1785: compute footprint →
cost of ownership) and the **Performance** vector (#1787: P95 per data endpoint), and records its
connector and concurrency profile as an open input. [`TESTING.md`](TESTING.md) §7 places load,
stress and soak in the **Test** and **Beta** stages, and #1655 asks for latency budgets per
week / month / quarter / year data slice.

Neither states **how much data** a reference organisation holds. Without that, a fixture cannot be
built to scale, latency budgets cannot be compared run to run, and a TCO figure has no denominator.
This document supplies those numbers.

## 2. How organisation size is defined

> **Organisation size is counted in ACTIVE people only.** People marked terminated in the HR
> system of record are excluded from every denominator, every per-person figure and every
> extrapolation.

The distinction is load-bearing, not cosmetic: in one measured installation **71 % of person
records were terminated**, so quoting record counts would have overstated the organisation by
**3.7×**.

Four populations exist and must never be substituted for one another:

| Population | Definition | Used for |
|---|---|---|
| **Active people** | HR system of record, active status, distinct work emails | **the org size** — every density figure |
| Person records incl. terminated | all rows in the people model | storage accounting only |
| Directory accounts | active accounts in the identity directory | identity-store sizing, authentication load |
| Identities present in activity data | distinct actors per metric class | fixture population per class |

The last one is the trap: measured per-class activity identities run **0.07× to 1.77×** the active
roster. Ratios **above 1.0** are correct, not errors — external meeting attendees, service
accounts, shared mailboxes, automation and departed employees still attached to historical records
all author rows. A fixture populated only from the employee list will never reproduce them, and
identity resolution is precisely what they stress.

## 3. The reference organisations

Aligned to the company-size tiers in [`product/VISION.md`](product/VISION.md) §6.4.2. One reference
organisation per tier, placed at the tier ceiling so that passing at the reference covers the tier
below it.

| Tier | Span (people) | Reference org | Active people |
|---|---|---|--:|
| Small teams | 5–50 | **REF-S** | 50 |
| Mid-size organizations | 50–500 | **REF-M** | 500 |
| Large organizations | 500–5,000 | **REF-L** | **3,000** |
| Enterprise organizations | 5,000+ | *not defined* | — |

**REF-L is the primary fixture.** At 78 % into the Large band it represents the tier's upper half
rather than its floor, and it matches the 3,000-user dataset already scoped for the shared load
harness, so one fixture serves every dependent scenario. **The cost is confidence:** 3,000 active
people is a **5.8–7.7× extrapolation** from installations of 392 and 521 active people, against
1.9–2.6× for a 1,000-person fixture — this is the largest single source of error in this document.
Note that the quality framework currently names *"~1,000 users"*; REF-L = 3,000 supersedes that
number and the framework text needs updating to match (tracked in #2215).

**REF-L covers the tier's upper half but not its ceiling.** A 5,000-person installation is 1.7×
REF-L, and no evidence in this study reaches either point directly. Figures for 5,000 people appear
below as an informational extrapolation only — not a defined reference organisation.

### 3.1 How much data a reference organisation holds

Two inputs: **how many active people**, and **how old the organisation is**. Insight is installed
into brownfield organisations, so accumulated history is the default — one year of activity is the
*floor*, not the target.

**Step 1 — one year of activity.** Active people × 365 days × **112 rows / 88 kB per person per
day**. At REF-L that is 122.5 M rows and 89.4 GiB.

**Step 2 — multiply each family by its age factor.** Older organisations hold more, but far less
than you would expect: if activity were flat, a 10-year-old organisation would hold 10 years of
data. It does not. An 18-year-old tracker corpus is worth **4.2 years** of today's volume, because
activity compounds — the early years are thin — and several sources refuse to serve old data at all.

| Family | Share of a year's rows | at 5 years | at 10 years | at 20 years | What limits it |
|---|--:|--:|--:|--:|---|
| Task | 45 % *(and 90 % of bytes)* | ×1.7 | ×2.5 | ×4.2 | nothing — trackers keep everything |
| Git | 29 % | ×1.7 | ×2.5 | ×3.4 | nothing at source; see limits |
| Collaboration | 15 % | **×1** | **×1** | **×1** | source serves 27 days (M365), 150 (Zoom) |
| CRM & support | 8 % | ×2.8 | ×3.2 | ×3.2 | when the CRM was adopted, not the org's age |
| HR & identity | 2 % | **×1** | **×1** | **×1** | no history — starts at connector install |
| AI | 1 % | **×1** | **×1** | **×1** | vendor keeps 7–10 months |
| Wiki | 1 % | ×3.2 | ×6.0 | ×11.5 | nothing — pages persist as content |

Between those columns the factors grow by a fixed amount per year, so interpolate linearly.

**Three of the seven families do not accumulate at all.** Collaboration, AI and HR are capped by
what the source will serve, not by the organisation's age — a twenty-year-old company and a
two-year-old one arrive with **identical** email, chat, meeting and AI corpora.

**Step 3 — add the stock**, the inventory that arrives whole (below): 0.5–2.5 M rows.

**The result.** **Ten years is the default** — mid-range of the two measured installs, and where
the model is best anchored. The *no history* column is the one-year-of-flow floor.

| Reference org | Active people | No history *(floor)* | 5 years old | **10 years old** *(default)* | 20 years old |
|---|--:|---|---|---|---|
| **REF-S** | 50 | 2.0 M · 1.5 GiB | 3.4 M · 2.5 GiB | **4.8 M · 3.7 GiB** | 6.9 M · 6.1 GiB |
| **REF-M** | 500 | 20.4 M · 14.9 GiB | 33.7 M · 25.2 GiB | **47.6 M · 37.5 GiB** | 69.0 M · 61.1 GiB |
| **REF-L** | 3,000 | 122.5 M · 89.4 GiB | 202.2 M · 150.9 GiB | **285.4 M · 224.8 GiB** | 414.3 M · 366.5 GiB |
| *(informational)* | 5,000 | 204.2 M · 149.1 GiB | 337.0 M · 251.5 GiB | *475.7 M · 374.7 GiB* | 690.5 M · 610.8 GiB |

Rows are logical (dedup-free); bytes are uncompressed. Double the people, double the numbers. **Bytes grow faster
than rows** — 4.1× versus 3.4× at twenty years — because 89.5 % of the bytes are the task family, the
second-steepest accumulator. Any storage or cost-of-ownership figure is therefore more sensitive to
the brownfield correction than any row count or query-latency figure.

**This is a bracket, not a point estimate.** The upper bound is an established enterprise that did
*not* grow into its headcount — its history is thin only because tools got chattier, measured at
**1.13× a year** in per-author intensity. At twenty years that is **1.9× the central figure**. Use the
central column as the sizing target and the upper bracket as the soak and headroom target.

| Scenario | 5 years | 10 years | 20 years |
|---|--:|--:|--:|
| As-configured *(what the measured installs actually ingest)* | 192 M · 149 GiB | 244 M · 218 GiB | 337 M · 354 GiB |
| **Central** *(above)* | **202 M · 151 GiB** | **285 M · 225 GiB** | **414 M · 366 GiB** |
| Mature, flat headcount | 421 M · 350 GiB | 622 M · 536 GiB | 790 M · 691 GiB |

**Plus STOCK — what arrives whole.** Inventory, not flow: it lands complete on day one and has no
taper. It is **0.4–2.0 % of the rows** but it is what stresses joins, identity resolution and
dimension lookups. Each entry is a **fixture input to be recorded**, never derived from headcount —
the two installations differ by up to **14× per active person** here while their activity *rates*
agree within 1.62×.

| Stock | Scales with | Per active person | REF-L (3,000) |
|---|---|--:|--:|
| Git repositories | estate age, M&A, monorepo policy | 3.5 – 30.5 | 10,500 – 91,500 |
| Git branches | **repository count** (12–16 per repo) | 40 – 380 | 127,000 – 1,470,000 |
| Wiki pages | age — the *flow* is edits; pages accumulate | 2.7 – 38.1 | 8,100 – 114,300 |
| Work items | age + tracker adoption | 182 – 380 | 546,000 – 1,140,000 |
| Boards / sprints | teams and projects, no person axis | — | 40–500 boards · 1,800–22,500 sprints |
| Tracker + directory accounts | **lifetime** headcount × connected tools | 5.8 – 20.2 | 17,400 – 60,600 |
| HR roster incl. terminated | lifetime employees, capped at HRIS adoption | 2.0 – 3.6 × active | 6,000 – 10,800 |
| Identity inputs / persons | **accounts × tools** (~7.6 rows/account), not age | 111 – 229 | 334,000 – 686,000 |
| **Total stock** | | | **0.5 – 2.5 M rows · 0.2 – 1.5 GiB** |

Repository inventory is the sharpest illustration: one install carries 10,548 repositories against
392 active people — **27 per person** — and 71 % of them were created in a single month by a
platform migration that rewrote their creation dates. It is a sizing input, never an organic rate.

## 4. Typical organisation data, by metric class

What an organisation produces, one row per **metric class**.

* **Rows per active person-day** — the observed span between the two installations. Two
  organisations, not a distribution: read it as *"a real organisation lands in here"*, never as a
  mean. *(one-sided)* marks a class only one installation populates — a coverage gap, not a zero.
* **p50 per participant-week** — rows per ISO week for the people who actually appear in the class.
  Contamination-resistant, so it is the figure to calibrate a generator against.
* **Adoption** — share of the active roster appearing in the class at all. Above 100 % is correct,
  not an error: external meeting attendees, service accounts, shared mailboxes, automation and
  departed employees still attached to history all author rows.
* **REF-L annual flow** — one year of rows at 3,000 active people, at each class's canonical layer
  (one layer per class, so the total is not the whole-install figure in §3.1). For a brownfield
  organisation multiply by that family's age factor from §3.1 — ×2.5 for task and git at ten years,
  ×6.0 for wiki, ×1 for collaboration, AI and HR.

| Metric class | Rows per active person-day | p50 per participant-week | Adoption | REF-L annual flow |
|---|---|--:|--:|--:|
| **Git** | | | | |
| `git_commits` | 0.76 – 1.44 | 7 – 8 | 47 – 84 % | 1,580,852 |
| `git_file_changes` | 4.62 – 5.22 | 19 – 31 | 43 – 68 % | 5,717,871 |
| `git_prs` | 0.12 – 0.23 | 3 – 4 | 34 – 36 % | 250,098 |
| `git_pr_comments` | 0.22 – 0.49 | 6 | 26 – 44 % | 533,046 |
| `git_pr_reviews` | 0.21 *(one-sided)* | 4 | 30 % | 231,812 |
| `git_branches` | 0.11 *(one-sided)* | — | — | — |
| **Task** | | | | |
| `task_history` | 5.71 – 9.60 | 25 – 30 | 58 – 58 % | 10,513,971 |
| `task_issues` | 0.34 – 0.41 | 3 – 4 | 52 – 69 % | 453,440 |
| `task_comments` | 0.32 – 0.82 | 4 | 53 – 58 % | 894,725 |
| `task_worklogs` | 0.17 – 0.91 | 5 – 13 | 32 – 47 % | 995,793 |
| `task_sprints` | 0.00 – 0.01 | — | — | — |
| **Collaboration** | | | | |
| `collab_chat` | 0.73 – 1.55 | 6 – 7 | 107 – 130 % | 1,694,732 |
| `collab_email` | 0.70 – 1.10 | 6 – 7 | 89 – 163 % | 1,205,704 |
| `collab_docs` | 0.50 – 0.66 | 5 – 6 | 73 – 109 % | 721,167 |
| `collab_meeting` | 0.37 – 0.58 | 4 | 100 – 177 % | 633,895 |
| **AI** | | | | |
| `ai_dev` | 0.11 – 0.20 | 4 – 6 | 30 – 36 % | 218,343 |
| `ai_chat` | 0.03 – 0.05 | 3 – 4 | 9 – 28 % | 50,589 |
| `ai_cost` | 0.00 – 1.01 | 1 – 40 | 20 – 29 % | — |
| `ai_api` | — | — | — | — |
| **Wiki** | | | | |
| `wiki_edits` | 0.08 – 0.17 | 3 | 33 – 33 % | 187,136 |
| `wiki_pages` | 0.02 – 0.02 | 0.98 – 1 | 20 – 23 % | 22,995 |
| `wiki_comments` | 0.00 – 0.01 | 1.76 – 2 | 8 – 9 % | 6,022 |
| `wiki_engagement` | 0.00 – 0.00 | — | — | 2,190 |
| **CRM & Support** | | | | |
| `crm_activities` | 1.39 *(one-sided)* | — | — | 1,523,145 |
| `crm_contacts` | 0.43 *(one-sided)* | — | — | 475,011 |
| `crm_accounts` | 0.12 *(one-sided)* | — | — | 128,006 |
| `crm_deals` | 0.03 *(one-sided)* | — | — | 30,770 |
| `support_tickets` | 0.03 *(one-sided)* | — | — | 33,178 |
| `support_events` | — | — | — | — |
| **HR & Identity** | | | | |
| `hr_hours` | 0.33 – 0.47 | 4 | 100 – 177 % | 509,941 |
| `hr_people` | 0.02 – 0.03 | — | — | 31,755 |
| `hr_events` | 0.07 *(one-sided)* | — | — | 81,030 |
| `identity_inputs` | — | — | — | — |
| `identity_aliases` | 0.06 *(one-sided)* | — | — | 68,985 |
| **total, canonical layer** | | | | **28,796,200** |

At the ten-year default the same classes carry roughly **2.5×** this in task and git, **6×** in wiki
and **1×** in collaboration, AI and HR — see §3.1.

**Two classes carry most of the volume** — `task_history` and `git_file_changes` are 57 % of the
canonical-layer rows, and `task_issues` and `task_history` are 82 % of its bytes. An organisation
that spreads its volume evenly across classes does not exist.

**Some classes do not scale with headcount** and must be set from another input: `git_branches`
from repository count, `task_sprints` flat (40–500 boards), `crm_contacts` from a customer count
(they are *external* people), `crm_accounts` and `crm_deals` from the sales sub-roster (7.1 % and
4.4 % adoption), `support_tickets` as a flat organisation rate, `wiki_engagement` from page count,
`identity_inputs` from accounts × rows-per-account.

**Row counts do not transfer between products; human rates do.** Chat row emission differs 2.12×
between two chat products for message volumes that agree within 9 %; a worklog entry is 1.81 h in
one organisation and 6.80 h in the other, so the same effort emits 3.8× the rows; the same logical
issue costs 80.6 kB in one tracker and 19.2 kB in another. Size a class on the human rate, then
apply the product's emission multiplier explicitly.

## 5. Evidence and limits

Measured on **two production installations** — 392 and 521 active people — with identical SQL and
matching ClickHouse builds, 2026-08-05.

Density **per person** transfers across products: issue creation agrees to **1.20×** across two
different trackers, git file changes to **1.13×** across two git products, chat messages to within
**9 %**. Of 22 classes measured on both, 15 agree within 2×, median 1.62×. **None of the seven
disagreements is a difference in how people work** — every one is a property of the connector.

Limits, stated rather than implied:

* **REF-L is a 5.8–7.7× extrapolation.** Both installations sit within 30 % of the mid/large tier
  boundary, so REF-M is near-measured, REF-L is reached by extrapolation and REF-S by extrapolating
  downward 8–10×. The 5,000-person figures have no support at all. A third measured installation
  above 1,000 active people would reduce the error more than any other work.
* **The transfer test is arithmetically circular** — both coefficients are per-person-per-day and
  headcount is the only multiplier, so the prediction error equals the coefficient ratio by
  construction. It measures transferability, not correctness.
* **Both installations are ~80 % one connector**, so any total is dominated by one product's record
  shape.
* **Concurrency and query mix are not measured**, and cannot be inferred from this data.
* **The git taper is inherited, not measured.** Both installations bound their git history with an
  operator-configured backfill floor — one has **zero** rows before 2025-01-01, the other before
  2026-01-05 — so neither can show how git accumulates. Git sources retain everything forever, so
  this is under-collection by configuration, not a source limit: an operator who changes that
  setting changes the volume. Its age factor is borrowed from the task family and capped by a
  growth decomposition. Git is **28.9 % of the rows**, so this is the largest unmeasured term.
* **CRM and support are one-sided** — measured on one installation only; the other has the connector
  provisioned and never synced.
* **The wiki age factor rests on two products that disagree 1.4×**, and the older one is corrected
  upward for truncation. Wiki is the steepest accumulator, so twenty years is where this matters.
* **Both readings of the taper are real, and which one dominates is a property of the connector.**
  Compounding is *demonstrated* for task and wiki — every year present and non-zero, no
  zero-then-jump anywhere. Truncation is *demonstrated* for git and collaboration — hard floors in
  the connector configuration and the vendor API. The model fits the observable either way, but a
  fixture built on the truncation classes is sensitive to settings an operator can change.
