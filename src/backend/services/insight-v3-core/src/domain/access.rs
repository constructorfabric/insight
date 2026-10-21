//! Who may see a dashboard, and what each kind of sight allows.
//!
//! One rule for every surface: a dashboard is seen by its owner, by whoever
//! it is shared with, and - while it predates ownership - by everyone.

use std::fmt;

use uuid::Uuid;

/// A person as the identity service knows them: the gateway token's subject.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) struct PersonId(Uuid);

impl PersonId {
    pub(crate) const fn new(id: Uuid) -> Self {
        Self(id)
    }

    pub(crate) fn parse(raw: &str) -> Option<Self> {
        Uuid::parse_str(raw.trim()).ok().map(Self)
    }

    pub(crate) fn uuid(self) -> Uuid {
        self.0
    }
}

impl fmt::Display for PersonId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.hyphenated().fmt(formatter)
    }
}

/// Who a dashboard, folder or tag belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) enum Owner {
    /// Stored before ownership existed.
    Unowned,
    Person(PersonId),
}

impl Owner {
    pub(crate) fn person(self) -> Option<PersonId> {
        match self {
            Self::Unowned => None,
            Self::Person(person) => Some(person),
        }
    }
}

/// How the caller sees one dashboard. No access at all is no dashboard.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Access {
    Owner,
    Viewer,
    Ownerless,
}

/// The one rule deciding whether, and how, the caller sees a dashboard.
pub(crate) fn access(caller: PersonId, owner: Owner, shared_with_caller: bool) -> Option<Access> {
    match owner {
        Owner::Person(person) if person == caller => Some(Access::Owner),
        Owner::Person(_) if shared_with_caller => Some(Access::Viewer),
        Owner::Person(_) => None,
        Owner::Unowned => Some(Access::Ownerless),
    }
}

/// Why a dashboard the caller sees still refuses them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Refused {
    NotOwner,
    Ownerless,
}

impl Access {
    /// Opening it, running its widgets and pinning it.
    pub(crate) fn may_open(self) -> bool {
        match self {
            Self::Owner | Self::Viewer | Self::Ownerless => true,
        }
    }

    /// Editing its body, renaming, deleting, duplicating it and setting its
    /// tags.
    pub(crate) fn change(self) -> Result<(), Refused> {
        match self {
            Self::Owner | Self::Ownerless => Ok(()),
            Self::Viewer => Err(Refused::NotOwner),
        }
    }

    /// Sharing it, withdrawing a share, and listing its shares.
    pub(crate) fn share(self) -> Result<(), Refused> {
        match self {
            Self::Owner => Ok(()),
            Self::Viewer => Err(Refused::NotOwner),
            Self::Ownerless => Err(Refused::Ownerless),
        }
    }

    /// Whose folders the caller files it in: their own, or the ownerless ones.
    pub(crate) fn shelf(self, caller: PersonId) -> Owner {
        match self {
            Self::Owner | Self::Viewer => Owner::Person(caller),
            Self::Ownerless => Owner::Unowned,
        }
    }

    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Owner => "owner",
            Self::Viewer => "viewer",
            Self::Ownerless => "ownerless",
        }
    }
}

#[cfg(test)]
mod tests;
