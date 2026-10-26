use async_trait::async_trait;
use chrono::Utc;

use super::{DASHBOARDS, MemoryDefinitions, Stored};
use crate::domain::access::{Caller, DashboardStanding, Grantee, Level, Owner, PersonId};
use crate::domain::dashboards::{
    DashboardCatalogue, DashboardPage, DashboardQuery, ListedDashboard, Owned, Seen,
};
use crate::domain::definition::{DefinitionId, DefinitionKind, DefinitionStoreError, Page};
use crate::domain::folders::FolderId;
use crate::domain::sharing::{Grant, Grants, Handover, Orphan, OrphanPage, Shared};

impl Stored {
    fn keeps(&self, dashboard: &str, caller: Caller, query: &DashboardQuery) -> bool {
        let owns =
            self.owner_of(dashboard) == Owner::Person(caller.person) && caller.is_administrator();
        let owned = match query.owned {
            None => true,
            Some(Owned::Me) => owns,
            Some(Owned::Shared) => self.owner_of(dashboard) != Owner::Unowned && !owns,
        };
        let filed = query
            .folder
            .is_none_or(|filter| self.in_folder(dashboard, caller, filter));
        let tagged = query
            .tags
            .as_ref()
            .is_none_or(|filter| self.carries_any(dashboard, filter));

        owned && filed && tagged
    }

    fn grants(&self, shared: Shared<'_>) -> Vec<Grant> {
        let mut grants: Vec<Grant> = match shared {
            Shared::Dashboard(id) => self
                .dashboard_grants
                .iter()
                .filter(|((held, _), _)| held == id.as_str())
                .map(|(_, grant)| grant.clone())
                .collect(),
            Shared::Folder(id) => self
                .folder_grants
                .iter()
                .filter(|((held, _), _)| *held == id)
                .map(|(_, grant)| grant.clone())
                .collect(),
        };
        grants.sort_by_key(|grant| (grant.created_at, grant.grantee));

        grants
    }
}

fn orphans(
    owned: impl Iterator<Item = Orphan>,
    administrators: &[PersonId],
    page: Page,
) -> OrphanPage {
    let mut orphans: Vec<Orphan> = owned
        .filter(|orphan| !administrators.contains(&orphan.owner))
        .collect();
    orphans.sort_by(|left, right| (&left.name, &left.id).cmp(&(&right.name, &right.id)));

    let total = orphans.len() as u64;
    let items = orphans
        .into_iter()
        .skip(usize::try_from(page.offset()).unwrap_or(usize::MAX))
        .take(usize::try_from(page.limit()).unwrap_or(usize::MAX))
        .collect();

    OrphanPage { items, total }
}

#[async_trait]
impl DashboardCatalogue for MemoryDefinitions {
    async fn standing(
        &self,
        dashboard: &DefinitionId,
        caller: Caller,
    ) -> Result<Option<DashboardStanding>, DefinitionStoreError> {
        self.writable()?;

        Ok(self.lock().standing(dashboard.as_str(), caller))
    }

    async fn page_dashboards(
        &self,
        caller: Caller,
        query: &DashboardQuery,
        page: Page,
    ) -> Result<DashboardPage, DefinitionStoreError> {
        let stored = self.lock();
        let matched: Vec<ListedDashboard> = stored
            .matching(DefinitionKind::Dashboard, &query.needle)
            .into_iter()
            .filter(|listed| stored.keeps(&listed.id, caller, query))
            .filter_map(|listed| {
                let seen = Seen::of(caller, &stored.standing(&listed.id, caller)?)?;

                Some(ListedDashboard {
                    id: listed.id,
                    name: listed.name,
                    owner: seen.owner,
                    access: seen.access,
                })
            })
            .collect();

        let total = matched.len() as u64;
        let items = matched
            .into_iter()
            .skip(usize::try_from(page.offset()).unwrap_or(usize::MAX))
            .take(usize::try_from(page.limit()).unwrap_or(usize::MAX))
            .collect();

        Ok(DashboardPage { items, total })
    }
}

#[async_trait]
impl Grants for MemoryDefinitions {
    async fn grants_of(&self, shared: Shared<'_>) -> Result<Vec<Grant>, DefinitionStoreError> {
        Ok(self.lock().grants(shared))
    }

    async fn grant(
        &self,
        shared: Shared<'_>,
        grantee: Grantee,
        level: Level,
        granted_by: PersonId,
    ) -> Result<(), DefinitionStoreError> {
        self.writable()?;
        let mut stored = self.lock();
        let fresh = Grant {
            grantee,
            level,
            granted_by,
            created_at: Utc::now(),
        };

        match shared {
            Shared::Dashboard(id) => {
                if !stored.has_dashboard(id.as_str()) {
                    return Err(DefinitionStoreError::Missing(id.as_str().to_owned()));
                }
                stored
                    .dashboard_grants
                    .entry((id.as_str().to_owned(), grantee))
                    .and_modify(|held| held.level = level)
                    .or_insert(fresh);
                stored.demote(id.as_str());
            }
            Shared::Folder(id) => {
                if !stored.folders.contains_key(&id) {
                    return Err(DefinitionStoreError::Missing(id.to_string()));
                }
                stored
                    .folder_grants
                    .entry((id, grantee))
                    .and_modify(|held| held.level = level)
                    .or_insert(fresh);
            }
        }

        Ok(())
    }

    async fn withdraw(
        &self,
        shared: Shared<'_>,
        grantee: Grantee,
    ) -> Result<(), DefinitionStoreError> {
        self.writable()?;
        let mut stored = self.lock();

        match shared {
            Shared::Dashboard(id) => {
                stored
                    .dashboard_grants
                    .remove(&(id.as_str().to_owned(), grantee));
                stored.demote(id.as_str());
            }
            Shared::Folder(id) => {
                stored.folder_grants.remove(&(id, grantee));
            }
        }

        Ok(())
    }

    async fn transfer(
        &self,
        dashboard: &DefinitionId,
        from: PersonId,
        to: PersonId,
    ) -> Result<Handover, DefinitionStoreError> {
        self.writable()?;
        let mut stored = self.lock();
        if stored.owners.get(dashboard.as_str()) != Some(&from) {
            return Ok(Handover::OwnerChanged);
        }

        // INVARIANT: all or nothing, as the real transaction is.
        let mut applied = stored.clone();
        let key = dashboard.as_str().to_owned();
        applied.owners.insert(key.clone(), to);
        applied
            .dashboard_grants
            .remove(&(key.clone(), Grantee::Person(to)));
        applied.demote(&key);
        match applied.carry_tags(&key, &key, Owner::Person(to)) {
            Ok(()) => {}
            Err(DefinitionStoreError::TagsFull) => return Ok(Handover::RecipientTagsFull),
            Err(error) => return Err(error),
        }
        *stored = applied;

        Ok(Handover::Handed)
    }

    async fn transfer_folder(
        &self,
        folder: FolderId,
        from: PersonId,
        to: PersonId,
    ) -> Result<Handover, DefinitionStoreError> {
        self.writable()?;
        let mut stored = self.lock();
        let Some(kept) = stored.folders.get_mut(&folder) else {
            return Ok(Handover::OwnerChanged);
        };
        if kept.owner != Owner::Person(from) {
            return Ok(Handover::OwnerChanged);
        }

        kept.owner = Owner::Person(to);
        stored.folder_grants.remove(&(folder, Grantee::Person(to)));

        Ok(Handover::Handed)
    }

    async fn page_orphaned(
        &self,
        administrators: &[PersonId],
        page: Page,
    ) -> Result<OrphanPage, DefinitionStoreError> {
        let stored = self.lock();
        let owned = stored
            .definitions
            .iter()
            .filter(|((table, _), _)| *table == DASHBOARDS)
            .filter_map(|((_, id), held)| {
                Some(Orphan {
                    id: id.clone(),
                    name: held.name.clone(),
                    owner: *stored.owners.get(id)?,
                    updated_at: held.updated_at,
                })
            });

        Ok(orphans(owned, administrators, page))
    }

    async fn page_orphaned_folders(
        &self,
        administrators: &[PersonId],
        page: Page,
    ) -> Result<OrphanPage, DefinitionStoreError> {
        let stored = self.lock();
        let owned = stored
            .folders
            .iter()
            .filter(|(id, _)| !stored.grants(Shared::Folder(**id)).is_empty())
            .filter_map(|(id, kept)| {
                Some(Orphan {
                    id: id.to_string(),
                    name: kept.name.as_str().to_owned(),
                    owner: kept.owner.person()?,
                    updated_at: kept.updated_at,
                })
            });

        Ok(orphans(owned, administrators, page))
    }
}
