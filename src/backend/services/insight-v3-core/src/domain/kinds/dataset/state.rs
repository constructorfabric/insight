use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DatasetState {
    Claimed,
    Ready,
    Removing,
}

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
