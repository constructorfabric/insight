use async_trait::async_trait;
use uuid::Uuid;

use super::{MemoryDefinitions, Stored};
use crate::domain::definition::{DefinitionId, DefinitionKind, Listed};
use crate::domain::pins::{MAX_PINS, PinError, Pins};

impl Stored {
    pub(super) fn drop_pins(&mut self, dashboard: &str) {
        for pins in self.pinned.values_mut() {
            pins.retain(|held| held != dashboard);
        }
    }
}

#[async_trait]
impl Pins for MemoryDefinitions {
    async fn pins_of(&self, person: Uuid) -> Result<Vec<Listed>, PinError> {
        let stored = self.lock();
        let Some(pins) = stored.pinned.get(&person) else {
            return Ok(Vec::new());
        };

        Ok(pins
            .iter()
            .filter_map(|id| stored.listed(DefinitionKind::Dashboard, id))
            .collect())
    }

    async fn pin(&self, person: Uuid, dashboard: &DefinitionId) -> Result<(), PinError> {
        self.writable()?;
        let mut stored = self.lock();
        if !stored.has_dashboard(dashboard) {
            return Err(PinError::DashboardNotFound(dashboard.as_str().to_owned()));
        }

        let pins = stored.pinned.entry(person).or_default();
        if pins.iter().any(|held| held == dashboard.as_str()) {
            return Ok(());
        }
        if pins.len() >= MAX_PINS {
            return Err(PinError::TooMany);
        }
        pins.push(dashboard.as_str().to_owned());

        Ok(())
    }

    async fn unpin(&self, person: Uuid, dashboard: &DefinitionId) -> Result<(), PinError> {
        self.writable()?;
        let mut stored = self.lock();
        if !stored.has_dashboard(dashboard) {
            return Err(PinError::DashboardNotFound(dashboard.as_str().to_owned()));
        }

        if let Some(pins) = stored.pinned.get_mut(&person) {
            pins.retain(|held| held != dashboard.as_str());
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests;
