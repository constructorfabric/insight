use serde_json::json;

use super::*;

fn heading(text: &str) -> Item {
    Item::Heading(HeadingItem {
        heading: text.to_owned(),
    })
}

fn widget(name: &str) -> Item {
    Item::Widget(WidgetItem {
        widget: name.to_owned(),
    })
}

#[test]
fn a_board_names_its_widgets_in_either_form() {
    assert_eq!(
        widgets(&json!({ "widgets": ["one", "two"] })),
        ["one", "two"]
    );
    assert_eq!(
        widgets(&json!({
            "items": [
                { "heading": "Per person" },
                { "widget": "one" },
                { "text": "Bots excluded." },
                { "widget": "two" }
            ]
        })),
        ["one", "two"]
    );
}

#[test]
fn a_rename_follows_a_widget_into_the_item_list() {
    let after = renamed(
        json!({
            "title": "Engineering",
            "items": [{ "heading": "Commits" }, { "widget": "old" }]
        }),
        "old",
        "new",
    );

    assert_eq!(
        after,
        json!({
            "title": "Engineering",
            "items": [{ "heading": "Commits" }, { "widget": "new" }]
        })
    );
}

#[test]
fn laying_a_board_out_keeps_its_title_and_drops_the_shorthand() {
    let laid = laid_out(
        &json!({ "title": "Engineering", "widgets": ["one", "two"] }),
        &[heading("Per person"), widget("two")],
    )
    .unwrap_or_else(|error| panic!("serialises: {error}"));

    assert_eq!(
        laid,
        json!({
            "title": "Engineering",
            "items": [{ "heading": "Per person" }, { "widget": "two" }]
        })
    );
}

#[test]
fn an_item_naming_two_things_at_once_is_refused() {
    let mixed: Result<Item, _> =
        serde_json::from_value(json!({ "widget": "one", "heading": "Two" }));

    assert!(mixed.is_err(), "{mixed:?}");
}
