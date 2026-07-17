"""
Demo persons + team profiles.

The 25-person organisation that the seed script populates: one CEO
above 4 team leads (development, sales, HR, support), each with 5 ICs.
The development-team lead's email is `VITE_DEV_USER_EMAIL` (the
dev-impersonation user); the other 24 persons get deterministic
`email_<team>_<NN>@company.nonpresent` addresses.

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
DEV_LEAD_UUID = "00000000-0000-0000-0000-000000000010"

CEO_UUID = "aaaaaaaa-0000-0000-0000-000000000001"
SALES_LEAD_UUID = "aaaaaaaa-0000-0000-0000-000000000020"
HR_LEAD_UUID = "aaaaaaaa-0000-0000-0000-000000000030"
SUPPORT_LEAD_UUID = "aaaaaaaa-0000-0000-0000-000000000040"

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
    team: str | None  # None for CEO
    role: str         # "ceo" | "lead" | "ic"
    parent_uuid: str | None  # report-to chain
    first_name: str = ""     # assigned deterministically in build_roster
    last_name: str = ""

    @property
    def display_name(self) -> str:
        return f"{self.first_name} {self.last_name}".strip()


# Deterministic human names, assigned by roster build-order index so re-runs
# are stable. Seeded into MariaDB `identity.persons` (value_type=display_name/
# first_name/last_name) so the UI shows names instead of falling back to email.
_FIRST_NAMES = [
    "Ava", "Liam", "Maya", "Noah", "Zoe", "Ethan", "Aria", "Leo", "Nora",
    "Kai", "Ivy", "Owen", "Mila", "Ezra", "Luna", "Finn", "Ruby", "Milo",
    "Sage", "Cole", "Iris", "Jude", "Elle", "Reid", "Wren", "Beau", "Faye",
    "Cruz", "Tess", "Rhys",
]
_LAST_NAMES = [
    "Carter", "Nguyen", "Patel", "Rivera", "Brooks", "Okafor", "Meyer",
    "Sato", "Flores", "Haas", "Kelly", "Novak", "Reyes", "Park", "Bauer",
    "Costa", "Lund", "Amari", "Dixon", "Frost", "Grant", "Hale", "Ivers",
    "Jansen", "Keir", "Lowe", "Mora", "Nash", "Okeefe", "Pratt",
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
    "development": TeamProfile(name="development", weights={
        "github":      1.5,   # heavy
        "jira":        0.8,
        "slack":       0.8,
        "m365":        0.6,
        "zoom":        0.6,
        "gmail":       0.4,
        "bamboohr":    0.6,
        "cursor":      1.2,
        "claude_team": 1.0,
        "chatgpt":     0.6,
    }),
    "sales": TeamProfile(name="sales", weights={
        "hubspot":    1.5,
        "salesforce": 1.0,
        "slack":      0.8,
        "m365":       1.0,
        "zoom":       1.2,
        "gmail":      1.2,
        "bamboohr":   0.4,
        "chatgpt":    0.6,
        "jira":       0.3,
    }),
    "hr": TeamProfile(name="hr", weights={
        "slack":    0.6,
        "m365":     0.8,
        "zoom":     0.6,
        "gmail":    0.8,
        "bamboohr": 1.5,
        "jira":     0.5,
        "chatgpt":  0.4,
    }),
    "support": TeamProfile(name="support", weights={
        "slack":               1.2,
        "m365":                0.8,
        "zoom":                0.5,
        "gmail":               0.8,
        "bamboohr":            0.4,
        "jira":                1.3,
        # No Zendesk connector in the repo — support rows use this
        # placeholder data_source so the per-team distinction is visible.
        "zendesk-placeholder": 1.5,
        "chatgpt":             0.5,
        "claude_team":         0.6,
    }),
}

COMPANY_EMAIL_SUFFIX = "company.nonpresent"

def build_email(person: str) -> str:
    return f"{person}@{COMPANY_EMAIL_SUFFIX}".lower()

def build_roster(dev_user_email: str) -> list[Person]:
    """Construct the 25-person roster anchored on `dev_user_email`."""
    if not dev_user_email:
        raise ValueError("VITE_DEV_USER_EMAIL is required to build the roster.")

    ceo = Person(
        uuid=CEO_UUID,
        email=build_email("email_ceo"),
        team=None,
        role="ceo",
        parent_uuid=None,
    )

    leads: list[Person] = [
        Person(uuid=DEV_LEAD_UUID, email=dev_user_email,
               team="development", role="lead", parent_uuid=CEO_UUID),
        Person(uuid=SALES_LEAD_UUID, email=build_email("email_sales_lead"),
               team="sales", role="lead", parent_uuid=CEO_UUID),
        Person(uuid=HR_LEAD_UUID, email=build_email("email_hr_lead"),
               team="hr", role="lead", parent_uuid=CEO_UUID),
        Person(uuid=SUPPORT_LEAD_UUID, email=build_email("email_support_lead"),
               team="support", role="lead", parent_uuid=CEO_UUID),
    ]

    ics: list[Person] = []
    for lead in leads:
        assert lead.team is not None
        tid = _TEAM_INDEX[lead.team]
        for n in range(1, 6):
            ics.append(Person(
                uuid=_ic_uuid(tid, n),
                email=build_email(f"email_{lead.team}_{n:02d}"),
                team=lead.team,
                role="ic",
                parent_uuid=lead.uuid,
            ))

    # Assign deterministic names by build-order index (CEO, leads, then ICs).
    return [
        replace(p, first_name=fn, last_name=ln)
        for i, p in enumerate([ceo, *leads, *ics])
        for fn, ln in [_name_at(i)]
    ]


def get_dev_user_email() -> str:
    """Resolve the dev user's email, honouring VITE_DEV_USER_EMAIL."""
    val = os.environ.get("VITE_DEV_USER_EMAIL", "").strip().lower()
    if not val:
        raise SystemExit(
            "ERROR: VITE_DEV_USER_EMAIL must be set in the seed environment.\n"
            "       It anchors the development team lead in the demo roster."
        )
    return val
