use serde_json::json;

use super::*;

fn places(body: &serde_json::Value) -> Vec<String> {
    check(body)
        .into_iter()
        .map(|violation| violation.field)
        .collect()
}

#[test]
fn a_declaration_that_reads_as_one_has_nothing_to_say_about_its_shape() {
    let sound = json!({
        "title": "Commits",
        "description": "one per commit",
        "source": { "kind": "stream" },
        "fields": [
            {
                "name": "day", "path": "day", "type": "datetime",
                "role": "time", "default_clock": true
            },
            {
                "name": "author", "path": "who.email", "type": "string",
                "person": "email", "absent_value": "unknown"
            }
        ],
        "row_identity": ["author"]
    });

    assert!(places(&sound).is_empty(), "{:?}", check(&sound));
}

/// The reason this check exists: a body the deserialiser stops on used to be
/// answered with one message about the text, and every other mistake in it
/// went unsaid.
#[test]
fn a_type_nothing_knows_does_not_hide_the_rest_of_the_body() {
    let body = json!({
        "title": "Commits",
        "source": { "kind": "stream" },
        "fields": [
            { "name": "day", "path": "day", "type": "date" },
            { "name": "lines", "path": "lines", "type": "int", "colour": "red" }
        ],
        "row_identity": [7]
    });

    assert_eq!(
        places(&body),
        vec!["fields[0].type", "fields[1].colour", "row_identity[0]"]
    );
}

#[test]
fn every_place_a_field_can_be_wrong_is_named() {
    let cases = [
        (
            json!({ "name": 1, "path": "a", "type": "int" }),
            "fields[0].name",
        ),
        (json!({ "path": "a", "type": "int" }), "fields[0].name"),
        (json!({ "name": "a", "type": "int" }), "fields[0].path"),
        (json!({ "name": "a", "path": "a" }), "fields[0].type"),
        (
            json!({ "name": "a", "path": "a", "type": "int", "role": "pivot" }),
            "fields[0].role",
        ),
        (
            json!({ "name": "a", "path": "a", "type": "int", "person": "name" }),
            "fields[0].person",
        ),
        (
            json!({ "name": "a", "path": "a", "type": "int", "default_clock": "yes" }),
            "fields[0].default_clock",
        ),
        (
            json!({ "name": "a", "path": "a", "type": "int", "absent_value": 0 }),
            "fields[0].absent_value",
        ),
    ];

    for (field, at) in cases {
        let body = json!({
            "title": "Commits",
            "source": { "kind": "stream" },
            "fields": [field.clone()]
        });

        assert_eq!(
            places(&body),
            vec![at],
            "should be refused at {at}: {field}"
        );
    }
}

#[test]
fn a_body_that_is_not_a_declaration_at_all_is_said_so_once() {
    assert_eq!(places(&json!([])), vec!["body"]);
    // A body saying neither what it is over nor what it holds is told both,
    // rather than sending a reader round once per missing half.
    assert_eq!(
        places(&json!({ "title": "Commits" })),
        vec!["source", "fields"]
    );
    assert_eq!(
        places(&json!({ "title": "Commits", "source": { "kind": "stream" }, "fields": {} })),
        vec!["fields"]
    );
    assert_eq!(
        places(&json!({
            "title": "Commits",
            "source": { "kind": "stream" },
            "fields": [],
            "extra": 1
        })),
        vec!["extra"]
    );
}

/// What is admissible is said in the refusal, so a form can offer it.
#[test]
fn a_refusal_names_what_would_be_admissible() {
    let body = json!({
        "title": "Commits",
        "source": { "kind": "stream" },
        "fields": [{ "name": "a", "path": "a", "type": "date" }]
    });
    let said = check(&body)
        .into_iter()
        .map(|violation| violation.detail)
        .collect::<Vec<_>>()
        .join(" ");

    for admissible in FieldType::ALL {
        assert!(
            said.contains(admissible),
            "should offer {admissible}: {said}"
        );
    }
}
