//! The definition store the tests use.
//!
//! It keeps definitions in a map, so a test that stores one and reads it back
//! asserts on behaviour rather than on a database's wire protocol.

use std::collections::BTreeMap;
use std::sync::Mutex;

use async_trait::async_trait;
use chrono::Utc;
use uuid::Uuid;

use crate::domain::definition::{
    Change, DefinitionId, DefinitionKind, DefinitionLabel, DefinitionStoreError, Definitions,
    Listed, ListedPage, Lookup, Page, StoredDefinition,
};
use crate::domain::folders::{
    Folder, FolderError, FolderFilter, FolderId, FolderList, FolderName, FolderSummary, Folders,
    MAX_FOLDERS,
};
use crate::domain::tags::TagName;

const DASHBOARDS: &str = "dashboards";

type Key = (&'static str, String);

#[derive(Debug, Default, Clone)]
struct Stored {
    definitions: BTreeMap<Key, StoredDefinition>,
    folders: BTreeMap<FolderId, FolderName>,
    filed: BTreeMap<String, FolderId>,
    tagged: BTreeMap<String, Vec<TagName>>,
    pinned: BTreeMap<Uuid, Vec<String>>,
}

impl Stored {
    fn has_dashboard(&self, id: &DefinitionId) -> bool {
        self.definitions
            .contains_key(&(DASHBOARDS, id.as_str().to_owned()))
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
            self.filed.remove(id.as_str());
            self.tagged.remove(id.as_str());
            self.drop_pins(id.as_str());
        }
        let key = MemoryDefinitions::key(kind, id);
        self.definitions.remove(&key).is_some()
    }

    fn name_taken(&self, name: &FolderName, except: Option<FolderId>) -> bool {
        let wanted = name.as_str().to_lowercase();

        self.folders
            .iter()
            .any(|(id, held)| Some(*id) != except && held.as_str().to_lowercase() == wanted)
    }

    fn in_folder(&self, dashboard: &str, filter: FolderFilter) -> bool {
        match filter {
            FolderFilter::Unfiled => !self.filed.contains_key(dashboard),
            FolderFilter::In(id) => self.filed.get(dashboard) == Some(&id),
        }
    }

    fn folder(&self, id: FolderId) -> Option<Folder> {
        self.folders.get(&id).map(|name| Folder {
            id,
            name: name.clone(),
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
    /// Set to fail every write, for the cases about a store that is down.
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
        kind: DefinitionKind,
        needle: &str,
        page: Page,
    ) -> Result<ListedPage, DefinitionStoreError> {
        Ok(paged(self.lock().matching(kind, needle), page))
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
                Change::CarryFolder { from, to } => {
                    if let Some(folder) = applied.filed.get(from.as_str()).copied() {
                        applied.filed.insert(to.as_str().to_owned(), folder);
                    }
                }
                Change::CarryTags { from, to } => {
                    if let Some(tags) = applied.tagged.get(from.as_str()).cloned() {
                        applied.tagged.insert(to.as_str().to_owned(), tags);
                    }
                }
            }
        }

        *stored = applied;

        Ok(())
    }
}

#[async_trait]
impl Folders for MemoryDefinitions {
    async fn list_folders(&self) -> Result<FolderList, FolderError> {
        let stored = self.lock();
        let mut folders: Vec<FolderSummary> = stored
            .folders
            .iter()
            .map(|(id, name)| FolderSummary {
                folder: Folder {
                    id: *id,
                    name: name.clone(),
                },
                dashboards: stored.filed.values().filter(|held| *held == id).count() as u64,
            })
            .collect();
        folders.sort_by_key(|summary| summary.folder.name.as_str().to_lowercase());

        let dashboards = stored
            .definitions
            .keys()
            .filter(|(table, _)| *table == DASHBOARDS)
            .count() as u64;

        Ok(FolderList {
            folders,
            unfiled: dashboards - stored.filed.len() as u64,
        })
    }

    async fn create_folder(&self, name: FolderName) -> Result<Folder, FolderError> {
        self.writable()?;
        let mut stored = self.lock();
        if stored.name_taken(&name, None) {
            return Err(FolderError::NameTaken(name.as_str().to_owned()));
        }
        if stored.folders.len() >= MAX_FOLDERS {
            return Err(FolderError::TooMany);
        }
        let id = FolderId::new();
        stored.folders.insert(id, name.clone());

        Ok(Folder { id, name })
    }

    async fn rename_folder(&self, id: FolderId, name: FolderName) -> Result<Folder, FolderError> {
        self.writable()?;
        let mut stored = self.lock();
        if !stored.folders.contains_key(&id) {
            return Err(FolderError::FolderNotFound(id));
        }
        if stored.name_taken(&name, Some(id)) {
            return Err(FolderError::NameTaken(name.as_str().to_owned()));
        }
        stored.folders.insert(id, name.clone());

        Ok(Folder { id, name })
    }

    async fn delete_folder(&self, id: FolderId) -> Result<bool, FolderError> {
        self.writable()?;
        let mut stored = self.lock();
        stored.filed.retain(|_, held| *held != id);

        Ok(stored.folders.remove(&id).is_some())
    }

    async fn folder_of(&self, dashboard: &DefinitionId) -> Result<Option<Folder>, FolderError> {
        let stored = self.lock();

        Ok(stored
            .filed
            .get(dashboard.as_str())
            .and_then(|id| stored.folder(*id)))
    }

    async fn file(
        &self,
        dashboard: &DefinitionId,
        folder: Option<FolderId>,
    ) -> Result<(), FolderError> {
        self.writable()?;
        let mut stored = self.lock();
        if !stored.has_dashboard(dashboard) {
            return Err(FolderError::DashboardNotFound(
                dashboard.as_str().to_owned(),
            ));
        }
        match folder {
            Some(id) if !stored.folders.contains_key(&id) => Err(FolderError::FolderNotFound(id)),
            Some(id) => {
                stored.filed.insert(dashboard.as_str().to_owned(), id);
                Ok(())
            }
            None => {
                stored.filed.remove(dashboard.as_str());
                Ok(())
            }
        }
    }

    async fn page_filed(
        &self,
        needle: &str,
        page: Page,
        filter: FolderFilter,
    ) -> Result<ListedPage, FolderError> {
        let stored = self.lock();
        let matched = stored
            .matching(DefinitionKind::Dashboard, needle)
            .into_iter()
            .filter(|listed| stored.in_folder(&listed.id, filter))
            .collect();

        Ok(paged(matched, page))
    }
}
