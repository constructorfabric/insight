use async_trait::async_trait;
use uuid::Uuid;

use super::{MemoryDefinitions, Stored};
use crate::domain::definition::DefinitionName;
use crate::domain::pins::{MAX_PINS, PinError, Pins};

impl Stored {
    pub(super) fn carry_pins(&mut self, from: &DefinitionName, to: &DefinitionName) {
        for pins in self.pinned.values_mut() {
            if let Some(place) = pins.iter().position(|held| held == from.as_str()) {
                pins.insert(place + 1, to.as_str().to_owned());
            }
        }
    }

    pub(super) fn drop_pins(&mut self, dashboard: &str) {
        for pins in self.pinned.values_mut() {
            pins.retain(|held| held != dashboard);
        }
    }
}

#[async_trait]
impl Pins for MemoryDefinitions {
    async fn pins_of(&self, person: Uuid) -> Result<Vec<String>, PinError> {
        Ok(self.lock().pinned.get(&person).cloned().unwrap_or_default())
    }

    async fn pin(&self, person: Uuid, dashboard: &DefinitionName) -> Result<(), PinError> {
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

    async fn unpin(&self, person: Uuid, dashboard: &DefinitionName) -> Result<(), PinError> {
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
