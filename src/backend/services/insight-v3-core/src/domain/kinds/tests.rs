use serde_json::json;

use super::*;

/// A dataset store holding nothing, for the kinds that never read one.
fn datasets() -> crate::store::datasets::memory::MemoryDatasets {
    crate::store::datasets::memory::MemoryDatasets::at(chrono::Utc::now())
}

/// Where a metric would be compiled, for the kinds that are never compiled.
fn nowhere() -> crate::domain::query::metric_query::People {
    crate::domain::query::metric_query::People::new("identity")
}

#[test]
fn a_widget_names_the_metric_it_draws() {
    let body = json!({ "type": "stat", "metric": "commits", "value": "total" });

    assert_eq!(
        refers_to(DefinitionKind::Widget, &body),
        vec![Reference::new(DefinitionKind::Metric, "commits")]
    );
}

#[test]
fn a_dashboard_names_its_widgets_in_either_form() {
    let listed = json!({ "items": [{ "widget": "a" }, { "heading": "b" }] });
    let shorthand = json!({ "widgets": ["a"] });

    let expected = vec![Reference::new(DefinitionKind::Widget, "a")];

    assert_eq!(refers_to(DefinitionKind::Dashboard, &listed), expected);
    assert_eq!(refers_to(DefinitionKind::Dashboard, &shorthand), expected);
}

#[test]
fn a_metric_names_no_definition() {
    let body = json!({ "table": "events", "fields": [] });

    assert!(refers_to(DefinitionKind::Metric, &body).is_empty());
}

#[test]
fn renaming_leaves_a_body_that_never_named_the_old_name_alone() {
    let cases = [
        (
            DefinitionKind::Widget,
            json!({ "type": "stat", "metric": "other", "value": "total" }),
        ),
        (DefinitionKind::Dashboard, json!({ "widgets": ["other"] })),
        (DefinitionKind::Metric, json!({ "table": "old" })),
    ];

    for (kind, body) in cases {
        assert_eq!(
            rename_reference(kind, body.clone(), "old", "new"),
            body,
            "should be untouched: {kind:?}"
        );
    }
}

#[test]
fn renaming_rewrites_every_place_a_kind_may_name_another() {
    let cases = [
        (
            DefinitionKind::Widget,
            json!({ "type": "stat", "metric": "old", "value": "total" }),
            json!({ "type": "stat", "metric": "new", "value": "total" }),
        ),
        (
            DefinitionKind::Dashboard,
            json!({ "items": [{ "widget": "old" }] }),
            json!({ "items": [{ "widget": "new" }] }),
        ),
        (
            DefinitionKind::Dashboard,
            json!({ "widgets": ["old", "kept"] }),
            json!({ "widgets": ["new", "kept"] }),
        ),
        (
            DefinitionKind::Dashboard,
            json!({ "widgets": "old" }),
            json!({ "widgets": "new" }),
        ),
    ];

    for (kind, before, after) in cases {
        assert_eq!(
            rename_reference(kind, before.clone(), "old", "new"),
            after,
            "should be rewritten: {before}"
        );
    }
}

#[test]
fn only_the_kinds_that_can_name_a_kind_are_worth_reading() {
    assert_eq!(
        referred_to_by(DefinitionKind::Metric),
        &[DefinitionKind::Widget]
    );
    assert_eq!(
        referred_to_by(DefinitionKind::Widget),
        &[DefinitionKind::Dashboard]
    );
    assert!(referred_to_by(DefinitionKind::Dashboard).is_empty());
}

#[test]
fn every_kind_a_body_names_lists_the_naming_kind_among_its_holders() {
    let cases = [
        (
            DefinitionKind::Metric,
            json!({ "table": "events", "fields": [] }),
        ),
        (
            DefinitionKind::Widget,
            json!({ "type": "stat", "metric": "commits", "value": "total" }),
        ),
        (
            DefinitionKind::Dashboard,
            json!({ "items": [{ "widget": "chart" }] }),
        ),
    ];

    for (kind, body) in cases {
        for reference in refers_to(kind, &body) {
            assert!(
                referred_to_by(reference.kind).contains(&kind),
                "{kind:?} names {reference:?}, so it must be among that kind's holders"
            );
        }
    }
}

#[tokio::test]
async fn a_dashboard_is_checked_through_the_kind_it_was_stored_under() {
    let body = json!({ "time_ranges": ["since_the_beginning"] });
    let store = crate::store::definitions::memory::MemoryDefinitions::new();

    let people = nowhere();
    let over = metric::CompileAgainst {
        database: "insight_datasets",
        people: &people,
    };
    let refusal = check(DefinitionKind::Dashboard, &body, &store, &datasets(), over).await;

    assert!(
        matches!(refusal, Err(KindError::Range(_))),
        "got {refusal:?}"
    );
}
