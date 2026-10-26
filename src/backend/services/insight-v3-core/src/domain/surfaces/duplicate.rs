use super::{CustomError, Surfaces, creating};
use crate::domain::access::Owner;
use crate::domain::definition::{Change, DefinitionId, DefinitionKind, DefinitionLabel};

impl Surfaces<'_> {
    /// Copies a dashboard the caller may change under a fresh id, as theirs:
    /// in each owned folder holding the original that they may arrange, and
    /// carrying its tags' names as their own tags.
    pub(crate) async fn duplicate(
        &self,
        from: &DefinitionId,
        name: &DefinitionLabel,
    ) -> Result<DefinitionId, CustomError> {
        self.changeable(from).await?;
        let body = self.get(DefinitionKind::Dashboard, from).await?;
        let to = DefinitionId::generate();

        let mut changes = creating(
            DefinitionKind::Dashboard,
            &to,
            name,
            body,
            self.caller.person,
        );
        changes.push(Change::CarryEntries {
            from: from.clone(),
            to: to.clone(),
            placer: self.caller,
        });
        changes.push(Change::CarryTags {
            from: from.clone(),
            to: to.clone(),
            scope: Owner::Person(self.caller.person),
        });

        self.definitions
            .apply(&changes)
            .await
            .map_err(CustomError::Store)?;

        Ok(to)
    }
}
