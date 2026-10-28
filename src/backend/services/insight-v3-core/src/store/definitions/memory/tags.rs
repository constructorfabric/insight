use std::collections::{BTreeMap, BTreeSet};

use async_trait::async_trait;

use super::{DASHBOARDS, MemoryDefinitions, Stored, Tagged};
use crate::domain::access::{Caller, Owner};
use crate::domain::definition::{DefinitionId, DefinitionStoreError};
use crate::domain::tags::{MAX_TAGS, TagError, TagFilter, TagName, TagSet, TagSummary, Tags};

impl Stored {
    fn spelling_of(&self, scope: Owner, wanted: &TagName) -> TagName {
        let folded = wanted.folded();

        self.tagged
            .values()
            .filter(|tagged| tagged.scope == scope)
            .flat_map(|tagged| &tagged.names)
            .find(|held| held.folded() == folded)
            .unwrap_or(wanted)
            .clone()
    }

    fn tag_count(&self, scope: Owner) -> usize {
        self.tagged
            .values()
            .filter(|tagged| tagged.scope == scope)
            .flat_map(|tagged| &tagged.names)
            .map(TagName::folded)
            .collect::<BTreeSet<_>>()
            .len()
    }

    pub(super) fn carries_any(&self, dashboard: &str, filter: &TagFilter) -> bool {
        let Some(carried) = self.tagged.get(dashboard) else {
            return false;
        };

        carried.names.iter().any(|held| {
            filter
                .names()
                .iter()
                .any(|wanted| wanted.folded() == held.folded())
        })
    }

    fn tag(&mut self, dashboard: &str, scope: Owner, names: &[TagName]) {
        let spelled: Vec<TagName> = names
            .iter()
            .map(|wanted| self.spelling_of(scope, wanted))
            .collect();
        if spelled.is_empty() {
            self.tagged.remove(dashboard);
            return;
        }

        self.tagged.insert(
            dashboard.to_owned(),
            Tagged {
                scope,
                names: spelled,
            },
        );
    }

    pub(super) fn carry_tags(
        &mut self,
        from: &str,
        to: &str,
        scope: Owner,
    ) -> Result<(), DefinitionStoreError> {
        let Some(carried) = self.tagged.get(from).map(|tagged| tagged.names.clone()) else {
            return Ok(());
        };

        self.tag(to, scope, &carried);
        if self.tag_count(scope) > MAX_TAGS {
            return Err(DefinitionStoreError::TagsFull);
        }

        Ok(())
    }
}

#[async_trait]
impl Tags for MemoryDefinitions {
    async fn list_tags(&self, caller: Caller) -> Result<Vec<TagSummary>, TagError> {
        let stored = self.lock();
        let mut counted: BTreeMap<String, TagSummary> = BTreeMap::new();

        for (dashboard, tagged) in &stored.tagged {
            if !stored.visible(dashboard, caller) {
                continue;
            }
            let distinct: BTreeMap<String, &TagName> = tagged
                .names
                .iter()
                .map(|name| (name.folded(), name))
                .collect();
            for (folded, name) in distinct {
                counted
                    .entry(folded)
                    .or_insert_with(|| TagSummary {
                        name: name.clone(),
                        dashboards: 0,
                    })
                    .dashboards += 1;
            }
        }

        Ok(counted.into_values().collect())
    }

    async fn tags_of(&self, dashboard: &DefinitionId) -> Result<Vec<TagName>, TagError> {
        let mut carried = self
            .lock()
            .tagged
            .get(dashboard.as_str())
            .map(|tagged| tagged.names.clone())
            .unwrap_or_default();
        carried.sort_by_key(TagName::folded);

        Ok(carried)
    }

    async fn set_tags(
        &self,
        dashboard: &DefinitionId,
        scope: Owner,
        tags: &TagSet,
    ) -> Result<(), TagError> {
        self.writable()?;
        let mut stored = self.lock();
        if !stored
            .definitions
            .contains_key(&(DASHBOARDS, dashboard.as_str().to_owned()))
        {
            return Err(TagError::DashboardNotFound(dashboard.as_str().to_owned()));
        }

        let mut applied = stored.clone();
        applied.tag(dashboard.as_str(), scope, tags.names());
        if applied.tag_count(scope) > MAX_TAGS {
            return Err(TagError::TooMany);
        }

        *stored = applied;

        Ok(())
    }
}

#[cfg(test)]
mod tests;
