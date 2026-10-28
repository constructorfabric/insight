use async_trait::async_trait;

use super::{MemoryDefinitions, Stored};
use crate::domain::access::Caller;
use crate::domain::definition::{DefinitionId, DefinitionKind, Listed};
use crate::domain::pins::{MAX_PINS, PinError, Pins};

impl Stored {
    pub(super) fn drop_pins(&mut self, dashboard: &str) {
        for pins in self.pinned.values_mut() {
            pins.retain(|held| held != dashboard);
        }
    }

    fn visible_pins(&self, caller: Caller) -> Vec<Listed> {
        let Some(pins) = self.pinned.get(&caller.person) else {
            return Vec::new();
        };

        pins.iter()
            .filter(|id| self.visible(id, caller))
            .filter_map(|id| self.listed(DefinitionKind::Dashboard, id))
            .collect()
    }
}

#[async_trait]
impl Pins for MemoryDefinitions {
    async fn pins_of(&self, caller: Caller) -> Result<Vec<Listed>, PinError> {
        Ok(self.lock().visible_pins(caller))
    }

    async fn pin(&self, caller: Caller, dashboard: &DefinitionId) -> Result<(), PinError> {
        self.writable()?;
        let mut stored = self.lock();
        if !stored.visible(dashboard.as_str(), caller) {
            return Err(PinError::DashboardNotFound(dashboard.as_str().to_owned()));
        }

        let held = stored.visible_pins(caller).len();
        let pins = stored.pinned.entry(caller.person).or_default();
        if pins.iter().any(|pinned| pinned == dashboard.as_str()) {
            return Ok(());
        }
        if held >= MAX_PINS {
            return Err(PinError::TooMany);
        }
        pins.push(dashboard.as_str().to_owned());

        Ok(())
    }

    async fn unpin(&self, caller: Caller, dashboard: &DefinitionId) -> Result<bool, PinError> {
        self.writable()?;
        let mut stored = self.lock();
        let Some(pins) = stored.pinned.get_mut(&caller.person) else {
            return Ok(false);
        };
        let before = pins.len();
        pins.retain(|held| held != dashboard.as_str());

        Ok(pins.len() < before)
    }
}

#[cfg(test)]
mod tests;
