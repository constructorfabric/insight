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

    fn parse(json: &str) -> Option<Self> {
        let presentation: Self = serde_json::from_str(json).ok()?;
        presentation
            .detail_columns
            .iter()
            .all(|column| is_column_key(&column.key))
            .then_some(presentation)
    }

    pub fn detail_keys(&self) -> impl Iterator<Item = &str> {
        self.detail_columns.iter().map(|column| column.key.as_str())
    }
}

/// SAFETY: a column key is inlined into generated SQL as a string literal, and
/// the ClickHouse driver scans the raw query text for `?` without regard for
/// quoting — a key carrying one would add a bind slot and shift every later
/// parameter, returning wrong rows with no error. The stored declaration is
/// free-form JSON, so the shape is enforced here rather than assumed: lowercase
/// snake case, which admits neither `?` nor a quote nor a backslash.
fn is_column_key(key: &str) -> bool {
    !key.is_empty()
        && key
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
        && !key.starts_with(|first: char| first.is_ascii_digit() || first == '_')
}

/// What a measure row holds under `evidence_presentation`. Three states rather
/// than an `Option`, because stored JSON that does not describe a presentation
/// is a configuration fault to report — not the absence of a declaration, which
/// is ordinary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StoredPresentation {
    Absent,
    Declared(EvidencePresentation),
    Unreadable,
}

impl StoredPresentation {
    pub fn read(stored: Option<&str>) -> Self {
        let Some(stored) = stored else {
            return Self::Absent;
        };
        EvidencePresentation::parse(stored).map_or(Self::Unreadable, Self::Declared)
    }

    /// How the measure presents, given the granularity its rows carry. `None`
    /// where the declaration is unreadable — the caller decides what an
    /// unusable declaration costs it.
    pub fn or_undeclared(&self, granularity: EvidenceGranularity) -> Option<EvidencePresentation> {
        match self {
            Self::Absent => Some(EvidencePresentation::undeclared(granularity)),
            Self::Declared(presentation) => Some(presentation.clone()),
            Self::Unreadable => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_declaration_whose_column_key_could_reach_sql_is_unreadable() {
        for key in ["pr?ref", "pr'ref", "pr\\ref", "PrRef", "1ref", "_ref", ""] {
            let json = format!(
                r#"{{"show_value":false,"detail_columns":[{{"key":"{key}","label":"Ref"}}]}}"#
            );
            assert_eq!(
                StoredPresentation::read(Some(&json)),
                StoredPresentation::Unreadable,
                "should refuse: {key:?}"
            );
        }
        let json = r#"{"show_value":false,"detail_columns":[{"key":"pr_ref","label":"Ref"}]}"#;
        assert!(matches!(
            StoredPresentation::read(Some(json)),
            StoredPresentation::Declared(_)
        ));
    }

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
        assert_eq!(
            StoredPresentation::read(Some(&stored)),
            StoredPresentation::Declared(declared)
        );
    }

    #[test]
    fn a_column_declaring_no_type_reads_as_text() {
        let StoredPresentation::Declared(parsed) = StoredPresentation::read(Some(
            r#"{"detail_columns":[{"key":"title","label":"Title"}],"show_value":true}"#,
        )) else {
            panic!("declaration must parse");
        };
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
                StoredPresentation::read(Some(stored)),
                StoredPresentation::Unreadable,
                "should reject: {stored:?}"
            );
        }
    }

    #[test]
    fn an_absent_declaration_and_an_unusable_one_read_apart() {
        let granularity = EvidenceGranularity::Event;

        assert_eq!(
            StoredPresentation::read(None).or_undeclared(granularity),
            Some(EvidencePresentation::undeclared(granularity)),
            "declaring nothing is ordinary"
        );
        assert_eq!(
            StoredPresentation::read(Some("not json")).or_undeclared(granularity),
            None,
            "an unusable declaration is a fault, not a default"
        );

        let declared = r#"{"detail_columns":[{"key":"ref","label":"Ref"}],"show_value":true}"#;
        let presentation = StoredPresentation::read(Some(declared))
            .or_undeclared(granularity)
            .unwrap_or_else(|| panic!("a readable declaration must resolve"));
        assert!(presentation.show_value);
        assert_eq!(presentation.detail_keys().collect::<Vec<_>>(), ["ref"]);
    }
}
