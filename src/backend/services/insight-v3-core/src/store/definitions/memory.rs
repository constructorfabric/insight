//! The definition store the tests use.
//!
//! It keeps definitions in a map, so a test that stores one and reads it back
//! asserts on behaviour rather than on a database's wire protocol.

use std::collections::BTreeMap;
use std::sync::Mutex;

use async_trait::async_trait;
use chrono::Utc;

use crate::domain::access::{Owner, PersonId};
use crate::domain::dashboards::Standing;
use crate::domain::definition::{
    Change, DefinitionId, DefinitionKind, DefinitionLabel, DefinitionStoreError, Definitions,
    LibraryKind, Listed, ListedPage, Lookup, Page, StoredDefinition,
};
use crate::domain::folders::{
    Folder, FolderError, FolderFilter, FolderId, FolderList, FolderName, FolderSummary, Folders,
    MAX_FOLDERS,
};
use crate::domain::sharing::Share;
use crate::domain::tags::TagName;

const DASHBOARDS: &str = "dashboards";

type Key = (&'static str, String);

#[derive(Debug, Clone)]
struct HeldFolder {
    name: FolderName,
    owner: Owner,
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
    shares: BTreeMap<(String, PersonId), Share>,
    folders: BTreeMap<FolderId, HeldFolder>,
    /// Where an ownerless dashboard is filed.
    filed: BTreeMap<String, FolderId>,
    /// Where each person filed an owned dashboard.
    placed: BTreeMap<(PersonId, String), FolderId>,
    tagged: BTreeMap<String, Tagged>,
    pinned: BTreeMap<PersonId, Vec<String>>,
}

impl Stored {
    fn has_dashboard(&self, id: &DefinitionId) -> bool {
        self.definitions
            .contains_key(&(DASHBOARDS, id.as_str().to_owned()))
    }

    fn owner_of(&self, dashboard: &str) -> Owner {
        self.owners
            .get(dashboard)
            .map_or(Owner::Unowned, |person| Owner::Person(*person))
    }

    fn standing(&self, dashboard: &str, caller: PersonId) -> Option<Standing> {
        if !self
            .definitions
            .contains_key(&(DASHBOARDS, dashboard.to_owned()))
        {
            return None;
        }

        Some(Standing {
            owner: self.owner_of(dashboard),
            shared_with_caller: self.shares.contains_key(&(dashboard.to_owned(), caller)),
        })
    }

    /// The same rule the store's predicate is.
    fn visible(&self, dashboard: &str, caller: PersonId) -> bool {
        self.standing(dashboard, caller)
            .and_then(|standing| standing.seen_by(caller))
            .is_some()
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
            self.shares.retain(|(held, _), _| held != dashboard);
            self.filed.remove(dashboard);
            self.placed.retain(|(_, held), _| held != dashboard);
            self.tagged.remove(dashboard);
            self.drop_pins(dashboard);
        }
        let key = MemoryDefinitions::key(kind, id);
        self.definitions.remove(&key).is_some()
    }

    /// The same rule the store's folder clauses are.
    fn in_folder(&self, dashboard: &str, caller: PersonId, filter: FolderFilter) -> bool {
        let held = match self.owner_of(dashboard) {
            Owner::Unowned => self.filed.get(dashboard),
            Owner::Person(_) => self.placed.get(&(caller, dashboard.to_owned())),
        };

        match filter {
            FolderFilter::Unfiled => held.is_none(),
            FolderFilter::In(id) => held == Some(&id),
        }
    }

    fn folder(&self, id: FolderId) -> Option<Folder> {
        self.folders.get(&id).map(|held| Folder {
            id,
            name: held.name.clone(),
            owner: held.owner,
        })
    }

    fn reachable_folder(&self, id: FolderId, caller: PersonId) -> Option<Folder> {
        self.folder(id).filter(|folder| match folder.owner {
            Owner::Unowned => true,
            Owner::Person(person) => person == caller,
        })
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
    fn matching(&self, kind: DefinitionKind, needle: &str) -> Vec<Listed> {
        let needle = needle.to_lowercase();
        let mut matched: Vec<Listed> = self
            .definitions
            .iter()
            .filter(|((table, _), _)| *table == kind.table())
            .filter(|(_, stored)| {
                stored.name.to_lowercase().contains(&needle)
                    || stored.body.to_string().to_lowercase().contains(&needle)
            })
            .map(|((_, id), stored)| Listed {
                id: id.clone(),
                name: stored.name.clone(),
            })
            .collect();
        matched.sort_by(|left, right| (&left.name, &left.id).cmp(&(&right.name, &right.id)));

        matched
    }
}

fn paged(matched: Vec<Listed>, page: Page) -> ListedPage {
    let total = matched.len() as u64;
    let items = matched
        .into_iter()
        .skip(usize::try_from(page.offset()).unwrap_or(usize::MAX))
        .take(usize::try_from(page.limit()).unwrap_or(usize::MAX))
        .collect();

    ListedPage { items, total }
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
        Ok(paged(self.lock().matching(kind.kind(), needle), page))
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
                Change::CarryPlacement { from, to, person } => {
                    let held = applied.placed.get(&(*person, from.as_str().to_owned()));
                    if let Some(folder) = held.copied() {
                        applied
                            .placed
                            .insert((*person, to.as_str().to_owned()), folder);
                    }
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
    async fn list_folders(&self, caller: PersonId) -> Result<FolderList, FolderError> {
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
            .filter_map(|id| stored.reachable_folder(*id, caller))
            .map(|folder| FolderSummary {
                dashboards: count(FolderFilter::In(folder.id)),
                folder,
            })
            .collect();
        folders.sort_by_key(|summary| {
            (
                summary.folder.name.as_str().to_lowercase(),
                summary.folder.id,
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
            HeldFolder {
                name: name.clone(),
                owner,
            },
        );

        Ok(Folder { id, name, owner })
    }

    async fn rename_folder(
        &self,
        caller: PersonId,
        id: FolderId,
        name: FolderName,
    ) -> Result<Folder, FolderError> {
        self.writable()?;
        let mut stored = self.lock();
        let Some(folder) = stored.reachable_folder(id, caller) else {
            return Err(FolderError::FolderNotFound(id));
        };
        stored.folders.insert(
            id,
            HeldFolder {
                name: name.clone(),
                owner: folder.owner,
            },
        );

        Ok(Folder {
            id,
            name,
            owner: folder.owner,
        })
    }

    async fn delete_folder(&self, caller: PersonId, id: FolderId) -> Result<bool, FolderError> {
        self.writable()?;
        let mut stored = self.lock();
        if stored.reachable_folder(id, caller).is_none() {
            return Ok(false);
        }
        stored.filed.retain(|_, held| *held != id);
        stored.placed.retain(|_, held| *held != id);

        Ok(stored.folders.remove(&id).is_some())
    }

    async fn folder_owner(&self, id: FolderId) -> Result<Option<Owner>, FolderError> {
        Ok(self.lock().folders.get(&id).map(|held| held.owner))
    }

    async fn folder_of(
        &self,
        dashboard: &DefinitionId,
        caller: PersonId,
    ) -> Result<Option<Folder>, FolderError> {
        let stored = self.lock();
        let held = match stored.owner_of(dashboard.as_str()) {
            Owner::Unowned => stored.filed.get(dashboard.as_str()),
            Owner::Person(_) => stored.placed.get(&(caller, dashboard.as_str().to_owned())),
        };

        Ok(held.and_then(|id| stored.folder(*id)))
    }

    async fn file(
        &self,
        dashboard: &DefinitionId,
        folders_of: Owner,
        folder: Option<FolderId>,
    ) -> Result<(), FolderError> {
        self.writable()?;
        let mut stored = self.lock();
        if !stored.has_dashboard(dashboard) {
            return Err(FolderError::DashboardNotFound(
                dashboard.as_str().to_owned(),
            ));
        }
        if let Some(id) = folder
            && !stored.folders.contains_key(&id)
        {
            return Err(FolderError::FolderNotFound(id));
        }

        let key = dashboard.as_str().to_owned();
        match (folders_of, folder) {
            (Owner::Unowned, Some(id)) => {
                stored.filed.insert(key, id);
            }
            (Owner::Unowned, None) => {
                stored.filed.remove(&key);
            }
            (Owner::Person(person), Some(id)) => {
                stored.placed.insert((person, key), id);
            }
            (Owner::Person(person), None) => {
                stored.placed.remove(&(person, key));
            }
        }

        Ok(())
    }
}
