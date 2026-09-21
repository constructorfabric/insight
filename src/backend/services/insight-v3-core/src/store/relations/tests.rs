use super::reads_each_row_once;

/// An allow-list, deliberately: an engine nobody thought of defaults to
/// refused rather than to silently counted twice. A `MaterializedView` is
/// the case that makes this matter — `system.tables` reports its engine as
/// `MaterializedView` while its rows really belong to a hidden inner table
/// that may well be a `ReplacingMergeTree`.
#[test]
fn only_an_engine_known_to_count_each_row_once_may_be_read_plainly() {
    let cases = [
        ("MergeTree", true),
        ("ReplicatedMergeTree", true),
        ("SharedMergeTree", true),
        ("View", true),
        ("ReplacingMergeTree", false),
        ("ReplicatedReplacingMergeTree", false),
        ("CollapsingMergeTree", false),
        ("VersionedCollapsingMergeTree", false),
        ("SummingMergeTree", false),
        ("AggregatingMergeTree", false),
        ("CoalescingMergeTree", false),
        ("GraphiteMergeTree", false),
        ("MaterializedView", false),
        ("Distributed", false),
        ("Merge", false),
        ("Buffer", false),
        ("Log", false),
        ("", false),
    ];

    for (engine, expected) in cases {
        assert_eq!(reads_each_row_once(engine), expected, "engine {engine}");
    }
}
