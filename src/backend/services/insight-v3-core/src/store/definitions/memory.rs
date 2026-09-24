//! The definition store the tests use.
//!
//! It keeps definitions in a map, so a test that stores one and reads it back
//! asserts on behaviour rather than on a database's wire protocol.

use std::collections::BTreeMap;
use std::sync::Mutex;

use async_trait::async_trait;

use crate::domain::definition::{
    Change, DefinitionKind, DefinitionName, DefinitionStoreError, Definitions, Lookup, NamePage,
    Page,
};
use crate::domain::folders::{
    Folder, FolderError, FolderFilter, FolderId, FolderList, FolderName, FolderSummary, Folders,
};

const DASHBOARDS: &str = "dashboards";

#[derive(Debug, Default, Clone)]
struct Stored {
    definitions: BTreeMap<(&'static str, String), serde_json::Value>,
    folders: BTreeMap<FolderId, FolderName>,
    filed: BTreeMap<String, FolderId>,
}

impl Stored {
    fn has_dashboard(&self, name: &DefinitionName) -> bool {
        self.definitions
            .contains_key(&(DASHBOARDS, name.as_str().to_owned()))
    }

    fn remove(&mut self, kind: DefinitionKind, name: &DefinitionName) -> bool {
        if kind.table() == DASHBOARDS {
            self.filed.remove(name.as_str());
        }
        self.definitions
            .remove(&MemoryDefinitions::key(kind, name))
            .is_some()
    }

    fn name_taken(&self, name: &FolderName, except: Option<FolderId>) -> bool {
        self.folders
            .iter()
            .any(|(id, held)| Some(*id) != except && held.same_as(name))
    }

    fn folder(&self, id: FolderId) -> Option<Folder> {
        self.folders.get(&id).map(|name| Folder {
            id,
            name: name.clone(),
        })
    }

    fn matching(&self, kind: DefinitionKind, needle: &str) -> Vec<String> {
        let needle = needle.to_lowercase();
        self.definitions
            .iter()
            .filter(|((table, _), _)| *table == kind.table())
            .filter(|((_, name), body)| {
                name.to_lowercase().contains(&needle)
                    || body.to_string().to_lowercase().contains(&needle)
            })
            .map(|((_, name), _)| name.clone())
            .collect()
    }
}

fn paged(matched: Vec<String>, page: Page) -> NamePage {
    let total = matched.len() as u64;
    let names = matched
        .into_iter()
        .skip(usize::try_from(page.offset()).unwrap_or(usize::MAX))
        .take(usize::try_from(page.limit()).unwrap_or(usize::MAX))
        .collect();

    NamePage { names, total }
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

    fn key(kind: DefinitionKind, name: &DefinitionName) -> (&'static str, String) {
        (kind.table(), name.as_str().to_owned())
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Stored> {
        self.stored
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn refuse() -> DefinitionStoreError {
        DefinitionStoreError::Database(sea_orm::DbErr::Custom("store is down".to_owned()))
    }

    fn writable(&self) -> Result<(), FolderError> {
        if self.failing {
            return Err(FolderError::Store(Self::refuse()));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;

#[async_trait]
impl Lookup for MemoryDefinitions {
    async fn get(
        &self,
        kind: DefinitionKind,
        name: &DefinitionName,
    ) -> Result<Option<serde_json::Value>, DefinitionStoreError> {
        Ok(self.lock().definitions.get(&Self::key(kind, name)).cloned())
    }
}

#[async_trait]
impl Definitions for MemoryDefinitions {
    async fn put(
        &self,
        kind: DefinitionKind,
        name: &DefinitionName,
        body: &serde_json::Value,
    ) -> Result<(), DefinitionStoreError> {
        if self.failing {
            return Err(Self::refuse());
        }
        self.lock()
            .definitions
            .insert(Self::key(kind, name), body.clone());

        Ok(())
    }

    async fn list(&self, kind: DefinitionKind) -> Result<Vec<String>, DefinitionStoreError> {
        Ok(self
            .lock()
            .definitions
            .keys()
            .filter(|(table, _)| *table == kind.table())
            .map(|(_, name)| name.clone())
            .collect())
    }

    async fn page(
        &self,
        kind: DefinitionKind,
        needle: &str,
        page: Page,
    ) -> Result<NamePage, DefinitionStoreError> {
        Ok(paged(self.lock().matching(kind, needle), page))
    }

    async fn delete(
        &self,
        kind: DefinitionKind,
        name: &DefinitionName,
    ) -> Result<bool, DefinitionStoreError> {
        Ok(self.lock().remove(kind, name))
    }

    async fn apply(&self, changes: &[Change]) -> Result<(), DefinitionStoreError> {
        if self.failing {
            return Err(Self::refuse());
        }
        // INVARIANT: all or nothing, as the real transaction is.
        let mut stored = self.lock();
        let mut applied = stored.clone();

        for change in changes {
            match change {
                Change::Put(kind, name, body) => {
                    applied
                        .definitions
                        .insert(Self::key(*kind, name), body.clone());
                }
                Change::Create(kind, name, body) => {
                    let key = Self::key(*kind, name);
                    if applied.definitions.contains_key(&key) {
                        return Err(DefinitionStoreError::NameTaken(name.as_str().to_owned()));
                    }
                    applied.definitions.insert(key, body.clone());
                }
                Change::Delete(kind, name) => {
                    applied.remove(*kind, name);
                }
                Change::CarryFolder { from, to } => {
                    if let Some(id) = applied.filed.get(from.as_str()).copied() {
                        applied.filed.insert(to.as_str().to_owned(), id);
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
        let id = FolderId::new();
        stored.folders.insert(id, name.clone());

        Ok(Folder { id, name })
    }

    async fn rename_folder(&self, id: FolderId, name: FolderName) -> Result<Folder, FolderError> {
        self.writable()?;
        let mut stored = self.lock();
        if !stored.folders.contains_key(&id) {
            return Err(FolderError::FolderNotFound);
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

    async fn folder_of(&self, dashboard: &DefinitionName) -> Result<Option<Folder>, FolderError> {
        let stored = self.lock();

        Ok(stored
            .filed
            .get(dashboard.as_str())
            .and_then(|id| stored.folder(*id)))
    }

    async fn file(
        &self,
        dashboard: &DefinitionName,
        folder: Option<FolderId>,
    ) -> Result<(), FolderError> {
        self.writable()?;
        let mut stored = self.lock();
        if !stored.has_dashboard(dashboard) {
            return Err(FolderError::DashboardNotFound);
        }
        match folder {
            Some(id) if !stored.folders.contains_key(&id) => Err(FolderError::FolderNotFound),
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
    ) -> Result<NamePage, FolderError> {
        let stored = self.lock();
        let matched = stored
            .matching(DefinitionKind::Dashboard, needle)
            .into_iter()
            .filter(|name| match filter {
                FolderFilter::Unfiled => !stored.filed.contains_key(name),
                FolderFilter::In(id) => stored.filed.get(name) == Some(&id),
            })
            .collect();

        Ok(paged(matched, page))
    }
}
