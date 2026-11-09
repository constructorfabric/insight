"""Every singular dbt test a connector or a task class ships must run somewhere.

An installation runs `dbt run` only. Its singular tests execute in two
selections: the scheduled catalog (`dbt test --selector data_quality`) and each
connector's post-sync step (`dbt test --select tag:connector_quality,tag:<slug>`,
where the slug is the connector's descriptor name). A test outside both runs on
no installation, and nothing notices when it is added.

The one other place a test runs is a CI suite that selects its whole directory.
Such a test carries `ci_only`, and only a directory some CI suite selects may
hold one.

`gold/` is out of scope: serving-layer invariants belong to no connector or
class, and the data-path rig's `dbt build` exercises them alongside their models.

Tags are read off each file's own `config(...)`, so this costs a glob and needs
no warehouse. The project declares no folder-level test config; a rule below
keeps it that way, because this guard could not see one.
"""

from __future__ import annotations

import re
from dataclasses import dataclass
from enum import Enum
from pathlib import Path

import pytest
import yaml

REPO_ROOT = Path(__file__).resolve().parents[3]
DBT_DIR = REPO_ROOT / "src/ingestion/dbt"
TESTS_DIR = DBT_DIR / "tests"
CONNECTORS_DIR = REPO_ROOT / "src/ingestion/connectors"
JIRA_SUITE_CONFTEST = TESTS_DIR / "jira/transform/conftest.py"
JIRA_WORKFLOW = REPO_ROOT / ".github/workflows/jira-field-history.yml"

SCHEDULED_TAG = "data_quality"
PER_SYNC_TAG = "connector_quality"
CI_ONLY_TAG = "ci_only"
SERVING_LAYER_DIR = "gold"

#: Directories a CI suite runs as a whole, mapped to the line that proves it.
CI_SELECTED_DIRS = {
    "jira": (JIRA_SUITE_CONFTEST, 'def invariants_hold(self, select: str = "tests/jira")'),
}

#: Tests that run on no installation yet. Shrinks only: an entry whose test
#: now carries a selection, or no longer exists, fails `test_the_allowlist_only_shrinks`.
UNSELECTED_ALLOWLIST = frozenset(
    {
        "ai/assert_cursor_event_cost_attributed.sql",
        "collaboration/assert_m365_staging_days_were_reported.sql",
        "git/assert_item_events_unique_grain.sql",
        "hr/assert_account_attribute_values_no_overlap.sql",
        "hr/assert_person_attribute_claims_unique_grain.sql",
        "wiki/assert_comment_replies_have_page_id.sql",
        "wiki/assert_comment_replies_parent_resolves.sql",
    }
)

_CONFIG_CALL = re.compile(r"\{\{\s*config\(")
_TAGS = re.compile(r"\btags\s*=\s*\[([^\]]*)\]", re.DOTALL)
_QUOTED = re.compile(r"""['"]([^'"]+)['"]""")


class Selection(Enum):
    SCHEDULED = "data_quality"
    PER_SYNC = "connector_quality"
    CI_ONLY = "ci_only"
    NONE = "none"


@dataclass(frozen=True)
class SingularTest:
    path: str
    tags: frozenset[str]

    @property
    def directory(self) -> str:
        return self.path.split("/", 1)[0] if "/" in self.path else ""


def _config_body(sql: str) -> str:
    """The argument list of the file's `{{ config(...) }}` call, or nothing."""
    start = _CONFIG_CALL.search(sql)
    if not start:
        return ""
    depth = 1
    for index in range(start.end(), len(sql)):
        depth += {"(": 1, ")": -1}.get(sql[index], 0)
        if depth == 0:
            return sql[start.end() : index]
    return ""


def tags_of(sql: str) -> frozenset[str]:
    declared = _TAGS.search(_config_body(sql))
    return frozenset(_QUOTED.findall(declared.group(1)) if declared else ())


def selection_of(test: SingularTest, connectors: frozenset[str]) -> Selection:
    """Which run executes this test; NONE when no installation and no CI suite does.

    A test claiming CI-only alongside a catalog tag is NONE too: the two say
    different things about where it runs, and only one can be true.
    """
    catalogued = bool({SCHEDULED_TAG, PER_SYNC_TAG} & test.tags)
    if CI_ONLY_TAG in test.tags:
        if catalogued or test.directory not in CI_SELECTED_DIRS:
            return Selection.NONE
        return Selection.CI_ONLY

    if SCHEDULED_TAG in test.tags:
        return Selection.SCHEDULED

    if PER_SYNC_TAG in test.tags and test.tags & connectors:
        return Selection.PER_SYNC

    return Selection.NONE


def _connectors() -> frozenset[str]:
    names = set()
    for path in CONNECTORS_DIR.glob("*/*/descriptor.yaml"):
        document = yaml.safe_load(path.read_text(encoding="utf-8")) or {}
        if document.get("name"):
            names.add(document["name"])
    return frozenset(names)


def _singular_tests() -> list[SingularTest]:
    found = []
    for path in sorted(TESTS_DIR.rglob("*.sql")):
        relative = path.relative_to(TESTS_DIR).as_posix()
        if relative.split("/", 1)[0] == SERVING_LAYER_DIR:
            continue
        found.append(SingularTest(path=relative, tags=tags_of(path.read_text(encoding="utf-8"))))
    return found


CONNECTORS = _connectors()
SINGULAR_TESTS = _singular_tests()


@pytest.mark.parametrize(
    ("path", "tags", "expected"),
    [
        ("task/a.sql", {"data_quality"}, Selection.SCHEDULED),
        ("task/a.sql", {"connector_quality", "jira"}, Selection.PER_SYNC),
        ("jira/a.sql", {"ci_only"}, Selection.CI_ONLY),
        ("task/a.sql", set(), Selection.NONE),
        ("task/a.sql", {"jira", "silver"}, Selection.NONE),
        ("task/a.sql", {"connector_quality"}, Selection.NONE),
        ("task/a.sql", {"connector_quality", "not_a_connector"}, Selection.NONE),
        ("task/a.sql", {"ci_only"}, Selection.NONE),
        ("jira/a.sql", {"ci_only", "connector_quality", "jira"}, Selection.NONE),
        ("a.sql", {"ci_only"}, Selection.NONE),
    ],
)
def test_a_test_runs_only_where_a_selection_reaches_it(
    path: str, tags: set[str], expected: Selection
) -> None:
    """`connector_quality` alone matches no post-sync step, which always intersects
    it with a connector slug; `ci_only` counts only where CI selects the directory."""
    test = SingularTest(path=path, tags=frozenset(tags))
    assert selection_of(test, frozenset({"jira", "youtrack"})) == expected, (
        f"{path} tagged {sorted(tags)} should be {expected.name}"
    )


def test_tags_are_read_from_the_config_call_only() -> None:
    """A `tags=` in a SQL comment or the body is not dbt config and selects nothing."""
    sql = (
        "{{ config(\n    severity='warn',\n    tags=['connector_quality', 'jira'],\n"
        "    meta={'title': 'x (y)'}\n) }}\n-- tags=['data_quality']\nSELECT 1"
    )
    assert tags_of(sql) == {"connector_quality", "jira"}
    assert tags_of("-- tags=['data_quality']\nSELECT 1") == frozenset()


def test_the_test_tree_parses() -> None:
    """A glob that finds nothing would make every rule below vacuous."""
    assert len(SINGULAR_TESTS) > 50, f"only {len(SINGULAR_TESTS)} singular tests under {TESTS_DIR}"
    assert {"jira", "youtrack", "allure"} <= CONNECTORS, f"descriptors name {sorted(CONNECTORS)}"
    assert any(SCHEDULED_TAG in test.tags for test in SINGULAR_TESTS), "no test is scheduled"
    assert any(PER_SYNC_TAG in test.tags for test in SINGULAR_TESTS), "no test runs per sync"


def test_the_project_sets_no_folder_level_test_config() -> None:
    """Tags set in dbt_project.yml would select tests this guard reads as untagged."""
    project = yaml.safe_load((DBT_DIR / "dbt_project.yml").read_text(encoding="utf-8"))
    assert "tests" not in project and "data_tests" not in project


@pytest.mark.parametrize(
    ("directory", "proof"), CI_SELECTED_DIRS.items(), ids=list(CI_SELECTED_DIRS)
)
def test_a_ci_selected_directory_is_still_selected_by_its_suite(
    directory: str, proof: tuple[Path, str]
) -> None:
    """`ci_only` is trusted only while the suite still runs the whole directory."""
    source, line = proof
    assert line in source.read_text(encoding="utf-8"), (
        f"{source.relative_to(REPO_ROOT)} no longer selects tests/{directory}; "
        f"drop it from CI_SELECTED_DIRS or restore the selection"
    )
    suite_dir = source.parent.relative_to(REPO_ROOT).as_posix()
    workflow = JIRA_WORKFLOW.read_text(encoding="utf-8")
    assert f"working-directory: {suite_dir}" in workflow, (
        f"no workflow runs the suite in {suite_dir}"
    )


def test_every_connector_and_class_test_runs_somewhere() -> None:
    """A test outside every selection is dead weight: it passes review, ships, and
    never executes against an installation's data."""
    unselected = sorted(
        test.path
        for test in SINGULAR_TESTS
        if selection_of(test, CONNECTORS) is Selection.NONE
        and test.path not in UNSELECTED_ALLOWLIST
    )
    assert unselected == [], (
        f"these singular tests run nowhere: {unselected}. Tag each `{SCHEDULED_TAG}` "
        f"(scheduled, silver/gold only), `{PER_SYNC_TAG}` plus its connector slug "
        f"(after that connector's sync), or `{CI_ONLY_TAG}` under a CI-selected "
        f"directory ({sorted(CI_SELECTED_DIRS)})"
    )


def test_the_allowlist_only_shrinks() -> None:
    """An entry that now runs somewhere, or no longer exists, would let a new
    unselected test of the same name in later."""
    unselected = {
        test.path for test in SINGULAR_TESTS if selection_of(test, CONNECTORS) is Selection.NONE
    }
    stale = sorted(UNSELECTED_ALLOWLIST - unselected)
    assert stale == [], f"remove these from UNSELECTED_ALLOWLIST: {stale}"
