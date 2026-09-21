use super::collapses;

/// An engine that does not collapse refuses `FINAL` outright, so asking for
/// it everywhere would break every read of a relation that does not need it.
#[test]
fn only_an_engine_that_keeps_superseded_rows_counts_as_collapsing() {
    let cases = [
        ("ReplacingMergeTree", true),
        ("ReplicatedReplacingMergeTree", true),
        ("SharedReplacingMergeTree", true),
        ("CollapsingMergeTree", true),
        ("VersionedCollapsingMergeTree", true),
        ("SummingMergeTree", true),
        ("AggregatingMergeTree", true),
        ("MergeTree", false),
        ("ReplicatedMergeTree", false),
        ("View", false),
        ("MaterializedView", false),
        ("Log", false),
        ("", false),
    ];

    for (engine, expected) in cases {
        assert_eq!(collapses(engine), expected, "engine {engine}");
    }
}
