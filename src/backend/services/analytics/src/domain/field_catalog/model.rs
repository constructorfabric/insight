use serde::Deserialize;

/// Semantic role a dataset field plays. Authored in `roles.yaml`; the compiler
/// and the expression validator (slice 3) read capability from these.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FieldRole {
    /// Tenant isolation field — the injected tenancy predicate binds here.
    Tenant,
    /// The measured entity key (e.g. a person email resolved downstream).
    Entity,
    /// A timestamp usable for period bucketing. A dataset may expose several.
    EventTime,
    /// A breakdown field.
    Dimension,
    /// A numeric field a measure can sum/avg/min/max.
    Value,
    /// A field a measure can count distinct over.
    Subject,
}

/// Normalized field type — the closed set the semantic layer reasons about,
/// derived from the ClickHouse column type in the snapshot. Warehouse-specific
/// spellings (`Nullable(...)`, `LowCardinality(...)`, `DateTime64(3)`) collapse
/// to one of these; nullability is tracked separately on [`Field`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldType {
    String,
    Int,
    UInt,
    Float,
    /// Fixed-point (`Decimal(p, s)`) — a distinct measurable type from `Float`
    /// so the compiler can preserve exact currency/amount arithmetic.
    Decimal,
    Date,
    DateTime,
}

impl FieldType {
    /// Normalize a ClickHouse column type into `(field_type, nullable)`.
    /// Returns `None` for a base type the semantic layer does not model, so the
    /// consistency test rejects an authored field the layer cannot type.
    pub fn normalize(ch_type: &str) -> Option<(Self, bool)> {
        let (inner, nullable) = match strip_wrapper(ch_type, "Nullable") {
            Some(inner) => (inner, true),
            None => (ch_type, false),
        };
        // LowCardinality wraps the storage, not the logical type; peel it (and a
        // Nullable it may itself wrap) without changing the result.
        if let Some(unwrapped) = strip_wrapper(inner, "LowCardinality") {
            return Self::normalize(unwrapped).map(|(ty, inner_null)| (ty, nullable || inner_null));
        }

        let base = inner.split('(').next().unwrap_or(inner).trim();
        let ty = match base {
            "String" | "FixedString" | "UUID" => Self::String,
            "Int8" | "Int16" | "Int32" | "Int64" | "Int128" | "Int256" => Self::Int,
            "UInt8" | "UInt16" | "UInt32" | "UInt64" | "UInt128" | "UInt256" => Self::UInt,
            "Float32" | "Float64" => Self::Float,
            "Decimal" | "Decimal32" | "Decimal64" | "Decimal128" | "Decimal256" => Self::Decimal,
            "Date" | "Date32" => Self::Date,
            "DateTime" | "DateTime64" => Self::DateTime,
            _ => return None,
        };
        Some((ty, nullable))
    }
}

/// Peel a single `Wrapper(...)` layer, returning the inner type spelling.
fn strip_wrapper<'a>(ch_type: &'a str, wrapper: &str) -> Option<&'a str> {
    let rest = ch_type.strip_prefix(wrapper)?.strip_prefix('(')?;
    rest.strip_suffix(')')
}

/// Dedup strategy the compiler inherits when it reads the dataset.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReadDiscipline {
    /// `ReplacingMergeTree` and friends — read with `FINAL`/dedup.
    Final,
    /// Already unique — read directly.
    None,
}

/// One exposed field of a dataset: its authored role plus the type joined in
/// from the ClickHouse snapshot.
#[derive(Debug, Clone, PartialEq)]
pub struct Field {
    pub name: String,
    pub role: FieldRole,
    pub ty: FieldType,
    pub nullable: bool,
}

/// A queryable relation with its exposed, typed, role-annotated fields — the
/// unit a measure aggregates and the validation universe an expression is
/// checked against.
#[derive(Debug, Clone, PartialEq)]
pub struct Dataset {
    pub key: String,
    pub database: String,
    pub table: String,
    pub read_discipline: ReadDiscipline,
    pub fields: Vec<Field>,
}

impl Dataset {
    /// Fully-qualified relation name, the key used to look the dataset up in the
    /// ClickHouse type snapshot.
    pub fn relation(&self) -> String {
        format!("{}.{}", self.database, self.table)
    }

    pub fn fields_with_role(&self, role: FieldRole) -> impl Iterator<Item = &Field> {
        self.fields.iter().filter(move |f| f.role == role)
    }
}

/// The whole catalog — every product dataset the semantic layer can reason
/// about. Built once from `roles.yaml` joined with `types.snapshot.json`.
#[derive(Debug, Clone, PartialEq)]
pub struct FieldCatalog {
    pub datasets: Vec<Dataset>,
}
