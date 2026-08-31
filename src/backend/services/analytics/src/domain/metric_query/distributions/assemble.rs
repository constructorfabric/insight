//! The rows a distribution answered, turned into the answer's own vocabulary.
//!
//! INVARIANT: the read reports observed bins and exact bounds only; every bin
//! edge is derived here, so an empty and an observed bin cannot disagree.

use std::collections::BTreeMap;
use std::num::NonZeroU32;

use serde::Deserialize;

use super::super::question::ValidatedSubjects;
use super::dto::{Histogram, HistogramBin, Quantile, SubjectDistribution};

/// One observed (subject, bin) pair plus that subject's exact bounds.
#[derive(Debug, Deserialize)]
pub(super) struct HistogramRow {
    entity_id: String,
    bin_idx: u32,
    entity_lo: f64,
    entity_hi: f64,
    /// `ClickHouse` quotes a 64-bit integer in JSON, so this arrives as text.
    #[serde(default, deserialize_with = "quoted_u64")]
    bin_count: Option<u64>,
}

/// One subject's values at the positions the question named, in that order.
#[derive(Debug, Deserialize)]
pub(super) struct QuantileRow {
    entity_id: String,
    quantile_values: Vec<f64>,
}

/// What one subject's reads answered, before the answer's shape is built.
#[derive(Debug, Default)]
struct Observed {
    bounds: Option<(f64, f64)>,
    counts: BTreeMap<u32, u64>,
    quantile_values: Vec<f64>,
}

pub(super) fn subject_distributions(
    subjects: &ValidatedSubjects,
    bins: Option<NonZeroU32>,
    quantiles: Option<&[f64]>,
    histogram_rows: Vec<HistogramRow>,
    quantile_rows: Vec<QuantileRow>,
) -> Vec<SubjectDistribution> {
    let mut observed: BTreeMap<String, Observed> = BTreeMap::new();
    for row in histogram_rows {
        let entry = observed.entry(row.entity_id).or_default();
        entry.bounds = Some((row.entity_lo, row.entity_hi));
        *entry.counts.entry(row.bin_idx).or_insert(0) += row.bin_count.unwrap_or(0);
    }
    for row in quantile_rows {
        observed.entry(row.entity_id).or_default().quantile_values = row.quantile_values;
    }

    let shape = |answered: &Observed, subject: Option<String>| SubjectDistribution {
        histogram: bins.map(|bins| histogram(answered, bins)),
        quantiles: quantiles.map(|quantiles| positions(answered, quantiles)),
        subject,
    };

    match subjects {
        // A tenant read groups by the one entity its rows carry, so the answer
        // is that one distribution — reported whether or not anything was
        // observed, exactly as a person with no rows still gets an entry.
        ValidatedSubjects::Tenant => {
            let answered = observed.into_values().next().unwrap_or_default();
            vec![shape(&answered, None)]
        }
        ValidatedSubjects::Persons(ids) => ids
            .iter()
            .map(|id| {
                let subject = id.to_string();
                let answered = observed.remove(&subject).unwrap_or_default();
                shape(&answered, Some(subject))
            })
            .collect(),
    }
}

/// The subject's own range cut into equal-width bins. A range collapsing to a
/// point is one bin, because there is no width to cut.
fn histogram(observed: &Observed, bins: NonZeroU32) -> Histogram {
    let Some((lo, hi)) = observed.bounds else {
        return Histogram {
            lo: None,
            hi: None,
            bins: Vec::new(),
        };
    };
    if hi <= lo {
        return Histogram {
            lo: Some(lo),
            hi: Some(hi),
            bins: vec![HistogramBin {
                lo,
                hi,
                count: observed.counts.values().sum(),
            }],
        };
    }

    let count = bins.get();
    let width = (hi - lo) / f64::from(count);
    Histogram {
        lo: Some(lo),
        hi: Some(hi),
        bins: (0..count)
            .map(|index| HistogramBin {
                lo: lo + f64::from(index) * width,
                hi: if index == count - 1 {
                    hi
                } else {
                    lo + f64::from(index + 1) * width
                },
                count: observed.counts.get(&index).copied().unwrap_or(0),
            })
            .collect(),
    }
}

/// INVARIANT: the read projects one value per position in the order the
/// question named them, so a short answer leaves the rest unknown rather than
/// shifting them.
fn positions(observed: &Observed, quantiles: &[f64]) -> Vec<Quantile> {
    quantiles
        .iter()
        .enumerate()
        .map(|(index, quantile)| Quantile {
            q: *quantile,
            value: observed.quantile_values.get(index).copied(),
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
    use uuid::Uuid;

    use super::*;

    fn people(ids: &[Uuid]) -> ValidatedSubjects {
        ValidatedSubjects::Persons(ids.to_vec())
    }

    fn person() -> Uuid {
        Uuid::from_u128(1)
    }

    fn bins(count: u32) -> Option<NonZeroU32> {
        NonZeroU32::new(count)
    }

    fn histogram_row(entity: &str, bin_idx: u32, lo: f64, hi: f64, count: u64) -> HistogramRow {
        HistogramRow {
            entity_id: entity.to_owned(),
            bin_idx,
            entity_lo: lo,
            entity_hi: hi,
            bin_count: Some(count),
        }
    }

    #[test]
    fn a_bin_count_arrives_quoted_or_bare_and_reads_the_same_either_way() {
        for spelling in [serde_json::json!("7"), serde_json::json!(7)] {
            let named = spelling.to_string();
            let row: HistogramRow = serde_json::from_value(serde_json::json!({
                "entity_id": person().to_string(),
                "bin_idx": 0,
                "entity_lo": 0.0,
                "entity_hi": 4.0,
                "bin_count": spelling,
            }))
            .expect("the row shape decodes");

            assert_eq!(row.bin_count, Some(7), "{named}");
        }
    }

    #[test]
    fn every_bin_of_the_range_is_reported_and_the_last_one_closes_on_the_maximum() {
        let distributions = subject_distributions(
            &people(&[person()]),
            bins(4),
            None,
            vec![
                histogram_row(&person().to_string(), 0, 0.0, 8.0, 3),
                histogram_row(&person().to_string(), 3, 0.0, 8.0, 1),
            ],
            Vec::new(),
        );

        let histogram = distributions[0]
            .histogram
            .as_ref()
            .expect("a histogram was asked for");
        assert_eq!(histogram.lo, Some(0.0));
        assert_eq!(histogram.hi, Some(8.0));
        assert_eq!(
            histogram
                .bins
                .iter()
                .map(|bin| (bin.lo, bin.hi, bin.count))
                .collect::<Vec<_>>(),
            vec![(0.0, 2.0, 3), (2.0, 4.0, 0), (4.0, 6.0, 0), (6.0, 8.0, 1),]
        );
    }

    #[test]
    fn a_range_collapsing_to_a_point_is_one_bin_rather_than_a_cut_of_no_width() {
        let distributions = subject_distributions(
            &people(&[person()]),
            bins(10),
            None,
            vec![histogram_row(&person().to_string(), 0, 5.0, 5.0, 4)],
            Vec::new(),
        );

        let histogram = distributions[0]
            .histogram
            .as_ref()
            .expect("a histogram was asked for");
        assert_eq!(
            histogram
                .bins
                .iter()
                .map(|bin| (bin.lo, bin.hi, bin.count))
                .collect::<Vec<_>>(),
            vec![(5.0, 5.0, 4)]
        );
    }

    #[test]
    fn a_subject_the_read_observed_nothing_for_is_still_reported() {
        let observed = person();
        let unobserved = Uuid::from_u128(2);

        let distributions = subject_distributions(
            &people(&[observed, unobserved]),
            bins(2),
            Some(&[0.5]),
            vec![histogram_row(&observed.to_string(), 0, 0.0, 4.0, 1)],
            vec![QuantileRow {
                entity_id: observed.to_string(),
                quantile_values: vec![2.0],
            }],
        );

        assert_eq!(distributions.len(), 2);
        assert_eq!(distributions[1].subject, Some(unobserved.to_string()));
        let histogram = distributions[1]
            .histogram
            .as_ref()
            .expect("a histogram was asked for");
        assert_eq!(histogram.lo, None);
        assert!(histogram.bins.is_empty());
        assert_eq!(
            distributions[1].quantiles,
            Some(vec![Quantile {
                q: 0.5,
                value: None
            }])
        );
    }

    #[test]
    fn each_position_keeps_the_value_the_read_projected_for_it() {
        let distributions = subject_distributions(
            &people(&[person()]),
            None,
            Some(&[0.25, 0.5, 0.9]),
            Vec::new(),
            vec![QuantileRow {
                entity_id: person().to_string(),
                quantile_values: vec![1.0, 4.0, 9.0],
            }],
        );

        assert_eq!(distributions[0].histogram, None);
        assert_eq!(
            distributions[0].quantiles,
            Some(vec![
                Quantile {
                    q: 0.25,
                    value: Some(1.0),
                },
                Quantile {
                    q: 0.5,
                    value: Some(4.0),
                },
                Quantile {
                    q: 0.9,
                    value: Some(9.0),
                },
            ])
        );
    }

    #[test]
    fn the_subjects_are_reported_in_the_order_the_question_named_them() {
        let first = person();
        let second = Uuid::from_u128(2);

        let distributions = subject_distributions(
            &people(&[first, second]),
            bins(1),
            None,
            vec![
                histogram_row(&second.to_string(), 0, 0.0, 1.0, 1),
                histogram_row(&first.to_string(), 0, 0.0, 1.0, 1),
            ],
            Vec::new(),
        );

        assert_eq!(
            distributions
                .iter()
                .map(|distribution| distribution.subject.clone())
                .collect::<Vec<_>>(),
            vec![Some(first.to_string()), Some(second.to_string())]
        );
    }

    #[test]
    fn a_tenant_distribution_is_one_entry_naming_nobody_whatever_the_read_observed() {
        let observed = subject_distributions(
            &ValidatedSubjects::Tenant,
            NonZeroU32::new(2),
            None,
            vec![
                histogram_row("acme-tenant", 0, 1.0, 3.0, 2),
                histogram_row("acme-tenant", 1, 1.0, 3.0, 1),
            ],
            Vec::new(),
        );
        let unobserved = subject_distributions(
            &ValidatedSubjects::Tenant,
            NonZeroU32::new(2),
            None,
            Vec::new(),
            Vec::new(),
        );

        for (named, answer) in [("observed", observed), ("unobserved", unobserved)] {
            assert_eq!(answer.len(), 1, "{named}: the tenant is one subject");
            assert_eq!(answer[0].subject, None, "{named}");
        }
    }
}
