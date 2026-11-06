use super::Crons;

type R = Result<(), Box<dyn std::error::Error>>;

#[test]
fn accepts_standard_cron_fields_and_normalizes_horizontal_whitespace() -> R {
    for value in [
        "* * * * *",
        "0 0 1 1 0",
        "59 23 31 12 6",
        "*/15 0-23/2 * * MON-FRI",
        "1,5-10/2,*/20 3,6 * JAN,jul,Dec SUN,tue-SAT/2",
        "0 3 ? * MON",
        "0 3 * * ?",
        "0/2 0/3 1/4 1/2 0/2",
        "0 3 * jan-dec sun-sat",
        "0 0 ? * ?",
        "  0\t  3 *  * *  ",
    ] {
        let crons = Crons::parse(&[value.to_owned()])
            .map_err(|e| format!("should accept {value:?}: {e}"))?
            .into_vec();
        assert_eq!(crons, [super::normalize(value)], "should accept: {value:?}");
    }
    let ten = (0..10)
        .map(|minute| format!("{minute} * * * *"))
        .collect::<Vec<_>>();
    assert_eq!(Crons::parse(&ten)?.into_vec().len(), 10);
    Ok(())
}

#[test]
fn rejects_unsupported_or_out_of_range_cron_syntax_with_a_problem() {
    let long = format!("{}0 3 * * *", " ".repeat(100));
    for value in [
        "",
        "0 0 * *",
        "0 0 0 * * *",
        "0 0 0 * * * 2026",
        "@daily",
        "@every 1h",
        "TZ=UTC 0 3 * * *",
        "CRON_TZ=UTC 0 3 * * *",
        "0 3 L * *",
        "0 3 1W * *",
        "0 3 * * MON#2",
        "60 3 * * *",
        "0 24 * * *",
        "0 3 0 * *",
        "0 3 32 * *",
        "0 3 * 0 *",
        "0 3 * 13 *",
        "0 3 * * 7",
        "0 3 * FOO *",
        "0 3 * * FOO",
        "0 3 * * FRI-MON",
        "5-1 3 * * *",
        "0 3 * * MON,",
        "0,,1 3 * * *",
        ",0 3 * * *",
        "0 3 * * ?/2",
        "0 3 *,? * *",
        "? 3 * * *",
        "*/0 3 * * *",
        "*/-1 3 * * *",
        "*/ 3 * * *",
        "*/1/2 3 * * *",
        "1-2-3 3 * * *",
        "0 3 * * *\n",
        "0\r 3 * * *",
        "0\0 3 * * *",
        "0\u{7f} 3 * * *",
        "0\u{a0}3 * * *",
        "+0 3 * * *",
        "0 3 * * 99999999999999999999",
        "*/999999999999999999999 3 * * *",
        &long,
    ] {
        let result = Crons::parse(&[value.to_owned()]);
        assert!(result.is_err(), "should reject: {value:?}");
        assert!(
            !result
                .err()
                .map(|error| error.problem)
                .unwrap_or_default()
                .is_empty(),
            "must name problem: {value:?}"
        );
    }
    for count in [0, 11] {
        assert!(
            Crons::parse(&vec!["0 3 * * *".to_owned(); count]).is_err(),
            "should reject count: {count}"
        );
    }
}

#[test]
fn calendar_validity_uses_robfig_day_matching_and_star_bits() {
    for (value, runs) in [
        ("0 6 31 2 *", false),
        ("0 6 30-31 FEB ?", false),
        ("0 6 31 APR,JUN,SEP,NOV */1", false),
        ("0 6 31 FEB *,MON", false),
        ("0 6 31 FEB MON,*/1", false),
        ("0 6 29 FEB *", true),
        ("0 6 29/2 FEB *", true),
        ("0 6 30/2 FEB *", false),
        ("0 6 31 FEB-MAR *", true),
        ("0 6 31 2 MON", true),
        ("0 6 31 2 0-6", true),
        ("0 6 31 2 */2", true),
        ("0 6 31 2 */2,MON", true),
        ("0 6 */2 FEB MON", true),
        ("0 6 */1 FEB MON", true),
    ] {
        let result = Crons::parse(&[value.to_owned()]);
        assert_eq!(result.is_ok(), runs, "{value}: {result:?}");
        if !runs {
            let Err(problem) = result else {
                panic!("should reject: {value}");
            };
            assert_eq!(problem.field, "crons[0]", "{value}");
            assert!(problem.problem.contains("never runs"), "{problem}");
            assert!(problem.problem.contains("no listed month"), "{problem}");
        }
    }
}

#[test]
fn cron_errors_name_the_expression_and_the_cron_field() {
    for (value, name) in [
        ("60 6 * * *", "minute"),
        ("0 25 * * *", "hour"),
        ("0 6 32 * *", "day of month"),
        ("0 6 * 13 *", "month"),
        ("0 6 * * 7", "day of week"),
    ] {
        let Err(problem) = Crons::parse(&["0 6 * * *".to_owned(), value.to_owned()]) else {
            panic!("should reject: {value}");
        };
        assert_eq!(problem.field, "crons[1]", "{value}");
        for detail in ["expression 1", value, name, "outside"] {
            assert!(
                problem.problem.contains(detail),
                "{value}: missing {detail}: {problem}"
            );
        }
    }
}

#[test]
fn whitespace_normalized_duplicates_identify_both_expressions() {
    let Err(problem) = Crons::parse(&["0 6 * * *".to_owned(), " 0\t6 * * * ".to_owned()]) else {
        panic!("should reject duplicate");
    };
    assert_eq!(problem.field, "crons[1]");
    for detail in ["expression 1", "duplicates crons[0]", "0 6 * * *"] {
        assert!(
            problem.problem.contains(detail),
            "missing {detail}: {problem}"
        );
    }
}
