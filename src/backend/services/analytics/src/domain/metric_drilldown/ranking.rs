use crate::domain::metric_definitions::ComputationSpec;

use super::dto::{EvidencePlan, MetricDrilldownEntity};
use super::row_shape::record_parts;

const POSITION: &str = "record_position";
const TOTAL: &str = "record_total";

#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) enum Ranking {
    Median,
    Percentile(f64),
}

impl Ranking {
    pub(super) fn of(plan: &EvidencePlan, entity: &MetricDrilldownEntity) -> Option<Self> {
        let MetricDrilldownEntity::Person { .. } = entity else {
            return None;
        };
        if record_parts(plan).is_some() {
            return None;
        }
        match plan.definition.spec {
            ComputationSpec::Median { .. } => Some(Self::Median),
            ComputationSpec::Percentile { q, .. } => Some(Self::Percentile(q)),
            ComputationSpec::Sum { .. }
            | ComputationSpec::Ratio { .. }
            | ComputationSpec::Stddev { .. }
            | ComputationSpec::DistinctCount { .. } => None,
        }
    }

    /// INVARIANT: the marked rows are the ones the tile's aggregate reads —
    /// `quantileExactInclusive(0.5)` averages the two middles of an even count,
    /// `quantileExact(q)` takes the record at `floor(q * n) + 1`.
    fn marker(self) -> String {
        match self {
            Self::Median => format!(
                "multiIf({TOTAL} % 2 = 1 AND {POSITION} = intDiv({TOTAL} + 1, 2), ' · median', \
                         {TOTAL} % 2 = 0 AND ({POSITION} = intDiv({TOTAL}, 2) OR {POSITION} = intDiv({TOTAL}, 2) + 1), \
                         ' · median pair', '')"
            ),
            Self::Percentile(q) => format!(
                "if({POSITION} = least(toUInt64(floor({q} * {TOTAL})) + 1, {TOTAL}), ' · p{label}', '')",
                label = (q * 100_000.0).round() / 1_000.0,
            ),
        }
    }

    pub(super) fn ranked_source(self, database: &str, table: &str, scope: &str) -> String {
        format!(
            "(SELECT * REPLACE (mapUpdate(details, map('position', \
                 if(contribution IS NULL, '', \
                    concat(toString({POSITION}), ' of ', toString({TOTAL}), {marker})))) AS details) \
              FROM (SELECT *, \
                           row_number() OVER (ORDER BY evidence.contribution ASC NULLS LAST, evidence.record_id ASC) AS {POSITION}, \
                           count(evidence.contribution) OVER () AS {TOTAL} \
                    FROM {database}.{table} AS evidence \
                    WHERE {scope})) AS evidence",
            marker = self.marker(),
        )
    }
}

pub(super) fn position_order() -> String {
    format!("evidence.{POSITION}")
}
