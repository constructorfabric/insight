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

# No module-level vector: this module is mixed. The gate journey is `security`
# and the rest are `reliability`, and a module default plus a per-test marker
# would leave both on the item and break `-m` selection.


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
@pytest.mark.reliability
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


@pytest.mark.requires_seed("admin_operator")
@pytest.mark.stand_smoke
@pytest.mark.reliability
def test_the_console_renders_the_figures_the_service_reports(
    page: Page,
    base_url: str,
    session_for: Callable[[str], PersonaSession],
) -> None:
    """The identity console is on screen after a deploy, with real figures.

    Marked `stand_smoke`: the post-deploy gate proves a seeded persona can
    read their org chart over HTTP, and that the shipped SPA renders the
    product's own screens — but nothing proved the ADMIN surface renders at
    all. A console that answers every API call and paints an empty shell
    passes the rest of the gate.

    Read-only by design. The gate runs against real deployed stands, and a
    correction is an append-only journal row: a smoke that wrote one would
    leave a decision behind on every deploy.

    The expectation comes from the service at run time, not from the manifest
    and not from a number written here — the point is that the figure on
    screen is the one the service reports, which no API test can show.
    """
    operator = session_for("admin_operator")
    queue = operator.client.get(identity_path("/v1/resolution/attention"))
    assert queue.status_code == 200, f"{queue.status_code} {queue.text[:300]}"
    observed = queue.json()["rates"]["observed"]
    assert observed > 0, "a seeded stand reports no observed accounts — nothing to render"

    sign_in(page, base_url, operator)

    console = IdentitiesView(page)
    console.go("queue")
    expect(console.heading()).to_be_visible()
    expect(console.rate_tile("Accounts")).to_contain_text(str(observed))


@pytest.mark.requires_seed("dev_lead")
@pytest.mark.security
def test_a_pasted_console_url_refuses_a_caller_without_the_admin_role(
    page: Page,
    base_url: str,
    session_for: Callable[[str], PersonaSession],
) -> None:
    """The nav hides the surface; the URL does not, so the screen refuses.

    #2486 AC-5 in the browser. The API half is swept over all 22 admin-gated
    operations in `tests/stand/api/identity/test_request_contracts.py`, and
    the frontend suite has its own gate cases — but those mock the client, so
    what they prove is that the component reacts to a mocked answer. Nothing
    proved that a real non-admin session, on a real deployed stand, is refused
    the screen.

    Both halves are asserted. The gate fails CLOSED while the role check is in
    flight, so the refusal alone would also be satisfied by a spinner that
    never resolved — the console's own figures must be absent as well.
    """
    lead = session_for("dev_lead")
    sign_in(page, base_url, lead)

    console = IdentitiesView(page)
    console.go("queue")

    expect(console.refusal()).to_be_visible()
    expect(page.get_by_text("Needs attention", exact=True)).not_to_be_visible()


@pytest.mark.requires_seed("admin_operator", "dev_lead")
@pytest.mark.reliability
def test_a_persons_window_lists_the_accounts_the_service_gives_them(
    page: Page,
    base_url: str,
    session_for: Callable[[str], PersonaSession],
    stand_manifest: Manifest,
) -> None:
    """#2486 AC-3. The console's other read path, through the browser.

    The accounts journey enters from an account and asks who holds it; this
    enters from a person and asks what they hold — a different endpoint
    (`/v1/resolution/persons/{id}/accounts`) behind a different mode, and the
    one an operator actually starts from when the question is "this person's
    work looks split".

    Every account expected here is read from the service at run time. Opened
    by deep link, so the window carries the person id rather than their name —
    search resolves values, not ids, and there is no card to name them on
    arrival.
    """
    operator = session_for("admin_operator")
    person = stand_manifest.fixture("dev_lead")

    owned = operator.client.get(identity_path(f"/v1/resolution/persons/{person.uuid}/accounts"))
    assert owned.status_code == 200, f"{owned.status_code} {owned.text[:300]}"
    accounts = [a["account_id"] for a in owned.json()["accounts"]]
    assert accounts, f"the service gives {person.display_name} no accounts — nothing to render"

    sign_in(page, base_url, operator)

    console = IdentitiesView(page)
    console.go("person", person=person.uuid)

    window = console.person_window(person.uuid)
    expect(window).to_be_visible()
    for account_id in accounts:
        expect(window).to_contain_text(account_id)
