use serde::{Deserialize, Serialize};

use super::definition::EvidenceGranularity;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceColumnType {
    #[default]
    String,
    Number,
    Date,
}

/// One human-facing column the drilldown projects out of an evidence row's
/// `details` map. `key` names the map entry the gold model writes; the label
/// and type are what the reader sees.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EvidenceDetailColumn {
    pub key: String,
    pub label: String,
    #[serde(default)]
    pub r#type: EvidenceColumnType,
}

/// How a measure's evidence reads: the detail columns its rows carry, in the
/// order the reader sees them, and whether the contributed number earns a
/// column of its own. Declared per measure in the builtin registry, stored on
/// the measure row, and probed against the evidence relation by the validator —
/// so the gold model and the serving layer agree through one declaration
/// instead of parallel edits.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EvidencePresentation {
    #[serde(default)]
    pub detail_columns: Vec<EvidenceDetailColumn>,
    pub show_value: bool,
}

impl EvidencePresentation {
    /// What a measure presents when it declares nothing: no detail columns, and
    /// a value column everywhere the row is not itself the thing counted.
    pub fn undeclared(granularity: EvidenceGranularity) -> Self {
        Self {
            detail_columns: Vec::new(),
            show_value: granularity != EvidenceGranularity::Event,
        }
    }

    /// Reads a declaration back from the measure row. `None` means the stored
    /// JSON does not describe a presentation — a configuration fault each
    /// caller reports in its own terms, never a silent fallback to the default.
    pub fn parse(json: &str) -> Option<Self> {
        serde_json::from_str(json).ok()
    }

    pub fn detail_keys(&self) -> impl Iterator<Item = &str> {
        self.detail_columns.iter().map(|column| column.key.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_undeclared_event_measure_withholds_the_value_column() {
        let event = EvidencePresentation::undeclared(EvidenceGranularity::Event);
        assert!(event.detail_columns.is_empty());
        assert!(!event.show_value);

        for granularity in [
            EvidenceGranularity::SourceSummary,
            EvidenceGranularity::DerivedPopulation,
        ] {
            assert!(
                EvidencePresentation::undeclared(granularity).show_value,
                "{granularity:?} rows are only readable with their number"
            );
        }
    }

    #[test]
    fn a_declaration_round_trips_through_its_stored_form() {
        let declared = EvidencePresentation {
            detail_columns: vec![
                EvidenceDetailColumn {
                    key: "ref".to_owned(),
                    label: "Ref".to_owned(),
                    r#type: EvidenceColumnType::String,
                },
                EvidenceDetailColumn {
                    key: "lines_added".to_owned(),
                    label: "Lines added".to_owned(),
                    r#type: EvidenceColumnType::Number,
                },
            ],
            show_value: false,
        };
        let stored = serde_json::to_string(&declared)
            .unwrap_or_else(|error| panic!("declaration must serialize: {error}"));
        assert_eq!(EvidencePresentation::parse(&stored), Some(declared));
    }

    #[test]
    fn a_column_declaring_no_type_reads_as_text() {
        let parsed = EvidencePresentation::parse(
            r#"{"detail_columns":[{"key":"title","label":"Title"}],"show_value":true}"#,
        )
        .unwrap_or_else(|| panic!("declaration must parse"));
        assert_eq!(parsed.detail_columns[0].r#type, EvidenceColumnType::String);
    }

    #[test]
    fn malformed_stored_declarations_are_not_mistaken_for_the_default() {
        for stored in [
            "not json",
            "{}",
            r#"{"show_value":true,"unexpected":1}"#,
            r#"{"detail_columns":[{"key":"ref"}],"show_value":true}"#,
        ] {
            assert_eq!(
                EvidencePresentation::parse(stored),
                None,
                "should reject: {stored:?}"
            );
        }
    }
}
