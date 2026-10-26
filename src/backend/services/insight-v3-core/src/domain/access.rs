//! Who may see a dashboard or a folder, and what each kind of sight allows.
//!
//! One rule for every surface: a thing is seen by its owner, by whoever a
//! grant reaches, and - for a dashboard - by whoever sees a folder carrying
//! it. Ownerless rows from before ownership are the administrators' alone.

use std::fmt;

use uuid::Uuid;

pub(crate) const DASHBOARD_NOT_EDITOR: &str =
    "only the owner or an editor can change this dashboard";
pub(crate) const DASHBOARD_NOT_OWNER: &str = "only the owner can delete this dashboard";
pub(crate) const DASHBOARD_NOT_SHARER: &str =
    "only the owner or an editor granted this dashboard directly or through everyone can share it";
pub(crate) const FOLDER_NOT_EDITOR: &str = "only the owner or an editor can change this folder";
pub(crate) const FOLDER_NOT_OWNER: &str = "only the owner can delete this folder";

/// What `Grantee::Everyone` is stored as.
const EVERYONE: &str = "everyone";

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

/// What the caller may do beyond viewing, as identity answers it per request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Role {
    Administrator,
    Member,
}

/// Who is asking, and in what role.
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

/// How much a grant gives, weakest first.
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

    /// What the level gives in `role`: a member is a viewer everywhere.
    fn capped(self, role: Role) -> Self {
        match (self, role) {
            (Self::Editor, Role::Administrator) => Self::Editor,
            (Self::Editor, Role::Member) | (Self::Viewer, Role::Administrator | Role::Member) => {
                Self::Viewer
            }
        }
    }
}

/// Who a grant reaches.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) enum Grantee {
    Person(PersonId),
    /// Every signed-in person, set only by an explicit grant.
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

/// What decides one caller's access to a folder: its owner, and the
/// strongest of its grants to the caller or to everyone.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct FolderStanding {
    pub(crate) owner: Owner,
    pub(crate) granted: Option<Level>,
}

/// How the caller sees one folder. No access at all is no folder.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FolderAccess {
    Owner,
    Editor,
    Viewer,
    Ownerless,
}

/// The one rule deciding whether, and how, the caller sees a folder. A
/// folder may be owned in any role.
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

/// What decides one caller's access to a dashboard: its owner, the strongest
/// of its grants to the caller or to everyone, and each folder holding it
/// through a carrying entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DashboardStanding {
    pub(crate) owner: Owner,
    pub(crate) granted: Option<Level>,
    pub(crate) carried_by: Vec<FolderStanding>,
}

/// How the caller sees one dashboard. No access at all is no dashboard.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Access {
    Owner,
    Editor,
    Viewer,
    Ownerless,
}

/// The one rule deciding whether, and how, the caller sees a dashboard.
///
/// INVARIANT: the store's visibility predicates keep exactly the rows this
/// answers `Some` for.
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

/// Why something the caller sees still refuses them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Refused {
    /// Only the owner or an editor may.
    NotEditor,
    /// Only the owner may.
    NotOwner,
    /// Only the owner or an editor by a grant to them or to everyone may,
    /// never an editor through a folder.
    EditorByFolder,
    /// Nobody may, since it predates ownership.
    Ownerless,
}

/// What an entry a caller makes carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Entry {
    /// The folder's audience reaches the dashboard.
    Carrying,
    /// The dashboard shows there only to who sees it anyway.
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
    /// Editing its body, renaming, arranging, setting its tags and
    /// duplicating it.
    pub(crate) fn change(self) -> Result<(), Refused> {
        match self {
            Self::Owner | Self::Editor | Self::Ownerless => Ok(()),
            Self::Viewer => Err(Refused::NotEditor),
        }
    }

    /// Listing, adding, changing and withdrawing its grants, given the
    /// strongest of its grants to the caller or to everyone: the standing a
    /// carrying entry needs.
    ///
    /// INVARIANT: the stores check the same standing in the write that
    /// changes a grant.
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

    /// What an entry of it made by the caller carries, given the strongest
    /// of its grants to them or to everyone: only its owner and a granted
    /// editor carry, never an editor through a folder.
    ///
    /// INVARIANT: the stores demote a carrying entry whose placer no longer
    /// qualifies by this rule whenever the dashboard's grants or owner
    /// change.
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
    /// What a carrying entry in the folder gives a dashboard.
    fn carries(self) -> Option<Level> {
        match self {
            Self::Owner | Self::Editor => Some(Level::Editor),
            Self::Viewer => Some(Level::Viewer),
            Self::Ownerless => None,
        }
    }

    /// Adding and removing entries, and renaming it.
    pub(crate) fn arrange(self) -> Result<(), Refused> {
        match self {
            Self::Owner | Self::Editor | Self::Ownerless => Ok(()),
            Self::Viewer => Err(Refused::NotEditor),
        }
    }

    /// Listing, adding, changing and withdrawing its grants; the caller must
    /// also be an administrator.
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

/// Whether a dashboard seen this way may sit in a folder seen that way: an
/// ownerless dashboard only in an ownerless folder, and an owned one only in
/// an owned folder.
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

/// Why a dashboard does not fit a folder.
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
