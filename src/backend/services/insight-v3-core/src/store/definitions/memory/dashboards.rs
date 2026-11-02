use async_trait::async_trait;
use chrono::Utc;

use super::{DASHBOARDS, MemoryDefinitions, Stored};
use crate::domain::access::{Caller, DashboardStanding, Grantee, Level, Owner, PersonId};
use crate::domain::dashboards::{
    DashboardCatalogue, DashboardPage, DashboardQuery, ListedDashboard, Order, Owned, Seen,
    Visibility,
};
use crate::domain::definition::{DefinitionId, DefinitionKind, DefinitionStoreError, Page};
use crate::domain::folders::{FolderId, MAX_FOLDERS};
use crate::domain::sharing::{
    Grant, Granting, Grants, Handover, Orphan, OrphanPage, Shared, Withdrawal,
};
use crate::domain::tags::TagName;

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

    fn visibility(&self, dashboard: &str) -> Visibility {
        let reach = |grantee: &Grantee| match grantee {
            Grantee::Person(_) => Visibility::People,
            Grantee::Everyone => Visibility::Everyone,
        };
        let dashboard_owner = self.owners.get(dashboard);
        let own = self
            .dashboard_grants
            .keys()
            .filter(|(held, _)| held == dashboard)
            .map(|(_, grantee)| reach(grantee));
        // INVARIANT: matches `folder_carrying` - a carrying folder shows the dashboard to its owner and its grantees.
        let carried = self
            .entries
            .iter()
            .filter(|((_, held), entry)| held == dashboard && entry.carries)
            .filter_map(|((folder, _), _)| {
                let owner = self.folders.get(folder)?.owner.person()?;
                Some((folder, owner))
            })
            .flat_map(|(folder, owner)| {
                let by_owner = (dashboard_owner != Some(&owner)).then_some(Visibility::People);
                let by_grants = self
                    .folder_grants
                    .keys()
                    .filter(move |(granting, _)| granting == folder)
                    .map(|(_, grantee)| reach(grantee));
                by_owner.into_iter().chain(by_grants)
            });

        own.chain(carried).max().unwrap_or(Visibility::Private)
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

    /// Who owns it now, and whether `by` may change its grants, as the
    /// store's locked read answers it; none when it is not stored.
    fn sharing_standing(&self, shared: Shared<'_>, by: PersonId) -> Option<(Owner, bool)> {
        let owner = match shared {
            Shared::Dashboard(id) => {
                if !self.has_dashboard(id.as_str()) {
                    return None;
                }
                self.owner_of(id.as_str())
            }
            Shared::Folder(id) => self.folders.get(&id)?.owner,
        };
        let editor = self.grants(shared).iter().any(|grant| {
            let reaches = match grant.grantee {
                Grantee::Person(person) => person == by,
                Grantee::Everyone => true,
            };
            grant.level == Level::Editor && reaches
        });

        Some((owner, owner == Owner::Person(by) || editor))
    }
}

fn missing(shared: Shared<'_>) -> DefinitionStoreError {
    match shared {
        Shared::Dashboard(id) => DefinitionStoreError::Missing(id.as_str().to_owned()),
        Shared::Folder(id) => DefinitionStoreError::Missing(id.to_string()),
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
        let mut matched: Vec<ListedDashboard> = stored
            .matching(DefinitionKind::Dashboard, &query.needle)
            .into_iter()
            .filter(|listed| stored.keeps(&listed.id, caller, query))
            .filter_map(|listed| {
                let seen = Seen::of(caller, &stored.standing(&listed.id, caller)?)?;
                let definition = stored.definitions.get(&(DASHBOARDS, listed.id.clone()))?;
                let updated_at = definition.updated_at;
                let title = definition
                    .body
                    .get("title")
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_owned);
                let visibility = stored.visibility(&listed.id);
                let mut carried = stored
                    .tagged
                    .get(&listed.id)
                    .map(|tagged| tagged.names.clone())
                    .unwrap_or_default();
                carried.sort_by_key(TagName::folded);
                let tags = carried
                    .iter()
                    .map(|name| name.as_str().to_owned())
                    .collect();

                Some(ListedDashboard {
                    id: listed.id,
                    name: listed.name,
                    owner: seen.owner,
                    access: seen.access,
                    updated_at,
                    visibility,
                    title,
                    tags,
                })
            })
            .collect();
        if query.order == Order::Recent {
            matched.sort_by(|left, right| {
                right
                    .updated_at
                    .cmp(&left.updated_at)
                    .then_with(|| left.id.cmp(&right.id))
            });
        }

        let total = matched.len() as u64;
        let items = matched
            .into_iter()
            .skip(usize::try_from(page.offset()).unwrap_or(usize::MAX))
            .take(usize::try_from(page.limit()).unwrap_or(usize::MAX))
            .collect();

        Ok(DashboardPage { items, total })
    }

    async fn visibility(
        &self,
        dashboard: &DefinitionId,
    ) -> Result<Option<Visibility>, DefinitionStoreError> {
        let stored = self.lock();
        let known = stored
            .definitions
            .contains_key(&(DASHBOARDS, dashboard.as_str().to_owned()));

        Ok(known.then(|| stored.visibility(dashboard.as_str())))
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
        by: PersonId,
    ) -> Result<Granting, DefinitionStoreError> {
        self.writable()?;
        let mut stored = self.lock();
        let (owner, may_share) = stored
            .sharing_standing(shared, by)
            .ok_or_else(|| missing(shared))?;
        if !may_share {
            return Ok(Granting::NotSharer);
        }
        if let Grantee::Person(person) = grantee
            && owner == Owner::Person(person)
        {
            return Ok(Granting::ToOwner);
        }
        let fresh = Grant {
            grantee,
            level,
            granted_by: by,
            created_at: Utc::now(),
        };

        match shared {
            Shared::Dashboard(id) => {
                stored
                    .dashboard_grants
                    .entry((id.as_str().to_owned(), grantee))
                    .and_modify(|held| held.level = level)
                    .or_insert(fresh);
                stored.demote(id.as_str());
            }
            Shared::Folder(id) => {
                stored
                    .folder_grants
                    .entry((id, grantee))
                    .and_modify(|held| held.level = level)
                    .or_insert(fresh);
            }
        }

        Ok(Granting::Done)
    }

    async fn withdraw(
        &self,
        shared: Shared<'_>,
        grantee: Grantee,
        by: PersonId,
    ) -> Result<Withdrawal, DefinitionStoreError> {
        self.writable()?;
        let mut stored = self.lock();
        let (_, may_share) = stored
            .sharing_standing(shared, by)
            .ok_or_else(|| missing(shared))?;
        if !may_share {
            return Ok(Withdrawal::NotSharer);
        }

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

        Ok(Withdrawal::Done)
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
            Err(DefinitionStoreError::TagsFull) => return Ok(Handover::RecipientFull),
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
        let held = stored
            .folders
            .values()
            .filter(|kept| kept.owner == Owner::Person(to))
            .count();
        let Some(kept) = stored.folders.get_mut(&folder) else {
            return Ok(Handover::OwnerChanged);
        };
        if kept.owner != Owner::Person(from) {
            return Ok(Handover::OwnerChanged);
        }
        if held >= MAX_FOLDERS {
            return Ok(Handover::RecipientFull);
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
