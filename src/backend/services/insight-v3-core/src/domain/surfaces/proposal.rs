use serde_json::Value;
use thiserror::Error;

use super::{CustomError, Surfaces, creating};
use crate::chat::Proposed;
use crate::domain::definition::{
    Change, Definition, DefinitionError, DefinitionId, DefinitionKind, DefinitionLabel,
    DefinitionStoreError,
};
use crate::domain::kinds;

/// Whether an item of a proposal made a definition or rewrote one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Arrival {
    Created,
    Replaced,
}

/// One item of a stored proposal.
#[derive(Debug)]
pub(crate) struct StoredItem {
    pub(crate) kind: DefinitionKind,
    pub(crate) id: DefinitionId,
    pub(crate) name: DefinitionLabel,
    pub(crate) arrival: Arrival,
}

#[derive(Debug, Error)]
pub(crate) enum ProposalError {
    #[error(transparent)]
    Name(DefinitionError),
    #[error("{} `{raw}` was not found", kind.singular())]
    MalformedId { kind: DefinitionKind, raw: String },
    #[error(transparent)]
    Refused(CustomError),
    #[error(transparent)]
    Store(DefinitionStoreError),
}

/// One proposed item, once its name and id are known good.
#[derive(Debug)]
struct Landing {
    kind: DefinitionKind,
    id: DefinitionId,
    /// Also what the other items of the proposal call it by.
    name: DefinitionLabel,
    arrival: Arrival,
    body: Value,
}

impl Surfaces<'_> {
    /// Stores what the assistant proposed, or none of it.
    ///
    /// Every name and id is parsed and every body checked before anything is
    /// written, so one refusal leaves the reader what they had rather than
    /// half a dashboard. A dashboard it creates is the caller's; one it
    /// replaces must be one the caller may change.
    pub(crate) async fn store_proposal(
        &self,
        metric: Option<Proposed>,
        widgets: Vec<Proposed>,
        dashboard: Option<Proposed>,
    ) -> Result<Vec<StoredItem>, ProposalError> {
        let asked = metric
            .map(|proposed| (DefinitionKind::Metric, proposed))
            .into_iter()
            .chain(
                widgets
                    .into_iter()
                    .map(|proposed| (DefinitionKind::Widget, proposed)),
            )
            .chain(dashboard.map(|proposed| (DefinitionKind::Dashboard, proposed)));
        let landing = asked
            .map(|(kind, proposed)| landed(kind, proposed))
            .collect::<Result<Vec<_>, _>>()?;
        let landing = pointed_at_ids(landing);

        self.replaceable(&landing).await?;
        let batch: Vec<Definition> = landing
            .iter()
            .map(|item| Definition::new(item.kind, item.id.clone(), item.body.clone()))
            .collect();
        self.check_batch(&batch)
            .await
            .map_err(ProposalError::Refused)?;

        let mut stored = Vec::with_capacity(landing.len());
        let mut changes = Vec::new();
        for item in landing {
            stored.push(StoredItem {
                kind: item.kind,
                id: item.id.clone(),
                name: item.name.clone(),
                arrival: item.arrival,
            });
            changes.extend(self.changes_of(item));
        }
        self.definitions
            .apply(&changes)
            .await
            .map_err(ProposalError::Store)?;

        Ok(stored)
    }

    /// Refuses a proposal replacing a dashboard the caller may not change,
    /// as missing when they cannot see it.
    async fn replaceable(&self, landing: &[Landing]) -> Result<(), ProposalError> {
        let replaced = landing
            .iter()
            .filter(|item| item.arrival == Arrival::Replaced)
            .filter(|item| item.kind == DefinitionKind::Dashboard);

        for item in replaced {
            self.changeable(&item.id)
                .await
                .map_err(ProposalError::Refused)?;
        }

        Ok(())
    }

    fn changes_of(&self, item: Landing) -> Vec<Change> {
        match item.arrival {
            Arrival::Replaced => vec![Change::Replace(item.kind, item.id, item.name, item.body)],
            Arrival::Created => creating(item.kind, &item.id, &item.name, item.body, self.caller),
        }
    }
}

/// The id an item is stored under: the one it replaces, or a fresh one.
fn landed(kind: DefinitionKind, proposed: Proposed) -> Result<Landing, ProposalError> {
    let name = DefinitionLabel::parse(&proposed.name).map_err(ProposalError::Name)?;
    let (id, arrival) = match proposed.id {
        Some(raw) => {
            let Ok(id) = DefinitionId::parse(&raw) else {
                return Err(ProposalError::MalformedId { kind, raw });
            };
            (id, Arrival::Replaced)
        }
        None => (DefinitionId::generate(), Arrival::Created),
    };

    Ok(Landing {
        kind,
        id,
        name,
        arrival,
        body: proposed.body,
    })
}

/// Every body, with each reference to another item of the proposal by its
/// name now naming that item's id. A reference to anything else is an id of
/// something built already, and is left as it is.
fn pointed_at_ids(landing: Vec<Landing>) -> Vec<Landing> {
    let keys: Vec<(DefinitionKind, String, String)> = landing
        .iter()
        .map(|item| {
            let key = item.name.as_str().to_owned();
            (item.kind, key, item.id.as_str().to_owned())
        })
        .collect();

    landing
        .into_iter()
        .map(|mut item| {
            let pairs: Vec<(&str, &str)> = keys
                .iter()
                .filter(|(kind, _, _)| kinds::referred_to_by(*kind).contains(&item.kind))
                .map(|(_, key, id)| (key.as_str(), id.as_str()))
                .collect();
            item.body = kinds::rename_references(item.kind, item.body, &pairs);
            item
        })
        .collect()
}
