use std::collections::HashMap;

use chrono::{Datelike, NaiveDate};
use serde::Serialize;

use super::forecast::{Lane, ScheduleItem, schedule};
use super::milestone::{Milestone, Placement as MonthPlacement, YearMonth};
use super::model::{Commitment, Gear};
use super::progress::LadderStep;
use super::sort::{Sort, order};
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
    pub(crate) placement: Placement,
    pub(crate) assignees: Vec<String>,
    pub(crate) closed: bool,
    /// Absent when no configured external source claims the gear's repository.
    pub(crate) issue_url: Option<String>,
    pub(crate) assignee_urls: Vec<AssigneeLink>,
}

/// Where a gear sits against the month window. Only `slot` carries an index
/// and only `overdue` a day count, so no other state can claim either.
///
/// A milestone names a month, not a day, so `days` counts from the day after
/// that month ended.
#[derive(Debug, Serialize, utoipa::ToSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum Placement {
    Slot { slot: usize },
    Overdue { days: i64 },
    Future,
    Backlog,
    Unrecognized,
    None,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub(crate) struct AssigneeLink {
    pub(crate) login: String,
    pub(crate) url: Option<String>,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub(crate) struct LaneDto {
    pub(crate) assignee: Option<String>,
    /// Absent for an unassigned lane, and where no configured source knows the
    /// account.
    pub(crate) assignee_url: Option<String>,
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
    sort: Sort,
    links: &ExternalSourceRegistry,
) -> GearRoadmapResponse {
    let window_start = YearMonth {
        year: today.year(),
        month: today.month(),
    };

    let items: Vec<ScheduleItem<'_>> = gears.iter().filter_map(schedule_item).collect();

    let sources = source_by_login(gears);

    let lanes = schedule(&items, today)
        .into_iter()
        .map(|lane| lane_dto(lane, &sources, links))
        .collect::<Vec<_>>();

    let mut ordered: Vec<&Gear> = gears.iter().collect();
    order(&mut ordered, sort, |gear| {
        placement_of(gear, window_start, today)
    });

    GearRoadmapResponse {
        capacity_man_days_per_person: CAPACITY_MAN_DAYS_PER_PERSON,
        window_start: format!("{:04}-{:02}", window_start.year, window_start.month),
        window_months: WINDOW_MONTHS,
        gears: ordered
            .into_iter()
            .map(|gear| gear_dto(gear, window_start, today, links, &sources))
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

fn lane_dto(lane: Lane, sources: &HashMap<&str, &str>, links: &ExternalSourceRegistry) -> LaneDto {
    let assignee_url = lane
        .assignee
        .as_deref()
        .and_then(|login| account_href(login, sources, links));

    LaneDto {
        assignee: lane.assignee,
        assignee_url,
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

fn gear_dto(
    gear: &Gear,
    window_start: YearMonth,
    today: NaiveDate,
    links: &ExternalSourceRegistry,
    sources: &HashMap<&str, &str>,
) -> GearDto {
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
        placement: placement_of(gear, window_start, today),
        assignees: gear.assignees.clone(),
        closed: gear.closed,
        issue_url: issue_url(gear, links),
        assignee_urls: gear
            .assignees
            .iter()
            .map(|login| AssigneeLink {
                login: login.clone(),
                url: account_href(login, sources, links),
            })
            .collect(),
    }
}

/// A login is recorded per gear, not per person, so the source it belongs to is
/// indexed once rather than searched for again per lane.
fn source_by_login(gears: &[Gear]) -> HashMap<&str, &str> {
    let mut sources = HashMap::new();

    for gear in gears {
        for login in &gear.assignees {
            sources
                .entry(login.as_str())
                .or_insert(gear.source_id.as_str());
        }
    }

    sources
}

fn account_href(
    login: &str,
    sources: &HashMap<&str, &str>,
    links: &ExternalSourceRegistry,
) -> Option<String> {
    links.account_href(GIT_PROVIDER, sources.get(login)?, login)
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

fn placement_of(gear: &Gear, window_start: YearMonth, today: NaiveDate) -> Placement {
    let Some(milestone) = gear.milestone.as_ref() else {
        return Placement::None;
    };

    let month = match milestone {
        Milestone::Due(month) => *month,
        Milestone::Backlog => return Placement::Backlog,
        Milestone::Unrecognized(_) => return Placement::Unrecognized,
    };

    match month.placement(window_start, WINDOW_MONTHS) {
        MonthPlacement::Overdue => Placement::Overdue {
            days: days_since_month_end(month, today),
        },
        MonthPlacement::Slot(slot) => Placement::Slot { slot },
        MonthPlacement::Future => Placement::Future,
    }
}

/// Days since the milestone month's last day, so the first day of the next
/// month counts as one day late.
fn days_since_month_end(month: YearMonth, today: NaiveDate) -> i64 {
    let Some(next_month) = NaiveDate::from_ymd_opt(
        if month.month == 12 {
            month.year + 1
        } else {
            month.year
        },
        if month.month == 12 {
            1
        } else {
            month.month + 1
        },
        1,
    ) else {
        return 0;
    };

    (today - next_month).num_days().saturating_add(1).max(0)
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

    use super::super::sort::{Direction, GearSort, Sort};
    use super::{
        CAPACITY_MAN_DAYS_PER_PERSON, GearRoadmapResponse, Placement, WINDOW_MONTHS, build,
    };
    use crate::domain::external_links::ExternalSourceRegistry;
    use crate::domain::gear_roadmap::model::{Gear, GearRow};

    fn build_for_test(gears: &[Gear], today: NaiveDate) -> GearRoadmapResponse {
        build(
            gears,
            today,
            Sort::default(),
            &ExternalSourceRegistry::default(),
        )
    }

    fn registry() -> ExternalSourceRegistry {
        ExternalSourceRegistry::default()
    }

    fn effort_gear(number: i64, effort: Option<f64>) -> Gear {
        let mut source = row(number);
        source.effort_man_days = effort.unwrap_or(0.0);
        Gear::from_row(source)
    }

    fn today() -> NaiveDate {
        NaiveDate::from_ymd_opt(2030, 8, 1).expect("valid date")
    }

    fn gear(number: i64, milestone_title: &str, status: &str) -> Gear {
        Gear::from_row(gear_row(number, milestone_title, status))
    }

    fn row(number: i64) -> GearRow {
        gear_row(number, "30.09", "Todo")
    }

    fn gear_row(number: i64, milestone_title: &str, status: &str) -> GearRow {
        GearRow {
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
        }
    }

    #[test]
    fn a_milestone_before_the_window_is_overdue_not_backlog() {
        let response = build_for_test(&[gear(1, "30.05", "Todo")], today());

        assert!(matches!(
            response.gears[0].placement,
            Placement::Overdue { .. }
        ));
    }

    #[test]
    fn an_overdue_gear_counts_the_days_since_its_milestone_month_ended() {
        // Milestone 30.05 ends 31 May; today is 1 August.
        let response = build_for_test(&[gear(1, "30.05", "Todo")], today());

        assert!(matches!(
            response.gears[0].placement,
            Placement::Overdue { days: 62 }
        ));
    }

    #[test]
    fn a_milestone_inside_the_window_carries_its_slot() {
        let response = build_for_test(&[gear(1, "30.09", "Todo")], today());

        assert!(matches!(
            response.gears[0].placement,
            Placement::Slot { slot: 1 }
        ));
    }

    fn sorted(gears: &[Gear], sort: GearSort, direction: Direction) -> Vec<i64> {
        build(gears, today(), Sort { sort, direction }, &registry())
            .gears
            .iter()
            .map(|gear| gear.number)
            .collect()
    }

    #[test]
    fn a_column_orders_the_gears_it_names() {
        let gears = [
            effort_gear(1, Some(5.0)),
            effort_gear(2, Some(60.0)),
            effort_gear(3, None),
        ];

        assert_eq!(
            sorted(&gears, GearSort::Effort, Direction::Desc),
            vec![2, 1, 3]
        );
    }

    #[test]
    fn a_gear_carrying_no_value_sorts_last_either_way() {
        let gears = [
            effort_gear(1, Some(5.0)),
            effort_gear(2, Some(60.0)),
            effort_gear(3, None),
        ];

        assert_eq!(
            sorted(&gears, GearSort::Effort, Direction::Asc),
            vec![1, 2, 3]
        );
    }

    #[test]
    fn milestone_order_leads_with_the_worst_overrun() {
        let gears = [
            gear(1, "30.09", "Todo"),
            gear(2, "30.05", "Todo"),
            gear(3, "30.07", "Todo"),
        ];

        assert_eq!(
            sorted(&gears, GearSort::Milestone, Direction::Asc),
            vec![2, 3, 1]
        );
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

        let response = build(
            &[gear(41, "30.09", "Todo")],
            today(),
            Sort::default(),
            &registry,
        );

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
    fn a_lane_names_the_account_page_of_the_person_in_it() {
        let sources = [crate::config::ExternalSourceConfig {
            id: "source-a".to_owned(),
            provider: crate::config::ExternalSourceProvider::Github,
            web_base_url: "https://git.example.test/".to_owned(),
        }];
        let registry = ExternalSourceRegistry::new(&sources).expect("valid sources");

        let response = build(
            &[gear(41, "30.09", "Todo")],
            today(),
            Sort::default(),
            &registry,
        );

        assert_eq!(
            response.lanes[0].assignee_url.as_deref(),
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
