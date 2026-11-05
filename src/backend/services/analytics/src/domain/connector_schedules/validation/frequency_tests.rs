use super::Crons;

type R = Result<(), Box<dyn std::error::Error>>;

fn strings(crons: &[&str]) -> Vec<String> {
    crons.iter().map(|cron| (*cron).to_owned()).collect()
}

#[test]
fn shortest_gap_uses_the_whole_schedule_and_the_gregorian_calendar() -> R {
    for (crons, gap) in [
        (vec!["*/30 * * * *"], None),
        (vec!["0,30 * * * *"], None),
        (vec!["0 * * * *", "30 * * * *"], None),
        (vec!["0 6 * * *"], None),
        (vec!["* * * * *"], Some(1)),
        (vec!["*/15 * * * *"], Some(15)),
        (vec!["0,20 * * * *"], Some(20)),
        (vec!["0 * * * *", "10 * * * *"], Some(10)),
        (vec!["50 23 * * *", "5 0 * * *"], Some(15)),
        (vec!["0 23 * * 1", "10 23 * * 3"], None),
        (vec!["55 23 31 * *", "5 0 1 * *"], Some(10)),
        (vec!["55 23 28 2 *", "5 0 1 3 *"], Some(10)),
        (vec!["55 23 29 2 *", "5 0 1 3 *"], Some(10)),
        (vec!["55 23 30 4 *", "5 0 2 5 *"], None),
        (vec!["55 23 31 12 *", "5 0 1 1 *"], Some(10)),
        (vec!["0 * * * *", "0 0 * * *"], Some(0)),
        (vec!["0,20,21 * * * *"], Some(1)),
        (vec!["0 23 31 2 MON", "10 23 31 2 WED"], None),
        (vec!["50 23 */1 * MON", "5 0 * * TUE"], Some(15)),
        (vec!["50 23 */2 * MON", "5 0 * * TUE"], Some(15)),
        (vec!["50 23 31 FEB MON", "5 0 * MAR TUE"], Some(15)),
        (vec!["50 23 31 FEB-MAR *,MON", "5 0 1 MAR TUE"], None),
    ] {
        let result = Crons::parse(&strings(&crons), 30);
        if let Some(gap) = gap {
            let Err(problem) = result else {
                panic!("should reject: {crons:?}");
            };
            assert!(
                problem.problem.contains(&format!("{gap} minutes apart")),
                "{crons:?}: {problem}"
            );
        } else {
            result.map_err(|problem| format!("should accept: {crons:?}: {problem}"))?;
        }
    }
    Ok(())
}

#[test]
fn equality_and_stand_minimum_are_respected() {
    for (cron, minimum, allowed) in [
        ("0,30 * * * *", 30, true),
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
fn single_expression_refusal_has_starts_and_one_weekday() {
    assert_refusal(
        &["*/15 * * * *"],
        "crons[0]",
        "expression 0 \"*/15 * * * *\" has starts 15 minutes apart (Sat 00:00 and 00:15 UTC); a schedule may start at most once every 30 minutes",
    );
}

#[test]
fn same_day_refusal_names_the_weekday_once() {
    for (crons, field, expected) in [
        (
            ["0 * * * *", "10 * * * *"],
            "crons[1]",
            "expressions 0 \"0 * * * *\" and 1 \"10 * * * *\" start 10 minutes apart (Sat 00:00 and 00:10 UTC); a schedule may start at most once every 30 minutes",
        ),
        (
            ["10 * * * *", "0 * * * *"],
            "crons[0]",
            "expressions 1 \"0 * * * *\" and 0 \"10 * * * *\" start 10 minutes apart (Sat 00:00 and 00:10 UTC); a schedule may start at most once every 30 minutes",
        ),
    ] {
        assert_refusal(&crons, field, expected);
    }
}

#[test]
fn across_midnight_refusal_names_both_weekdays() {
    for (crons, expected) in [
        (
            ["50 23 * * *", "5 0 * * *"],
            "expressions 0 \"50 23 * * *\" and 1 \"5 0 * * *\" start 15 minutes apart (Sat 23:50 and Sun 00:05 UTC); a schedule may start at most once every 30 minutes",
        ),
        (
            ["50 23 * * MON", "5 0 * * TUE"],
            "expressions 0 \"50 23 * * MON\" and 1 \"5 0 * * TUE\" start 15 minutes apart (Mon 23:50 and Tue 00:05 UTC); a schedule may start at most once every 30 minutes",
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

#[test]
#[ignore = "release timing: run explicitly with --release --ignored --nocapture"]
fn ten_passing_expressions_timing() -> R {
    let crons = (0..10)
        .map(|index| {
            let hours = (index..24)
                .step_by(10)
                .map(|hour| hour.to_string())
                .collect::<Vec<_>>()
                .join(",");
            let days = (1..=31)
                .filter(|day| (day * (index + 3) + index * 7) % 11 < 5)
                .take(12)
                .map(|day| day.to_string())
                .collect::<Vec<_>>()
                .join(",");
            let months = (1..=12)
                .filter(|month| *month == 1 || (month * (index + 5) + index * 3) % 7 < 5)
                .map(|month| month.to_string())
                .collect::<Vec<_>>()
                .join(",");
            format!("0,30 {hours} {days} {months} {}", index % 7)
        })
        .collect::<Vec<_>>();
    let start = std::time::Instant::now();
    let mut slowest = std::time::Duration::ZERO;
    for _ in 0..100 {
        let iteration = std::time::Instant::now();
        std::hint::black_box(Crons::parse(std::hint::black_box(&crons), 30)?);
        slowest = slowest.max(iteration.elapsed());
    }
    eprintln!(
        "ten passing expressions: {:?} mean, {slowest:?} maximum (100 iterations)",
        start.elapsed() / 100,
    );
    Ok(())
}
