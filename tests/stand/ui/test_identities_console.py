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

**The account is the journey's own, under the suite's scratch connector.**
`scratch.py` rule 5: the correction journal cannot be deleted from, so a
decision written under a seeded connector blocks the next seed of the stand.
A journey that reassigned a real employee's account would leave exactly that
— which is why the account here is created by this test, through the API,
under `SCRATCH_SOURCE_TYPE` / `SCRATCH_SOURCE_ID`, and ends in `exclude`, the
one end state the seed preflight exempts.

The console reaches it by its `?acct=` deep link rather than through the
account search, which is what makes this possible at all: the search lists
accounts a connector observed, while the account window reads one by its
triple whether or not any connector ever reported it.

No metric value is asserted. Every expectation is an identity fact read at
runtime — the manifest's people, and the account the service itself reports.
"""

from __future__ import annotations

from collections.abc import Callable

import pytest
from insight_stand import (
    SCRATCH_SOURCE_ID,
    SCRATCH_SOURCE_TYPE,
    Manifest,
    PersonaSession,
    identity_path,
    scratch_name,
)
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


def _correct(session: PersonaSession, verb: str, body: dict[str, object]) -> None:
    """A correction over HTTP — setup and teardown, never the thing under test."""
    response = session.client.post(identity_path(f"/v1/resolution/{verb}"), json_body=body)
    assert response.status_code == 200, f"{verb}: {response.status_code} {response.text[:300]}"


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

    account_id = scratch_name("console")
    account = {"source": SCRATCH_SOURCE_TYPE, "source_id": SCRATCH_SOURCE_ID, "id": account_id}

    # Pre-registration: `bind` is the one verb that accepts an account no
    # connector reported, which is what gives this journey a subject of its
    # own instead of a seeded person's.
    _correct(
        operator,
        "bind",
        {
            "bindings": [{"account": account, "person_id": holder.uuid}],
            "comment": "stand ui journey: the account this test decides about",
        },
    )

    try:
        sign_in(page, base_url, operator)

        console = IdentitiesView(page)
        console.go("accounts", acct=account_key(SCRATCH_SOURCE_TYPE, SCRATCH_SOURCE_ID, account_id))
        window = console.account_window(account_id)
        expect(window).to_be_visible()
        # By person id, not by name. The window renders a person CARD only
        # where it can hydrate one, and an account no connector observed has
        # nothing to hydrate from — so it falls back to the raw id. That is
        # the stricter oracle anyway: two people can share a display name.
        expect(console.current_binding(window)).to_contain_text(holder.uuid)

        console.person_search().fill(new_holder.display_name)
        console.person_option(new_holder.display_name).click()
        expect(console.confirmation("Bind this account?")).to_be_visible()
        console.confirm("Bind").click()

        # The service's answer, not the console's optimism: reload and read
        # the screen again from whatever the store now holds.
        page.reload()
        window = console.account_window(account_id)
        binding = console.current_binding(window)
        expect(binding).to_contain_text(new_holder.uuid)
        expect(binding).not_to_contain_text(holder.uuid)

        owned = operator.client.get(
            identity_path(f"/v1/resolution/persons/{new_holder.uuid}/accounts")
        )
        assert owned.status_code == 200, f"{owned.status_code} {owned.text[:300]}"
        assert account_id in [a["account_id"] for a in owned.json()["accounts"]], (
            "the screen shows the new holder but the service does not list the "
            "account under them"
        )
    finally:
        _correct(
            operator,
            "exclude",
            {"account": account, "comment": "stand ui journey: scratch cleanup"},
        )

    owned = operator.client.get(
        identity_path(f"/v1/resolution/persons/{new_holder.uuid}/accounts")
    )
    assert account_id not in [a["account_id"] for a in owned.json()["accounts"]], (
        "the excluded scratch account still lists under the person — the stand is dirty"
    )
