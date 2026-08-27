//! The rows a comparison answered, turned into the answer's own vocabulary.
//!
//! INVARIANT: every target the question named is reported; one the read
//! answered no row for is unobserved rather than absent.

use std::collections::BTreeMap;

use serde::Deserialize;
use uuid::Uuid;

use super::dto::{PopulationSpread, TargetComparison};

/// One target's own value beside the spread of the population it sits in.
#[derive(Debug, Deserialize)]
pub(super) struct ComparisonRow {
    entity_id: String,
    target_value: Option<f64>,
    p25: Option<f64>,
    median: Option<f64>,
    p75: Option<f64>,
    min: Option<f64>,
    max: Option<f64>,
    /// `ClickHouse` quotes a 64-bit integer in JSON, so this arrives as text.
    #[serde(default, deserialize_with = "quoted_u64")]
    n: Option<u64>,
}

pub(super) fn target_comparisons(
    rows: Vec<ComparisonRow>,
    targets: &[Uuid],
) -> Vec<TargetComparison> {
    let mut answered: BTreeMap<String, ComparisonRow> = rows
        .into_iter()
        .map(|row| (row.entity_id.clone(), row))
        .collect();

    targets
        .iter()
        .map(|target| {
            let subject = target.to_string();
            match answered.remove(&subject) {
                None => TargetComparison {
                    subject,
                    value: None,
                    population: PopulationSpread {
                        n: 0,
                        p25: None,
                        median: None,
                        p75: None,
                        min: None,
                        max: None,
                    },
                },
                Some(row) => TargetComparison {
                    subject,
                    value: row.target_value,
                    population: PopulationSpread {
                        n: row.n.unwrap_or(0),
                        p25: row.p25,
                        median: row.median,
                        p75: row.p75,
                        min: row.min,
                        max: row.max,
                    },
                },
            }
        })
        .collect()
}

fn quoted_u64<'de, D>(deserializer: D) -> Result<Option<u64>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = Option::<serde_json::Value>::deserialize(deserializer)?;
    match value {
        None | Some(serde_json::Value::Null) => Ok(None),
        Some(serde_json::Value::Number(number)) => number
            .as_u64()
            .ok_or_else(|| serde::de::Error::custom("expected unsigned integer"))
            .map(Some),
        Some(serde_json::Value::String(value)) => value
            .parse::<u64>()
            .map(Some)
            .map_err(serde::de::Error::custom),
        Some(_) => Err(serde::de::Error::custom("expected unsigned integer")),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn row(entity_id: &str, value: Option<f64>, n: &serde_json::Value) -> ComparisonRow {
        serde_json::from_value(serde_json::json!({
            "entity_id": entity_id,
            "target_value": value,
            "p25": 4.0,
            "median": 7.0,
            "p75": 11.0,
            "min": 1.0,
            "max": 20.0,
            "n": n,
        }))
        .expect("the row shape decodes")
    }

    #[test]
    fn a_population_size_arrives_quoted_or_bare_and_reads_the_same_either_way() {
        let first = Uuid::from_u128(1);

        for spelling in [serde_json::json!("9"), serde_json::json!(9)] {
            let named = spelling.to_string();
            let comparisons = target_comparisons(
                vec![row(&first.to_string(), Some(12.0), &spelling)],
                &[first],
            );

            assert_eq!(comparisons[0].population.n, 9, "{named}");
        }
    }

    #[test]
    fn each_target_keeps_the_spread_of_the_population_its_own_row_reported() {
        let first = Uuid::from_u128(1);
        let second = Uuid::from_u128(2);
        let mut narrow = row(&second.to_string(), Some(3.0), &serde_json::json!(2));
        narrow.p25 = None;
        narrow.median = None;
        narrow.p75 = None;
        narrow.min = None;
        narrow.max = None;

        let comparisons = target_comparisons(
            vec![
                row(&first.to_string(), Some(12.0), &serde_json::json!(9)),
                narrow,
            ],
            &[first, second],
        );

        assert_eq!(comparisons[0].population.n, 9);
        assert_eq!(comparisons[0].population.median, Some(7.0));
        assert_eq!(comparisons[1].population.n, 2);
        assert_eq!(
            comparisons[1].population.median, None,
            "a population under the floor reports its size and nothing else"
        );
    }

    #[test]
    fn a_target_the_read_answered_no_row_for_is_still_reported() {
        let answered = Uuid::from_u128(1);
        let unanswered = Uuid::from_u128(2);

        let comparisons = target_comparisons(
            vec![row(
                &answered.to_string(),
                Some(12.0),
                &serde_json::json!(9),
            )],
            &[answered, unanswered],
        );

        assert_eq!(
            comparisons.len(),
            2,
            "every target the question named is answered for"
        );
        assert_eq!(comparisons[1].subject, unanswered.to_string());
        assert_eq!(comparisons[1].value, None);
        assert_eq!(comparisons[1].population.n, 0);
        assert_eq!(comparisons[1].population.p25, None);
    }

    #[test]
    fn the_targets_are_reported_in_the_order_the_question_named_them() {
        let first = Uuid::from_u128(1);
        let second = Uuid::from_u128(2);

        let comparisons = target_comparisons(
            vec![
                row(&second.to_string(), Some(3.0), &serde_json::json!(9)),
                row(&first.to_string(), Some(12.0), &serde_json::json!(9)),
            ],
            &[first, second],
        );

        assert_eq!(
            comparisons
                .iter()
                .map(|comparison| comparison.subject.as_str())
                .collect::<Vec<_>>(),
            vec![first.to_string(), second.to_string()]
        );
    }
}
