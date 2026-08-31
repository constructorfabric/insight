use chrono::{Datelike, NaiveDate};
use serde::Serialize;

use super::forecast::{Lane, ScheduleItem, schedule};
use super::milestone::{Milestone, Placement, YearMonth};
use super::model::{Commitment, Gear};
use super::progress::LadderStep;
use crate::domain::external_links::ExternalSourceRegistry;

const GIT_PROVIDER: &str = "github";

const WINDOW_MONTHS: usize = 9;
const CAPACITY_MAN_DAYS_PER_PERSON: f64 = 1.0;

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub(crate) struct GearRoadmapResponse {
    pub(crate) capacity_man_days_per_person: f64,
    pub(crate) window_start: String,
    pub(crate) window_months: usize,
    pub(crate) gears: Vec<GearDto>,
    pub(crate) lanes: Vec<LaneDto>,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub(crate) struct GearDto {
    pub(crate) number: i64,
    pub(crate) title: String,
    pub(crate) subsystem: Option<String>,
    pub(crate) status_percent: Option<u8>,
    pub(crate) design_percent: Option<u8>,
    pub(crate) sdk_percent: Option<u8>,
    pub(crate) commitment: String,
    pub(crate) priority: Option<String>,
    pub(crate) effort_man_days: Option<f64>,
    pub(crate) remaining_man_days: Option<f64>,
    pub(crate) milestone: Option<String>,
    pub(crate) placement: String,
    pub(crate) slot: Option<usize>,
    pub(crate) assignees: Vec<String>,
    pub(crate) closed: bool,
    /// Absent when no configured external source claims the gear's repository.
    pub(crate) issue_url: Option<String>,
    pub(crate) assignee_urls: Vec<AssigneeLink>,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub(crate) struct AssigneeLink {
    pub(crate) login: String,
    pub(crate) url: Option<String>,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub(crate) struct LaneDto {
    pub(crate) assignee: Option<String>,
    pub(crate) spans: Vec<SpanDto>,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub(crate) struct SpanDto {
    pub(crate) gear_number: i64,
    pub(crate) start: String,
    pub(crate) end: String,
}

impl toolkit::api::api_dto::ResponseApiDto for GearRoadmapResponse {}

pub(crate) fn build(
    gears: &[Gear],
    today: NaiveDate,
    links: &ExternalSourceRegistry,
) -> GearRoadmapResponse {
    let window_start = YearMonth {
        year: today.year(),
        month: today.month(),
    };

    let items: Vec<ScheduleItem<'_>> = gears.iter().filter_map(schedule_item).collect();

    let lanes = schedule(&items, today)
        .into_iter()
        .map(lane_dto)
        .collect::<Vec<_>>();

    GearRoadmapResponse {
        capacity_man_days_per_person: CAPACITY_MAN_DAYS_PER_PERSON,
        window_start: format!("{:04}-{:02}", window_start.year, window_start.month),
        window_months: WINDOW_MONTHS,
        gears: gears
            .iter()
            .map(|gear| gear_dto(gear, window_start, links))
            .collect(),
        lanes,
    }
}

fn schedule_item(gear: &Gear) -> Option<ScheduleItem<'_>> {
    if gear.closed {
        return None;
    }

    Some(ScheduleItem {
        gear_number: gear.number,
        remaining_man_days: gear.remaining_man_days()?,
        assignee: gear.assignees.first().map(String::as_str),
    })
}

fn lane_dto(lane: Lane) -> LaneDto {
    LaneDto {
        assignee: lane.assignee,
        spans: lane
            .spans
            .into_iter()
            .map(|span| SpanDto {
                gear_number: span.gear_number,
                start: span.start.to_string(),
                end: span.end.to_string(),
            })
            .collect(),
    }
}

fn gear_dto(gear: &Gear, window_start: YearMonth, links: &ExternalSourceRegistry) -> GearDto {
    let (placement, slot) = placement_of(gear, window_start);

    GearDto {
        number: gear.number,
        title: gear.title.clone(),
        subsystem: gear.subsystem.clone(),
        status_percent: gear.status.and_then(LadderStep::percent_complete),
        design_percent: gear.design.and_then(LadderStep::percent_complete),
        sdk_percent: gear.sdk.and_then(LadderStep::percent_complete),
        commitment: commitment_label(gear.commitment).to_owned(),
        priority: gear.priority.clone(),
        effort_man_days: gear.effort_man_days,
        remaining_man_days: gear.remaining_man_days(),
        milestone: milestone_label(gear.milestone.as_ref()),
        placement,
        slot,
        assignees: gear.assignees.clone(),
        closed: gear.closed,
        issue_url: issue_url(gear, links),
        assignee_urls: gear
            .assignees
            .iter()
            .map(|login| AssigneeLink {
                login: login.clone(),
                url: links.account_href(GIT_PROVIDER, &gear.source_id, login),
            })
            .collect(),
    }
}

fn issue_url(gear: &Gear, links: &ExternalSourceRegistry) -> Option<String> {
    links
        .evidence_links(
            GIT_PROVIDER,
            &gear.source_id,
            "issue",
            Some(&gear.repo_full_name),
            Some(&format!("{}#{}", gear.repo_full_name, gear.number)),
        )
        .record
}

fn placement_of(gear: &Gear, window_start: YearMonth) -> (String, Option<usize>) {
    let Some(milestone) = gear.milestone.as_ref() else {
        return ("none".to_owned(), None);
    };

    let month = match milestone {
        Milestone::Due(month) => *month,
        Milestone::Backlog => return ("backlog".to_owned(), None),
        Milestone::Unrecognized(_) => return ("unrecognized".to_owned(), None),
    };

    match month.placement(window_start, WINDOW_MONTHS) {
        Placement::Overdue => ("overdue".to_owned(), None),
        Placement::Slot(slot) => ("slot".to_owned(), Some(slot)),
        Placement::Future => ("future".to_owned(), None),
    }
}

fn milestone_label(milestone: Option<&Milestone>) -> Option<String> {
    match milestone? {
        Milestone::Due(month) => Some(format!("{:04}-{:02}", month.year, month.month)),
        Milestone::Backlog => Some("backlog".to_owned()),
        Milestone::Unrecognized(title) => Some(title.clone()),
    }
}

fn commitment_label(commitment: Commitment) -> &'static str {
    match commitment {
        Commitment::Committed => "committed",
        Commitment::Planned => "planned",
        Commitment::Unstated => "unstated",
    }
}

#[cfg(test)]
#[expect(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "test setup panics on a broken fixture"
)]
mod tests {
    use chrono::NaiveDate;

    use super::{CAPACITY_MAN_DAYS_PER_PERSON, GearRoadmapResponse, WINDOW_MONTHS, build};
    use crate::domain::external_links::ExternalSourceRegistry;
    use crate::domain::gear_roadmap::model::{Gear, GearRow};

    fn build_for_test(gears: &[Gear], today: NaiveDate) -> GearRoadmapResponse {
        build(gears, today, &ExternalSourceRegistry::default())
    }

    fn today() -> NaiveDate {
        NaiveDate::from_ymd_opt(2030, 8, 1).expect("valid date")
    }

    fn gear(number: i64, milestone_title: &str, status: &str) -> Gear {
        Gear::from_row(GearRow {
            number,
            title: format!("CORE - Module {number}"),
            status: status.to_owned(),
            design: "Done".to_owned(),
            sdk: "N/A".to_owned(),
            commitment: "COMMITMENT".to_owned(),
            priority: "1 (1.1.1)".to_owned(),
            effort_man_days: 10.0,
            milestone_title: milestone_title.to_owned(),
            assignees: vec!["dev-one".to_owned()],
            closed: false,
            repo_full_name: "example-org/example-repo".to_owned(),
            source_id: "source-a".to_owned(),
        })
    }

    #[test]
    fn a_milestone_before_the_window_is_overdue_not_backlog() {
        let response = build_for_test(&[gear(1, "30.05", "Todo")], today());

        assert_eq!(response.gears[0].placement, "overdue");
        assert_eq!(response.gears[0].slot, None);
    }

    #[test]
    fn a_milestone_inside_the_window_carries_its_slot() {
        let response = build_for_test(&[gear(1, "30.09", "Todo")], today());

        assert_eq!(response.gears[0].placement, "slot");
        assert_eq!(response.gears[0].slot, Some(1));
    }

    #[test]
    fn the_response_states_the_capacity_it_assumed() {
        let response = build_for_test(&[], today());

        assert!(
            (response.capacity_man_days_per_person - CAPACITY_MAN_DAYS_PER_PERSON).abs()
                < f64::EPSILON
        );
        assert_eq!(response.window_months, WINDOW_MONTHS);
        assert_eq!(response.window_start, "2030-08");
    }

    #[test]
    fn a_configured_source_turns_a_gear_into_links() {
        let sources = [crate::config::ExternalSourceConfig {
            id: "source-a".to_owned(),
            provider: crate::config::ExternalSourceProvider::Github,
            web_base_url: "https://git.example.test/".to_owned(),
        }];
        let registry = ExternalSourceRegistry::new(&sources).expect("valid sources");

        let response = build(&[gear(41, "30.09", "Todo")], today(), &registry);

        assert_eq!(
            response.gears[0].issue_url.as_deref(),
            Some("https://git.example.test/example-org/example-repo/issues/41")
        );
        assert_eq!(
            response.gears[0].assignee_urls[0].url.as_deref(),
            Some("https://git.example.test/dev-one")
        );
    }

    #[test]
    fn a_finished_gear_takes_no_lane_time() {
        let response = build_for_test(&[gear(1, "30.09", "Done")], today());

        assert!(response.lanes.is_empty());
    }

    #[test]
    fn remaining_work_is_scheduled_from_today() {
        let response = build_for_test(&[gear(7, "30.09", "Todo")], today());

        assert_eq!(response.lanes.len(), 1);
        assert_eq!(response.lanes[0].assignee.as_deref(), Some("dev-one"));
        assert_eq!(response.lanes[0].spans[0].gear_number, 7);
        assert_eq!(response.lanes[0].spans[0].start, "2030-08-01");
        assert_eq!(response.lanes[0].spans[0].end, "2030-08-10");
    }
}
