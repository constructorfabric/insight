use std::fmt::Display;

#[derive(Debug, PartialEq, Eq)]
enum CoverageWarning<'a> {
    NotCovered { dimension: &'a str },
    ProbeFailed { dimension: &'a str, error: String },
}

fn warnings<'a, E: Display>(
    results: impl IntoIterator<Item = (&'a str, Result<bool, E>)>,
) -> Vec<CoverageWarning<'a>> {
    results
        .into_iter()
        .filter_map(|(dimension, result)| match result {
            Ok(true) => None,
            Ok(false) => Some(CoverageWarning::NotCovered { dimension }),
            Err(error) => Some(CoverageWarning::ProbeFailed {
                dimension,
                error: error.to_string(),
            }),
        })
        .collect()
}

pub(super) fn report<'a, E: Display>(
    metric_key: &str,
    results: impl IntoIterator<Item = (&'a str, Result<bool, E>)>,
) {
    for warning in warnings(results) {
        match warning {
            CoverageWarning::NotCovered { dimension } => {
                tracing::warn!(
                    metric_key = %metric_key,
                    dimension,
                    "declared dimension is not covered on all rows"
                );
            }
            CoverageWarning::ProbeFailed { dimension, error } => {
                tracing::warn!(
                    error = %error,
                    metric_key = %metric_key,
                    dimension,
                    "metric dimension probe failed"
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use insight_log_context::test_support::capture_output;
    use serde_json::{Value, json};

    use super::*;

    type R = Result<(), Box<dyn std::error::Error>>;

    #[test]
    fn dimension_coverage_reports_only_gaps_and_failures() -> R {
        let probes = [
            ("tool", Ok(true)),
            ("kind", Ok(false)),
            ("scope", Err("synthetic probe failure")),
        ];

        assert_eq!(
            warnings(probes),
            vec![
                CoverageWarning::NotCovered { dimension: "kind" },
                CoverageWarning::ProbeFailed {
                    dimension: "scope",
                    error: "synthetic probe failure".to_owned(),
                },
            ]
        );

        let output = capture_output(|| report("example.metric", probes));
        let fields = output
            .lines()
            .map(|line| {
                serde_json::from_str::<Value>(line).map(|mut record| record["fields"].take())
            })
            .collect::<Result<Vec<_>, _>>()?;

        assert_eq!(
            fields,
            vec![
                json!({
                    "metric_key": "example.metric",
                    "dimension": "kind",
                    "message": "declared dimension is not covered on all rows",
                }),
                json!({
                    "metric_key": "example.metric",
                    "dimension": "scope",
                    "error": "synthetic probe failure",
                    "message": "metric dimension probe failed",
                }),
            ]
        );
        Ok(())
    }

    #[test]
    fn dimension_coverage_reports_nothing_when_every_dimension_is_covered() {
        let probes = [("tool", Ok::<bool, &str>(true)), ("kind", Ok(true))];

        assert!(warnings(probes).is_empty());
        assert!(capture_output(|| report("example.metric", probes)).is_empty());
    }
}
