//! The definition store the tests use.
//!
//! It keeps definitions in a map, so a test that stores one and reads it back
//! asserts on behaviour rather than on a database's wire protocol.

use std::collections::{BTreeMap, HashMap};
use std::sync::Mutex;

use async_trait::async_trait;
use chrono::{DateTime, Utc};

use crate::domain::access::{
    Caller, DashboardStanding, Entry, FolderStanding, Grantee, Level, Owner, PersonId, access,
    folder_access,
};
use crate::domain::definition::{
    Change, DefinitionId, DefinitionKind, DefinitionLabel, DefinitionStoreError, Definitions,
    LibraryEntry, LibraryKind, LibraryPage, Listed, ListedPage, Lookup, Page, StoredDefinition,
};
use crate::domain::folders::{
    Folder, FolderError, FolderFilter, FolderId, FolderList, FolderName, FolderSummary, Folders,
    HeldFolder, MAX_FOLDERS, SeenFolder,
};
use crate::domain::sharing::Grant;
use crate::domain::tags::TagName;

const DASHBOARDS: &str = "dashboards";

type Key = (&'static str, String);

#[derive(Debug, Clone)]
struct KeptFolder {
    name: FolderName,
    owner: Owner,
    updated_at: DateTime<Utc>,
}

/// One dashboard's place in one folder.
#[derive(Debug, Clone, Copy)]
struct KeptEntry {
    placed_by: Option<PersonId>,
    carries: bool,
}

/// The tags one dashboard carries, all of one owner's scope.
#[derive(Debug, Clone)]
struct Tagged {
    scope: Owner,
    names: Vec<TagName>,
}

#[derive(Debug, Default, Clone)]
struct Stored {
    definitions: BTreeMap<Key, StoredDefinition>,
    /// Dashboards with an owner; any other is ownerless.
    owners: BTreeMap<String, PersonId>,
    dashboard_grants: BTreeMap<(String, Grantee), Grant>,
    folders: BTreeMap<FolderId, KeptFolder>,
    folder_grants: BTreeMap<(FolderId, Grantee), Grant>,
    entries: BTreeMap<(FolderId, String), KeptEntry>,
    tagged: BTreeMap<String, Tagged>,
    pinned: BTreeMap<PersonId, Vec<String>>,
}

/// The strongest of these grants reaching `caller`.
fn reaching<'g>(
    grants: impl Iterator<Item = (&'g Grantee, &'g Grant)>,
    caller: Caller,
) -> Option<Level> {
    grants
        .filter(|(grantee, _)| match grantee {
            Grantee::Person(person) => *person == caller.person,
            Grantee::Everyone => true,
        })
        .map(|(_, grant)| grant.level)
        .max()
}

impl Stored {
    fn has_dashboard(&self, id: &str) -> bool {
        self.definitions.contains_key(&(DASHBOARDS, id.to_owned()))
    }

    fn owner_of(&self, dashboard: &str) -> Owner {
        self.owners
            .get(dashboard)
            .map_or(Owner::Unowned, |person| Owner::Person(*person))
    }

    fn folder_standing(&self, id: FolderId, caller: Caller) -> Option<FolderStanding> {
        let kept = self.folders.get(&id)?;
        let grants = self
            .folder_grants
            .iter()
            .filter(|((folder, _), _)| *folder == id)
            .map(|((_, grantee), grant)| (grantee, grant));

        Some(FolderStanding {
            owner: kept.owner,
            granted: reaching(grants, caller),
        })
    }

    fn held_folder(&self, id: FolderId, caller: Caller) -> Option<HeldFolder> {
        let kept = self.folders.get(&id)?;

        Some(HeldFolder {
            folder: Folder {
                id,
                name: kept.name.clone(),
                owner: kept.owner,
            },
            standing: self.folder_standing(id, caller)?,
        })
    }

    fn folder_seen(&self, id: FolderId, caller: Caller) -> bool {
        self.folder_standing(id, caller)
            .and_then(|standing| folder_access(caller, standing))
            .is_some()
    }

    fn standing(&self, dashboard: &str, caller: Caller) -> Option<DashboardStanding> {
        if !self.has_dashboard(dashboard) {
            return None;
        }
        let grants = self
            .dashboard_grants
            .iter()
            .filter(|((held, _), _)| held == dashboard)
            .map(|((_, grantee), grant)| (grantee, grant));
        let carried_by = self
            .entries
            .iter()
            .filter(|((_, held), entry)| held == dashboard && entry.carries)
            .filter_map(|((folder, _), _)| self.folder_standing(*folder, caller))
            .collect();

        Some(DashboardStanding {
            owner: self.owner_of(dashboard),
            granted: reaching(grants, caller),
            carried_by,
        })
    }

    /// The same rule the store's predicate is.
    fn visible(&self, dashboard: &str, caller: Caller) -> bool {
        self.standing(dashboard, caller)
            .and_then(|standing| access(caller, &standing))
            .is_some()
    }

    /// Whether `person` may make a carrying entry of the dashboard by its
    /// owner and grants alone, as the store's demotion reads it.
    fn qualifies(&self, dashboard: &str, person: PersonId) -> bool {
        if self.owners.get(dashboard) == Some(&person) {
            return true;
        }

        self.dashboard_grants
            .iter()
            .filter(|((held, _), grant)| held == dashboard && grant.level == Level::Editor)
            .any(|((_, grantee), _)| match grantee {
                Grantee::Person(granted) => *granted == person,
                Grantee::Everyone => true,
            })
    }

    /// Whether an entry `person` places now carries, as the store's insert
    /// reads it: an ownerless dashboard's always does.
    fn may_carry(&self, dashboard: &str, person: PersonId) -> bool {
        self.owner_of(dashboard) == Owner::Unowned || self.qualifies(dashboard, person)
    }

    /// Stops each carrying entry of an owned dashboard whose placer no
    /// longer qualifies from carrying.
    fn demote(&mut self, dashboard: &str) {
        if self.owner_of(dashboard) == Owner::Unowned {
            return;
        }
        let demoted: Vec<(FolderId, String)> = self
            .entries
            .iter()
            .filter(|((_, held), entry)| held == dashboard && entry.carries)
            .filter(|(_, entry)| {
                entry
                    .placed_by
                    .is_some_and(|placer| !self.qualifies(dashboard, placer))
            })
            .map(|(key, _)| key.clone())
            .collect();

        for key in demoted {
            if let Some(entry) = self.entries.get_mut(&key) {
                entry.carries = false;
            }
        }
    }

    fn write(&mut self, key: Key, name: &str, body: serde_json::Value) {
        let stored = StoredDefinition {
            name: name.to_owned(),
            body,
            updated_at: Utc::now(),
        };
        self.definitions.insert(key, stored);
    }

    fn create(
        &mut self,
        kind: DefinitionKind,
        id: &DefinitionId,
        name: &DefinitionLabel,
        body: &serde_json::Value,
    ) -> Result<(), DefinitionStoreError> {
        let key = MemoryDefinitions::key(kind, id);
        if self.definitions.contains_key(&key) {
            return Err(DefinitionStoreError::IdTaken(id.as_str().to_owned()));
        }
        self.write(key, name.as_str(), body.clone());

        Ok(())
    }

    fn replace(
        &mut self,
        kind: DefinitionKind,
        id: &DefinitionId,
        name: &DefinitionLabel,
        body: &serde_json::Value,
    ) -> bool {
        let key = MemoryDefinitions::key(kind, id);
        if !self.definitions.contains_key(&key) {
            return false;
        }
        self.write(key, name.as_str(), body.clone());

        true
    }

    fn remove(&mut self, kind: DefinitionKind, id: &DefinitionId) -> bool {
        if kind.table() == DASHBOARDS {
            let dashboard = id.as_str();
            self.owners.remove(dashboard);
            self.dashboard_grants
                .retain(|(held, _), _| held != dashboard);
            self.entries.retain(|(_, held), _| held != dashboard);
            self.tagged.remove(dashboard);
            self.drop_pins(dashboard);
        }
        let key = MemoryDefinitions::key(kind, id);
        self.definitions.remove(&key).is_some()
    }

    /// The same rule the store's folder clauses are.
    fn in_folder(&self, dashboard: &str, caller: Caller, filter: FolderFilter) -> bool {
        let mut holding = self
            .entries
            .keys()
            .filter(|(_, held)| held == dashboard)
            .map(|(folder, _)| *folder)
            .filter(|folder| self.folder_seen(*folder, caller));

        match filter {
            FolderFilter::Unfiled => holding.next().is_none(),
            FolderFilter::In(id) => holding.any(|folder| folder == id),
        }
    }

    /// Copies each owned folder entry of `from` the placer may arrange to
    /// `to`, carrying.
    fn carry_entries(&mut self, from: &str, to: &str, placer: Caller) {
        let kept: Vec<FolderId> = self
            .entries
            .keys()
            .filter(|(_, held)| held == from)
            .map(|(folder, _)| *folder)
            .filter(|folder| {
                let Some(standing) = self.folder_standing(*folder, placer) else {
                    return false;
                };
                standing.owner != Owner::Unowned
                    && folder_access(placer, standing).is_some_and(|seen| seen.arrange().is_ok())
            })
            .collect();

        for folder in kept {
            self.entries.insert(
                (folder, to.to_owned()),
                KeptEntry {
                    placed_by: Some(placer.person),
                    carries: true,
                },
            );
        }
    }

    fn listed(&self, kind: DefinitionKind, id: &str) -> Option<Listed> {
        self.definitions
            .get(&(kind.table(), id.to_owned()))
            .map(|stored| Listed {
                id: id.to_owned(),
                name: stored.name.clone(),
            })
    }

    /// Ordered by name, then id, as the store's page is.
    fn matching(&self, kind: DefinitionKind, needle: &str) -> Vec<LibraryEntry> {
        let needle = needle.to_lowercase();
        let mut matched: Vec<LibraryEntry> = self
            .definitions
            .iter()
            .filter(|((table, _), _)| *table == kind.table())
            .filter(|(_, stored)| {
                stored.name.to_lowercase().contains(&needle)
                    || stored.body.to_string().to_lowercase().contains(&needle)
            })
            .map(|((_, id), stored)| LibraryEntry {
                id: id.clone(),
                name: stored.name.clone(),
                body: stored.body.clone(),
                updated_at: stored.updated_at,
            })
            .collect();
        matched.sort_by(|left, right| (&left.name, &left.id).cmp(&(&right.name, &right.id)));

        matched
    }
}

fn paged(matched: Vec<LibraryEntry>, page: Page) -> LibraryPage {
    let total = matched.len() as u64;
    let items = matched
        .into_iter()
        .skip(usize::try_from(page.offset()).unwrap_or(usize::MAX))
        .take(usize::try_from(page.limit()).unwrap_or(usize::MAX))
        .collect();

    LibraryPage { items, total }
}

#[derive(Debug, Default)]
pub(crate) struct MemoryDefinitions {
    stored: Mutex<Stored>,
    /// Set to fail every write, and every look at who may see a dashboard,
    /// for the cases about a store that is down.
    failing: bool,
}

impl MemoryDefinitions {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// A store that refuses every write, for the cases about one that is down.
    pub(crate) fn refusing() -> Self {
        Self {
            failing: true,
            ..Self::default()
        }
    }

    fn key(kind: DefinitionKind, id: &DefinitionId) -> Key {
        (kind.table(), id.as_str().to_owned())
    }

    /// Stores `body` under `id`, labelled by the id itself, whatever it held.
    ///
    /// For a test that needs a definition in place without saying how it got
    /// there.
    #[expect(
        clippy::unused_async,
        clippy::unused_async_trait_impl,
        reason = "awaited as the store's own writes are"
    )]
    pub(crate) async fn seed(
        &self,
        kind: DefinitionKind,
        id: &DefinitionId,
        body: &serde_json::Value,
    ) -> Result<(), DefinitionStoreError> {
        self.writable()?;
        self.lock()
            .write(Self::key(kind, id), id.as_str(), body.clone());

        Ok(())
    }

    /// Stores a dashboard under `id`, labelled by the id, as `owner`'s.
    pub(crate) async fn seed_owned(
        &self,
        id: &DefinitionId,
        body: &serde_json::Value,
        owner: PersonId,
    ) -> Result<(), DefinitionStoreError> {
        self.seed(DefinitionKind::Dashboard, id, body).await?;
        self.lock().owners.insert(id.as_str().to_owned(), owner);

        Ok(())
    }

    /// Grants the dashboard to `viewer` as a viewer, as `owner` would.
    pub(crate) async fn seed_viewer(
        &self,
        dashboard: &DefinitionId,
        viewer: PersonId,
        owner: PersonId,
    ) -> Result<(), DefinitionStoreError> {
        let granting = crate::domain::sharing::Grants::grant(
            self,
            crate::domain::sharing::Shared::Dashboard(dashboard),
            Grantee::Person(viewer),
            Level::Viewer,
            owner,
        )
        .await?;
        assert_eq!(
            granting,
            crate::domain::sharing::Granting::Done,
            "the owner grants it"
        );

        Ok(())
    }

    /// A folder no one owns, as one from before ownership is.
    pub(crate) fn seed_ownerless_folder(&self, name: &str) -> FolderId {
        let id = FolderId::new();
        let name = FolderName::parse(name).unwrap_or_else(|error| panic!("{error}"));
        self.lock().folders.insert(
            id,
            KeptFolder {
                name,
                owner: Owner::Unowned,
                updated_at: Utc::now(),
            },
        );

        id
    }

    /// An entry as the migration copies a legacy folder: carrying, placed by
    /// no one.
    pub(crate) fn seed_entry(&self, folder: FolderId, dashboard: &DefinitionId) {
        self.lock().entries.insert(
            (folder, dashboard.as_str().to_owned()),
            KeptEntry {
                placed_by: None,
                carries: true,
            },
        );
    }

    /// Whether the dashboard's entry in the folder carries, or none without
    /// one.
    pub(crate) fn carries(&self, folder: FolderId, dashboard: &DefinitionId) -> Option<bool> {
        self.lock()
            .entries
            .get(&(folder, dashboard.as_str().to_owned()))
            .map(|entry| entry.carries)
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Stored> {
        self.stored
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn refuse() -> DefinitionStoreError {
        DefinitionStoreError::Database(sea_orm::DbErr::Custom("store is down".to_owned()))
    }

    fn writable(&self) -> Result<(), DefinitionStoreError> {
        if self.failing {
            return Err(Self::refuse());
        }
        Ok(())
    }
}

mod dashboards;
mod pins;
mod tags;
#[cfg(test)]
mod tests;

#[async_trait]
impl Lookup for MemoryDefinitions {
    async fn get(
        &self,
        kind: DefinitionKind,
        id: &DefinitionId,
    ) -> Result<Option<serde_json::Value>, DefinitionStoreError> {
        Ok(self
            .lock()
            .definitions
            .get(&Self::key(kind, id))
            .map(|stored| stored.body.clone()))
    }
}

#[async_trait]
impl Definitions for MemoryDefinitions {
    async fn read(
        &self,
        kind: DefinitionKind,
        id: &DefinitionId,
    ) -> Result<Option<StoredDefinition>, DefinitionStoreError> {
        Ok(self.lock().definitions.get(&Self::key(kind, id)).cloned())
    }

    #[cfg(test)]
    async fn create(
        &self,
        kind: DefinitionKind,
        id: &DefinitionId,
        name: &DefinitionLabel,
        body: &serde_json::Value,
    ) -> Result<(), DefinitionStoreError> {
        self.writable()?;
        self.lock().create(kind, id, name, body)
    }

    async fn replace(
        &self,
        kind: DefinitionKind,
        id: &DefinitionId,
        name: &DefinitionLabel,
        body: &serde_json::Value,
    ) -> Result<bool, DefinitionStoreError> {
        self.writable()?;
        Ok(self.lock().replace(kind, id, name, body))
    }

    async fn list(&self, kind: DefinitionKind) -> Result<Vec<String>, DefinitionStoreError> {
        Ok(self
            .lock()
            .definitions
            .keys()
            .filter(|(table, _)| *table == kind.table())
            .map(|(_, id)| id.clone())
            .collect())
    }

    async fn page(
        &self,
        kind: LibraryKind,
        needle: &str,
        page: Page,
    ) -> Result<ListedPage, DefinitionStoreError> {
        let found = paged(self.lock().matching(kind.kind(), needle), page);
        let items = found
            .items
            .into_iter()
            .map(|entry| Listed {
                id: entry.id,
                name: entry.name,
            })
            .collect();

        Ok(ListedPage {
            items,
            total: found.total,
        })
    }

    async fn page_entries(
        &self,
        kind: LibraryKind,
        needle: &str,
        page: Page,
    ) -> Result<LibraryPage, DefinitionStoreError> {
        Ok(paged(self.lock().matching(kind.kind(), needle), page))
    }

    async fn names_of(
        &self,
        kind: LibraryKind,
        ids: &[&str],
    ) -> Result<HashMap<String, String>, DefinitionStoreError> {
        let stored = self.lock();

        Ok(ids
            .iter()
            .filter_map(|id| {
                stored
                    .definitions
                    .get(&(kind.kind().table(), (*id).to_owned()))
                    .map(|found| ((*id).to_owned(), found.name.clone()))
            })
            .collect())
    }

    async fn delete(
        &self,
        kind: DefinitionKind,
        id: &DefinitionId,
    ) -> Result<bool, DefinitionStoreError> {
        Ok(self.lock().remove(kind, id))
    }

    async fn apply(&self, changes: &[Change]) -> Result<(), DefinitionStoreError> {
        self.writable()?;
        // INVARIANT: all or nothing, as the real transaction is.
        let mut stored = self.lock();
        let mut applied = stored.clone();

        for change in changes {
            match change {
                Change::Create(kind, id, name, body) => applied.create(*kind, id, name, body)?,
                Change::Replace(kind, id, name, body) => {
                    if !applied.replace(*kind, id, name, body) {
                        return Err(DefinitionStoreError::Missing(id.as_str().to_owned()));
                    }
                }
                Change::Own { dashboard, owner } => {
                    applied.owners.insert(dashboard.as_str().to_owned(), *owner);
                }
                Change::CarryEntries { from, to, placer } => {
                    applied.carry_entries(from.as_str(), to.as_str(), *placer);
                }
                Change::CarryTags { from, to, scope } => {
                    applied.carry_tags(from.as_str(), to.as_str(), *scope)?;
                }
            }
        }

        *stored = applied;

        Ok(())
    }
}

#[async_trait]
impl Folders for MemoryDefinitions {
    async fn list_folders(&self, caller: Caller) -> Result<FolderList, FolderError> {
        let stored = self.lock();
        let visible: Vec<String> = stored
            .definitions
            .keys()
            .filter(|(table, id)| *table == DASHBOARDS && stored.visible(id, caller))
            .map(|(_, id)| id.clone())
            .collect();
        let count = |filter: FolderFilter| {
            visible
                .iter()
                .filter(|id| stored.in_folder(id, caller, filter))
                .count() as u64
        };

        let mut folders: Vec<FolderSummary> = stored
            .folders
            .keys()
            .filter_map(|id| stored.held_folder(*id, caller))
            .filter_map(|held| SeenFolder::of(caller, held))
            .map(|seen| FolderSummary {
                dashboards: count(FolderFilter::In(seen.folder.id)),
                seen,
            })
            .collect();
        folders.sort_by_key(|summary| {
            (
                summary.seen.folder.name.as_str().to_lowercase(),
                summary.seen.folder.id,
            )
        });

        Ok(FolderList {
            folders,
            unfiled: count(FolderFilter::Unfiled),
        })
    }

    async fn create_folder(
        &self,
        owner: PersonId,
        name: FolderName,
    ) -> Result<Folder, FolderError> {
        self.writable()?;
        let mut stored = self.lock();
        let held = stored
            .folders
            .values()
            .filter(|folder| folder.owner == Owner::Person(owner))
            .count();
        if held >= MAX_FOLDERS {
            return Err(FolderError::TooMany);
        }

        let id = FolderId::new();
        let owner = Owner::Person(owner);
        stored.folders.insert(
            id,
            KeptFolder {
                name: name.clone(),
                owner,
                updated_at: Utc::now(),
            },
        );

        Ok(Folder { id, name, owner })
    }

    async fn held_folder(
        &self,
        id: FolderId,
        caller: Caller,
    ) -> Result<Option<HeldFolder>, DefinitionStoreError> {
        self.writable()?;

        Ok(self.lock().held_folder(id, caller))
    }

    async fn rename_folder(&self, id: FolderId, name: &FolderName) -> Result<bool, FolderError> {
        self.writable()?;
        let mut stored = self.lock();
        let Some(kept) = stored.folders.get_mut(&id) else {
            return Ok(false);
        };
        kept.name = name.clone();
        kept.updated_at = Utc::now();

        Ok(true)
    }

    async fn delete_folder(&self, id: FolderId) -> Result<bool, FolderError> {
        self.writable()?;
        let mut stored = self.lock();
        stored.entries.retain(|(folder, _), _| *folder != id);
        stored.folder_grants.retain(|(folder, _), _| *folder != id);

        Ok(stored.folders.remove(&id).is_some())
    }

    async fn folders_holding(
        &self,
        dashboard: &DefinitionId,
        caller: Caller,
    ) -> Result<Vec<Folder>, FolderError> {
        let stored = self.lock();
        let mut holding: Vec<Folder> = stored
            .entries
            .keys()
            .filter(|(_, held)| held == dashboard.as_str())
            .filter_map(|(folder, _)| stored.held_folder(*folder, caller))
            .filter_map(|held| SeenFolder::of(caller, held))
            .map(|seen| seen.folder)
            .collect();
        holding.sort_by_key(|folder| (folder.name.as_str().to_lowercase(), folder.id));

        Ok(holding)
    }

    async fn add_entry(
        &self,
        folder: FolderId,
        dashboard: &DefinitionId,
        caller: Caller,
        entry: Entry,
    ) -> Result<Entry, FolderError> {
        self.writable()?;
        let mut stored = self.lock();
        if !stored.visible(dashboard.as_str(), caller) {
            return Err(FolderError::DashboardNotFound(
                dashboard.as_str().to_owned(),
            ));
        }
        if !stored.folders.contains_key(&folder) {
            return Err(FolderError::FolderNotFound(folder));
        }

        let carrying = entry.carries() && stored.may_carry(dashboard.as_str(), caller.person);
        let kept = stored
            .entries
            .entry((folder, dashboard.as_str().to_owned()))
            .or_insert(KeptEntry {
                placed_by: Some(caller.person),
                carries: false,
            });
        if carrying {
            *kept = KeptEntry {
                placed_by: Some(caller.person),
                carries: true,
            };
        }

        Ok(if kept.carries {
            Entry::Carrying
        } else {
            Entry::Filing
        })
    }

    async fn remove_entry(
        &self,
        folder: FolderId,
        dashboard: &DefinitionId,
        caller: Caller,
    ) -> Result<bool, FolderError> {
        self.writable()?;
        let mut stored = self.lock();
        if !stored.visible(dashboard.as_str(), caller) {
            return Ok(false);
        }

        Ok(stored
            .entries
            .remove(&(folder, dashboard.as_str().to_owned()))
            .is_some())
    }
}
