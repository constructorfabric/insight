"""
Demo persons + team profiles.

The 25-person organisation that the seed script populates: one CEO
above 4 team leads (development, sales, HR, support), each with 5 ICs.
The development-team lead's email is `DEV_USER_EMAIL`; the other 24
persons get deterministic `email_<team>_<NN>@company.nonpresent`
addresses.

A 26th person, the ADMIN OPERATOR, is in the roster but not in the
organisation: no team, no place in the org chart, no activity. It exists
so the admin-gated API surface has a caller, and it is kept outside the
org so that granting it cannot move a single metric or change what any
other person can see. See `build_roster`.

`TEAM_PROFILES` below maps a per-team source-type to a numeric
multiplier (0 = no rows; 1 = baseline; >1 = heavier). The row
generators consult these weights to decide which silver rows a given
person produces and at what volume.
"""

from __future__ import annotations

import os
from dataclasses import dataclass, field, replace

# ─── Fixed UUIDs ────────────────────────────────────────────────────────
# The dev lead's UUID matches the value the original dev-compose.sh seed
# inserts, so re-runs across both scripts converge on the same row.
# Default tenant for the demo organisation. Mirrors TENANT_DEFAULT_ID in
# docker-compose.yml and deploy/compose/keycloak/gen-realm.py.
TENANT_DEFAULT = "00000000-df51-5b42-9538-d2b56b7ee953"

# A SECOND tenant, holding exactly one person and nothing else.
#
# Cross-tenant refusal is the one authorization property a single-tenant stand
# cannot show at all: with one tenant there is no caller who should be refused,
# so a service that ignored `tenant_id` entirely would pass every test. One
# person is all it takes — the assertion is that they are refused, which needs
# no data of their own.
#
# Deliberately NOT part of the organisation: no org-chart edge, no team, and no
# activity, so they cannot appear in another persona's subtree or move a metric.
# The same reasoning as the admin operator, one tenant over.
TENANT_OTHER = "11111111-1111-4111-8111-111111111111"
OTHER_TENANT_PERSON_UUID = "dddddddd-0000-0000-0000-000000000001"

DEV_LEAD_UUID = "00000000-0000-0000-0000-000000000010"

CEO_UUID = "aaaaaaaa-0000-0000-0000-000000000001"
SALES_LEAD_UUID = "aaaaaaaa-0000-0000-0000-000000000020"
HR_LEAD_UUID = "aaaaaaaa-0000-0000-0000-000000000030"
SUPPORT_LEAD_UUID = "aaaaaaaa-0000-0000-0000-000000000040"

# The admin operator (see `ADMIN_ROLE_NAME`). Its own `cccccccc-` block rather
# than an `aaaaaaaa-` lead slot, because it is not a member of the organisation.
ADMIN_OPERATOR_UUID = "cccccccc-0000-0000-0000-000000000001"

# Name of the `identity.roles` row the operator is granted. The row itself is
# created by the identity-resolution migrations, not by this seed.
ADMIN_ROLE_NAME = "admin"

# Author for every dev-seed observation (Guid.Empty == "system").
AUTHOR_PERSON_UUID = "00000000-0000-0000-0000-000000000000"

# Fixed insight_source_id used by every dev-seed observation, org-chart
# edge, and account_person_map row. Matches what the original
# dev-compose.sh seed used so the persons unique-key absorbs both.
DEV_SEED_SOURCE_ID = "00000000-0000-0000-0000-000000000001"
DEV_SEED_SOURCE_TYPE = "dev-seed"

# `org_chart` rows MUST use this source_type — the identity service's
# visibility CTE walks org_chart only where insight_source_type matches
# its configured `org_chart_source_type` (default 'bamboohr').
# See VisibilityService + Sql.Visibility.IsTargetInVisibleSet.
ORG_CHART_SOURCE_TYPE = "bamboohr"

_TEAM_INDEX: dict[str, int] = {"development": 1, "sales": 2, "hr": 3, "support": 4}


def _ic_uuid(team_id: int, n: int) -> str:
    """Build the IC UUID for the n-th IC on the given team."""
    return f"bbbbbbbb-0000-0000-0000-0000000{team_id}000{n}"


# ─── Person model ────────────────────────────────────────────────────────


@dataclass(frozen=True)
class Person:
    uuid: str
    email: str
    team: str | None  # None for the CEO and the admin operator
    role: str  # "ceo" | "lead" | "ic" | "admin"
    parent_uuid: str | None  # report-to chain; None = no org_chart edge
    first_name: str = ""  # assigned deterministically in build_roster
    last_name: str = ""

    @property
    def display_name(self) -> str:
        return f"{self.first_name} {self.last_name}".strip()


# Deterministic human names, assigned by roster build-order index so re-runs
# are stable. Seeded into MariaDB `identity.persons` (value_type=display_name/
# first_name/last_name) so the UI shows names instead of falling back to email.
_FIRST_NAMES = [
    "Ava",
    "Liam",
    "Maya",
    "Noah",
    "Zoe",
    "Ethan",
    "Aria",
    "Leo",
    "Nora",
    "Kai",
    "Ivy",
    "Owen",
    "Mila",
    "Ezra",
    "Luna",
    "Finn",
    "Ruby",
    "Milo",
    "Sage",
    "Cole",
    "Iris",
    "Jude",
    "Elle",
    "Reid",
    "Wren",
    "Beau",
    "Faye",
    "Cruz",
    "Tess",
    "Rhys",
]
_LAST_NAMES = [
    "Carter",
    "Nguyen",
    "Patel",
    "Rivera",
    "Brooks",
    "Okafor",
    "Meyer",
    "Sato",
    "Flores",
    "Haas",
    "Kelly",
    "Novak",
    "Reyes",
    "Park",
    "Bauer",
    "Costa",
    "Lund",
    "Amari",
    "Dixon",
    "Frost",
    "Grant",
    "Hale",
    "Ivers",
    "Jansen",
    "Keir",
    "Lowe",
    "Mora",
    "Nash",
    "Okeefe",
    "Pratt",
]


def _name_at(index: int) -> tuple[str, str]:
    return (
        _FIRST_NAMES[index % len(_FIRST_NAMES)],
        _LAST_NAMES[index % len(_LAST_NAMES)],
    )


# ─── Team profile ────────────────────────────────────────────────────────


@dataclass(frozen=True)
class TeamProfile:
    name: str
    # Per-source-type activity weight. 0 = no rows. 1 = baseline.
    # Generators use these as direct multipliers on per-day Poisson
    # means.
    weights: dict[str, float] = field(default_factory=dict)


TEAM_PROFILES: dict[str, TeamProfile] = {
    "development": TeamProfile(
        name="development",
        weights={
            "github": 1.5,  # heavy
            "jira": 0.8,
            "slack": 0.8,
            "m365": 0.6,
            "zoom": 0.6,
            "gmail": 0.4,
            "bamboohr": 0.6,
            "cursor": 1.2,
            "claude_team": 1.0,
            "chatgpt": 0.6,
        },
    ),
    "sales": TeamProfile(
        name="sales",
        weights={
            "hubspot": 1.5,
            "salesforce": 1.0,
            "slack": 0.8,
            "m365": 1.0,
            "zoom": 1.2,
            "gmail": 1.2,
            "bamboohr": 0.4,
            "chatgpt": 0.6,
            "jira": 0.3,
        },
    ),
    "hr": TeamProfile(
        name="hr",
        weights={
            "slack": 0.6,
            "m365": 0.8,
            "zoom": 0.6,
            "gmail": 0.8,
            "bamboohr": 1.5,
            "jira": 0.5,
            "chatgpt": 0.4,
        },
    ),
    "support": TeamProfile(
        name="support",
        weights={
            "slack": 1.2,
            "m365": 0.8,
            "zoom": 0.5,
            "gmail": 0.8,
            "bamboohr": 0.4,
            "jira": 1.3,
            # No Zendesk connector in the repo — support rows use this
            # placeholder data_source so the per-team distinction is visible.
            "zendesk-placeholder": 1.5,
            "chatgpt": 0.5,
            "claude_team": 0.6,
        },
    ),
}

COMPANY_EMAIL_SUFFIX = "company.nonpresent"


def build_email(person: str) -> str:
    return f"{person}@{COMPANY_EMAIL_SUFFIX}".lower()


def build_roster(dev_user_email: str) -> list[Person]:
    """The 25-person organisation anchored on `dev_user_email`, plus the operator."""
    if not dev_user_email:
        raise ValueError("DEV_USER_EMAIL is required to build the roster.")

    ceo = Person(
        uuid=CEO_UUID,
        email=build_email("email_ceo"),
        team=None,
        role="ceo",
        parent_uuid=None,
    )

    leads: list[Person] = [
        Person(
            uuid=DEV_LEAD_UUID,
            email=dev_user_email,
            team="development",
            role="lead",
            parent_uuid=CEO_UUID,
        ),
        Person(
            uuid=SALES_LEAD_UUID,
            email=build_email("email_sales_lead"),
            team="sales",
            role="lead",
            parent_uuid=CEO_UUID,
        ),
        Person(
            uuid=HR_LEAD_UUID,
            email=build_email("email_hr_lead"),
            team="hr",
            role="lead",
            parent_uuid=CEO_UUID,
        ),
        Person(
            uuid=SUPPORT_LEAD_UUID,
            email=build_email("email_support_lead"),
            team="support",
            role="lead",
            parent_uuid=CEO_UUID,
        ),
    ]

    ics: list[Person] = []
    for lead in leads:
        assert lead.team is not None
        tid = _TEAM_INDEX[lead.team]
        for n in range(1, 6):
            ics.append(
                Person(
                    uuid=_ic_uuid(tid, n),
                    email=build_email(f"email_{lead.team}_{n:02d}"),
                    team=lead.team,
                    role="ic",
                    parent_uuid=lead.uuid,
                )
            )

    # The admin operator: an account that ADMINISTERS the product rather than a
    # person the product measures. Deliberately outside the organisation, and
    # both fields below are load-bearing rather than incidental:
    #
    #   parent_uuid=None  `seed_org_chart` emits an edge only for a person with
    #                     a parent, so this produces NO org_chart row in either
    #                     direction — nobody reports to the operator and the
    #                     operator reports to nobody. Their own `/v1/subchart`
    #                     is an empty forest, and no other person's view moves.
    #   team=None         every activity generator skips a teamless person, so
    #                     the operator contributes no silver rows and cannot
    #                     shift a metric.
    #
    # Together those keep the operator invisible to every existing assertion.
    # The CEO is teamless too, but IS in the org chart as a parent; this one is
    # absent from it entirely.
    admin_operator = Person(
        uuid=ADMIN_OPERATOR_UUID,
        email=build_email("email_admin_operator"),
        team=None,
        role="admin",
        parent_uuid=None,
    )

    # Assign deterministic names by build-order index (CEO, leads, ICs, then the
    # operator). The operator goes LAST so adding it cannot renumber anybody:
    # the index drives `_name_at`, and inserting it earlier would rename all 25
    # existing people and churn every display name in the seeded data.
    return [
        replace(p, first_name=fn, last_name=ln)
        for i, p in enumerate([ceo, *leads, *ics, admin_operator])
        for fn, ln in [_name_at(i)]
    ]


def build_other_tenant_roster() -> list[Person]:
    """The second tenant's entire population: one lead, alone.

    Returned as its own roster rather than appended to `build_roster` so the
    demo organisation is untouched — the names in that list are assigned by
    build-order index, and anything inserted into it renames people and churns
    every display name in the seeded data.

    Their name is fixed rather than drawn from `_name_at`, for the same reason.
    """
    return [
        Person(
            uuid=OTHER_TENANT_PERSON_UUID,
            email=build_email("email_other_tenant_lead"),
            team=None,
            role="lead",
            parent_uuid=None,
            first_name="Vera",
            last_name="Kovac",
        )
    ]


def get_dev_user_email() -> str:
    """Resolve the dev user's email, honouring DEV_USER_EMAIL."""
    val = os.environ.get("DEV_USER_EMAIL", "").strip().lower()
    if not val:
        raise SystemExit(
            "ERROR: DEV_USER_EMAIL must be set in the seed environment.\n"
            "       It anchors the development team lead in the demo roster."
        )
    return val


# fakeidp's users.yaml pins its first user's `sub` to "fakeidp|dev" (stable
# regardless of FAKEIDP_DEV_USER_EMAIL overriding the email) — that's the only
# fakeidp identity a fresh dev/demo/CI stack ever logs in as.
_FAKEIDP_DEV_LEAD_EXTERNAL_ID = "fakeidp|dev"


def get_login_id_pairs(roster: list[Person]) -> list[tuple[str, str]]:
    """Resolve the `(person_uuid, external_id)` pairs to seed as
    `value_type='id'` login-bootstrap observations, for the ACTIVE login IdP.

    dev-compose.sh's `--auth` flag (`AUTH_MODE`, forwarded into this
    container's environment) selects which IdP fixture applies, and the two
    are NOT symmetric in how many personas can actually log in:
    - keycloak: gen-realm.py sets EVERY realm user's Keycloak `id` to that
      person's OWN roster UUID (`"id": person.uuid`), and Keycloak issues
      `sub` equal to the user's internal id verbatim — so every seeded
      persona's external id IS their own roster uuid, and the whole roster
      can log in (matching the realm, which seeds all of them).
    - fakeidp (default): its users.yaml only pins a handful of FIXED test
      identities unrelated to the demo roster (dev/alice/bob/carol — see
      services/fakeidp/users.yaml), of which only the first ("fakeidp|dev")
      corresponds to a roster member (the dev lead, anchored via
      DEV_USER_EMAIL). Only that one persona can log in.
    Getting this wrong means the login-bootstrap 403s: the seeded
    `value_type='id'` row would carry a value the id_token never presents.
    """
    mode = os.environ.get("AUTH_MODE", "fakeidp").strip().lower()
    if mode == "keycloak":
        return [(p.uuid, p.uuid) for p in roster]
    return [(DEV_LEAD_UUID, _FAKEIDP_DEV_LEAD_EXTERNAL_ID)]


def get_idp_source_type() -> str:
    """Resolve the login IdP's identity-resolution source_type, honouring
    IDP_SOURCE_TYPE — MUST match the authenticator's `idp.source_type`, or the
    dev-lead's seeded value_type='id' row won't be the one the login-bootstrap
    lookup finds."""
    return os.environ.get("IDP_SOURCE_TYPE", "fakeidp").strip() or "fakeidp"
