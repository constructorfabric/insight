from __future__ import annotations

from source_gitlab.streams.base import parse_diff_counts


class TestParseDiffCounts:
    def test_counts_added_and_removed_within_hunk(self) -> None:
        text = "@@ -1,2 +1,2 @@\n context\n-old line\n+new line\n+another\n"
        assert parse_diff_counts({"diff": text}) == (2, 1, False)

    def test_content_lines_starting_with_repeated_markers_are_counted(self) -> None:
        text = "@@ -1,1 +1,1 @@\n---y\n+++x\n"
        assert parse_diff_counts({"diff": text}) == (1, 1, False)

    def test_skips_file_header_preamble(self) -> None:
        text = (
            "diff --git a/f b/f\n"
            "--- a/f\n"
            "+++ b/f\n"
            "@@ -1,1 +1,1 @@\n"
            "-gone\n"
            "+added\n"
        )
        assert parse_diff_counts({"diff": text}) == (1, 1, False)

    def test_too_large_or_collapsed_marks_truncated(self) -> None:
        assert parse_diff_counts({"too_large": True}) == (None, None, True)
        assert parse_diff_counts({"collapsed": True}) == (None, None, True)

    def test_empty_diff(self) -> None:
        assert parse_diff_counts({"diff": ""}) == (0, 0, False)
