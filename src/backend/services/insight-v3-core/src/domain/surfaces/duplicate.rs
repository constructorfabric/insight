use super::{CustomError, Surfaces};
use crate::domain::definition::{Change, DefinitionKind, DefinitionName};

impl Surfaces<'_> {
    pub(crate) async fn duplicate(
        &self,
        from: &DefinitionName,
        to: &DefinitionName,
    ) -> Result<(), CustomError> {
        let body = self.get(DefinitionKind::Dashboard, from).await?;

        let changes = [
            Change::Create(DefinitionKind::Dashboard, to.clone(), body),
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
            .map_err(CustomError::Store)
    }
}
