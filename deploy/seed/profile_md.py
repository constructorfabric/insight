"""Render `deploy/seed/PROFILE.md` from a manifest document.

PROFILE.md is the human-readable companion to `manifest.json`: it tells a
person (or an agent planning a test) what a seeded stand actually contains —
who the personas are, which fixtures exist, what the data window is, and
which metrics carry known expected values.

It is GENERATED. `render_profile` is a pure function of the manifest, so the
page can never drift from the machine-readable document it describes, and a
staleness check is a byte comparison.

This module does not import `os`: it is structurally incapable of picking up
ambient state, which is what keeps the committed page a function of committed
bytes rather than of whoever last ran the generator.
"""

from __future__ import annotations

from pathlib import Path
from typing import Any

REGEN_COMMAND = "python3 deploy/seed/render_profile.py"
CHECK_COMMAND = "python3 deploy/seed/render_profile.py --check"


def profile_path() -> Path:
    return Path(__file__).resolve().parent / "PROFILE.md"


def _table(headers: list[str], rows: list[list[str]]) -> list[str]:
    out = ["| " + " | ".join(headers) + " |", "|" + "|".join(["---"] * len(headers)) + "|"]
    out += ["| " + " | ".join(r) + " |" for r in rows]
    return out


def _cell(value: Any) -> str:
    if value is None:
        return "—"
    if isinstance(value, bool):
        return "yes" if value else "no"
    if isinstance(value, list):
        return ", ".join(str(v) for v in value) if value else "—"
    return str(value)


def render_profile(doc: dict[str, Any]) -> str:
    """Render the page. Pure function of `doc`; no clock, no env, no I/O."""
    lines: list[str] = [
        "<!-- GENERATED FILE — do not hand-edit.",
        f"     Regenerate: {REGEN_COMMAND}",
        f"     Verify:     {CHECK_COMMAND}",
        "     Content is derived from deploy/seed/manifest.py + profiles.py. -->",
        "",
        "# Seed Profile",
        "",
        "What a stand seeded by `deploy/seed` contains. Generated from the same",
        "builder that writes `manifest.json`, so the two cannot disagree.",
        "",
        "## Stand summary",
        "",
    ]

    lines += _table(
        ["Field", "Value"],
        [
            ["tenant", f"`{_cell(doc['tenant'])}`"],
            ["tenant (other)", f"`{_cell((doc.get('tenants') or {}).get('other'))}`"],
            ["realm", f"`{_cell(doc['realm']['name'])}`"],
            ["anchor_date", f"`{_cell(doc['anchor_date'])}`"],
            ["data_window", f"`{_cell(doc['data_window'])}`"],
            ["seed_revision", f"`{_cell(doc['seed_revision'])}`"],
            ["manifest_version", _cell(doc["manifest_version"])],
        ],
    )
    lines += [
        "",
        "`anchor_date` is the last day carrying seeded activity. It is resolved",
        "once per run from `SEED_ANCHOR_DATE` (an ISO date, or the literal",
        "`today`) and defaults to yesterday UTC — the current day is excluded so",
        "a partial day does not fight the gold views' day-aligned aggregates.",
        "Pin it to reproduce a dataset exactly; the value above is the one this",
        "page was rendered against, not necessarily the one on your stand.",
        "",
        "## Roster",
        "",
        f"{len(doc['personas'])} people, all but one in the default tenant. The",
        "exception is `other_tenant_lead`, who exists ONLY so cross-tenant refusal",
        "has a caller to refuse — no team, no org-chart edge, no activity, so they",
        "cannot appear in another persona's subtree or move a metric.",
        "",
        "`uuid` is both the Keycloak user id and the",
        "`identity.persons` person id, so a login and an API row refer to the same",
        "person.",
        "",
    ]

    lines += _table(
        ["email", "display_name", "team", "role", "realm roles", "uuid"],
        [
            [
                f"`{p['email']}`",
                _cell(p["display_name"]),
                _cell(p["team"]),
                _cell(p["role"]),
                _cell(p["realm_roles"]),
                f"`{p['uuid']}`",
            ]
            for p in doc["personas"]
        ],
    )

    lines += [
        "",
        "No password appears here or in `manifest.json`. Personas are referenced",
        "by identity; the shared local login secret lives in the compose env and",
        "the generated Keycloak realm.",
        "",
        "## Fixtures",
        "",
        "Stable names a test declares its data requirements against. The names are",
        "a contract — they describe a role in the org, not a particular person — so",
        "renaming one breaks every test that declares it.",
        "",
    ]

    lines += _table(
        ["fixture", "email", "team", "role", "uuid"],
        [
            [
                f"`{name}`",
                f"`{f['email']}`",
                _cell(f["team"]),
                _cell(f["role"]),
                f"`{f['uuid']}`",
            ]
            for name, f in sorted(doc["fixtures"].items())
        ],
    )

    catalogue = doc.get("catalogue") or {}
    lines += [
        "",
        "## Catalogue rows",
        "",
        "Rows the product provisions by operator or migration, so no endpoint",
        "creates them and no test fixture can either — the suite holds no",
        "database connection. Seeded by `deploy/seed/analytics.py` and named",
        "here so a test reads the name rather than hardcoding one.",
        "",
    ]
    override = catalogue.get("definition_override")
    if override:
        lines += [
            f"`metric_definitions` — `{override['metric_key']}` carries the tenant label",
            f"`{override['label']}`, so a listing that served the product default instead",
            "is visible on sight.",
        ]
    else:
        lines += [
            "**No tenant `metric_definitions` override.** Nothing proves the listing",
            "resolves a tenant's label over the product default.",
        ]

    lines += ["", "## Populated / golden metrics", ""]
    if doc["golden_metrics"]:
        lines += _table(
            ["metric_key", "scope", "window", "expected", "derivation"],
            [
                [
                    f"`{g['metric_key']}`",
                    _cell(g.get("scope")),
                    _cell(g.get("window")),
                    _cell(g.get("expected")),
                    _cell(g.get("derivation")),
                ]
                for g in doc["golden_metrics"]
            ],
        )
    else:
        lines += [
            "**None.** The golden set is empty, and that is a deliberate state",
            "rather than an oversight.",
            "",
            f"> {doc.get('golden_metrics_note', '')}",
            "",
            "A test suite consuming this manifest therefore asserts no metric",
            "values. That is a visible gap; a populated-but-guessed set would be a",
            "silent wrong answer. See `deploy/seed/golden_metrics.py` for the",
            "criteria an entry must meet before it is added.",
        ]

    lines += [
        "",
        "## Capabilities",
        "",
    ]
    lines += _table(
        ["capability", "value"],
        [[f"`{k}`", _cell(v)] for k, v in sorted(doc["capabilities"].items())],
    )
    lines += [
        "",
        "`ingestion: no` — compose seeds the silver and gold layers directly; no",
        "connector runs, so the ingestion path is not exercised on this stand.",
        "",
        "`service_principals: yes` — the authenticator's token listener is published,",
        "so a runner can exchange an RFC 7523 assertion for a service principal and",
        "exercise the `/internal/*` routes only a service may call. A stand that",
        "keeps that listener in-cluster reports `no`, and those tests skip with a",
        "reason rather than failing.",
        "",
        "`idp` reflects the environment the seed was run with. This page is",
        "rendered against a canonical environment, so it shows the default rather",
        "than your stand's value — read `manifest.json` for what is actually",
        "running.",
        "",
    ]

    return "\n".join(lines)


def write_profile(doc: dict[str, Any], path: Path | None = None) -> Path:
    target = path or profile_path()
    target.write_text(render_profile(doc), encoding="utf-8", newline="\n")
    return target
