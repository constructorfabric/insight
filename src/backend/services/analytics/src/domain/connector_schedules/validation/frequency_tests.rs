use super::Crons;

fn strings(crons: &[&str]) -> Vec<String> {
    crons.iter().map(|cron| (*cron).to_owned()).collect()
}

fn assert_gap(crons: &[&str], minimum: u16, gap: Option<u16>) {
    let result = Crons::parse(&strings(crons), minimum);
    if let Some(gap) = gap {
        let Err(problem) = result else {
            panic!("should reject: {crons:?}");
        };
        assert!(
            problem.problem.contains(&format!("{gap} minutes apart")),
            "{crons:?}: {problem}"
        );
    } else {
        assert!(result.is_ok(), "should accept: {crons:?}: {result:?}");
    }
}

#[test]
fn adjacent_gaps_use_the_closest_sorted_starts() {
    for (cron, gap) in [
        ("0,20 * * * *", 20),
        ("0,20,21 * * * *", 1),
        ("45,0,15 6 * * *", 15),
    ] {
        assert_gap(&[cron], 30, Some(gap));
    }
}

#[test]
fn midnight_wrap_is_included_in_the_shortest_gap() {
    for crons in [vec!["50 23 * * *", "5 0 * * *"], vec!["5,50 0,23 * * *"]] {
        assert_gap(&crons, 30, Some(15));
        assert_gap(&crons, 15, None);
    }
}

#[test]
fn coincident_starts_in_different_expressions_have_zero_gap() {
    for crons in [["0 * * * *", "0 0 * * *"], ["0 6 * * MON", "0 6 * * TUE"]] {
        assert_gap(&crons, 30, Some(0));
    }
}

#[test]
fn a_single_daily_start_has_a_full_day_gap() {
    for cron in ["0 6 * * *", "59 23 * * *", "0 6 29 FEB *"] {
        assert_gap(&[cron], 1440, None);
        assert_gap(&[cron], 1441, Some(1440));
    }
}

#[test]
fn every_hour_starts_have_sixty_minute_gaps() {
    for cron in ["0 * * * *", "59 * * * *"] {
        assert_gap(&[cron], 60, None);
        assert_gap(&[cron], 61, Some(60));
    }
}

#[test]
fn date_disjoint_starts_are_conservatively_refused() {
    for crons in [
        ["0 23 * * MON", "10 23 * * WED"],
        ["0 23 1 * *", "10 23 2 * *"],
        ["0 23 * JAN *", "10 23 * FEB *"],
        ["0 23 31 FEB MON", "10 23 31 FEB WED"],
    ] {
        assert_gap(&crons, 30, Some(10));
    }
}

#[test]
fn step_and_range_fields_expand_into_all_daily_starts() {
    for (cron, minimum, gap) in [
        ("*/30 * * * *", 30, None),
        ("*/30 * * * *", 31, Some(30)),
        ("0-59/20 * * * *", 30, Some(20)),
        ("0 9-17 * * *", 60, None),
        ("0 9-17 * * *", 61, Some(60)),
    ] {
        assert_gap(&[cron], minimum, gap);
    }
}

#[test]
fn several_expressions_share_one_sorted_day() {
    for (crons, gap) in [
        (vec!["0 12 * * *", "15 6 * * *", "0 6 * * *"], Some(15)),
        (
            vec!["0 18 * * *", "30 6 * * *", "0 6 * * *", "30 18 * * *"],
            None,
        ),
        (
            vec!["0 12 * * *", "0 18 * * *", "0 0 * * *", "10 0 * * *"],
            Some(10),
        ),
    ] {
        assert_gap(&crons, 30, gap);
    }
}

#[test]
fn equality_and_stand_minimum_are_respected() {
    for (cron, minimum, allowed) in [
        ("0,29 * * * *", 30, false),
        ("0,30 * * * *", 30, true),
        ("0,30 * * * *", 31, false),
        ("0,30 * * * *", 60, false),
        ("0 * * * *", 60, true),
        ("* * * * *", 1, true),
        ("0 6 * * *", 1440, true),
        ("0 6,18 * * *", 1440, false),
    ] {
        assert_eq!(
            Crons::parse(&strings(&[cron]), minimum).is_ok(),
            allowed,
            "{cron}: minimum {minimum}"
        );
    }
}

fn assert_refusal(crons: &[&str], field: &str, expected: &str) {
    let Err(problem) = Crons::parse(&strings(crons), 30) else {
        panic!("should reject frequency: {crons:?}");
    };
    assert_eq!(problem.field, field, "{crons:?}");
    assert_eq!(problem.problem, expected, "{crons:?}");
}

#[test]
fn single_expression_refusal_has_starts_and_utc() {
    assert_refusal(
        &["*/15 * * * *"],
        "crons[0]",
        "expression 0 \"*/15 * * * *\" has starts 15 minutes apart (00:00 and 00:15 UTC); a schedule may start at most once every 30 minutes",
    );
}

#[test]
fn same_day_refusal_names_both_expressions() {
    for (crons, field, expected) in [
        (
            ["0 * * * *", "10 * * * *"],
            "crons[1]",
            "expressions 0 \"0 * * * *\" and 1 \"10 * * * *\" start 10 minutes apart (00:00 and 00:10 UTC); a schedule may start at most once every 30 minutes",
        ),
        (
            ["10 * * * *", "0 * * * *"],
            "crons[0]",
            "expressions 1 \"0 * * * *\" and 0 \"10 * * * *\" start 10 minutes apart (00:00 and 00:10 UTC); a schedule may start at most once every 30 minutes",
        ),
    ] {
        assert_refusal(&crons, field, expected);
    }
}

#[test]
fn across_midnight_refusal_names_both_times() {
    for (crons, expected) in [
        (
            ["50 23 * * *", "5 0 * * *"],
            "expressions 0 \"50 23 * * *\" and 1 \"5 0 * * *\" start 15 minutes apart (23:50 and 00:05 UTC); a schedule may start at most once every 30 minutes",
        ),
        (
            ["50 23 * * MON", "5 0 * * TUE"],
            "expressions 0 \"50 23 * * MON\" and 1 \"5 0 * * TUE\" start 15 minutes apart (23:50 and 00:05 UTC); a schedule may start at most once every 30 minutes",
        ),
    ] {
        assert_refusal(&crons, "crons[1]", expected);
    }
}

#[test]
fn all_expression_errors_take_precedence_over_frequency() {
    for (cron, reason) in [("0 6 31 2 *", "never runs"), ("0 25 * * *", "hour")] {
        let Err(problem) = Crons::parse(&strings(&["* * * * *", cron]), 30) else {
            panic!("should reject expression: {cron}");
        };
        assert_eq!(problem.field, "crons[1]", "{cron}");
        assert!(problem.problem.contains(reason), "{cron}: {problem}");
        assert!(
            !problem.problem.contains("minutes apart"),
            "{cron}: {problem}"
        );
    }
}
