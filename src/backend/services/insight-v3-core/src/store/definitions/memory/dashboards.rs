use async_trait::async_trait;
use chrono::Utc;

use super::{DASHBOARDS, MemoryDefinitions, Stored};
use crate::domain::access::{Owner, PersonId};
use crate::domain::dashboards::{
    DashboardCatalogue, DashboardPage, DashboardQuery, ListedDashboard, Owned, Standing,
};
use crate::domain::definition::{DefinitionId, DefinitionKind, DefinitionStoreError, Page};
use crate::domain::sharing::{Handover, Orphan, OrphanPage, Share, Sharing};

impl Stored {
    fn keeps(&self, dashboard: &str, caller: PersonId, query: &DashboardQuery) -> bool {
        let owned = match query.owned {
            None => true,
            Some(Owned::Me) => self.owner_of(dashboard) == Owner::Person(caller),
            Some(Owned::Shared) => {
                self.owner_of(dashboard) != Owner::Person(caller)
                    && self.shares.contains_key(&(dashboard.to_owned(), caller))
            }
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
}

#[async_trait]
impl DashboardCatalogue for MemoryDefinitions {
    async fn standing(
        &self,
        dashboard: &DefinitionId,
        caller: PersonId,
    ) -> Result<Option<Standing>, DefinitionStoreError> {
        self.writable()?;

        Ok(self.lock().standing(dashboard.as_str(), caller))
    }

    async fn page_dashboards(
        &self,
        caller: PersonId,
        query: &DashboardQuery,
        page: Page,
    ) -> Result<DashboardPage, DefinitionStoreError> {
        let stored = self.lock();
        let matched: Vec<ListedDashboard> = stored
            .matching(DefinitionKind::Dashboard, &query.needle)
            .into_iter()
            .filter(|listed| stored.keeps(&listed.id, caller, query))
            .filter_map(|listed| {
                let seen = stored.standing(&listed.id, caller)?.seen_by(caller)?;

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
impl Sharing for MemoryDefinitions {
    async fn shares_of(
        &self,
        dashboard: &DefinitionId,
    ) -> Result<Vec<Share>, DefinitionStoreError> {
        let mut shares: Vec<Share> = self
            .lock()
            .shares
            .iter()
            .filter(|((held, _), _)| held == dashboard.as_str())
            .map(|(_, share)| share.clone())
            .collect();
        shares.sort_by_key(|share| (share.created_at, share.person));

        Ok(shares)
    }

    async fn share(
        &self,
        dashboard: &DefinitionId,
        person: PersonId,
        granted_by: PersonId,
    ) -> Result<(), DefinitionStoreError> {
        self.writable()?;
        let mut stored = self.lock();
        if !stored.has_dashboard(dashboard) {
            return Err(DefinitionStoreError::Missing(dashboard.as_str().to_owned()));
        }

        stored
            .shares
            .entry((dashboard.as_str().to_owned(), person))
            .or_insert_with(|| Share {
                person,
                granted_by,
                created_at: Utc::now(),
            });

        Ok(())
    }

    async fn withdraw(
        &self,
        dashboard: &DefinitionId,
        person: PersonId,
    ) -> Result<(), DefinitionStoreError> {
        self.writable()?;
        let mut stored = self.lock();
        let key = (dashboard.as_str().to_owned(), person);

        stored.shares.remove(&key);
        stored.placed.remove(&(person, key.0.clone()));
        if let Some(pins) = stored.pinned.get_mut(&person) {
            pins.retain(|held| held != dashboard.as_str());
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
        applied.shares.remove(&(key.clone(), to));
        match applied.carry_tags(&key, &key, Owner::Person(to)) {
            Ok(()) => {}
            Err(DefinitionStoreError::TagsFull) => return Ok(Handover::RecipientTagsFull),
            Err(error) => return Err(error),
        }
        *stored = applied;

        Ok(Handover::Handed)
    }

    async fn page_orphaned(
        &self,
        administrators: &[PersonId],
        page: Page,
    ) -> Result<OrphanPage, DefinitionStoreError> {
        let stored = self.lock();
        let mut orphans: Vec<Orphan> = stored
            .definitions
            .iter()
            .filter(|((table, _), _)| *table == DASHBOARDS)
            .filter_map(|((_, id), held)| {
                let owner = *stored.owners.get(id)?;

                Some(Orphan {
                    id: id.clone(),
                    name: held.name.clone(),
                    owner,
                    updated_at: held.updated_at,
                })
            })
            .filter(|orphan| !administrators.contains(&orphan.owner))
            .collect();
        orphans.sort_by(|left, right| (&left.name, &left.id).cmp(&(&right.name, &right.id)));

        let total = orphans.len() as u64;
        let items = orphans
            .into_iter()
            .skip(usize::try_from(page.offset()).unwrap_or(usize::MAX))
            .take(usize::try_from(page.limit()).unwrap_or(usize::MAX))
            .collect();

        Ok(OrphanPage { items, total })
    }
}
