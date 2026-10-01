use toolkit_canonical_errors::CanonicalError;

use super::super::error::UsageError;
use super::super::sort_direction::SortDirection;

// INVARIANT: `ORDER BY` takes no bind, so these names are spliced in as-is and
// must stay literals, never caller text.
pub(super) trait SortKey: Copy {
    const DEFAULT: Self;
    const EXPECTED: &'static str;
    const TIE_BREAK: &'static [&'static str];

    fn from_param(raw: &str) -> Option<Self>;

    fn ranked(self) -> &'static [&'static str];
}

#[derive(Debug, Clone, Copy)]
pub(super) enum PeopleSort {
    Visits,
    PageViews,
    LastSeen,
}

impl SortKey for PeopleSort {
    const DEFAULT: Self = Self::Visits;
    const EXPECTED: &'static str = "expected one of: visits, page_views, last_seen";
    const TIE_BREAK: &'static [&'static str] = &["person"];

    fn from_param(raw: &str) -> Option<Self> {
        match raw {
            "visits" => Some(Self::Visits),
            "page_views" => Some(Self::PageViews),
            "last_seen" => Some(Self::LastSeen),
            _ => None,
        }
    }

    fn ranked(self) -> &'static [&'static str] {
        match self {
            Self::Visits => &["visits", "page_views"],
            Self::PageViews => &["page_views", "visits"],
            Self::LastSeen => &["last_ts"],
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub(super) enum PagesSort {
    Views,
    Visitors,
}

impl SortKey for PagesSort {
    const DEFAULT: Self = Self::Views;
    const EXPECTED: &'static str = "expected one of: views, visitors";
    const TIE_BREAK: &'static [&'static str] = &["path"];

    fn from_param(raw: &str) -> Option<Self> {
        match raw {
            "views" => Some(Self::Views),
            "visitors" => Some(Self::Visitors),
            _ => None,
        }
    }

    fn ranked(self) -> &'static [&'static str] {
        match self {
            Self::Views => &["views"],
            Self::Visitors => &["visitors"],
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub(super) enum ActionsSort {
    Opens,
    People,
}

impl SortKey for ActionsSort {
    const DEFAULT: Self = Self::Opens;
    const EXPECTED: &'static str = "expected one of: opens, people";
    const TIE_BREAK: &'static [&'static str] = &["event_name", "target"];

    fn from_param(raw: &str) -> Option<Self> {
        match raw {
            "opens" => Some(Self::Opens),
            "people" => Some(Self::People),
            _ => None,
        }
    }

    fn ranked(self) -> &'static [&'static str] {
        match self {
            Self::Opens => &["opens"],
            Self::People => &["people"],
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub(super) struct Order<K> {
    key: K,
    direction: SortDirection,
}

impl<K: SortKey> Default for Order<K> {
    fn default() -> Self {
        Self {
            key: K::DEFAULT,
            direction: SortDirection::Desc,
        }
    }
}

impl<K: SortKey> Order<K> {
    pub(super) fn parse(
        sort: Option<&str>,
        direction: Option<&str>,
    ) -> Result<Self, CanonicalError> {
        let key = match sort {
            None => K::DEFAULT,
            Some(raw) => K::from_param(raw).ok_or_else(|| violation("sort", K::EXPECTED))?,
        };

        let direction = SortDirection::from_param(direction)
            .ok_or_else(|| violation("direction", SortDirection::EXPECTED))?;

        Ok(Self { key, direction })
    }

    pub(super) fn clause(self, of: &str) -> String {
        let direction = self.direction.as_sql();
        let ranked = self
            .key
            .ranked()
            .iter()
            .map(|column| format!("{of}{column} {direction}"));
        let tie_break = K::TIE_BREAK.iter().map(|column| format!("{of}{column}"));

        ranked.chain(tie_break).collect::<Vec<_>>().join(", ")
    }
}

fn violation(field: &str, description: &str) -> CanonicalError {
    UsageError::invalid_argument()
        .with_field_violation(field, description, "UNSUPPORTED")
        .create()
}

#[cfg(test)]
mod tests;
