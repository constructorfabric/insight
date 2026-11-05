use super::*;
use crate::domain::connector_schedules::test_support::acknowledgement;

#[test]
fn status_precedence_uses_the_current_revision_and_successful_observation() {
    for (name, revision, crons, applied, attempted, failed, expected) in [
        (
            "newer save",
            3,
            vec!["0 3 * * *"],
            Some(2),
            2,
            false,
            ScheduleStatus::Pending,
        ),
        (
            "current failure",
            2,
            vec!["0 3 * * *"],
            Some(2),
            2,
            true,
            ScheduleStatus::Error,
        ),
        (
            "older failure",
            3,
            vec!["0 3 * * *"],
            Some(2),
            2,
            true,
            ScheduleStatus::Pending,
        ),
        (
            "acknowledged clear",
            3,
            vec![],
            Some(3),
            3,
            false,
            ScheduleStatus::Inherited,
        ),
        (
            "unacknowledged clear",
            3,
            vec![],
            Some(2),
            2,
            false,
            ScheduleStatus::Pending,
        ),
        (
            "verified override",
            2,
            vec!["0 3 * * *"],
            Some(2),
            2,
            false,
            ScheduleStatus::Applied,
        ),
        (
            "failed first apply",
            2,
            vec!["0 3 * * *"],
            None,
            2,
            true,
            ScheduleStatus::Error,
        ),
        (
            "no successful observation",
            2,
            vec!["0 3 * * *"],
            None,
            1,
            true,
            ScheduleStatus::Pending,
        ),
        (
            "failed clear",
            3,
            vec![],
            Some(2),
            3,
            true,
            ScheduleStatus::Error,
        ),
    ] {
        let mut ack = acknowledgement();
        ack.applied_revision = applied;
        ack.attempted_revision = attempted;
        ack.last_error = failed.then(|| "synthetic readback failure".to_owned());
        let crons: Vec<String> = crons.into_iter().map(str::to_owned).collect();
        assert_eq!(
            status(revision, Some(&crons), Some(&ack)),
            expected,
            "{name}"
        );
    }
}

#[test]
fn missing_acknowledgements_without_an_override_are_inherited() {
    for revision in [0, 1, u64::MAX] {
        for requested in [None, Some([].as_slice())] {
            assert_eq!(
                status(revision, requested, None),
                ScheduleStatus::Inherited,
                "{revision}, {requested:?}"
            );
        }
    }
}

#[test]
fn requests_without_acknowledgements_are_pending() {
    let crons = ["0 3 * * *".to_owned()];
    for revision in [1, u64::MAX] {
        assert_eq!(
            status(revision, Some(&crons), None),
            ScheduleStatus::Pending,
            "{revision}"
        );
    }
}
