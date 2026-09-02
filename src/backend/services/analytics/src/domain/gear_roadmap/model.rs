use serde::Deserialize;

use super::milestone::Milestone;
use super::progress::LadderStep;

#[derive(Debug, Deserialize, clickhouse::Row)]
pub(crate) struct GearRow {
    pub(crate) number: i64,
    pub(crate) title: String,
    pub(crate) status: String,
    pub(crate) design: String,
    pub(crate) sdk: String,
    pub(crate) commitment: String,
    pub(crate) priority: String,
    pub(crate) effort_man_days: f64,
    pub(crate) milestone_title: String,
    pub(crate) assignees: Vec<String>,
    pub(crate) closed: bool,
    pub(crate) repo_full_name: String,
    pub(crate) source_id: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Commitment {
    Committed,
    Planned,
    Unstated,
}

impl Commitment {
    fn parse(label: &str) -> Self {
        match label {
            "COMMITMENT" => Self::Committed,
            "not commitment" => Self::Planned,
            _ => Self::Unstated,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Gear {
    pub(crate) number: i64,
    pub(crate) title: String,
    pub(crate) subsystem: Option<String>,
    pub(crate) status: Option<LadderStep>,
    pub(crate) design: Option<LadderStep>,
    pub(crate) sdk: Option<LadderStep>,
    pub(crate) commitment: Commitment,
    pub(crate) priority: Option<String>,
    pub(crate) effort_man_days: Option<f64>,
    pub(crate) milestone: Option<Milestone>,
    pub(crate) assignees: Vec<String>,
    pub(crate) closed: bool,
    pub(crate) repo_full_name: String,
    pub(crate) source_id: String,
}

impl Gear {
    pub(crate) fn from_row(row: GearRow) -> Self {
        let effort_man_days = (row.effort_man_days > 0.0).then_some(row.effort_man_days);

        Self {
            number: row.number,
            subsystem: subsystem_of(&row.title),
            title: row.title,
            status: LadderStep::parse(&row.status),
            design: LadderStep::parse(&row.design),
            sdk: LadderStep::parse(&row.sdk),
            commitment: Commitment::parse(&row.commitment),
            priority: (!row.priority.is_empty()).then_some(row.priority),
            effort_man_days,
            milestone: Milestone::parse(&row.milestone_title),
            assignees: row.assignees,
            closed: row.closed,
            repo_full_name: row.repo_full_name,
            source_id: row.source_id,
        }
    }

    /// A milestone as one number, so months order against each other. Backlog
    /// and unreadable titles carry none — they name no month to compare.
    pub(crate) fn milestone_sort_key(&self) -> Option<f64> {
        match self.milestone.as_ref()? {
            Milestone::Due(month) => Some(f64::from(month.year) * 12.0 + f64::from(month.month)),
            Milestone::Backlog | Milestone::Unrecognized(_) => None,
        }
    }

    /// Done on the board, or closed on the tracker — either one settles it, and
    /// the two disagree in practice.
    pub(crate) fn is_delivered(&self) -> bool {
        self.closed || self.status == Some(LadderStep::Complete)
    }

    pub(crate) fn remaining_man_days(&self) -> Option<f64> {
        let effort = self.effort_man_days?;
        let done = self
            .status
            .and_then(LadderStep::percent_complete)
            .unwrap_or(0);

        Some(effort * f64::from(100 - done) / 100.0)
    }
}

fn subsystem_of(title: &str) -> Option<String> {
    let (prefix, _) = title.split_once(" - ")?;

    let prefix = prefix.trim();

    prefix
        .chars()
        .all(|character| character.is_ascii_uppercase())
        .then(|| prefix.to_owned())
}

#[cfg(test)]
mod tests {
    use super::{Commitment, Gear, GearRow, subsystem_of};
    use crate::domain::gear_roadmap::milestone::{Milestone, YearMonth};
    use crate::domain::gear_roadmap::progress::LadderStep;

    fn row() -> GearRow {
        GearRow {
            number: 41,
            title: "CORE - Example Module".to_owned(),
            status: "80%".to_owned(),
            design: "Done".to_owned(),
            sdk: "N/A".to_owned(),
            commitment: "COMMITMENT".to_owned(),
            priority: "1 (1.1.1)".to_owned(),
            effort_man_days: 30.0,
            milestone_title: "30.08".to_owned(),
            assignees: vec!["dev-one".to_owned()],
            closed: false,
            repo_full_name: "example-org/example-repo".to_owned(),
            source_id: "source-a".to_owned(),
        }
    }

    #[test]
    fn the_subsystem_is_the_prefix_before_the_first_dash() {
        assert_eq!(
            subsystem_of("CORE - Example Module"),
            Some("CORE".to_owned())
        );
    }

    #[test]
    fn a_title_without_a_prefix_has_no_subsystem() {
        assert_eq!(subsystem_of("Example Module"), None);
    }

    #[test]
    fn a_row_maps_onto_parsed_gear_fields() {
        let gear = Gear::from_row(row());

        assert_eq!(gear.number, 41);
        assert_eq!(gear.subsystem, Some("CORE".to_owned()));
        assert_eq!(gear.status, Some(LadderStep::Partial(80)));
        assert_eq!(gear.design, Some(LadderStep::Complete));
        assert_eq!(gear.sdk, Some(LadderStep::NotApplicable));
        assert_eq!(gear.commitment, Commitment::Committed);
        assert_eq!(gear.effort_man_days, Some(30.0));
        assert_eq!(
            gear.milestone,
            Some(Milestone::Due(YearMonth {
                year: 2030,
                month: 8
            }))
        );
    }

    #[test]
    fn a_gear_keeps_the_repository_its_issue_lives_in() {
        let gear = Gear::from_row(row());

        assert_eq!(gear.repo_full_name, "example-org/example-repo");
        assert_eq!(gear.source_id, "source-a");
    }

    #[test]
    fn remaining_effort_discounts_the_implementation_progress() {
        let gear = Gear::from_row(row());

        assert_eq!(gear.remaining_man_days(), Some(6.0));
    }

    #[test]
    fn a_gear_without_an_estimate_has_no_remaining_effort() {
        let mut source = row();
        source.effort_man_days = 0.0;

        let gear = Gear::from_row(source);

        assert_eq!(gear.remaining_man_days(), None);
    }
}
