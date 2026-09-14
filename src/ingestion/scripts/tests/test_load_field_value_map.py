"""Unit tests for the operator field-value-map loader.

Parsing, in-bundle de-duplication and the anti-join are free functions over
values, so the whole decision logic is exercised without ClickHouse; only the
statement text is asserted for the write path.
"""

from __future__ import annotations

import sys
import urllib.error
from pathlib import Path

import pytest

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))

from load_field_value_map import (  # noqa: E402
    DEFAULT_TIMEOUT,
    Default,
    Mapping,
    MappingFileError,
    _http_client,
    deduplicate,
    defaults_insert_statement,
    insert_statement,
    parse_bundle,
    parse_defaults,
    read_bundle,
    select_missing,
)

HEADER = "tenant_id\tinsight_source_id\tdata_source\tfield\tsource_key\tdisplay_name\ttarget_value\tnote"
ROW = "zz-test\tsrc-1\tjira\tissue_type\tstory\tStory\ttask"
OTHER_ROW = "zz-test\tsrc-1\tgithub\tissue_type\t10404\tDefect\tbug"
DEFAULT_ROW = "zz-test\tsrc-1\tissue_type\ttask"


def parse(text: str, name: str = "a.tsv") -> list[Mapping]:
    return parse_bundle([(name, text)])


def parse_default(text: str, name: str = "defaults.tsv") -> list[Default]:
    return parse_defaults([(name, text)])


class TestParsing:
    def test_business_columns_become_a_mapping(self):
        assert parse(ROW) == [Mapping("zz-test", "src-1", "jira", "issue_type", "story", "Story", "task")]

    def test_a_quoted_value_may_carry_the_delimiter(self):
        mapping = parse('zz-test\tsrc-1\tjira\tissue_type\tstory\t"Story\tor Task"\ttask\tunquoted, with a comma')[0]
        assert (mapping.display_name, mapping.note) == ("Story\tor Task", "unquoted, with a comma")

    def test_padding_around_a_value_is_dropped(self):
        line = "  zz-test \t src-1\tjira\t issue_type \t story \tStory\t task "
        assert parse(line) == [Mapping("zz-test", "src-1", "jira", "issue_type", "story", "Story", "task")]

    @pytest.mark.parametrize(
        ("line", "expected"),
        [(ROW, ""), (f"{ROW}\tagreed with the operator", "agreed with the operator"), (f"{ROW}\t", "")],
        ids=["seven", "eight", "empty-note"],
    )
    def test_the_note_column_is_optional(self, line, expected):
        assert parse(line)[0].note == expected

    @pytest.mark.parametrize(
        "text",
        [
            "",
            "\n\n",
            "# just a comment\n",
            "   \n",
            f"{HEADER}\n",
            f"{HEADER}\n# trailing comment\n",
            f"# lead\n\n{HEADER}\n",
            f"{HEADER.upper()}\n",
        ],
        ids=[
            "empty",
            "blanks",
            "comment",
            "whitespace",
            "header",
            "header-then-comment",
            "comment-then-header",
            "upper-case-header",
        ],
    )
    def test_noise_lines_declare_nothing(self, text):
        assert parse(text) == []

    def test_a_header_below_the_data_is_rejected(self):
        with pytest.raises(MappingFileError, match="field 'field'"):
            parse(f"{HEADER}\n{ROW}\n{HEADER}\n")

    @pytest.mark.parametrize(
        ("line", "problem"),
        [
            ("zz-test\tsrc-1\tjira\tissue_type\tstory\tStory", "expected 7-8 tab-separated columns, got 6"),
            (f"{ROW}\ta\tb", "expected 7-8 tab-separated columns, got 9"),
            ("\tsrc-1\tjira\tissue_type\tstory\tStory\ttask", "the first 7 columns are all required"),
            ("zz-test\tsrc-1\tjira\tissue_type\tstory\tStory\t", "the first 7 columns are all required"),
        ],
        ids=["too-few", "too-many", "blank-first", "blank-last-required"],
    )
    def test_a_malformed_line_names_its_file_and_line(self, line, problem):
        with pytest.raises(MappingFileError) as error:
            parse(f"# lead\n{line}\n")
        assert str(error.value) == f"a.tsv:2: {problem}"

    @pytest.mark.parametrize(
        "line",
        ["zz-test\tsrc-1\tjira\tissue_type\tstory\t\ttask", "zz-test\tsrc-1\tjira\tissue_type\tstory\ttask"],
        ids=["blank", "omitted"],
    )
    def test_a_decision_without_the_name_it_was_made_against_is_rejected(self, line):
        """`display_name` is what makes a later rename of the vendor value detectable."""
        with pytest.raises(MappingFileError):
            parse(line)

    def test_a_field_outside_the_supported_set_is_rejected_naming_the_set(self):
        with pytest.raises(MappingFileError, match="field 'status' is not supported; supported fields: issue_type"):
            parse("zz-test\tsrc-1\tjira\tstatus\tdone\tDone\ttask")

    @pytest.mark.parametrize(
        ("line", "problem"),
        [
            ("zz-test\tsrc-1\tjira\tissue_type\tstory\tStory\tbugg", "'bugg' for field 'issue_type' is not one of"),
            ("zz-test\tsrc-1\tjira\tissue_type\tstory\tStory\tBug", "'Bug' for field 'issue_type' is not one of"),
            (
                "zz-test\tsrc-1\tjira\tissue_type\tstory\tStory\tdone",
                "'done' for field 'issue_type' is not one of bug, task, unknown",
            ),
        ],
        ids=["typo", "wrong-case", "outside-the-domain"],
    )
    def test_a_target_value_outside_the_field_domain_is_rejected(self, line, problem):
        with pytest.raises(MappingFileError, match=problem):
            parse(line)

    def test_every_bad_line_in_the_bundle_is_reported(self):
        with pytest.raises(MappingFileError) as error:
            parse_bundle([("a.tsv", "bad\n"), ("b.tsv", f"{ROW}\nalso bad\n")])
        assert str(error.value).splitlines() == [
            "a.tsv:1: expected 7-8 tab-separated columns, got 1",
            "b.tsv:2: expected 7-8 tab-separated columns, got 1",
        ]

    def test_one_bad_line_rejects_the_whole_bundle(self):
        with pytest.raises(MappingFileError):
            parse(f"{ROW}\nbad\n")

    def test_reads_every_tsv_in_the_directory(self, tmp_path):
        (tmp_path / "a.tsv").write_text(f"{HEADER}\n{ROW}\n")
        (tmp_path / "b.tsv").write_text(ROW.replace("\tstory\tStory\t", "\ttask\tTask\t") + "\n")
        (tmp_path / "notes.md").write_text("not a bundle file")
        mappings, defaults = read_bundle(tmp_path)
        assert [m.source_key for m in mappings] == ["story", "task"]
        assert defaults == []


class TestDeduplication:
    def test_a_key_declared_twice_keeps_the_first_decision(self):
        mappings = parse(f"{ROW}\n{ROW.replace('Story\ttask', 'Story\tunknown')}\n")
        assert [m.target_value for m in deduplicate(mappings)] == ["task"]

    def test_distinct_keys_all_survive(self):
        mappings = parse(f"{ROW}\n{OTHER_ROW}\n")
        assert len(deduplicate(mappings)) == 2


class TestAntiJoin:
    def test_a_key_with_a_stored_decision_is_not_written(self):
        mapping = parse(ROW)[0]
        assert select_missing([mapping], {mapping.key}) == []

    def test_a_key_without_a_stored_decision_is_written(self):
        mapping = parse(ROW)[0]
        assert select_missing([mapping], {("zz-other", "src-1", "jira", "issue_type", "story")}) == [mapping]

    def test_a_stored_decision_is_matched_on_the_key_alone(self):
        """A different target_value for a stored key is still a stored decision."""
        mapping = parse(ROW.replace("Story\ttask", "Story\tunknown"))[0]
        assert select_missing([mapping], {mapping.key}) == []

    def test_duplicates_are_collapsed_before_the_anti_join(self):
        mappings = parse(f"{ROW}\n{ROW.replace('Story\ttask', 'Story\tunknown')}\n")
        assert [m.target_value for m in select_missing(mappings, set())] == ["task"]


class TestInsertStatement:
    def test_loader_owns_the_temporal_columns(self):
        sql = insert_statement(parse(f"{ROW}\tnote text"), recorded_by="gitops")
        head, _, body = sql.partition("FORMAT TSV\n")
        assert head.startswith(
            "INSERT INTO config.field_value_map (tenant_id, insight_source_id, data_source, "
            "field, source_key, valid_from, recorded_at, target_value, display_name, note, recorded_by) "
        )
        assert "field, source_key, toDateTime64(0, 3), now64(3), target_value, display_name" in head
        assert head.endswith(
            "note, 'gitops' FROM input('tenant_id String, insight_source_id String, "
            "data_source String, field String, source_key String, display_name String, "
            "target_value String, note String') "
        )
        assert body == "zz-test\tsrc-1\tjira\tissue_type\tstory\tStory\ttask\tnote text\n"

    def test_all_rows_go_out_in_one_statement(self):
        mappings = parse(f"{ROW}\n{OTHER_ROW}\n")
        sql = insert_statement(mappings, recorded_by="gitops")
        assert sql.count("INSERT INTO") == 1
        assert sql.partition("FORMAT TSV\n")[2].count("\n") == 2

    def test_recorded_by_is_quoted_as_a_literal(self):
        sql = insert_statement(parse(ROW), recorded_by="o'brien")
        assert "'o\\'brien'" in sql


class TestDefaultsParsing:
    def test_the_four_columns_become_a_default(self):
        assert parse_default(DEFAULT_ROW) == [Default("zz-test", "src-1", "issue_type", "task")]

    @pytest.mark.parametrize(
        "text",
        ["", "# only a comment\n", "tenant_id\tinsight_source_id\tfield\tdefault_value\n"],
        ids=["empty", "comment", "header"],
    )
    def test_noise_lines_declare_nothing(self, text):
        assert parse_default(text) == []

    @pytest.mark.parametrize(
        ("line", "problem"),
        [
            ("zz-test\tsrc-1\tissue_type", "expected 4 tab-separated columns, got 3"),
            (f"{DEFAULT_ROW}\textra", "expected 4 tab-separated columns, got 5"),
            ("zz-test\tsrc-1\t\ttask", "all 4 columns are required"),
        ],
        ids=["too-few", "too-many", "blank-field"],
    )
    def test_a_malformed_line_names_its_file_and_line(self, line, problem):
        with pytest.raises(MappingFileError) as error:
            parse_default(f"# lead\n{line}\n")
        assert str(error.value) == f"defaults.tsv:2: {problem}"

    def test_a_field_outside_the_supported_set_is_rejected_naming_the_set(self):
        with pytest.raises(MappingFileError, match="field 'status' is not supported; supported fields: issue_type"):
            parse_default("zz-test\tsrc-1\tstatus\ttask")

    @pytest.mark.parametrize(
        ("value", "problem"),
        [
            ("other", "'other' for field 'issue_type' is not one of bug, task, unknown"),
            ("Bug", "'Bug' for field 'issue_type'"),
        ],
        ids=["outside-the-domain", "wrong-case"],
    )
    def test_a_default_value_outside_the_field_domain_is_rejected(self, value, problem):
        with pytest.raises(MappingFileError, match=problem):
            parse_default(f"zz-test\tsrc-1\tissue_type\t{value}")


class TestDefaultsAntiJoin:
    def test_a_key_with_a_stored_decision_is_not_written(self):
        default = parse_default(DEFAULT_ROW)[0]
        assert select_missing([default], {default.key}) == []

    def test_a_key_without_a_stored_decision_is_written(self):
        default = parse_default(DEFAULT_ROW)[0]
        assert select_missing([default], {("zz-other", "src-1", "issue_type")}) == [default]

    def test_duplicates_are_collapsed_before_the_anti_join(self):
        defaults = parse_default(f"{DEFAULT_ROW}\n{DEFAULT_ROW.replace('task', 'unknown')}\n")
        assert [d.default_value for d in select_missing(defaults, set())] == ["task"]


class TestDefaultsInsertStatement:
    def test_loader_owns_the_temporal_columns(self):
        sql = defaults_insert_statement(parse_default(DEFAULT_ROW), recorded_by="gitops")
        head, _, body = sql.partition("FORMAT TSV\n")
        assert head.startswith(
            "INSERT INTO config.field_value_defaults "
            "(tenant_id, insight_source_id, field, valid_from, recorded_at, default_value, recorded_by) "
        )
        assert "toDateTime64(0, 3), now64(3)" in head
        assert head.endswith(
            "'gitops' FROM input('tenant_id String, insight_source_id String, field String, default_value String') "
        )
        assert body == "zz-test\tsrc-1\tissue_type\ttask\n"

    def test_all_rows_go_out_in_one_statement(self):
        defaults = parse_default(f"{DEFAULT_ROW}\nzz-test\tsrc-2\tissue_type\tunknown\n")
        sql = defaults_insert_statement(defaults, recorded_by="gitops")
        assert sql.count("INSERT INTO") == 1
        assert sql.partition("FORMAT TSV\n")[2].count("\n") == 2


class TestBundleRouting:
    """`defaults.tsv` feeds the defaults table; every other `*.tsv` feeds the map."""

    def test_defaults_tsv_routes_to_the_defaults_table(self, tmp_path):
        (tmp_path / "a.tsv").write_text(f"{ROW}\n")
        (tmp_path / "defaults.tsv").write_text(f"{DEFAULT_ROW}\n")
        mappings, defaults = read_bundle(tmp_path)
        assert [m.source_key for m in mappings] == ["story"]
        assert defaults == [Default("zz-test", "src-1", "issue_type", "task")]

    def test_a_defaults_line_is_never_parsed_as_a_mapping(self, tmp_path):
        (tmp_path / "defaults.tsv").write_text(f"{DEFAULT_ROW}\n")
        mappings, defaults = read_bundle(tmp_path)
        assert mappings == []
        assert len(defaults) == 1

    def test_bad_lines_in_both_files_are_all_reported(self, tmp_path):
        (tmp_path / "a.tsv").write_text("bad\n")
        (tmp_path / "defaults.tsv").write_text("also bad\n")
        with pytest.raises(MappingFileError) as error:
            read_bundle(tmp_path)
        assert str(error.value).splitlines() == [
            "a.tsv:1: expected 7-8 tab-separated columns, got 1",
            "defaults.tsv:1: expected 4 tab-separated columns, got 1",
        ]


UNESCAPE = {"\\": "\\", "b": "\b", "f": "\f", "r": "\r", "n": "\n", "t": "\t", "0": "\0", "'": "'"}


def unescape(field: str) -> str:
    """Decode ClickHouse TabSeparated escapes — the reader side of the wire."""
    out: list[str] = []
    escaped = False
    for char in field:
        if escaped:
            out.append(UNESCAPE[char])
            escaped = False
        elif char == "\\":
            escaped = True
        else:
            out.append(char)
    assert not escaped
    return "".join(out)


class TestTsvEscaping:
    """A parsed value may hold a tab, a newline or a backslash; the wire may not."""

    @pytest.mark.parametrize(
        "value",
        ["Done\tand dusted", "Done\nand dusted", "Done\rand dusted", "Done\\and dusted", "back\\\\slash", "plain"],
        ids=["tab", "newline", "carriage-return", "backslash", "double-backslash", "plain"],
    )
    @pytest.mark.parametrize("column", ["display_name", "note"], ids=["display_name", "note"])
    def test_a_delimiter_inside_a_value_round_trips(self, value, column):
        mapping = Mapping("zz-test", "src-1", "jira", "issue_type", "story", "Story", "task", "")
        mapping = Mapping(*[value if field == column else getattr(mapping, field) for field in mapping.__annotations__])
        body = insert_statement([mapping], recorded_by="gitops").partition("FORMAT TSV\n")[2]

        assert body.count("\n") == 1, "the value must not open a second row"
        fields = body.rstrip("\n").split("\t")
        assert len(fields) == 8, "the value must not open a second column"
        assert [unescape(field) for field in fields] == [
            "zz-test",
            "src-1",
            "jira",
            "issue_type",
            "story",
            value if column == "display_name" else "Story",
            "task",
            value if column == "note" else "",
        ]

    def test_a_quoted_tab_from_the_bundle_reaches_the_wire_escaped(self):
        mapping = parse('zz-test\tsrc-1\tjira\tissue_type\tstory\t"Done\tand dusted"\ttask')[0]
        body = insert_statement([mapping], recorded_by="gitops").partition("FORMAT TSV\n")[2]
        assert body == "zz-test\tsrc-1\tjira\tissue_type\tstory\tDone\\tand dusted\ttask\t\n"

    def test_every_row_still_ends_on_its_own_line(self):
        mappings = [
            Mapping("zz-test", "src-1", "jira", "issue_type", "story", "A\nB", "task"),
            Mapping("zz-test", "src-1", "jira", "issue_type", "bug", "C\tD", "bug"),
        ]
        body = insert_statement(mappings, recorded_by="gitops").partition("FORMAT TSV\n")[2]
        assert body.count("\n") == 2


class TestHttpTimeout:
    """The loader is a post-upgrade hook; a stalled endpoint must fail it, not hang it."""

    @pytest.fixture(autouse=True)
    def _credentials(self, monkeypatch):
        monkeypatch.setenv("CLICKHOUSE_URL", "http://ch:8123")
        monkeypatch.setenv("CLICKHOUSE_USER", "insight")
        monkeypatch.setenv("CLICKHOUSE_PASSWORD", "secret")
        monkeypatch.delenv("FIELD_VALUE_MAP_TIMEOUT", raising=False)

    @staticmethod
    def _recorder(seen):
        class Response:
            def __enter__(self):
                return self

            def __exit__(self, *_):
                return False

            def read(self):
                return b""

        def urlopen(_request, timeout=None):
            seen.append(timeout)
            return Response()

        return urlopen

    def test_every_request_carries_the_default_timeout(self):
        seen: list[float] = []
        execute, fetch_rows = _http_client(urlopen=self._recorder(seen))
        execute("SELECT 1")
        fetch_rows("SELECT 1")
        assert seen == [DEFAULT_TIMEOUT, DEFAULT_TIMEOUT]

    def test_the_env_var_overrides_it(self, monkeypatch):
        monkeypatch.setenv("FIELD_VALUE_MAP_TIMEOUT", "2.5")
        seen: list[float] = []
        execute, _ = _http_client(urlopen=self._recorder(seen))
        execute("SELECT 1")
        assert seen == [2.5]

    @pytest.mark.parametrize("raw", ["soon", "0", "-1"], ids=["not-a-number", "zero", "negative"])
    def test_a_bad_timeout_is_rejected_before_any_request(self, monkeypatch, raw):
        monkeypatch.setenv("FIELD_VALUE_MAP_TIMEOUT", raw)
        with pytest.raises(SystemExit, match="FIELD_VALUE_MAP_TIMEOUT"):
            _http_client(urlopen=self._recorder([]))

    @pytest.mark.parametrize(
        "error", [TimeoutError(), urllib.error.URLError(TimeoutError())], ids=["read-timeout", "connect-timeout"]
    )
    def test_a_timeout_is_reported_as_one(self, error):
        def urlopen(_request, timeout=None):
            raise error

        execute, _ = _http_client(urlopen=urlopen)
        with pytest.raises(SystemExit, match=r"did not answer within 30.0s \(http://ch:8123/\)"):
            execute("SELECT 1")

    def test_any_other_transport_error_still_surfaces(self):
        def urlopen(_request, timeout=None):
            raise urllib.error.URLError("connection refused")

        execute, _ = _http_client(urlopen=urlopen)
        with pytest.raises(urllib.error.URLError):
            execute("SELECT 1")
