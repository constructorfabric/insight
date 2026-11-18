//! The metrics a composite combines, read from the store and admitted.

use crate::domain::datasets::{self, Datasets, Ready};
use crate::domain::definition::{DefinitionKind, Lookup};
use crate::domain::kinds::KindError;
use crate::domain::kinds::metric::answerable::EffectiveClock;
use crate::domain::query::composite::{
    Admitted, Alias, Candidate, Composite, CompositeError, Stacked,
};
use crate::domain::query::metric::Metric;
use crate::domain::query::metric_query::over::Over;
use crate::domain::query::metric_query::{
    ColumnKind, CompiledQuery, MetricQuery, MetricRunError, MetricRunner, People, TableEngine,
};
use crate::domain::query::time_window::Window;

/// A composite's children, each read as the plain query it is and admitted:
/// each answers one row per key.
#[derive(Debug)]
pub(crate) struct Children {
    /// In the order the composite combines them.
    children: Vec<Child>,
    admitted: Admitted,
}

#[derive(Debug)]
struct Child {
    alias: Alias,
    query: MetricQuery,
    /// The dataset it reads, when it reads one rather than a table.
    dataset: Option<Ready>,
    /// How its table is read. A dataset's records are read the same way
    /// whatever this says.
    engine: TableEngine,
}

impl Child {
    fn candidate(&self) -> Candidate<'_> {
        let declaration = self.dataset.as_ref().map(|ready| &ready.declaration);

        Candidate {
            alias: &self.alias,
            query: &self.query,
            kinds: self.query.column_kinds(declaration, false),
            dated: self.clock().is_some(),
        }
    }

    fn clock(&self) -> Option<EffectiveClock> {
        match &self.dataset {
            Some(ready) => EffectiveClock::of(&self.query, &ready.declaration),
            None => EffectiveClock::of_table(&self.query),
        }
    }
}

/// Reads every metric a composite combines and admits them together.
///
/// A child can be edited after the composite is stored, so every run reads
/// and admits them again rather than trusting the write that came before.
pub(crate) async fn children(
    composite: &Composite,
    definitions: &dyn Lookup,
    datasets: &dyn Datasets,
) -> Result<Children, KindError> {
    let mut children = Vec::with_capacity(composite.combine().len());

    for combined in composite.combine() {
        let alias = combined.alias.as_str();
        let Some(body) = definitions
            .get(DefinitionKind::Metric, &combined.metric)
            .await
            .map_err(KindError::Store)?
        else {
            return Err(KindError::Composite(CompositeError::Missing {
                alias: alias.to_owned(),
                metric: combined.metric.as_str().to_owned(),
            }));
        };
        let query = match Metric::parse(body) {
            Ok(Metric::Query(query)) => query,
            Ok(Metric::Composite(_)) => {
                return Err(KindError::Composite(CompositeError::Nested(
                    alias.to_owned(),
                )));
            }
            Err(error) => {
                return Err(KindError::Composite(CompositeError::Unreadable {
                    alias: alias.to_owned(),
                    reason: error.to_string(),
                }));
            }
        };
        let dataset = match query.dataset() {
            Some(named) => Some(
                datasets::ready(datasets, named)
                    .await
                    .map_err(KindError::Datasets)?
                    .ok_or_else(|| KindError::DatasetNotReady(named.to_owned()))?,
            ),
            None => None,
        };

        children.push(Child {
            alias: combined.alias.clone(),
            query,
            dataset,
            engine: TableEngine::Other,
        });
    }

    let candidates: Vec<Candidate<'_>> = children.iter().map(Child::candidate).collect();
    let admitted = composite.admit(&candidates).map_err(KindError::Composite)?;

    Ok(Children { children, admitted })
}

impl Children {
    /// Whether a window over the composite has a date to select by.
    pub(crate) fn dated(&self) -> bool {
        self.admitted.dated()
    }

    /// What each result column holds, in result order.
    pub(crate) fn column_kinds(&self, clocked: bool) -> Vec<(String, ColumnKind)> {
        self.admitted.column_kinds(clocked)
    }

    /// The clock a window over the composite selects by: the first child's,
    /// when every child has one.
    pub(crate) fn clock(&self) -> Option<EffectiveClock> {
        if !self.dated() {
            return None;
        }

        self.children.first().and_then(Child::clock)
    }

    /// How each child's table is read, as the warehouse holds it now.
    pub(crate) async fn read_engines(
        &mut self,
        runner: &MetricRunner,
    ) -> Result<(), MetricRunError> {
        for child in &mut self.children {
            if child.dataset.is_none() {
                child.engine = runner
                    .engine_of(child.query.database(), child.query.table_name())
                    .await?;
            }
        }

        Ok(())
    }

    /// The composite over a window: each child compiled as a subquery, then
    /// stacked and folded. `database` is where every dataset's records are.
    pub(crate) fn compile(
        &self,
        composite: &Composite,
        people: &People,
        window: &Window,
        database: &str,
    ) -> Result<CompiledQuery, CompositeError> {
        let mut stacked = Vec::with_capacity(self.children.len());
        for child in &self.children {
            let over = child.dataset.as_ref().map(|ready| Over {
                declaration: &ready.declaration,
                database,
                table: &ready.table,
            });
            let compiled = child
                .query
                .compile_inner(people, window, child.engine, over)
                .map_err(|source| CompositeError::Compile {
                    alias: child.alias.as_str().to_owned(),
                    source,
                })?;

            stacked.push(Stacked {
                alias: &child.alias,
                query: &child.query,
                compiled,
            });
        }

        let bucketed = window.grain().is_some() && self.dated();

        Ok(composite.compile(&stacked, bucketed))
    }
}
