use serde::Serialize;
use utoipa::ToSchema;

use crate::domain::metric_definitions::{MetricDefinition, MetricFormat};
use crate::infra::identity::IdentityProfile;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ReportColumnDataType {
    Text,
    Date,
    Number,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
pub struct ReportColumnMetadata {
    pub key: String,
    pub label: String,
    pub data_type: ReportColumnDataType,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub format: Option<MetricFormat>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unit: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum PlannedColumnSource {
    PersonDisplay,
    PersonAttribute(String),
    SupervisorDisplay,
    SupervisorAttribute(String),
    PeriodLabel,
    PeriodFrom,
    PeriodTo,
    Metric(usize),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PlannedColumn {
    pub(crate) metadata: ReportColumnMetadata,
    pub(crate) source: PlannedColumnSource,
}

pub(crate) fn plan_columns(
    profiles: &[IdentityProfile],
    metrics: &[MetricDefinition],
) -> Vec<PlannedColumn> {
    let mut columns = Vec::new();
    if !profiles.is_empty() {
        columns.push(text_column(
            "person",
            "Person",
            PlannedColumnSource::PersonDisplay,
        ));

        columns.extend(populated_profile_columns(profiles));
    }

    columns.extend([
        text_column("period", "Period", PlannedColumnSource::PeriodLabel),
        date_column("from", "From", PlannedColumnSource::PeriodFrom),
        date_column("to", "To", PlannedColumnSource::PeriodTo),
    ]);
    columns.extend(
        metrics
            .iter()
            .enumerate()
            .map(|(index, metric)| PlannedColumn {
                metadata: ReportColumnMetadata {
                    key: metric.base.key.clone(),
                    label: metric.base.label.clone(),
                    data_type: ReportColumnDataType::Number,
                    format: Some(metric.base.format),
                    unit: metric.base.unit.clone(),
                },
                source: PlannedColumnSource::Metric(index),
            }),
    );

    columns
}

pub(crate) fn profile_text<'a>(
    source: &PlannedColumnSource,
    profile: &'a IdentityProfile,
) -> Option<&'a str> {
    match source {
        PlannedColumnSource::PersonDisplay => {
            first_attribute(&profile.attributes, &["display_name", "username", "email"])
        }
        PlannedColumnSource::PersonAttribute(attribute) => {
            profile.attributes.get(attribute).map(String::as_str)
        }
        PlannedColumnSource::SupervisorDisplay => profile
            .supervisor
            .as_ref()
            .and_then(|person| first_attribute(&person.attributes, &["display_name", "username"])),
        PlannedColumnSource::SupervisorAttribute(attribute) => profile
            .supervisor
            .as_ref()
            .and_then(|person| person.attributes.get(attribute))
            .map(String::as_str),
        PlannedColumnSource::PeriodLabel
        | PlannedColumnSource::PeriodFrom
        | PlannedColumnSource::PeriodTo
        | PlannedColumnSource::Metric(_) => None,
    }
}

fn populated_profile_columns(profiles: &[IdentityProfile]) -> Vec<PlannedColumn> {
    [
        text_column(
            "email",
            "Email",
            PlannedColumnSource::PersonAttribute("email".to_owned()),
        ),
        text_column(
            "division",
            "Division",
            PlannedColumnSource::PersonAttribute("division".to_owned()),
        ),
        text_column(
            "department",
            "Department",
            PlannedColumnSource::PersonAttribute("department".to_owned()),
        ),
        text_column(
            "job_title",
            "Job title",
            PlannedColumnSource::PersonAttribute("job_title".to_owned()),
        ),
        text_column("manager", "Manager", PlannedColumnSource::SupervisorDisplay),
        text_column(
            "manager_email",
            "Manager email",
            PlannedColumnSource::SupervisorAttribute("email".to_owned()),
        ),
        text_column(
            "status",
            "Status",
            PlannedColumnSource::PersonAttribute("status".to_owned()),
        ),
    ]
    .into_iter()
    .filter(|column| {
        profiles.iter().any(|profile| {
            profile_text(&column.source, profile).is_some_and(|value| !value.trim().is_empty())
        })
    })
    .collect()
}

fn text_column(key: &str, label: &str, source: PlannedColumnSource) -> PlannedColumn {
    PlannedColumn {
        metadata: ReportColumnMetadata {
            key: key.to_owned(),
            label: label.to_owned(),
            data_type: ReportColumnDataType::Text,
            format: None,
            unit: None,
        },
        source,
    }
}

fn date_column(key: &str, label: &str, source: PlannedColumnSource) -> PlannedColumn {
    PlannedColumn {
        metadata: ReportColumnMetadata {
            key: key.to_owned(),
            label: label.to_owned(),
            data_type: ReportColumnDataType::Date,
            format: None,
            unit: None,
        },
        source,
    }
}

fn first_attribute<'a>(
    attributes: &'a std::collections::BTreeMap<String, String>,
    keys: &[&str],
) -> Option<&'a str> {
    keys.iter()
        .filter_map(|key| attributes.get(*key))
        .find(|value| !value.trim().is_empty())
        .map(String::as_str)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use uuid::Uuid;

    use super::*;
    use crate::domain::metric_definitions::definition::{
        AliasCollapse, ComputationSpec, MetricBase, MetricDirection, MetricInput, MetricInputRole,
        ObservationRelation, ObservationSource,
    };
    use crate::infra::identity::IdentityProfileRelationship;

    fn attributes(values: &[(&str, &str)]) -> BTreeMap<String, String> {
        values
            .iter()
            .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
            .collect()
    }

    fn profile(
        id: u128,
        values: &[(&str, &str)],
        supervisor: Option<&[(&str, &str)]>,
    ) -> IdentityProfile {
        IdentityProfile {
            person_id: Uuid::from_u128(id),
            attributes: attributes(values),
            supervisor: supervisor.map(|values| IdentityProfileRelationship {
                person_id: Uuid::from_u128(id + 100),
                attributes: attributes(values),
            }),
        }
    }

    fn metric(key: &str, label: &str, format: MetricFormat) -> MetricDefinition {
        MetricDefinition {
            base: MetricBase {
                key: key.to_owned(),
                label: label.to_owned(),
                short_label: None,
                description: None,
                explanation: None,
                entity_type: "person".to_owned(),
                format,
                unit: Some("items".to_owned()),
                direction: MetricDirection::Neutral,
                peer_cohort_key: None,
                allowed_dimensions: vec![],
            },
            spec: ComputationSpec::Sum {
                value: MetricInput {
                    role: MetricInputRole::Value,
                    observation: ObservationSource::Managed(
                        ObservationRelation::parse("test_metric_observations")
                            .unwrap_or_else(|| panic!("fixture relation must parse")),
                    ),
                    source_key: "test".to_owned(),
                    measure_key: "value".to_owned(),
                    alias_collapse: AliasCollapse::Sum,
                },
            },
            transform: None,
        }
    }

    #[test]
    fn includes_only_supported_populated_profile_columns_in_report_order() {
        let profiles = vec![
            profile(
                1,
                &[
                    ("username", "example-user"),
                    ("email", "one@example.com"),
                    ("division", "Platform"),
                    ("department", "Engineering"),
                    ("job_title", "Engineer"),
                    ("status", "Active"),
                    ("employee_id", "123"),
                    ("custom_field", "Example 01"),
                ],
                None,
            ),
            profile(
                2,
                &[
                    ("display_name", "Example Two"),
                    ("email", "two@example.com"),
                    ("first_name", "Example"),
                    ("last_name", "Two"),
                ],
                Some(&[
                    ("display_name", "Example Manager"),
                    ("email", "manager@example.com"),
                    ("job_title", "Director"),
                    ("employee_id", "456"),
                ]),
            ),
        ];
        let metrics = vec![
            metric("git.commits", "Commits", MetricFormat::Integer),
            metric("git.review_rate", "Review rate", MetricFormat::Percent),
        ];

        let columns = plan_columns(&profiles, &metrics);
        let keys = columns
            .iter()
            .map(|column| column.metadata.key.as_str())
            .collect::<Vec<_>>();

        assert_eq!(
            keys,
            [
                "person",
                "email",
                "division",
                "department",
                "job_title",
                "manager",
                "manager_email",
                "status",
                "period",
                "from",
                "to",
                "git.commits",
                "git.review_rate",
            ]
        );
        assert_eq!(
            profile_text(&columns[0].source, &profiles[0]),
            Some("example-user")
        );
        assert_eq!(
            profile_text(&columns[5].source, &profiles[1]),
            Some("Example Manager")
        );
        assert_eq!(columns[11].metadata.format, Some(MetricFormat::Integer));
        assert_eq!(columns[12].metadata.format, Some(MetricFormat::Percent));
        assert_eq!(columns[1].metadata.label, "Email");
        assert_eq!(columns[6].metadata.label, "Manager email");
    }

    #[test]
    fn omits_every_optional_profile_column_when_profiles_have_no_attributes() {
        let profiles = vec![profile(1, &[], None)];

        let columns = plan_columns(&profiles, &[]);
        let keys = columns
            .iter()
            .map(|column| column.metadata.key.as_str())
            .collect::<Vec<_>>();

        assert_eq!(keys, ["person", "period", "from", "to"]);
        assert_eq!(profile_text(&columns[0].source, &profiles[0]), None);
    }

    #[test]
    fn tenant_columns_contain_only_periods_and_metrics() {
        let metrics = vec![metric("ci.builds", "Builds", MetricFormat::Integer)];

        let columns = plan_columns(&[], &metrics);
        let keys = columns
            .iter()
            .map(|column| column.metadata.key.as_str())
            .collect::<Vec<_>>();

        assert_eq!(keys, ["period", "from", "to", "ci.builds"]);
    }
}
