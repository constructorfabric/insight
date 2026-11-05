use super::*;

#[test]
fn status_compares_origin_and_ordered_whitespace_normalized_crons() {
    let crons = ["0 3 * * *".to_owned(), "0 6 * * *".to_owned()];
    for (requested, origin, applied_crons, expected) in [
        (true, None, vec![], ScheduleStatus::Pending),
        (true, Some("api"), crons.to_vec(), ScheduleStatus::Applied),
        (
            true,
            Some("api"),
            vec![" 0\t3 * * * ".to_owned(), "0 6 * * *".to_owned()],
            ScheduleStatus::Applied,
        ),
        (
            true,
            Some("annotation"),
            crons.to_vec(),
            ScheduleStatus::Pending,
        ),
        (
            true,
            Some("descriptor"),
            crons.to_vec(),
            ScheduleStatus::Pending,
        ),
        (
            true,
            Some("default"),
            crons.to_vec(),
            ScheduleStatus::Pending,
        ),
        (
            true,
            Some("unknown"),
            crons.to_vec(),
            ScheduleStatus::Pending,
        ),
        (
            true,
            Some("api"),
            vec![crons[1].clone(), crons[0].clone()],
            ScheduleStatus::Pending,
        ),
        (
            true,
            Some("api"),
            vec![crons[0].clone()],
            ScheduleStatus::Pending,
        ),
        (
            true,
            Some("api"),
            vec!["0 4 * * *".to_owned()],
            ScheduleStatus::Pending,
        ),
        (false, Some("api"), crons.to_vec(), ScheduleStatus::Pending),
        (
            false,
            Some("annotation"),
            crons.to_vec(),
            ScheduleStatus::Inherited,
        ),
        (
            false,
            Some("descriptor"),
            crons.to_vec(),
            ScheduleStatus::Inherited,
        ),
        (
            false,
            Some("default"),
            crons.to_vec(),
            ScheduleStatus::Inherited,
        ),
        (
            false,
            Some("unknown"),
            crons.to_vec(),
            ScheduleStatus::Inherited,
        ),
        (false, None, vec![], ScheduleStatus::Inherited),
    ] {
        let applied = origin.map(|origin| RecordedSchedule {
            crons: applied_crons,
            timezone: Some("UTC".to_owned()),
            origin: origin.to_owned(),
            suspended: true,
            inherited: None,
            recorded_at: DateTime::UNIX_EPOCH,
        });
        let requested = requested.then_some(crons.as_slice());
        assert_eq!(
            status(requested, applied.as_ref()),
            expected,
            "requested={requested:?}, applied={applied:?}"
        );
    }
}

#[test]
fn recorded_schedule_serializes_the_observed_timezone() -> Result<(), Box<dyn std::error::Error>> {
    for timezone in [Some("UTC"), None] {
        let schedule = RecordedSchedule {
            crons: vec!["0 6 * * *".to_owned()],
            timezone: timezone.map(str::to_owned),
            origin: "api".to_owned(),
            suspended: false,
            inherited: None,
            recorded_at: DateTime::UNIX_EPOCH,
        };
        let json = serde_json::to_value(schedule)?;
        assert_eq!(json["timezone"], serde_json::to_value(timezone)?);
    }
    Ok(())
}

#[test]
fn a_pass_that_may_predate_the_save_never_counts_as_not_applied() {
    let saved = DateTime::UNIX_EPOCH + chrono::Duration::hours(1);
    let requested = RequestedSchedule {
        crons: vec!["0 6 * * *".to_owned()],
        updated_by: Uuid::from_u128(1),
        updated_at: saved,
    };
    for (offsets, expected) in [
        (vec![], ScheduleStatus::Pending),
        (vec![15], ScheduleStatus::Pending),
        (vec![15, -15], ScheduleStatus::Pending),
        (vec![15, 0], ScheduleStatus::Pending),
        (vec![30, 15], ScheduleStatus::NotApplied),
    ] {
        let completed_passes: Vec<_> = offsets
            .iter()
            .map(|minutes| saved + chrono::Duration::minutes(*minutes))
            .collect();
        let (status, since) = observed_status(Some(&requested), None, &completed_passes);
        assert_eq!(
            status, expected,
            "completed_passes relative to save: {offsets:?}"
        );
        assert_eq!(
            since,
            (expected == ScheduleStatus::NotApplied).then(|| completed_passes[0]),
            "{offsets:?}"
        );
    }
}

#[test]
fn completed_passes_never_turn_applied_or_inherited_into_not_applied() {
    let saved = DateTime::UNIX_EPOCH;
    let completed_passes = [
        saved + chrono::Duration::minutes(30),
        saved + chrono::Duration::minutes(15),
    ];
    for crons in [vec!["0 6 * * *".to_owned()], vec![]] {
        let requested = RequestedSchedule {
            crons: crons.clone(),
            updated_by: Uuid::from_u128(1),
            updated_at: saved,
        };
        let applied = RecordedSchedule {
            crons,
            timezone: Some("UTC".to_owned()),
            origin: if requested.crons.is_empty() {
                "default"
            } else {
                "api"
            }
            .to_owned(),
            suspended: false,
            inherited: None,
            recorded_at: completed_passes[0],
        };
        let (status, since) = observed_status(Some(&requested), Some(&applied), &completed_passes);
        assert_ne!(status, ScheduleStatus::NotApplied);
        assert!(since.is_none());
    }
}
