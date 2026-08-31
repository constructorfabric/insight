//! Field-catalog vocabulary: what a dataset field is and what it may be used
//! for. Types come from the schema artifact; roles are authored.

use serde::{Deserialize, Serialize};

/// What a row — and so every value read from it — is keyed by.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Default,
    Serialize,
    Deserialize,
    utoipa::ToSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum EntityType {
    /// One row per observed person: a read resolves identities and reports a
    /// value per person.
    #[default]
    Person,
    /// One row per tenant: there is no person to resolve, and the tenant is
    /// the single subject every value belongs to.
    Tenant,
}

impl EntityType {
    pub fn as_db(self) -> &'static str {
        match self {
            Self::Person => "person",
            Self::Tenant => "tenant",
        }
    }
}

impl std::fmt::Display for EntityType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_db())
    }
}

/// A field's type, normalized from the warehouse's spelling.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FieldType {
    pub class: TypeClass,
    pub nullable: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TypeClass {
    /// Text, including enumerations and fixed-width strings.
    Text,
    /// Whole and fractional numbers alike.
    Number,
    Boolean,
    Date,
    Timestamp,
    Uuid,
    /// Arrays, tuples and maps: nameable, never measurable or groupable.
    Composite,
}

/// INVARIANT: a field carries at most one of these; display roles are declared
/// separately so one field can both group and label.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FieldRole {
    /// Identifies who or what a row is about.
    Entity,
    /// Groupable: a stable value key, optionally paired with a display label.
    Dimension,
    /// Numeric, aggregatable.
    Measurable,
    /// A timestamp candidate for time bucketing.
    EventTime,
    /// The tenant discriminator the injected tenancy predicate binds to.
    Tenant,
}

/// What a field contributes to a drilldown row, independent of [`FieldRole`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DisplayRole {
    /// Names the thing the row is: a PR title, a document name.
    Title,
    /// The human-quotable handle: a PR number, a commit hash.
    Reference,
    /// Who did it, as displayed.
    Actor,
    /// Where it lives, as displayed.
    Location,
    /// A URL to the record in its source system.
    Link,
}

/// How a relation must be read for its rows to be the deduplicated truth.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReadDiscipline {
    /// Rows are already unique; read them directly.
    Direct,
    /// A replacing engine: reads collapse duplicates by the sorting key.
    Collapsing,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CatalogField {
    pub name: String,
    pub field_type: FieldType,
    pub role: Option<FieldRole>,
    pub display: Vec<DisplayRole>,
    /// For a dimension: the field carrying its human label.
    pub label_field: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CatalogDataset {
    pub key: String,
    pub database: String,
    pub relation: String,
    /// What one row is about. INVARIANT: this is the dataset's grain, and a
    /// metric may only be authored at the grain of every dataset it reads.
    pub entity_type: EntityType,
    pub read_discipline: ReadDiscipline,
    pub sorting_key: Vec<String>,
    /// INVARIANT: a row-by-row read orders by the sorting key AND these, since
    /// the sorting key alone does not separate rows that tie on it.
    pub row_identity: Vec<String>,
    pub fields: Vec<CatalogField>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FieldCatalog {
    pub datasets: Vec<CatalogDataset>,
}

impl FieldType {
    /// An unknown type name is admitted as composite: the catalog records that
    /// the field exists without claiming it can be measured or grouped.
    pub fn parse(raw: &str) -> Self {
        let trimmed = raw.trim();
        if let Some(inner) = strip_wrapper(trimmed, "Nullable") {
            return Self {
                class: Self::parse(inner).class,
                nullable: true,
            };
        }
        if let Some(inner) = strip_wrapper(trimmed, "LowCardinality") {
            return Self::parse(inner);
        }
        let head = trimmed
            .split_once('(')
            .map_or(trimmed, |(head, _)| head)
            .trim();
        let class = match head {
            "String" | "FixedString" | "IPv4" | "IPv6" => TypeClass::Text,
            _ if head.starts_with("Enum") => TypeClass::Text,
            "Bool" => TypeClass::Boolean,
            "Date" | "Date32" => TypeClass::Date,
            "DateTime" | "DateTime64" => TypeClass::Timestamp,
            "UUID" => TypeClass::Uuid,
            "Float32" | "Float64" | "Decimal" => TypeClass::Number,
            _ if head.starts_with("Int") || head.starts_with("UInt") => TypeClass::Number,
            _ => TypeClass::Composite,
        };
        Self {
            class,
            nullable: false,
        }
    }

    pub fn admits(self, role: FieldRole) -> bool {
        match role {
            FieldRole::Entity | FieldRole::Tenant => {
                matches!(self.class, TypeClass::Text | TypeClass::Uuid)
            }
            FieldRole::Dimension => matches!(
                self.class,
                TypeClass::Text | TypeClass::Uuid | TypeClass::Boolean | TypeClass::Number
            ),
            FieldRole::Measurable => matches!(self.class, TypeClass::Number),
            FieldRole::EventTime => matches!(self.class, TypeClass::Date | TypeClass::Timestamp),
        }
    }
}

impl ReadDiscipline {
    /// The discipline a ClickHouse engine name implies.
    pub fn for_engine(engine: &str) -> Self {
        if engine.starts_with("Replacing") || engine.starts_with("Collapsing") {
            Self::Collapsing
        } else {
            Self::Direct
        }
    }
}

impl CatalogDataset {
    pub fn field(&self, name: &str) -> Option<&CatalogField> {
        self.fields.iter().find(|field| field.name == name)
    }

    pub fn fields_with_role(&self, role: FieldRole) -> impl Iterator<Item = &CatalogField> {
        self.fields
            .iter()
            .filter(move |field| field.role == Some(role))
    }
}

impl FieldCatalog {
    pub fn dataset(&self, key: &str) -> Option<&CatalogDataset> {
        self.datasets.iter().find(|dataset| dataset.key == key)
    }
}

fn strip_wrapper<'a>(raw: &'a str, wrapper: &str) -> Option<&'a str> {
    raw.strip_prefix(wrapper)
        .and_then(|rest| rest.strip_prefix('('))
        .and_then(|rest| rest.strip_suffix(')'))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn clickhouse_types_map_to_classes() {
        let cases = [
            ("String", TypeClass::Text, false),
            ("Nullable(String)", TypeClass::Text, true),
            ("Enum8('set' = 1, 'add' = 2)", TypeClass::Text, false),
            ("LowCardinality(String)", TypeClass::Text, false),
            ("Nullable(LowCardinality(String))", TypeClass::Text, true),
            ("Int64", TypeClass::Number, false),
            ("Nullable(UInt8)", TypeClass::Number, true),
            ("Nullable(Decimal(38, 9))", TypeClass::Number, true),
            ("Float64", TypeClass::Number, false),
            ("Date", TypeClass::Date, false),
            ("Nullable(Date32)", TypeClass::Date, true),
            ("DateTime64(3)", TypeClass::Timestamp, false),
            ("Nullable(DateTime)", TypeClass::Timestamp, true),
            ("Nullable(UUID)", TypeClass::Uuid, true),
            ("Map(String, String)", TypeClass::Composite, false),
            ("Array(String)", TypeClass::Composite, false),
            (
                "Array(Tuple(key String, value String, label Nullable(String)))",
                TypeClass::Composite,
                false,
            ),
        ];
        for (raw, class, nullable) in cases {
            let parsed = FieldType::parse(raw);
            assert_eq!(parsed.class, class, "{raw}");
            assert_eq!(parsed.nullable, nullable, "{raw}");
        }
    }

    #[test]
    fn unknown_types_stay_composite_rather_than_guessing() {
        assert_eq!(
            FieldType::parse("Variant(String, Int64)").class,
            TypeClass::Composite
        );
    }

    #[test]
    fn roles_admit_only_types_they_can_use() {
        let text = FieldType::parse("String");
        let number = FieldType::parse("Nullable(Int64)");
        let date = FieldType::parse("Date");
        let composite = FieldType::parse("Map(String, String)");

        assert!(text.admits(FieldRole::Entity));
        assert!(text.admits(FieldRole::Dimension));
        assert!(!text.admits(FieldRole::Measurable));
        assert!(!text.admits(FieldRole::EventTime));

        assert!(number.admits(FieldRole::Measurable));
        assert!(number.admits(FieldRole::Dimension));
        assert!(!number.admits(FieldRole::Entity));

        assert!(date.admits(FieldRole::EventTime));
        assert!(!date.admits(FieldRole::Dimension));

        for role in [
            FieldRole::Entity,
            FieldRole::Dimension,
            FieldRole::Measurable,
            FieldRole::EventTime,
            FieldRole::Tenant,
        ] {
            assert!(!composite.admits(role), "{role:?}");
        }
    }

    #[test]
    fn replacing_engines_read_collapsed() {
        assert_eq!(
            ReadDiscipline::for_engine("ReplacingMergeTree"),
            ReadDiscipline::Collapsing
        );
        assert_eq!(
            ReadDiscipline::for_engine("MergeTree"),
            ReadDiscipline::Direct
        );
        assert_eq!(ReadDiscipline::for_engine("View"), ReadDiscipline::Direct);
    }
}
