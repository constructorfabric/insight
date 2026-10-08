from pathlib import Path

import pytest
from insight_datapath.suite_scan import scan_suites, seed_calls, suite_of, unplanned


@pytest.mark.parametrize(
    ("source", "relations", "readable"),
    [
        (
            'S = "bronze_x"\nT = "seats"\nseeder.seed_records(S, T, [row])\n',
            {("bronze_x", "seats")},
            True,
        ),
        ('seed_records("bronze_x", "seats", rows)\n', {("bronze_x", "seats")}, True),
        ('seeder.seed_records(schema_for(x), "seats", rows)\n', set(), False),
        ('clear(cfg, [("bronze_x", "seats")])\n', set(), True),
    ],
)
def test_a_seed_call_is_read_through_module_constants(
    source: str, relations: set[tuple[str, str]], readable: bool
) -> None:
    assert seed_calls(source) == (frozenset(relations), readable), f"should read {source!r}"


def _write(root: Path, path: str, text: str) -> None:
    target = root / path
    target.parent.mkdir(parents=True, exist_ok=True)
    target.write_text(text, encoding="utf-8")


@pytest.fixture
def tree(tmp_path: Path) -> Path:
    _write(
        tmp_path, "tests/lib/insight_datapath/records.py", 'TEMPLATE = "people.yaml#/templates/e"\n'
    )
    _write(
        tmp_path,
        "tests/datapath/metrics/git/commits.test.yaml",
        "bronze:\n  bronze_github.commits:\n    - $ref: ../templates/git.yaml#/templates/c\n",
    )
    _write(tmp_path, "tests/datapath/metrics/git/nested/test_x.py", 'T = "bronze_gitlab.users"\n')
    _write(tmp_path, "tests/datapath/metrics/templates/git.yaml", "templates: {}\n")
    _write(tmp_path, "tests/datapath/metrics/schemas/bronze_github.commits.yaml", "{}\n")
    _write(
        tmp_path,
        "tests/datapath/identity/test_y.py",
        "from insight_datapath.records import employee\n"
        'S = "bronze_x"\nseeder.seed_records(S, other(), [])\n',
    )
    return tmp_path


def test_a_tree_reading_names_seeds_templates_and_unreadable_modules(tree: Path) -> None:
    suites = scan_suites(tree)

    assert suites.metric_classes == ("git",)
    assert suites.seeds["git"] == {("bronze_github", "commits"), ("bronze_gitlab", "users")}
    assert suites.template_users == {
        "tests/datapath/metrics/templates/git.yaml": {"git"},
        "tests/datapath/metrics/templates/people.yaml": {"identity"},
    }
    assert suites.opaque == ("tests/datapath/identity/test_y.py",)


@pytest.mark.parametrize(
    ("path", "suite"),
    [
        ("tests/datapath/metrics/git/test_commits.py", "git"),
        ("tests/datapath/identity/test_y.py", "identity"),
        ("tests/datapath/meta/test_reset.py", None),
        ("tests/datapath/metrics/conftest.py", None),
    ],
)
def test_a_test_belongs_to_the_suite_its_directory_names(
    tmp_path: Path, path: str, suite: str | None
) -> None:
    assert suite_of(tmp_path / path, tmp_path) == suite, f"should place {path} in {suite}"


def test_a_run_reports_every_relation_its_tree_reading_missed() -> None:
    planned = {"ai": frozenset({("bronze_x", "a")})}
    seeded = {"ai": [("bronze_x", "a"), ("bronze_x", "b")], "identity": [("bronze_y", "c")]}
    assert unplanned(seeded, planned) == {
        "ai": [("bronze_x", "b")],
        "identity": [("bronze_y", "c")],
    }
