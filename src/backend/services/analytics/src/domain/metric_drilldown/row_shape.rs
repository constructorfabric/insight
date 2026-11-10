use crate::domain::metric_definitions::ComputationSpec;
use crate::domain::metric_definitions::definition::RatioDenominatorAggregation;

use super::dto::EvidencePlan;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum RowShape {
    Record,
    FoldedRecord,
    DailyRatio,
    RecordRatio,
    RecordsPerSubject,
}

impl RowShape {
    pub(super) fn of(plan: &EvidencePlan) -> Self {
        let ComputationSpec::Ratio {
            denominator_aggregation,
            ..
        } = &plan.definition.spec
        else {
            return if record_parts(plan).is_some() {
                Self::FoldedRecord
            } else {
                Self::Record
            };
        };
        let both_halves_are_records = plan
            .inputs
            .iter()
            .all(|input| input.presentation.names_a_record());
        match denominator_aggregation {
            RatioDenominatorAggregation::Sum if both_halves_are_records => Self::RecordRatio,
            RatioDenominatorAggregation::DistinctCount if both_halves_are_records => {
                Self::RecordsPerSubject
            }
            RatioDenominatorAggregation::Sum | RatioDenominatorAggregation::DistinctCount => {
                Self::DailyRatio
            }
        }
    }
}

pub(super) fn record_parts(plan: &EvidencePlan) -> Option<&str> {
    plan.inputs
        .iter()
        .find_map(|input| input.presentation.record_parts.as_deref())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::metric_definitions::definition::MetricInputRole;
    use crate::domain::metric_drilldown::test_support::{
        commit_input, input, plan, ratio_over, summary_input,
    };

    #[test]
    fn a_ratio_lists_records_whenever_both_halves_are_records() {
        for (inputs, aggregation, expected) in [
            (
                commit_input as fn(MetricInputRole, &str) -> _,
                RatioDenominatorAggregation::Sum,
                RowShape::RecordRatio,
            ),
            (
                commit_input,
                RatioDenominatorAggregation::DistinctCount,
                RowShape::RecordsPerSubject,
            ),
            (
                summary_input,
                RatioDenominatorAggregation::Sum,
                RowShape::DailyRatio,
            ),
            (
                summary_input,
                RatioDenominatorAggregation::DistinctCount,
                RowShape::DailyRatio,
            ),
        ] {
            assert_eq!(
                RowShape::of(&ratio_over(inputs, aggregation)),
                expected,
                "should be {expected:?} for {aggregation:?}"
            );
        }
    }

    #[test]
    fn a_measure_split_into_parts_folds_back_into_its_record() {
        let mut folded = commit_input(MetricInputRole::Value, "code_lines_added");
        folded.presentation.record_parts = Some("file_extension".to_owned());
        let sum = |inputs| {
            plan(
                ComputationSpec::Sum {
                    value: input(MetricInputRole::Value, "code_lines_added"),
                },
                inputs,
            )
        };

        assert_eq!(RowShape::of(&sum(vec![folded])), RowShape::FoldedRecord);
        assert_eq!(
            RowShape::of(&sum(vec![commit_input(
                MetricInputRole::Value,
                "commit_count"
            )])),
            RowShape::Record
        );
    }
}
