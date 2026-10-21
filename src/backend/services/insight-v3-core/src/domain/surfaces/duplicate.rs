use super::{CustomError, Surfaces};
use crate::domain::definition::{Change, DefinitionId, DefinitionKind, DefinitionLabel};

impl Surfaces<'_> {
    /// Copies a dashboard under a fresh id, with its folder and its tags.
    pub(crate) async fn duplicate(
        &self,
        from: &DefinitionId,
        name: &DefinitionLabel,
    ) -> Result<DefinitionId, CustomError> {
        let body = self.get(DefinitionKind::Dashboard, from).await?;
        let to = DefinitionId::generate();

        let changes = [
            Change::Create(DefinitionKind::Dashboard, to.clone(), name.clone(), body),
            Change::CarryFolder {
                from: from.clone(),
                to: to.clone(),
            },
            Change::CarryTags {
                from: from.clone(),
                to: to.clone(),
            },
        ];

        self.definitions
            .apply(&changes)
            .await
            .map_err(CustomError::Store)?;

        Ok(to)
    }
}
