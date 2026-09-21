use serde_json::json;

use super::*;

fn declaration(value: serde_json::Value) -> Declaration {
    serde_json::from_value(value).unwrap_or_else(|error| panic!("the fixture parses: {error}"))
}

fn commits() -> Declaration {
    declaration(json!({
        "title": "Commits",
        "description": "One record per commit.",
        "source": { "kind": "stream" },
        "fields": [
            {
                "name": "day", "path": "committed\\.at", "type": "datetime",
                "role": "time", "default_clock": true
            },
            {
                "name": "author", "path": "author.email", "type": "string",
                "role": "dimension", "absent_value": "unknown",
                "description": "Who wrote it"
            },
            { "name": "lines", "path": "changed.lines", "type": "int", "role": "measurable" }
        ],
        "row_identity": ["author"]
    }))
}

#[test]
fn a_reader_is_told_what_the_records_mean() {
    let described = describe("commits", &commits());

    assert!(described.contains("commits: Commits"), "{described}");
    assert!(described.contains("One record per commit."), "{described}");
    assert!(described.contains("day (date and time)"), "{described}");
    assert!(described.contains("the record's main date"), "{described}");
    assert!(described.contains("something to measure"), "{described}");
    assert!(described.contains("Who wrote it"), "{described}");
    assert!(
        described.contains("shown as `unknown` where it is missing"),
        "{described}"
    );
}

#[test]
fn a_reader_is_told_which_records_count_as_one() {
    let described = describe("commits", &commits());

    assert!(
        described.contains("the same record when they agree on: author"),
        "{described}"
    );
}

#[test]
fn where_a_value_sits_and_what_holds_it_are_left_out() {
    let described = describe("commits", &commits());

    for hidden in [
        "committed",
        "author.email",
        "changed.lines",
        "raw_data",
        "path",
    ] {
        assert!(
            !described.contains(hidden),
            "`{hidden}` is this service's own business: {described}"
        );
    }
}

#[test]
fn a_dataset_with_no_identity_says_nothing_about_it() {
    let plain = declaration(json!({
        "title": "Events",
        "source": { "kind": "stream" },
        "fields": [{ "name": "day", "path": "day", "type": "datetime" }]
    }));

    let described = describe("events", &plain);

    assert!(!described.contains("the same record"), "{described}");
}

#[test]
fn with_nothing_declared_a_reader_is_told_who_declares_one() {
    let described = describe_all(&[]);

    assert!(described.contains("No datasets"), "{described}");
    assert!(described.contains("administrator"), "{described}");
}

#[test]
fn every_declared_dataset_is_described() {
    let described = describe_all(&[
        ("commits".to_owned(), commits()),
        ("events".to_owned(), commits()),
    ]);

    assert!(described.contains("commits: Commits"), "{described}");
    assert!(described.contains("events: Commits"), "{described}");
}
