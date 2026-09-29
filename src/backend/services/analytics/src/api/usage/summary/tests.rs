use super::*;

#[test]
fn every_caller_value_in_a_read_is_a_placeholder() {
    let visitors = "V";
    for sql in [totals_sql(visitors), by_day_sql(visitors)] {
        assert_eq!(
            sql.matches('?').count(),
            3,
            "a read that does not bind exactly the window has interpolated a value: {sql}"
        );
    }
}
