use toolkit_canonical_errors::Problem;

use super::*;

type R = Result<(), Box<dyn std::error::Error>>;

fn refusal<K: SortKey>(sort: Option<&str>, direction: Option<&str>) -> serde_json::Value {
    let Err(error) = Order::<K>::parse(sort, direction) else {
        panic!("should refuse: sort={sort:?} direction={direction:?}")
    };

    serde_json::to_value(Problem::from(error)).unwrap_or_default()
}

#[test]
fn each_table_orders_only_by_its_own_columns() {
    for key in ["visits", "page_views", "last_seen"] {
        assert!(
            Order::<PeopleSort>::parse(Some(key), None).is_ok(),
            "people should accept: {key}"
        );
    }
    for key in ["views", "visitors"] {
        assert!(
            Order::<PagesSort>::parse(Some(key), None).is_ok(),
            "pages should accept: {key}"
        );
    }
    for key in ["opens", "people"] {
        assert!(
            Order::<ActionsSort>::parse(Some(key), None).is_ok(),
            "actions should accept: {key}"
        );
    }

    for key in ["views", "opens", "VISITS", "", "visits DESC; --"] {
        assert!(
            Order::<PeopleSort>::parse(Some(key), None).is_err(),
            "people should refuse: {key:?}"
        );
    }
    for key in ["visits", "opens", "path"] {
        assert!(
            Order::<PagesSort>::parse(Some(key), None).is_err(),
            "pages should refuse: {key:?}"
        );
    }
    for key in ["visits", "views", "target"] {
        assert!(
            Order::<ActionsSort>::parse(Some(key), None).is_err(),
            "actions should refuse: {key:?}"
        );
    }
}

#[test]
fn a_refused_sort_names_the_parameter_and_what_it_accepts() {
    let refused = refusal::<PagesSort>(Some("path"), None);

    assert_eq!(refused["status"], 400);
    assert_eq!(refused["context"]["field_violations"][0]["field"], "sort");
    assert!(refused.to_string().contains("views, visitors"), "{refused}");
}

#[test]
fn a_refused_direction_names_the_parameter() {
    for direction in ["down", "ASC", ""] {
        let refused = refusal::<PeopleSort>(Some("visits"), Some(direction));

        assert_eq!(refused["status"], 400, "should refuse: {direction:?}");
        assert_eq!(
            refused["context"]["field_violations"][0]["field"],
            "direction"
        );
    }
}

#[test]
fn no_params_keep_the_order_each_table_had_before_it_was_sortable() -> R {
    assert_eq!(
        Order::<PeopleSort>::parse(None, None)?.clause(""),
        "visits DESC, page_views DESC, person"
    );
    assert_eq!(
        Order::<PagesSort>::parse(None, None)?.clause(""),
        "views DESC, path"
    );
    assert_eq!(
        Order::<ActionsSort>::parse(None, None)?.clause(""),
        "opens DESC, event_name, target"
    );
    Ok(())
}

#[test]
fn every_order_ends_in_a_tie_break_so_a_refetch_returns_the_same_rows() -> R {
    let cases = [
        (
            Order::<PeopleSort>::parse(Some("visits"), Some("asc"))?.clause(""),
            "visits ASC, page_views ASC, person",
        ),
        (
            Order::<PeopleSort>::parse(Some("page_views"), Some("desc"))?.clause(""),
            "page_views DESC, visits DESC, person",
        ),
        (
            Order::<PeopleSort>::parse(Some("last_seen"), Some("asc"))?.clause(""),
            "last_ts ASC, person",
        ),
        (
            Order::<PagesSort>::parse(Some("visitors"), Some("asc"))?.clause(""),
            "visitors ASC, path",
        ),
        (
            Order::<ActionsSort>::parse(Some("people"), Some("desc"))?.clause(""),
            "people DESC, event_name, target",
        ),
    ];

    for (clause, want) in cases {
        assert_eq!(clause, want);
    }
    Ok(())
}

#[test]
fn a_direction_alone_flips_the_default_column() -> R {
    assert_eq!(
        Order::<PagesSort>::parse(None, Some("asc"))?.clause(""),
        "views ASC, path"
    );
    Ok(())
}

#[test]
fn a_clause_names_its_columns_through_a_subquery_alias() -> R {
    assert_eq!(
        Order::<PeopleSort>::parse(Some("last_seen"), Some("desc"))?.clause("u."),
        "u.last_ts DESC, u.person"
    );
    Ok(())
}
