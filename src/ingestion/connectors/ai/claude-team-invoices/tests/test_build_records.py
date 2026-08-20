"""Degradation rules of a run, exercised without a socket.

`build_records` is handed a `fetch_lines` that returns or raises; every rule
about what reaches bronze when a hop fails is decided here, so none of it needs
a network, a cluster or the CDK to verify.
"""

import base64
import logging
from collections.abc import Mapping, Sequence
from typing import Any

import pytest
from source_claude_team_invoices.stripe_chain import (
    CHAIN_FAILED,
    CHAIN_NO_URL,
    CHAIN_OK,
    CHAIN_UNPARSABLE,
    StripeChainError,
    UrlFormatDrift,
    build_records,
    unique_key_parts,
)
from tests.test_stripe_chain import EXTRA_USAGE_LINE, SUBSCRIPTION_LINE

EPHEMERAL_KEY = "ek_live_super_secret_value"


def _hosted(entity: str, nonce: str = "n1", acct: str = "acct_1ABC") -> tuple[str, str]:
    """A hosted URL shaped the way the vendor's are, plus its path segment.

    The segment is base64 of `<acct>,_<entity>,<rotating>`; the vendor regenerates
    that trailing part on every list call, so two calls for one invoice differ in
    the URL and still decode to the same identity.
    """
    payload = f"{acct},_{entity},{nonce}"
    segment = base64.urlsafe_b64encode(payload.encode()).decode().rstrip("=")
    return f"https://invoice.stripe.com/i/{acct}/live_{segment}?s=ap", f"live_{segment}"


GOOD_URL, _GOOD_TOKEN = _hosted("ent1ABC")
FAILING_URL, FAILING_TOKEN = _hosted("entFAILS")


def invoice(url: str | None = GOOD_URL, **over: Any) -> dict[str, Any]:
    base = {
        "hosted_invoice_url": url,
        "status": "paid",
        "created_ts": 1756771200,
        "currency": "usd",
        "total": 3300,
        "total_excluding_tax": 3000,
        "num_seats": 3,
        "payment_intent": "pi_1",
    }
    base.update(over)
    return base


def lines_ok(acct: str, token: str) -> tuple[str, Sequence[Mapping[str, Any]]]:
    return "in_1ABC", [SUBSCRIPTION_LINE, EXTRA_USAGE_LINE]


def lines_raise(acct: str, token: str) -> tuple[str, Sequence[Mapping[str, Any]]]:
    # The key would be in scope here in production; the message must not carry it.
    raise StripeChainError("bootstrap response carried no invoice_id/ephemeral_key")


def test_an_enriched_invoice_yields_its_own_row_and_one_per_line() -> None:
    records = list(build_records([invoice()], lines_ok))
    assert len(records) == 3
    assert {r["chain_status"] for r in records} == {CHAIN_OK}
    assert [r["seat_unit_amount"] for r in records] == [None, 1000, None]


def test_an_invoice_is_dated_by_the_window_its_lines_charge_for() -> None:
    """Raised at the period boundary, its own creation day sits in the month before."""
    own = next(iter(build_records([invoice()], lines_ok)))
    assert own["period_start_ts"] == 1754092800, "the earliest window any line covers"
    assert own["period_start_ts"] != own["invoice_created_ts"]


def test_an_invoice_with_no_lines_has_only_its_creation_day() -> None:
    own = next(iter(build_records([invoice()], lines_raise)))
    assert (own["period_start_ts"], own["period_end_ts"]) == (None, None)


def test_the_invoice_money_sits_on_the_invoice_row_alone() -> None:
    """Carried on each line instead, one invoice would count once per line."""
    own, *lines = list(build_records([invoice()], lines_ok))
    assert own["line_id"] is None
    assert (own["invoice_total_excluding_tax"], own["invoice_total"]) == (3000, 3300)
    assert own["invoice_num_seats"] == 3
    assert [r["invoice_total_excluding_tax"] for r in lines] == [None, None]
    assert [r["invoice_num_seats"] for r in lines] == [None, None]
    # What a line needs to place itself does ride along on it.
    assert all(r["invoice_created_ts"] == 1756771200 for r in [own, *lines])


def test_an_unparsable_url_keeps_the_money_and_marks_the_gap() -> None:
    # Alongside a healthy invoice: one bad URL out of a set is a data gap, while
    # a set that is entirely bad is drift — see the two boundary tests below.
    records = list(build_records([invoice(url="https://elsewhere.example/i/a/b"), invoice()], lines_ok))
    row = next(r for r in records if r["chain_status"] == CHAIN_UNPARSABLE)
    assert row["chain_status"] == CHAIN_UNPARSABLE
    assert row["invoice_total_excluding_tax"] == 3000, "the ledger survives"
    assert row["seat_unit_amount"] is None and row["line_id"] is None


def test_an_invoice_offering_no_url_is_a_state_of_its_own_not_a_format_change() -> None:
    """A draft invoice carries no hosted URL; calling that drift kills the sync."""
    records = list(build_records([invoice(url=None)], lines_ok))
    assert [r["chain_status"] for r in records] == [CHAIN_NO_URL]
    assert records[0]["invoice_total_excluding_tax"] == 3000, "the ledger survives"


def test_urls_the_vendor_never_offered_stay_out_of_the_drift_ratio() -> None:
    """Otherwise one draft invoice in a small set aborts the whole sync."""
    absent = [invoice(url=None) for _ in range(3)]
    records = list(build_records([*absent, invoice(url="bad"), invoice()], lines_ok))
    assert sum(1 for r in records if r["chain_status"] == CHAIN_NO_URL) == 3
    assert sum(1 for r in records if r["chain_status"] == CHAIN_UNPARSABLE) == 1


def test_a_lone_invoice_without_a_url_does_not_fail_the_run() -> None:
    assert [r["chain_status"] for r in build_records([invoice(url=None)], lines_ok)] == [CHAIN_NO_URL]


def test_a_failed_chain_does_not_stop_the_run(caplog: pytest.LogCaptureFixture) -> None:
    invoices = [invoice(payment_intent="pi_bad"), invoice(payment_intent="pi_good")]
    calls = {"n": 0}

    def flaky(acct, token):
        calls["n"] += 1
        if calls["n"] == 1:
            raise StripeChainError("stripe said no")
        return lines_ok(acct, token)

    with caplog.at_level(logging.WARNING):
        records = list(build_records(invoices, flaky))

    assert [r["chain_status"] for r in records] == [CHAIN_FAILED, CHAIN_OK, CHAIN_OK, CHAIN_OK]
    assert records[0]["invoice_total_excluding_tax"] == 3000
    assert "stripe chain failed" in caplog.text


def test_a_seat_price_of_an_unexpected_type_degrades_the_invoice_visibly(caplog: pytest.LogCaptureFixture) -> None:
    """Returning absence instead would put every seat price at NULL while the
    chain still read as complete — indistinguishable from a vendor pricing no seats."""

    def lines_with_a_string_price(acct, token):
        return "in_1ABC", [dict(SUBSCRIPTION_LINE, hosted_invoice_unit_amount="1000")]

    with caplog.at_level(logging.WARNING):
        records = list(build_records([invoice()], lines_with_a_string_price))

    assert [r["chain_status"] for r in records] == [CHAIN_FAILED]
    assert records[0]["invoice_total_excluding_tax"] == 3000, "the ledger survives"
    # The type it arrived as, by name — a `failed` row alone would send the
    # operator to the egress and Stripe-Version checks the remediation names.
    assert "hosted_invoice_unit_amount arrived as str" in caplog.text


def test_a_seat_line_the_vendor_left_unpriced_does_not_degrade_the_invoice() -> None:
    """Absence is a state this connector reports; only a changed type is a fault."""

    def lines_unpriced(acct, token):
        return "in_1ABC", [dict(SUBSCRIPTION_LINE, hosted_invoice_unit_amount=None)]

    records = list(build_records([invoice()], lines_unpriced))
    assert [r["chain_status"] for r in records] == [CHAIN_OK, CHAIN_OK]
    assert [r["seat_unit_amount"] for r in records] == [None, None]


def test_a_draft_is_not_emitted_at_all(caplog: pytest.LogCaptureFixture) -> None:
    """Its total can still change and it has no payment intent — two of the fields
    an invoice's row is keyed on — so it would duplicate on finalisation."""
    with caplog.at_level(logging.INFO):
        records = list(build_records([invoice(status="draft", payment_intent=None, url=None)], lines_ok))

    assert records == []
    assert "skipped 1 draft invoice(s)" in caplog.text


def test_a_draft_does_not_hide_the_invoices_beside_it() -> None:
    drafts = [invoice(status="draft", payment_intent=None, url=None) for _ in range(2)]
    records = list(build_records([*drafts, invoice()], lines_ok))
    assert [r["chain_status"] for r in records] == [CHAIN_OK, CHAIN_OK, CHAIN_OK]
    assert [r["line_id"] for r in records] == [None, "il_standard", "il_prepaid"]


def test_a_draft_stays_out_of_the_drift_ratio() -> None:
    """Counted in, two drafts beside one bad URL would tip a healthy run into drift."""
    drafts = [invoice(status="draft", url=None) for _ in range(2)]
    records = list(build_records([*drafts, invoice(url="bad"), invoice()], lines_ok))
    assert sum(1 for r in records if r["chain_status"] == CHAIN_UNPARSABLE) == 1
    assert sum(1 for r in records if r["chain_status"] == CHAIN_OK) == 3


def test_a_finalised_invoice_without_a_url_is_still_reported() -> None:
    """The status exists for this case now that drafts never reach it."""
    records = list(build_records([invoice(url=None)], lines_ok))
    assert [r["chain_status"] for r in records] == [CHAIN_NO_URL]


def test_a_run_of_unparsable_urls_fails_instead_of_writing_priceless_rows() -> None:
    invoices = [invoice(url="https://elsewhere.example/x") for _ in range(3)] + [invoice()]
    with pytest.raises(UrlFormatDrift) as raised:
        list(build_records(invoices, lines_ok))
    assert "3 of 4" in str(raised.value)


def test_a_single_bad_url_among_many_is_tolerated() -> None:
    invoices = [invoice(url="https://elsewhere.example/x")] + [invoice() for _ in range(9)]
    records = list(build_records(invoices, lines_ok))
    assert sum(1 for r in records if r["chain_status"] == CHAIN_UNPARSABLE) == 1
    assert sum(1 for r in records if r["chain_status"] == CHAIN_OK) == 27, "9 invoices, each with two lines"


def test_an_empty_invoice_list_is_not_drift() -> None:
    assert list(build_records([], lines_ok)) == []


def test_the_drift_guard_is_a_majority_not_a_single_failure() -> None:
    """Half the offered URLs failing is tolerated; more than half is not."""
    two = [invoice(url="bad"), invoice()]
    assert len(list(build_records(two, lines_ok))) == 4, "1 of 2 is not yet drift"

    with pytest.raises(UrlFormatDrift):
        list(build_records([invoice(url="bad")], lines_ok))


def test_neither_a_record_nor_a_log_line_carries_the_ephemeral_key(caplog: pytest.LogCaptureFixture) -> None:
    """The key authorises the line calls and must escape by neither route.

    The key is put where each route would pick it up: in line fields, which only
    the projection keeps out of a row, and in the failure message of the hop that
    carries it in its own URL, which only a type-and-status log keeps out of the
    log. A stub that never puts it in scope would assert nothing.
    """

    def lines_with_key(acct, token):
        if token == FAILING_TOKEN:
            raise RuntimeError(f"401 for https://invoicedata.stripe.com/hosted_invoice_page/{acct}/{EPHEMERAL_KEY}")
        leaky = dict(
            SUBSCRIPTION_LINE, ephemeral_key=EPHEMERAL_KEY, request_headers={"Authorization": f"Bearer {EPHEMERAL_KEY}"}
        )
        return "in_1ABC", [leaky]

    with caplog.at_level(logging.WARNING):
        records = list(build_records([invoice(), invoice(url=FAILING_URL), invoice(url="bad")], lines_with_key))

    assert any(r["chain_status"] == CHAIN_FAILED for r in records), "the failing hop was exercised"
    blob = repr(records) + caplog.text
    assert EPHEMERAL_KEY not in blob
    assert "ek_" not in blob


def test_key_parts_identify_a_line_by_stripe_ids_and_an_invoice_by_the_wrapper() -> None:
    records = list(build_records([invoice(url="bad"), invoice()], lines_ok))
    gap = next(r for r in records if r["chain_status"] == CHAIN_UNPARSABLE)
    line = next(r for r in records if r["line_id"])
    assert unique_key_parts(line) == ("in_1ABC", "il_standard")
    assert unique_key_parts(gap) == ("invoice", 1756771200, "pi_1", 3300, None)


def test_an_invoice_keeps_one_key_across_a_failure_and_a_later_recovery() -> None:
    """On a second key the recovered lines land beside the gap instead of replacing it."""
    unenriched = next(iter(build_records([invoice()], lines_raise)))
    enriched = next(iter(build_records([invoice()], lines_ok)))
    assert unique_key_parts(unenriched) == unique_key_parts(enriched)
    assert (unenriched["chain_status"], enriched["chain_status"]) == (CHAIN_FAILED, CHAIN_OK)
    assert unenriched["invoice_id"] is None and enriched["invoice_id"] == "in_1ABC"


def test_two_scrapes_of_one_invoice_share_a_key_though_its_url_rotated() -> None:
    """The vendor re-issues a fresh URL on every list call; only the identity
    inside it is the invoice, so that is what the key is built from."""
    first, _ = _hosted("ent1ABC", nonce="1756771200-n1")
    later, _ = _hosted("ent1ABC", nonce="1756800000-n2")
    assert first != later, "the fixture must actually rotate the URL"

    a = next(iter(build_records([invoice(url=first)], lines_raise)))
    b = next(iter(build_records([invoice(url=later)], lines_raise)))
    assert unique_key_parts(a) == unique_key_parts(b) == ("invoice", "acct_1ABC,_ent1ABC")


def test_an_invoice_whose_url_carries_no_identity_falls_back_to_the_wrapper() -> None:
    """A documented limit, not a bug: with no identity the invoice cannot be tied
    to its own enriched row, so the mutable fields are all that is left."""
    without_url = next(iter(build_records([invoice(url=None)], lines_ok)))
    assert unique_key_parts(without_url) == ("invoice", 1756771200, "pi_1", 3300, None)


def test_two_gaps_of_one_batch_stay_two_rows() -> None:
    """Creation timestamps collide across a batch; the amount separates them."""

    def lines_fail(acct, token):
        raise RuntimeError("a hop answered badly")

    # Distinct invoices carry distinct identities, which is what separates them.
    same_second = [
        invoice(url=_hosted("entONE")[0], payment_intent=None, total=3300),
        invoice(url=_hosted("entTWO")[0], payment_intent=None, total=4400),
    ]
    keys = {unique_key_parts(r) for r in build_records(same_second, lines_fail)}
    assert len(keys) == 2, "two invoices sharing a second must not collapse into one key"


def test_two_lines_of_one_invoice_get_different_keys() -> None:
    lines = [r for r in build_records([invoice()], lines_ok) if r["line_id"]]
    assert unique_key_parts(lines[0]) != unique_key_parts(lines[1])


# An invoice the connector has already chained. Bronze holds its lines under the
# invoice id below, and a settled invoice's lines do not move — so the two hops
# are what a later run skips, never the row.
ENRICHED = {"acct_1ABC,_ent1ABC": {"invoice_id": "in_1ABC", "period_start_ts": 1756684800, "period_end_ts": 1759276800}}


def lines_never(acct: str, token: str) -> tuple[str, Sequence[Mapping[str, Any]]]:
    raise AssertionError("the chain ran for an invoice that was already enriched")


def test_an_already_enriched_invoice_is_not_chained_again() -> None:
    rows = list(build_records([invoice()], lines_never, enriched=ENRICHED))
    assert [r["chain_status"] for r in rows] == ["ok"]
    assert rows[0]["invoice_id"] == "in_1ABC"


def test_it_carries_the_period_the_chain_found_because_the_listing_has_none() -> None:
    """The period comes from the lines, so skipping the hops would lose it — and a
    row without it would replace a good row with a worse one."""
    row = next(iter(build_records([invoice()], lines_never, enriched=ENRICHED)))
    assert (row["period_start_ts"], row["period_end_ts"]) == (1756684800, 1759276800)


def test_the_wrappers_money_is_read_again_rather_than_remembered() -> None:
    """Only the two hops are skipped. A total the vendor restates still lands."""
    row = next(iter(build_records([invoice(total=9900, total_excluding_tax=9000)], lines_never, enriched=ENRICHED)))
    assert (row["invoice_total"], row["invoice_total_excluding_tax"]) == (9900, 9000)


def test_an_enriched_invoice_repeats_the_key_it_already_has() -> None:
    """Same identity, so the row replaces its own rather than standing beside it."""
    chained = next(iter(build_records([invoice()], lines_ok)))
    skipped = next(iter(build_records([invoice()], lines_never, enriched=ENRICHED)))
    assert unique_key_parts(chained) == unique_key_parts(skipped)


def test_an_invoice_absent_from_the_map_still_chains() -> None:
    other = {"acct_1ABC,_someone_else": {"invoice_id": "in_OTHER"}}
    rows = list(build_records([invoice()], lines_ok, enriched=other))
    assert [r["chain_status"] for r in rows] == ["ok", "ok", "ok"], "its own row plus one per line"


def test_no_map_at_all_chains_everything() -> None:
    """The first sync after a deploy carries no state and behaves as before."""
    rows = list(build_records([invoice()], lines_ok, enriched={}))
    assert len([r for r in rows if r["line_id"]]) == 2
