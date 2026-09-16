//! Where a dataset is between being named and being usable, and who is
//! moving it.

use std::fmt;

/// A dataset as its row records it. A dataset with no row is absent.
///
/// Claimed and Removing both hold the name against every other writer: one so
/// an unfinished create can be repeated, the other so nothing takes the name
/// while its records are being dropped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DatasetState {
    Claimed,
    Ready,
    Removing,
}

/// What an attempt holds a dataset for.
///
/// Replacing a ready declaration takes none: it touches no table, so it
/// decides and commits in one transaction rather than leasing the dataset.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Operation {
    Create,
    Remove,
}

impl DatasetState {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Claimed => "claimed",
            Self::Ready => "ready",
            Self::Removing => "removing",
        }
    }

    /// The state as the row spells it, or nothing for a word this service
    /// never wrote.
    pub(crate) fn parse(value: &str) -> Option<Self> {
        match value {
            "claimed" => Some(Self::Claimed),
            "ready" => Some(Self::Ready),
            "removing" => Some(Self::Removing),
            _ => None,
        }
    }
}

impl Operation {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Create => "create",
            Self::Remove => "remove",
        }
    }

    pub(crate) fn parse(value: &str) -> Option<Self> {
        match value {
            "create" => Some(Self::Create),
            "remove" => Some(Self::Remove),
            _ => None,
        }
    }
}

impl fmt::Display for DatasetState {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl fmt::Display for Operation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

#[cfg(test)]
mod tests;
