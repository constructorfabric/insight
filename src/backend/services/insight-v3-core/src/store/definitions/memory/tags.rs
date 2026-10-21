use std::collections::{BTreeMap, BTreeSet};

use async_trait::async_trait;

use super::{MemoryDefinitions, Stored, paged};
use crate::domain::definition::{DefinitionId, DefinitionKind, ListedPage, Page};
use crate::domain::folders::FolderFilter;
use crate::domain::tags::{MAX_TAGS, TagError, TagFilter, TagName, TagSet, TagSummary, Tags};

impl Stored {
    fn spelling_of(&self, wanted: &TagName) -> TagName {
        let folded = wanted.folded();

        self.tagged
            .values()
            .flatten()
            .find(|held| held.folded() == folded)
            .unwrap_or(wanted)
            .clone()
    }

    fn tag_count(&self) -> usize {
        self.tagged
            .values()
            .flatten()
            .map(TagName::folded)
            .collect::<BTreeSet<_>>()
            .len()
    }

    fn carries_any(&self, dashboard: &str, filter: &TagFilter) -> bool {
        let Some(carried) = self.tagged.get(dashboard) else {
            return false;
        };

        carried.iter().any(|held| {
            filter
                .names()
                .iter()
                .any(|wanted| wanted.folded() == held.folded())
        })
    }
}

#[async_trait]
impl Tags for MemoryDefinitions {
    async fn list_tags(&self) -> Result<Vec<TagSummary>, TagError> {
        let stored = self.lock();
        let mut counted: BTreeMap<String, TagSummary> = BTreeMap::new();

        for tag in stored.tagged.values().flatten() {
            counted
                .entry(tag.folded())
                .or_insert_with(|| TagSummary {
                    name: tag.clone(),
                    dashboards: 0,
                })
                .dashboards += 1;
        }

        Ok(counted.into_values().collect())
    }

    async fn tags_of(&self, dashboard: &DefinitionId) -> Result<Vec<TagName>, TagError> {
        let mut carried = self
            .lock()
            .tagged
            .get(dashboard.as_str())
            .cloned()
            .unwrap_or_default();
        carried.sort_by_key(TagName::folded);

        Ok(carried)
    }

    async fn set_tags(&self, dashboard: &DefinitionId, tags: &TagSet) -> Result<(), TagError> {
        self.writable()?;
        let mut stored = self.lock();
        if !stored.has_dashboard(dashboard) {
            return Err(TagError::DashboardNotFound(dashboard.as_str().to_owned()));
        }

        let spelled: Vec<TagName> = tags
            .names()
            .iter()
            .map(|wanted| stored.spelling_of(wanted))
            .collect();
        let mut applied = stored.clone();
        applied
            .tagged
            .insert(dashboard.as_str().to_owned(), spelled);
        if applied.tag_count() > MAX_TAGS {
            return Err(TagError::TooMany);
        }

        *stored = applied;

        Ok(())
    }

    async fn page_tagged(
        &self,
        needle: &str,
        page: Page,
        folder: Option<FolderFilter>,
        tags: &TagFilter,
    ) -> Result<ListedPage, TagError> {
        let stored = self.lock();
        let matched = stored
            .matching(DefinitionKind::Dashboard, needle)
            .into_iter()
            .filter(|listed| folder.is_none_or(|filter| stored.in_folder(&listed.id, filter)))
            .filter(|listed| stored.carries_any(&listed.id, tags))
            .collect();

        Ok(paged(matched, page))
    }
}

#[cfg(test)]
mod tests;
