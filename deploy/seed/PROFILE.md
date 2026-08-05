<!-- GENERATED FILE — do not hand-edit.
     Regenerate: python3 deploy/seed/render_profile.py
     Verify:     python3 deploy/seed/render_profile.py --check
     Content is derived from deploy/seed/manifest.py + profiles.py. -->

# Seed Profile

What a stand seeded by `deploy/seed` contains. Generated from the same
builder that writes `manifest.json`, so the two cannot disagree.

## Stand summary

| Field | Value |
|---|---|
| tenant | `00000000-df51-5b42-9538-d2b56b7ee953` |
| tenant (other) | `11111111-1111-4111-8111-111111111111` |
| realm | `insight` |
| anchor_date | `2026-06-30` |
| data_window | `2026-05-02..2026-06-30` |
| seed_revision | `35aeb2b31c302e8a` |
| manifest_version | 1 |

`anchor_date` is the last day carrying seeded activity. It is resolved
once per run from `SEED_ANCHOR_DATE` (an ISO date, or the literal
`today`) and defaults to yesterday UTC — the current day is excluded so
a partial day does not fight the gold views' day-aligned aggregates.
Pin it to reproduce a dataset exactly; the value above is the one this
page was rendered against, not necessarily the one on your stand.

## Roster

27 people, all but one in the default tenant. The
exception is `other_tenant_lead`, who exists ONLY so cross-tenant refusal
has a caller to refuse — no team, no org-chart edge, no activity, so they
cannot appear in another persona's subtree or move a metric.

`uuid` is both the Keycloak user id and the
`identity.persons` person id, so a login and an API row refer to the same
person.

| email | display_name | team | role | realm roles | uuid |
|---|---|---|---|---|---|
| `email_ceo@company.nonpresent` | Ava Carter | — | ceo | insight-admin, insight-lead | `aaaaaaaa-0000-0000-0000-000000000001` |
| `email_development_lead@company.nonpresent` | Liam Nguyen | development | lead | insight-lead | `00000000-0000-0000-0000-000000000010` |
| `email_sales_lead@company.nonpresent` | Maya Patel | sales | lead | insight-lead | `aaaaaaaa-0000-0000-0000-000000000020` |
| `email_hr_lead@company.nonpresent` | Noah Rivera | hr | lead | insight-lead | `aaaaaaaa-0000-0000-0000-000000000030` |
| `email_support_lead@company.nonpresent` | Zoe Brooks | support | lead | insight-lead | `aaaaaaaa-0000-0000-0000-000000000040` |
| `email_development_01@company.nonpresent` | Ethan Okafor | development | ic | insight-member | `bbbbbbbb-0000-0000-0000-000000010001` |
| `email_development_02@company.nonpresent` | Aria Meyer | development | ic | insight-member | `bbbbbbbb-0000-0000-0000-000000010002` |
| `email_development_03@company.nonpresent` | Leo Sato | development | ic | insight-member | `bbbbbbbb-0000-0000-0000-000000010003` |
| `email_development_04@company.nonpresent` | Nora Flores | development | ic | insight-member | `bbbbbbbb-0000-0000-0000-000000010004` |
| `email_development_05@company.nonpresent` | Kai Haas | development | ic | insight-member | `bbbbbbbb-0000-0000-0000-000000010005` |
| `email_sales_01@company.nonpresent` | Ivy Kelly | sales | ic | insight-member | `bbbbbbbb-0000-0000-0000-000000020001` |
| `email_sales_02@company.nonpresent` | Owen Novak | sales | ic | insight-member | `bbbbbbbb-0000-0000-0000-000000020002` |
| `email_sales_03@company.nonpresent` | Mila Reyes | sales | ic | insight-member | `bbbbbbbb-0000-0000-0000-000000020003` |
| `email_sales_04@company.nonpresent` | Ezra Park | sales | ic | insight-member | `bbbbbbbb-0000-0000-0000-000000020004` |
| `email_sales_05@company.nonpresent` | Luna Bauer | sales | ic | insight-member | `bbbbbbbb-0000-0000-0000-000000020005` |
| `email_hr_01@company.nonpresent` | Finn Costa | hr | ic | insight-member | `bbbbbbbb-0000-0000-0000-000000030001` |
| `email_hr_02@company.nonpresent` | Ruby Lund | hr | ic | insight-member | `bbbbbbbb-0000-0000-0000-000000030002` |
| `email_hr_03@company.nonpresent` | Milo Amari | hr | ic | insight-member | `bbbbbbbb-0000-0000-0000-000000030003` |
| `email_hr_04@company.nonpresent` | Sage Dixon | hr | ic | insight-member | `bbbbbbbb-0000-0000-0000-000000030004` |
| `email_hr_05@company.nonpresent` | Cole Frost | hr | ic | insight-member | `bbbbbbbb-0000-0000-0000-000000030005` |
| `email_support_01@company.nonpresent` | Iris Grant | support | ic | insight-member | `bbbbbbbb-0000-0000-0000-000000040001` |
| `email_support_02@company.nonpresent` | Jude Hale | support | ic | insight-member | `bbbbbbbb-0000-0000-0000-000000040002` |
| `email_support_03@company.nonpresent` | Elle Ivers | support | ic | insight-member | `bbbbbbbb-0000-0000-0000-000000040003` |
| `email_support_04@company.nonpresent` | Reid Jansen | support | ic | insight-member | `bbbbbbbb-0000-0000-0000-000000040004` |
| `email_support_05@company.nonpresent` | Wren Keir | support | ic | insight-member | `bbbbbbbb-0000-0000-0000-000000040005` |
| `email_admin_operator@company.nonpresent` | Beau Lowe | — | admin | insight-admin | `cccccccc-0000-0000-0000-000000000001` |
| `email_other_tenant_lead@company.nonpresent` | Vera Kovac | — | lead | insight-lead | `dddddddd-0000-0000-0000-000000000001` |

No password appears here or in `manifest.json`. Personas are referenced
by identity; the shared local login secret lives in the compose env and
the generated Keycloak realm.

## Fixtures

Stable names a test declares its data requirements against. The names are
a contract — they describe a role in the org, not a particular person — so
renaming one breaks every test that declares it.

| fixture | email | team | role | uuid |
|---|---|---|---|---|
| `admin_operator` | `email_admin_operator@company.nonpresent` | — | admin | `cccccccc-0000-0000-0000-000000000001` |
| `ceo` | `email_ceo@company.nonpresent` | — | ceo | `aaaaaaaa-0000-0000-0000-000000000001` |
| `dev_lead` | `email_development_lead@company.nonpresent` | development | lead | `00000000-0000-0000-0000-000000000010` |
| `development_ic` | `email_development_01@company.nonpresent` | development | ic | `bbbbbbbb-0000-0000-0000-000000010001` |
| `hr_ic` | `email_hr_01@company.nonpresent` | hr | ic | `bbbbbbbb-0000-0000-0000-000000030001` |
| `hr_lead` | `email_hr_lead@company.nonpresent` | hr | lead | `aaaaaaaa-0000-0000-0000-000000000030` |
| `other_tenant_lead` | `email_other_tenant_lead@company.nonpresent` | — | lead | `dddddddd-0000-0000-0000-000000000001` |
| `sales_ic` | `email_sales_01@company.nonpresent` | sales | ic | `bbbbbbbb-0000-0000-0000-000000020001` |
| `sales_lead` | `email_sales_lead@company.nonpresent` | sales | lead | `aaaaaaaa-0000-0000-0000-000000000020` |
| `support_ic` | `email_support_01@company.nonpresent` | support | ic | `bbbbbbbb-0000-0000-0000-000000040001` |
| `support_lead` | `email_support_lead@company.nonpresent` | support | lead | `aaaaaaaa-0000-0000-0000-000000000040` |

## Catalogue rows

Rows the product provisions by operator or migration, so no endpoint
creates them and no test fixture can either — the suite holds no
database connection. Seeded by `deploy/seed/analytics.py` and named
here so a test reads the name rather than hardcoding one.

**No tenant `metric_definitions` override.** Nothing proves the listing
resolves a tenant's label over the product default.

## Populated / golden metrics

**None.** The golden set is empty, and that is a deliberate state
rather than an oversight.

> empty: no measured inventory records an exact expected value; see deploy/seed/golden_metrics.py for the criteria to add one

A test suite consuming this manifest therefore asserts no metric
values. That is a visible gap; a populated-but-guessed set would be a
silent wrong answer. See `deploy/seed/golden_metrics.py` for the
criteria an entry must meet before it is added.

## Capabilities

| capability | value |
|---|---|
| `idp` | fakeidp |
| `ingestion` | no |
| `service_principals` | yes |

`ingestion: no` — compose seeds the silver and gold layers directly; no
connector runs, so the ingestion path is not exercised on this stand.

`service_principals: yes` — the authenticator's token listener is published,
so a runner can exchange an RFC 7523 assertion for a service principal and
exercise the `/internal/*` routes only a service may call. A stand that
keeps that listener in-cluster reports `no`, and those tests skip with a
reason rather than failing.

`idp` reflects the environment the seed was run with. This page is
rendered against a canonical environment, so it shows the default rather
than your stand's value — read `manifest.json` for what is actually
running.
