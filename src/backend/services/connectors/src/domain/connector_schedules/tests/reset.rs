use super::*;

#[test]
fn reset_requires_matching_crons_timezone_and_origin_before_reporting_default() {
    let target = InheritedSchedule {
        crons: vec!["0 4 * * *".to_owned()],
        origin: ScheduleOrigin::Descriptor,
    };
    for (cron, timezone, origin) in [
        ("0 6 * * *", Some("UTC"), ScheduleOrigin::Descriptor),
        ("0 4 * * *", None, ScheduleOrigin::Descriptor),
        (
            "0 4 * * *",
            Some("Europe/London"),
            ScheduleOrigin::Descriptor,
        ),
        ("0 4 * * *", Some("UTC"), ScheduleOrigin::Annotation),
        ("0 4 * * *", Some("UTC"), ScheduleOrigin::Default),
    ] {
        let mut observed = live(origin, cron);
        observed.timezone = timezone.map(str::to_owned);
        assert_eq!(
            dto::status(&[], Some(&observed), Some(&target)),
            (
                ScheduleStatus::Pending,
                Some(PendingReason::DifferentSchedule)
            ),
            "{observed:?}"
        );
    }
    assert_eq!(
        dto::status(
            &[],
            Some(&live(ScheduleOrigin::Descriptor, "0 4 * * *")),
            None,
        ),
        (
            ScheduleStatus::Pending,
            Some(PendingReason::AwaitingReconcile)
        )
    );
}
