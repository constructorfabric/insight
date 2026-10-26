//! The widgets a dashboard draws, reached through the dashboard so that
//! whoever sees it can draw it without reading the library.

use serde_json::Value;

use super::{CustomError, Surfaces, not_found};
use crate::domain::definition::{DefinitionId, DefinitionKind};
use crate::domain::kinds;
use crate::domain::kinds::metric::answerable::EffectiveClock;

/// One widget a dashboard draws, as its reader is given it.
#[derive(Debug)]
pub(crate) struct DrawnWidget {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) body: Value,
    /// The clock of the metric it draws, which decides whether it follows
    /// the board's window.
    pub(crate) clock: Option<EffectiveClock>,
}

/// Which metric of a drawn widget a reader asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Reading {
    /// The metric it draws.
    Drawn,
    /// Its detail, or the metric it draws when it names none.
    Drilled,
}

impl Surfaces<'_> {
    /// Every widget a dashboard the caller sees draws, each once and in its
    /// order. One it names that is no longer stored is left out.
    pub(crate) async fn drawn_widgets(
        &self,
        dashboard: &DefinitionId,
    ) -> Result<Vec<DrawnWidget>, CustomError> {
        let body = self.dashboard_body(dashboard).await?;
        let mut drawn = Vec::new();

        for named in kinds::dashboard::drawn(&body) {
            let Ok(id) = DefinitionId::parse(&named) else {
                continue;
            };
            let Some(stored) = self
                .definitions
                .read(DefinitionKind::Widget, &id)
                .await
                .map_err(CustomError::Store)?
            else {
                continue;
            };
            let clock = self.drawn_clock(&stored.body).await?;

            drawn.push(DrawnWidget {
                id: named,
                name: stored.name,
                body: stored.body,
                clock,
            });
        }

        Ok(drawn)
    }

    /// The metric a reader of a dashboard the caller sees asks one of its
    /// widgets for. A widget the dashboard does not draw is not found.
    pub(crate) async fn drawn_metric(
        &self,
        dashboard: &DefinitionId,
        widget: &DefinitionId,
        reading: Reading,
    ) -> Result<DefinitionId, CustomError> {
        let body = self.dashboard_body(dashboard).await?;
        let missing = || not_found(DefinitionKind::Widget, widget);
        if !kinds::dashboard::drawn(&body)
            .iter()
            .any(|named| named == widget.as_str())
        {
            return Err(missing());
        }

        let stored = self
            .definitions
            .get(DefinitionKind::Widget, widget)
            .await
            .map_err(CustomError::Store)?
            .ok_or_else(missing)?;
        let named = match reading {
            Reading::Drawn => kinds::widget::drawn_metric(&stored),
            Reading::Drilled => kinds::widget::drilled_metric(&stored),
        };

        named
            .and_then(|named| DefinitionId::parse(named).ok())
            .ok_or_else(|| CustomError::NotFound {
                kind: DefinitionKind::Metric,
                id: named.unwrap_or_default().to_owned(),
            })
    }

    async fn dashboard_body(&self, dashboard: &DefinitionId) -> Result<Value, CustomError> {
        self.seen(dashboard).await?;

        self.definitions
            .get(DefinitionKind::Dashboard, dashboard)
            .await
            .map_err(CustomError::Store)?
            .ok_or_else(|| not_found(DefinitionKind::Dashboard, dashboard))
    }

    async fn drawn_clock(&self, widget: &Value) -> Result<Option<EffectiveClock>, CustomError> {
        let Some(metric) =
            kinds::widget::drawn_metric(widget).and_then(|named| DefinitionId::parse(named).ok())
        else {
            return Ok(None);
        };
        let Some(body) = self
            .definitions
            .get(DefinitionKind::Metric, &metric)
            .await
            .map_err(CustomError::Store)?
        else {
            return Ok(None);
        };

        Ok(self.clock_of(DefinitionKind::Metric, &body).await)
    }
}
