"""Unit tests for the operator value-map loader.

Parsing, in-bundle de-duplication and the anti-join are free functions over
values, so the whole decision logic is exercised without ClickHouse; only the
statement text is asserted for the write path.
"""

from __future__ import annotations

import sys
from pathlib import Path

import pytest

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))

from load_task_value_map import (  # noqa: E402
    Mapping,
    MappingFileError,
    deduplicate,
    insert_statement,
    parse_bundle,
    read_bundle,
    select_missing,
)

HEADER = "tenant_id\tinsight_source_id\tdata_source\tfield_id\tvalue_id\tcanonical_value\tvalue_display\tnote"
ROW = "zz-test\tsrc-1\tjira\ttype\t10404\tbug"
STATUS_ROW = "zz-test\tsrc-1\tgithub\tstate\tclosed:completed\tdone"


def parse(text: str, name: str = "a.tsv") -> list[Mapping]:
    return parse_bundle([(name, text)])


class TestParsing:
    def test_business_columns_become_a_mapping(self):
        assert parse(ROW) == [Mapping("zz-test", "src-1", "jira", "type", "10404", "bug")]

    def test_a_quoted_value_may_carry_the_delimiter(self):
        mapping = parse(f'{ROW}\t"Done\tand dusted"\tunquoted, with a comma')[0]
        assert (mapping.value_display, mapping.note) == ("Done\tand dusted", "unquoted, with a comma")

    def test_padding_around_a_value_is_dropped(self):
        line = "  zz-test \t src-1\tjira\ttype\t 10404 \t bug "
        assert parse(line) == [Mapping("zz-test", "src-1", "jira", "type", "10404", "bug")]

    @pytest.mark.parametrize(
        ("line", "expected"),
        [
            (ROW, ("", "")),
            (f"{ROW}\tDone", ("Done", "")),
            (f"{ROW}\tDone\tagreed with the operator", ("Done", "agreed with the operator")),
            (f"{ROW}\t\t", ("", "")),
        ],
        ids=["six", "seven", "eight", "empty-optionals"],
    )
    def test_optional_columns_default_to_empty(self, line, expected):
        mapping = parse(line)[0]
        assert (mapping.value_display, mapping.note) == expected

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
        with pytest.raises(MappingFileError, match="canonical_value 'canonical_value'"):
            parse(f"{HEADER}\n{ROW}\n{HEADER}\n")

    @pytest.mark.parametrize(
        ("line", "problem"),
        [
            ("zz-test\tsrc-1\tjira\ttype\t10404", "expected 6-8 tab-separated columns, got 5"),
            (f"{ROW}\ta\tb\tc", "expected 6-8 tab-separated columns, got 9"),
            ("\tsrc-1\tjira\ttype\t10404\tbug", "the first 6 columns are all required"),
            ("zz-test\tsrc-1\tjira\ttype\t10404\t", "the first 6 columns are all required"),
        ],
        ids=["too-few", "too-many", "blank-first", "blank-last-required"],
    )
    def test_a_malformed_line_names_its_file_and_line(self, line, problem):
        with pytest.raises(MappingFileError) as error:
            parse(f"# lead\n{line}\n")
        assert str(error.value) == f"a.tsv:2: {problem}"

    @pytest.mark.parametrize(
        ("line", "problem"),
        [
            ("zz-test\tsrc-1\tgithub\tstate\tclosed:completed\tdonne", "'donne' is not one of"),
            ("zz-test\tsrc-1\tgithub\tstate\tclosed:completed\tDone", "'Done' is not one of"),
            ("zz-test\tsrc-1\tjira\ttype\t10404\tdone", "'done' is not one of bug, other, unknown"),
        ],
        ids=["typo", "wrong-case", "outside-the-issue-kind-domain"],
    )
    def test_a_canonical_value_outside_its_domain_is_rejected(self, line, problem):
        with pytest.raises(MappingFileError, match=problem):
            parse(line)

    def test_every_bad_line_in_the_bundle_is_reported(self):
        with pytest.raises(MappingFileError) as error:
            parse_bundle([("a.tsv", "bad\n"), ("b.tsv", f"{ROW}\nalso bad\n")])
        assert str(error.value).splitlines() == [
            "a.tsv:1: expected 6-8 tab-separated columns, got 1",
            "b.tsv:2: expected 6-8 tab-separated columns, got 1",
        ]

    def test_one_bad_line_rejects_the_whole_bundle(self):
        with pytest.raises(MappingFileError):
            parse(f"{ROW}\nbad\n")

    def test_reads_every_tsv_in_the_directory(self, tmp_path):
        (tmp_path / "a.tsv").write_text(f"{HEADER}\n{ROW}\n")
        (tmp_path / "b.tsv").write_text(ROW.replace("\t10404\t", "\t10405\t") + "\n")
        (tmp_path / "notes.md").write_text("not a bundle file")
        assert [m.value_id for m in read_bundle(tmp_path)] == ["10404", "10405"]


class TestDeduplication:
    def test_a_key_declared_twice_keeps_the_first_decision(self):
        mappings = parse(f"{ROW}\n{ROW.replace('bug', 'other')}\n")
        assert [m.canonical_value for m in deduplicate(mappings)] == ["bug"]

    def test_distinct_keys_all_survive(self):
        mappings = parse(f"{ROW}\n{STATUS_ROW}\n")
        assert len(deduplicate(mappings)) == 2


class TestAntiJoin:
    def test_a_key_with_a_stored_decision_is_not_written(self):
        mapping = parse(ROW)[0]
        assert select_missing([mapping], {mapping.key}) == []

    def test_a_key_without_a_stored_decision_is_written(self):
        mapping = parse(ROW)[0]
        assert select_missing([mapping], {("other", "src-1", "jira", "type", "10404")}) == [mapping]

    def test_a_stored_decision_is_matched_on_the_key_alone(self):
        """A different canonical_value for a stored key is still a stored decision."""
        mapping = parse(f"{ROW.replace('bug', 'other')}")[0]
        assert select_missing([mapping], {mapping.key}) == []

    def test_duplicates_are_collapsed_before_the_anti_join(self):
        mappings = parse(f"{ROW}\n{ROW.replace('bug', 'other')}\n")
        assert [m.canonical_value for m in select_missing(mappings, set())] == ["bug"]


class TestInsertStatement:
    def test_loader_owns_the_temporal_columns(self):
        sql = insert_statement(parse(f"{ROW}\tDone\tnote text"), recorded_by="gitops")
        head, _, body = sql.partition("FORMAT TSV\n")
        assert "toDateTime64(0, 3), now64(3)" in head
        assert head.endswith(
            "'gitops' FROM input('tenant_id String, insight_source_id String, "
            "data_source String, field_id String, value_id String, canonical_value String, "
            "value_display String, note String') "
        )
        assert body == "zz-test\tsrc-1\tjira\ttype\t10404\tbug\tDone\tnote text\n"

    def test_all_rows_go_out_in_one_statement(self):
        mappings = parse(f"{ROW}\n{STATUS_ROW}\n")
        sql = insert_statement(mappings, recorded_by="gitops")
        assert sql.count("INSERT INTO") == 1
        assert sql.partition("FORMAT TSV\n")[2].count("\n") == 2

    def test_recorded_by_is_quoted_as_a_literal(self):
        sql = insert_statement(parse(ROW), recorded_by="o'brien")
        assert "'o\\'brien'" in sql
