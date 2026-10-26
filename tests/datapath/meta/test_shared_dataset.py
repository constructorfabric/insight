from dataclasses import replace
from pathlib import Path

import pytest
from insight_datapath.dataset_batches import content_tokens, overlap, plan_batches
from insight_datapath.fixture_loader import IdentityAccount, TestYaml, load
from insight_datapath.shared_dataset import combine, namespace

METRICS = Path(__file__).parents[1] / "metrics"


def fixture() -> TestYaml:
    return load(
        METRICS / "ai/ai_lines.test.yaml",
        substitutions={
            "tenant": "00000000-0000-0000-0000-000000000001",
            "supervisor_email": "lead@example.com",
            "supervisor_name": "Test Lead",
        },
    )


def test_namespaces_separate_people_cohorts_and_keys_without_changing_values() -> None:
    original = fixture()
    first = namespace(original, key="a", caller_email="lead@example.com")
    second = namespace(original, key="b", caller_email="lead@example.com")
    merged = combine([first, second])
    people = merged.bronze["bronze_bamboohr.employees"]
    assert len({person["id"] for person in people}) == 6
    assert len({person["workEmail"] for person in people}) == 6
    assert len({person["department"] for person in people}) == 2
    assert {person["supervisorEmail"] for person in people} == {"lead@example.com"}
    rows = merged.bronze["bronze_cursor.cursor_daily_usage"]
    assert [row["acceptedLinesAdded"] for row in rows] == [40, 40, 20, 10] * 2
    assert rows[0] == rows[1]
    assert rows[0]["unique_key"] != rows[4]["unique_key"]
    assert original.bronze["bronze_bamboohr.employees"][0]["workEmail"] == "alice@example.com"


def test_namespace_is_independent_of_selection_and_order() -> None:
    original = fixture()
    first = namespace(original, key="ai/ai_lines.test.yaml", caller_email="lead@example.com")
    other = namespace(original, key="another", caller_email="lead@example.com")
    combine([other, first])
    assert (
        namespace(original, key="ai/ai_lines.test.yaml", caller_email="lead@example.com") == first
    )


def test_raw_payload_and_identity_bindings_follow_the_same_namespace() -> None:
    original = replace(
        fixture(),
        identity_aliases={"alice@example.com": ["alias@example.com"]},
        identity_accounts=[
            IdentityAccount("cursor", "cursor-test", "account-a", "alice@example.com"),
            IdentityAccount("cursor", "cursor-test", "account-b", "excluded"),
        ],
    )
    case = namespace(original, key="example", caller_email="lead@example.com")
    person = case.seeded.bronze["bronze_bamboohr.employees"][0]
    assert person["raw_data"]["id"] == person["id"]
    assert person["raw_data"]["workEmail"] == person["workEmail"]
    assert person["raw_data"]["department"] == person["department"]
    assert case.seeded.identity_aliases == {
        case.prefix + "alice@example.com": [case.prefix + "alias@example.com"]
    }
    assert case.seeded.identity_accounts[0].source_id == case.prefix + "cursor-test"
    assert case.seeded.identity_accounts[0].account_id == "account-a"
    assert case.seeded.identity_accounts[1].person == "excluded"


def test_configuration_rows_cannot_silently_join_a_shared_dataset() -> None:
    original = replace(fixture(), bronze={"config.example": []})
    with pytest.raises(ValueError, match="configuration"):
        namespace(original, key="example", caller_email="lead@example.com")


def test_git_content_collisions_are_rejected_before_seeding() -> None:
    original = load(
        METRICS / "git/git_metrics.test.yaml",
        substitutions={
            "tenant": "00000000-0000-0000-0000-000000000001",
            "supervisor_email": "lead@example.com",
            "supervisor_name": "Test Lead",
        },
    )
    first = namespace(original, key="git/a", caller_email="lead@example.com")
    second = namespace(original, key="git/b", caller_email="lead@example.com")
    with pytest.raises(ValueError, match="overlapping"):
        combine([first, second])


def test_git_batches_preserve_prefix_and_content_identity_conflicts() -> None:
    specs = {
        "a": content_tokens({"sha": "ABCDEF", "post_image_oid": "blob-a"}),
        "b": content_tokens({"raw_data": '{"merge_commit_sha": "abc"}'}),
        "c": content_tokens({"pre_image_oid": "blob-a"}),
        "d": content_tokens({"sha": "xyz"}),
    }
    groups = plan_batches(specs)
    assert groups == plan_batches(dict(reversed(list(specs.items()))))
    assert groups["a"] != groups["b"]
    assert groups["a"] != groups["c"]
    assert len(set(groups.values())) == 2
    for name, group in groups.items():
        for other, other_group in groups.items():
            if other != name and group == other_group:
                assert not overlap(specs[name], specs[other])


def test_tasks_census_keys_keep_the_derived_source_formula() -> None:
    original = load(
        METRICS / "tasks/tasks_closed_deleted_excluded.test.yaml",
        substitutions={
            "tenant": "00000000-0000-0000-0000-000000000001",
            "supervisor_email": "lead@example.com",
            "supervisor_name": "Test Lead",
        },
    )
    case = namespace(original, key="tasks/example", caller_email="lead@example.com")
    for row in case.seeded.bronze["bronze_jira.jira_issue_census"]:
        assert row["unique_key"] == f"{row['tenant_id']}-{row['source_id']}-{row['jira_id']}"
    for table, rows in case.seeded.bronze.items():
        if table.startswith("config."):
            assert all(row["insight_source_id"].startswith(case.prefix) for row in rows)


def test_configuration_cannot_escape_the_scenarios_sources() -> None:
    original = fixture()
    original = replace(
        original,
        bronze={
            **original.bronze,
            "config.field_value_defaults": [{"insight_source_id": "unseeded"}],
        },
    )
    with pytest.raises(ValueError, match="seeded source"):
        namespace(original, key="tasks/example", caller_email="lead@example.com")
