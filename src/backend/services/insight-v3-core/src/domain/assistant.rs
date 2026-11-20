use super::access::Caller;
use super::dashboards::DashboardQuery;
use super::datasets::{self, Datasets};
use super::definition::{
    DefinitionKind, DefinitionStoreError, Listed, ListedPage, MAX_PAGE_LIMIT, Page,
};
use super::folders::DefinitionStore;
use super::kinds::dataset::declaration::Declaration;
use super::kinds::dataset::describe::{describe, describe_all};
use crate::chat::{Catalogue, Schemas};

#[derive(Debug, Default)]
pub(crate) struct Briefing {
    pub(crate) datasets: String,
    pub(crate) catalogue: Catalogue,
    pub(crate) allowed: Vec<String>,
}

#[derive(Debug)]
pub(crate) struct Assistant<'a> {
    definitions: &'a dyn DefinitionStore,
    datasets: &'a dyn Datasets,
}

impl<'a> Assistant<'a> {
    pub(crate) fn new(definitions: &'a dyn DefinitionStore, datasets: &'a dyn Datasets) -> Self {
        Self {
            definitions,
            datasets,
        }
    }

    pub(crate) async fn briefing(&self, caller: Caller) -> Briefing {
        let declared = self.declared().await;

        Briefing {
            allowed: declared.iter().map(|(name, _)| name.clone()).collect(),
            datasets: describe_all(&declared),
            catalogue: self.catalogue(caller).await,
        }
    }

    pub(crate) async fn described_datasets(&self) -> String {
        describe_all(&self.declared().await)
    }

    async fn declared(&self) -> Vec<(String, Declaration)> {
        let names = match self.datasets.list().await {
            Ok(names) => names,
            Err(error) => {
                tracing::warn!(error = ?error, "could not list the datasets for the chat");
                return Vec::new();
            }
        };

        let mut declared = Vec::with_capacity(names.len());
        for name in names {
            if let Some(held) = self.ready(&name).await {
                declared.push((name, held));
            }
        }

        declared
    }

    async fn ready(&self, name: &str) -> Option<Declaration> {
        match datasets::ready(self.datasets, name).await {
            Ok(held) => Some(held?.declaration),
            Err(error) => {
                tracing::warn!(error = ?error, name, "could not read a dataset for the chat");
                None
            }
        }
    }

    async fn catalogue(&self, caller: Caller) -> Catalogue {
        let mut built = Vec::with_capacity(DefinitionKind::ALL.len());

        for kind in DefinitionKind::ALL {
            built.push((kind, self.listed(kind, caller).await));
        }

        Catalogue::new(built)
    }

    async fn page_of(
        &self,
        kind: DefinitionKind,
        caller: Caller,
        page: Page,
    ) -> Result<ListedPage, DefinitionStoreError> {
        if let Some(library) = kind.library() {
            return self.definitions.page(library, "", page).await;
        }

        let found = self
            .definitions
            .page_dashboards(caller, &DashboardQuery::default(), page)
            .await?;

        Ok(ListedPage {
            items: found
                .items
                .into_iter()
                .map(|dashboard| Listed {
                    id: dashboard.id,
                    name: dashboard.name,
                })
                .collect(),
            total: found.total,
        })
    }

    async fn listed(&self, kind: DefinitionKind, caller: Caller) -> Vec<Listed> {
        let mut listed = Vec::new();

        loop {
            let offset = listed.len() as u64;
            let page = match Page::parse(Some(MAX_PAGE_LIMIT), Some(offset)) {
                Ok(page) => page,
                Err(error) => {
                    tracing::warn!(%error, ?kind, "could not page definitions for the chat");
                    return listed;
                }
            };
            let found = match self.page_of(kind, caller, page).await {
                Ok(found) => found,
                Err(error) => {
                    tracing::warn!(error = ?error, ?kind, "could not list definitions for the chat");
                    return listed;
                }
            };

            let read = found.items.len();
            listed.extend(found.items);
            if read == 0 || listed.len() as u64 >= found.total {
                return listed;
            }
        }
    }
}

#[derive(Debug)]
pub(crate) struct DatasetSchemas<'a> {
    datasets: &'a dyn Datasets,
    definitions: &'a dyn DefinitionStore,
}

impl<'a> DatasetSchemas<'a> {
    pub(crate) fn new(datasets: &'a dyn Datasets, definitions: &'a dyn DefinitionStore) -> Self {
        Self {
            datasets,
            definitions,
        }
    }
}

#[async_trait::async_trait]
impl Schemas for DatasetSchemas<'_> {
    async fn describe(&self, datasets: &[String]) -> String {
        let assistant = Assistant::new(self.definitions, self.datasets);
        let mut written = String::new();

        for name in datasets {
            if let Some(declaration) = assistant.ready(name).await {
                written.push_str(&describe(name, &declaration));
            } else {
                written.push_str(name);
                written.push_str(": no dataset of that name is ready\n");
            }
        }

        written
    }
}
