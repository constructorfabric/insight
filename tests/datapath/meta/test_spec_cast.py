"""Which of a spec's addresses the product is expected to turn into people."""

from __future__ import annotations

from typing import Any

from insight_datapath.spec_runner import all_persona_emails, declared_people

CALLER = "lead@example.com"


def _bronze(employees: list[dict[str, Any]], **rest: list[dict[str, Any]]) -> dict[str, Any]:
    return {"bronze_bamboohr.employees": employees, **rest}


def test_an_employment_record_is_how_a_spec_declares_a_person() -> None:
    bronze = _bronze([{"workEmail": "alice@example.com"}, {"workEmail": "bob@example.com"}])
    assert declared_people(bronze) == {"alice@example.com", "bob@example.com"}


def test_an_address_that_only_authored_something_is_not_declared() -> None:
    bronze = _bronze(
        [{"workEmail": "alice@example.com"}],
        **{"bronze_github.commits": [{"author_email": "drive-by@example.com"}]},
    )
    assert declared_people(bronze) == {"alice@example.com"}
    assert "drive-by@example.com" in all_persona_emails(bronze)


def test_a_spec_that_seeds_no_records_declares_nobody() -> None:
    assert declared_people({"bronze_github.workflow_runs": [{"id": 1}]}) == set()


def test_the_caller_belongs_to_the_instance_not_to_the_spec() -> None:
    bronze = _bronze([{"workEmail": "alice@example.com"}, {"workEmail": CALLER}])
    assert declared_people(bronze, excluding=CALLER) == {"alice@example.com"}


def test_a_record_is_matched_however_its_address_is_spelt() -> None:
    bronze = _bronze([{"workEmail": " Alice@Example.COM "}])
    assert declared_people(bronze) == {"alice@example.com"}


def test_a_record_without_an_address_declares_nobody() -> None:
    assert declared_people(_bronze([{"workEmail": None}, {"firstName": "Nameless"}])) == set()
