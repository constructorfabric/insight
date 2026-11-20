use std::fmt;

use uuid::Uuid;

pub(crate) const DASHBOARD_NOT_EDITOR: &str =
    "only the owner or an editor can change this dashboard";
pub(crate) const DASHBOARD_NOT_OWNER: &str = "only the owner can delete this dashboard";
pub(crate) const DASHBOARD_NOT_SHARER: &str =
    "only the owner or an editor granted this dashboard directly or through everyone can share it";
pub(crate) const FOLDER_NOT_EDITOR: &str = "only the owner or an editor can change this folder";
pub(crate) const FOLDER_NOT_OWNER: &str = "only the owner can delete this folder";

const EVERYONE: &str = "everyone";

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Role {
    Administrator,
    Member,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Caller {
    pub(crate) person: PersonId,
    pub(crate) role: Role,
}

impl Caller {
    pub(crate) const fn administrator(person: PersonId) -> Self {
        Self {
            person,
            role: Role::Administrator,
        }
    }

    #[cfg(test)]
    pub(crate) const fn member(person: PersonId) -> Self {
        Self {
            person,
            role: Role::Member,
        }
    }

    pub(crate) fn is_administrator(self) -> bool {
        match self.role {
            Role::Administrator => true,
            Role::Member => false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) enum Owner {
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) enum Level {
    Viewer,
    Editor,
}

impl Level {
    pub(crate) fn parse(raw: &str) -> Option<Self> {
        match raw {
            "viewer" => Some(Self::Viewer),
            "editor" => Some(Self::Editor),
            _ => None,
        }
    }

    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Viewer => "viewer",
            Self::Editor => "editor",
        }
    }

    fn capped(self, role: Role) -> Self {
        match (self, role) {
            (Self::Editor, Role::Administrator) => Self::Editor,
            (Self::Editor, Role::Member) | (Self::Viewer, Role::Administrator | Role::Member) => {
                Self::Viewer
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) enum Grantee {
    Person(PersonId),
    Everyone,
}

impl Grantee {
    pub(crate) fn parse(raw: &str) -> Option<Self> {
        if raw == EVERYONE {
            return Some(Self::Everyone);
        }

        PersonId::parse(raw).map(Self::Person)
    }
}

impl fmt::Display for Grantee {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Person(person) => person.fmt(formatter),
            Self::Everyone => formatter.write_str(EVERYONE),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct FolderStanding {
    pub(crate) owner: Owner,
    pub(crate) granted: Option<Level>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FolderAccess {
    Owner,
    Editor,
    Viewer,
    Ownerless,
}

pub(crate) fn folder_access(caller: Caller, standing: FolderStanding) -> Option<FolderAccess> {
    match standing.owner {
        Owner::Unowned => match caller.role {
            Role::Administrator => Some(FolderAccess::Ownerless),
            Role::Member => None,
        },
        Owner::Person(owner) if owner == caller.person => Some(FolderAccess::Owner),
        Owner::Person(_) => standing
            .granted
            .map(|level| match level.capped(caller.role) {
                Level::Editor => FolderAccess::Editor,
                Level::Viewer => FolderAccess::Viewer,
            }),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DashboardStanding {
    pub(crate) owner: Owner,
    pub(crate) granted: Option<Level>,
    pub(crate) carried_by: Vec<FolderStanding>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Access {
    Owner,
    Editor,
    Viewer,
    Ownerless,
}

pub(crate) fn access(caller: Caller, standing: &DashboardStanding) -> Option<Access> {
    let Owner::Person(owner) = standing.owner else {
        return match caller.role {
            Role::Administrator => Some(Access::Ownerless),
            Role::Member => None,
        };
    };
    if owner == caller.person && caller.is_administrator() {
        return Some(Access::Owner);
    }

    let carried = standing
        .carried_by
        .iter()
        .filter_map(|folder| folder_access(caller, *folder))
        .filter_map(FolderAccess::carries)
        .max();
    let strongest = standing.granted.max(carried)?;

    match strongest.capped(caller.role) {
        Level::Editor => Some(Access::Editor),
        Level::Viewer => Some(Access::Viewer),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Refused {
    NotEditor,
    NotOwner,
    EditorByFolder,
    Ownerless,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Entry {
    Carrying,
    Filing,
}

impl Entry {
    pub(crate) fn carries(self) -> bool {
        match self {
            Self::Carrying => true,
            Self::Filing => false,
        }
    }
}

impl Access {
    pub(crate) fn change(self) -> Result<(), Refused> {
        match self {
            Self::Owner | Self::Editor | Self::Ownerless => Ok(()),
            Self::Viewer => Err(Refused::NotEditor),
        }
    }

    pub(crate) fn share(self, granted: Option<Level>) -> Result<(), Refused> {
        match (self, granted) {
            (Self::Owner, None | Some(Level::Viewer | Level::Editor))
            | (Self::Editor, Some(Level::Editor)) => Ok(()),
            (Self::Editor, None | Some(Level::Viewer)) => Err(Refused::EditorByFolder),
            (Self::Viewer, None | Some(Level::Viewer | Level::Editor)) => Err(Refused::NotEditor),
            (Self::Ownerless, None | Some(Level::Viewer | Level::Editor)) => {
                Err(Refused::Ownerless)
            }
        }
    }

    pub(crate) fn delete(self) -> Result<(), Refused> {
        match self {
            Self::Owner | Self::Ownerless => Ok(()),
            Self::Editor | Self::Viewer => Err(Refused::NotOwner),
        }
    }

    pub(crate) fn entry(self, granted: Option<Level>) -> Entry {
        match (self, granted) {
            (Self::Owner | Self::Ownerless, None | Some(Level::Viewer | Level::Editor))
            | (Self::Editor, Some(Level::Editor)) => Entry::Carrying,
            (Self::Editor, None | Some(Level::Viewer))
            | (Self::Viewer, None | Some(Level::Viewer | Level::Editor)) => Entry::Filing,
        }
    }

    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Owner => "owner",
            Self::Editor => "editor",
            Self::Viewer => "viewer",
            Self::Ownerless => "ownerless",
        }
    }
}

impl FolderAccess {
    fn carries(self) -> Option<Level> {
        match self {
            Self::Owner | Self::Editor => Some(Level::Editor),
            Self::Viewer => Some(Level::Viewer),
            Self::Ownerless => None,
        }
    }

    pub(crate) fn arrange(self) -> Result<(), Refused> {
        match self {
            Self::Owner | Self::Editor | Self::Ownerless => Ok(()),
            Self::Viewer => Err(Refused::NotEditor),
        }
    }

    pub(crate) fn share(self) -> Result<(), Refused> {
        match self {
            Self::Owner | Self::Editor => Ok(()),
            Self::Viewer => Err(Refused::NotEditor),
            Self::Ownerless => Err(Refused::Ownerless),
        }
    }

    pub(crate) fn delete(self) -> Result<(), Refused> {
        match self {
            Self::Owner | Self::Ownerless => Ok(()),
            Self::Editor | Self::Viewer => Err(Refused::NotOwner),
        }
    }

    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Owner => "owner",
            Self::Editor => "editor",
            Self::Viewer => "viewer",
            Self::Ownerless => "ownerless",
        }
    }
}

pub(crate) fn shelved(dashboard: Access, folder: FolderAccess) -> Result<(), Shelf> {
    match (dashboard, folder) {
        (Access::Ownerless, FolderAccess::Ownerless)
        | (
            Access::Owner | Access::Editor | Access::Viewer,
            FolderAccess::Owner | FolderAccess::Editor | FolderAccess::Viewer,
        ) => Ok(()),
        (Access::Ownerless, FolderAccess::Owner | FolderAccess::Editor | FolderAccess::Viewer) => {
            Err(Shelf::OwnerlessDashboard)
        }
        (Access::Owner | Access::Editor | Access::Viewer, FolderAccess::Ownerless) => {
            Err(Shelf::OwnerlessFolder)
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Shelf {
    OwnerlessDashboard,
    OwnerlessFolder,
}

impl Shelf {
    pub(crate) fn said(self) -> &'static str {
        match self {
            Self::OwnerlessDashboard => "an ownerless dashboard goes only in an ownerless folder",
            Self::OwnerlessFolder => "an ownerless folder holds only ownerless dashboards",
        }
    }
}

#[cfg(test)]
mod tests;
