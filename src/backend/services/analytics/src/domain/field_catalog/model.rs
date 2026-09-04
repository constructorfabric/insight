//! The relations the warehouse carries and the type of every column in them.

/// A column's type, normalized from the warehouse's spelling.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FieldType {
    pub class: TypeClass,
    pub nullable: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TypeClass {
    Text,
    Number,
    Boolean,
    Date,
    Timestamp,
    Uuid,
    /// INVARIANT: never groupable or measurable.
    Composite,
}

/// How a relation must be scanned for its rows to be the deduplicated truth.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReadDiscipline {
    Plain,
    /// A replacing engine: every read must collapse superseded rows.
    Final,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CatalogColumn {
    pub name: String,
    pub field_type: FieldType,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CatalogRelation {
    pub database: String,
    pub relation: String,
    pub read_discipline: ReadDiscipline,
    pub columns: Vec<CatalogColumn>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FieldCatalog {
    pub relations: Vec<CatalogRelation>,
}

impl FieldType {
    // SAFETY: an unknown type name is admitted as composite, so a declaration
    // naming the column is refused rather than the column silently invented.
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
}

impl ReadDiscipline {
    // INVARIANT: a replacing engine keeps superseded rows until a merge that may
    // never come, so every read of one must collapse them.
    pub fn for_engine(engine: &str) -> Self {
        if engine.starts_with("Replacing") || engine.starts_with("Collapsing") {
            Self::Final
        } else {
            Self::Plain
        }
    }
}

impl CatalogRelation {
    pub fn column(&self, name: &str) -> Option<&CatalogColumn> {
        self.columns.iter().find(|column| column.name == name)
    }
}

impl FieldCatalog {
    pub fn relation(&self, database: &str, relation: &str) -> Option<&CatalogRelation> {
        self.relations
            .iter()
            .find(|candidate| candidate.database == database && candidate.relation == relation)
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
        ];

        for (raw, class, nullable) in cases {
            let parsed = FieldType::parse(raw);
            assert_eq!(parsed.class, class, "{raw}");
            assert_eq!(parsed.nullable, nullable, "{raw}");
        }
    }

    #[test]
    fn an_unknown_type_stays_composite_rather_than_being_guessed() {
        assert_eq!(
            FieldType::parse("Variant(String, Int64)").class,
            TypeClass::Composite
        );
    }

    #[test]
    fn a_replacing_engine_must_be_read_collapsed() {
        assert_eq!(
            ReadDiscipline::for_engine("ReplacingMergeTree"),
            ReadDiscipline::Final
        );
        assert_eq!(
            ReadDiscipline::for_engine("CollapsingMergeTree"),
            ReadDiscipline::Final
        );
        assert_eq!(
            ReadDiscipline::for_engine("MergeTree"),
            ReadDiscipline::Plain
        );
        assert_eq!(ReadDiscipline::for_engine("View"), ReadDiscipline::Plain);
    }
}
