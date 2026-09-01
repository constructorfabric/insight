"""Journey — an operator reassigns an account in Manage → Identities.

#2486 scenario 5. Why this is a browser test and not an API test: the four
correction verbs are already proven over HTTP in `tests/stand/api/identity/`
and in the identity rig, so nothing here is trying to re-prove that the
service binds an account. What no API call can show is that a decision an
operator makes THROUGH THE SCREEN reaches the service and comes back — and
the frontend's own suite cannot show it either, because every one of its
cases mocks the identity client, which makes "the change survives a refresh"
unprovable by construction: a reload there restarts the mock.

So this journey is the only place where the console, the gateway, the service
and the store are the real ones at the same time.

**The account is reassigned and put back, not attached from nothing.** A
seeded stand has no unbound account to attach — every observed account
belongs to the employee it was reported for — and staging one would mean
changing seed data the whole suite reads. Reassignment is the same verb
through the same screen, and it is the only correction whose undo restores
the visible state exactly: the account returns to the holder it had. What
remains is journal rows, which is what every operator decision leaves behind
by design (`tests/stand/api/identity/test_resolution.py` says the same about
its own round trip).

No metric value is asserted. Every expectation is an identity fact read at
runtime — the manifest's people, and the account the service itself reports.
"""

from __future__ import annotations

from collections.abc import Callable

import pytest
from insight_stand import Manifest, PersonaSession, identity_path
from playwright.sync_api import BrowserContext, Page, expect

from .flows import sign_in
from .pages.identities_view import IdentitiesView, account_key

pytestmark = pytest.mark.reliability


@pytest.fixture
def context(context: BrowserContext) -> BrowserContext:
    """The first journey here that drives the PORTAL rather than the legacy shell.

    `conftest.py` writes `insight.legacyShell` into every context, because the
    journeys written before this one were written against that shell. The
    identities console exists only in the portal, so this one takes the hatch
    back out. Init scripts run in the order they were added, and both run
    before any app code, so removing the key after the shared fixture set it
    is the whole override — the surrounding fixture keeps working for every
    other module unchanged.
    """
    context.add_init_script("window.localStorage.removeItem('insight.legacyShell')")
    return context


def _bind(session: PersonaSession, account: dict[str, str], person_id: str, why: str) -> None:
    """Put a binding back, over HTTP — teardown, never the thing under test."""
    response = session.client.post(
        identity_path("/v1/resolution/bind"),
        json_body={
            "bindings": [{"account": account, "person_id": person_id}],
            "comment": f"stand ui journey: {why}",
        },
    )
    assert response.status_code == 200, f"restore: {response.status_code} {response.text[:300]}"


@pytest.mark.requires_seed("admin_operator", "dev_lead", "development_ic")
def test_an_operator_reassigns_an_account_and_the_change_outlives_a_reload(
    page: Page,
    base_url: str,
    session_for: Callable[[str], PersonaSession],
    stand_manifest: Manifest,
) -> None:
    """A correction made on screen is the service's answer afterwards.

    The reload is the assertion that matters. Without it the test would pass
    on a console that renders its own optimistic result and never sent
    anything — the exact failure the mocked frontend suite cannot see.
    """
    operator = session_for("admin_operator")
    holder = stand_manifest.fixture("dev_lead")
    new_holder = stand_manifest.fixture("development_ic")

    # The account to move, and who has it, read from the service rather than
    # named here: which connector reports the roster is the seed's business,
    # and a journey that hardcoded one would fail as a broken test the day
    # that changed.
    found = operator.client.get(
        identity_path("/v1/resolution/accounts"), params={"q": holder.email, "limit": 1}
    )
    assert found.status_code == 200, f"{found.status_code} {found.text[:300]}"
    matches = found.json()["items"]
    assert matches, f"no observed account carries {holder.email} — is the stand seeded?"

    match = matches[0]
    account = {
        "source": match["source"],
        "source_id": match["source_id"],
        "id": match["account_id"],
    }
    assert match["person"] and match["person"]["person_id"] == holder.uuid, (
        f"the account for {holder.email} is held by {match['person']}, not by the "
        "person the manifest names — the stand is not in its seeded state"
    )

    sign_in(page, base_url, operator)

    label = match["email"] or match["username"] or match["account_id"]

    console = IdentitiesView(page)
    console.go(
        "accounts",
        acct=account_key(match["source"], match["source_id"], match["account_id"]),
    )
    window = console.account_window(label)
    expect(window).to_be_visible()
    expect(console.current_binding(window)).to_contain_text(holder.display_name)

    try:
        console.person_search().fill(new_holder.display_name)
        console.person_option(new_holder.display_name).click()
        expect(console.confirmation("Bind this account?")).to_be_visible()
        console.confirm("Bind").click()

        # The service's answer, not the console's optimism: reload and read
        # the screen again from whatever the store now holds.
        page.reload()
        window = console.account_window(label)
        binding = console.current_binding(window)
        expect(binding).to_contain_text(new_holder.display_name)
        expect(binding).not_to_contain_text(holder.display_name)

        owned = operator.client.get(
            identity_path(f"/v1/resolution/persons/{new_holder.uuid}/accounts")
        )
        assert owned.status_code == 200, f"{owned.status_code} {owned.text[:300]}"
        assert account["id"] in [a["account_id"] for a in owned.json()["accounts"]], (
            "the screen shows the new holder but the service does not list the "
            "account under them"
        )
    finally:
        _bind(operator, account, holder.uuid, "return the account to its seeded holder")

    restored = operator.client.get(
        identity_path(f"/v1/resolution/persons/{holder.uuid}/accounts")
    )
    assert account["id"] in [a["account_id"] for a in restored.json()["accounts"]], (
        "the account did not return to its seeded holder — the stand is dirty"
    )
