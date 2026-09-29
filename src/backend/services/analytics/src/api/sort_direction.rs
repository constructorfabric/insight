#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SortDirection {
    Asc,
    Desc,
}

impl SortDirection {
    pub(crate) const EXPECTED: &'static str = "expected one of: asc, desc";

    pub(crate) fn from_param(raw: Option<&str>) -> Option<Self> {
        match raw {
            None | Some("desc") => Some(Self::Desc),
            Some("asc") => Some(Self::Asc),
            Some(_) => None,
        }
    }

    pub(crate) fn as_sql(self) -> &'static str {
        match self {
            Self::Asc => "ASC",
            Self::Desc => "DESC",
        }
    }
}
