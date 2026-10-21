use uuid::Uuid;

use super::*;

#[test]
fn listing_and_counting_pins_keep_only_dashboards_the_person_sees() {
    let person = PersonId::new(Uuid::from_u128(7));

    for template in [PINS_OF, COUNT_PINS] {
        let statement = visible_pins(template, person);

        assert!(statement.sql.contains(VISIBLE), "{}", statement.sql);
        assert!(
            statement.sql.contains("dashboard_pins.person = ?"),
            "{}",
            statement.sql
        );
        let bound = statement.values.as_ref().map_or(0, |values| values.0.len());
        assert_eq!(
            bound,
            statement.sql.matches('?').count(),
            "{}",
            statement.sql
        );
    }
}

#[test]
fn a_pin_lands_only_on_a_dashboard_the_person_sees() {
    let person = PersonId::new(Uuid::from_u128(7));
    let dashboard = DefinitionId::parse("delivery").unwrap_or_else(|error| panic!("{error}"));

    let statement = insert_pin(person, &dashboard);

    assert!(statement.sql.contains(VISIBLE), "{}", statement.sql);
    assert!(
        statement.sql.contains("FROM dashboards"),
        "{}",
        statement.sql
    );
    let bound = statement.values.as_ref().map_or(0, |values| values.0.len());
    assert_eq!(
        bound,
        statement.sql.matches('?').count(),
        "{}",
        statement.sql
    );
}
