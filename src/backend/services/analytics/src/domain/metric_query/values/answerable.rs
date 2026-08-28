//! Which read answers a values question, decided from the question's shape
//! alone. INVARIANT: validation and the catalogue both decide here, so every
//! combination advertised is one a request may name, and no other is.

use super::super::error::QueryError;
use super::dto::{CompareOffset, Fold, Grain};

/// INVARIANT: these enumerate their enums; a variant added and not listed here
/// is a combination no caller is ever told about.
const GRAINS: [Grain; 4] = [Grain::Total, Grain::Day, Grain::Week, Grain::Month];
const FOLDS: [Fold; 2] = [Fold::PerSubject, Fold::Combined];
const SUBJECTS: [SubjectsAsk; 2] = [SubjectsAsk::Persons, SubjectsAsk::Tenant];
const COMPARE_OFFSETS: [CompareOffset; 4] = [
    CompareOffset::PreviousPeriod,
    CompareOffset::Month,
    CompareOffset::Quarter,
    CompareOffset::Year,
];

/// Which read answers one question, decided here and nowhere else.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueryShape {
    /// One value per subject over the whole window.
    SubjectTotal,
    /// One value per subject per split group, over the whole window.
    SubjectSplit,
    /// One value per split group, folded over every subject.
    CombinedSplit,
    /// One series per subject and split group, plus the window total.
    SubjectSeries,
}

/// How many of a split's groups a question keeps.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum SplitAsk {
    None,
    EveryGroup,
    TopGroups,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum SubjectsAsk {
    Persons,
    Tenant,
}

/// A question stripped to what answerability depends on: no dates, no people,
/// no dimension names — only the shape.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Ask {
    pub grain: Grain,
    pub fold: Fold,
    pub split: SplitAsk,
    pub subjects: SubjectsAsk,
}

/// Which read answers a question of this shape, or why none does.
pub(super) fn shape_of(ask: Ask) -> Result<QueryShape, QueryError> {
    let shape = match (ask.grain, ask.fold, ask.split) {
        (Grain::Total, Fold::PerSubject, SplitAsk::None) => QueryShape::SubjectTotal,
        (Grain::Total, Fold::PerSubject, SplitAsk::EveryGroup) => QueryShape::SubjectSplit,
        (Grain::Total, Fold::PerSubject, SplitAsk::TopGroups) => {
            return Err(QueryError::Unanswerable {
                reason: "a per-subject split over the whole window reports every group; \
                         keeping only the top ones needs a time grain or a combined fold",
            });
        }
        (Grain::Total, Fold::Combined, SplitAsk::EveryGroup | SplitAsk::TopGroups) => {
            QueryShape::CombinedSplit
        }
        (Grain::Total, Fold::Combined, SplitAsk::None) => {
            return Err(QueryError::Unanswerable {
                reason: "a combined value is reported per split group, so folding every subject \
                         together names at least one dimension",
            });
        }
        (Grain::Day | Grain::Week | Grain::Month, Fold::PerSubject, _) => QueryShape::SubjectSeries,
        (Grain::Day | Grain::Week | Grain::Month, Fold::Combined, _) => {
            return Err(QueryError::Unanswerable {
                reason: "a combined value folds the window whole, so it is asked at the total \
                         grain",
            });
        }
    };

    answerable_for(ask.subjects, shape)
}

/// INVARIANT: a dataset records rows per observed person and never for the
/// tenant, so a tenant-wide question must report no subject of its own.
fn answerable_for(subjects: SubjectsAsk, shape: QueryShape) -> Result<QueryShape, QueryError> {
    match (subjects, shape) {
        (SubjectsAsk::Persons, _) | (SubjectsAsk::Tenant, QueryShape::CombinedSplit) => Ok(shape),
        (
            SubjectsAsk::Tenant,
            QueryShape::SubjectTotal | QueryShape::SubjectSplit | QueryShape::SubjectSeries,
        ) => Err(QueryError::Unanswerable {
            reason: "no dataset records a row keyed by the tenant, so a tenant-wide question is \
                     answered with its subjects folded together",
        }),
    }
}

/// Every question shape that is answerable at all for a metric, where
/// `splittable` says whether the metric declares a dimension to break out by.
fn answerable(splittable: bool) -> impl Iterator<Item = Ask> {
    let splits: &'static [SplitAsk] = if splittable {
        &[SplitAsk::None, SplitAsk::EveryGroup, SplitAsk::TopGroups]
    } else {
        &[SplitAsk::None]
    };

    GRAINS.into_iter().flat_map(move |grain| {
        FOLDS.into_iter().flat_map(move |fold| {
            splits.iter().flat_map(move |split| {
                SUBJECTS.into_iter().map(move |subjects| Ask {
                    grain,
                    fold,
                    split: *split,
                    subjects,
                })
            })
        })
    })
}

/// The grains a metric's values may be asked at.
pub(in crate::domain::metric_query) fn offered_grains(splittable: bool) -> Vec<Grain> {
    GRAINS
        .into_iter()
        .filter(|grain| {
            answerable(splittable).any(|ask| ask.grain == *grain && shape_of(ask).is_ok())
        })
        .collect()
}

/// The folds a metric's values may be asked with.
pub(in crate::domain::metric_query) fn offered_folds(splittable: bool) -> Vec<Fold> {
    FOLDS
        .into_iter()
        .filter(|fold| answerable(splittable).any(|ask| ask.fold == *fold && shape_of(ask).is_ok()))
        .collect()
}

/// The earlier windows a values question may be set beside. Every offset is a
/// property of the window asked, not of the metric.
pub(in crate::domain::metric_query) fn offered_compare_offsets() -> Vec<CompareOffset> {
    COMPARE_OFFSETS.to_vec()
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn ask(grain: Grain, fold: Fold, split: SplitAsk) -> Ask {
        Ask {
            grain,
            fold,
            split,
            subjects: SubjectsAsk::Persons,
        }
    }

    #[test]
    fn a_metric_without_a_dimension_is_asked_per_subject_only() {
        assert_eq!(offered_folds(false), vec![Fold::PerSubject]);
        assert_eq!(
            offered_grains(false),
            vec![Grain::Total, Grain::Day, Grain::Week, Grain::Month]
        );
    }

    #[test]
    fn a_dimension_is_what_makes_a_combined_fold_answerable() {
        assert_eq!(offered_folds(true), vec![Fold::PerSubject, Fold::Combined]);

        assert!(shape_of(ask(Grain::Total, Fold::Combined, SplitAsk::None)).is_err());
        assert_eq!(
            shape_of(ask(Grain::Total, Fold::Combined, SplitAsk::EveryGroup)).ok(),
            Some(QueryShape::CombinedSplit)
        );
    }

    #[test]
    fn a_per_subject_split_over_the_whole_window_keeps_every_group() {
        assert_eq!(
            shape_of(ask(Grain::Total, Fold::PerSubject, SplitAsk::EveryGroup)).ok(),
            Some(QueryShape::SubjectSplit)
        );
        assert!(shape_of(ask(Grain::Total, Fold::PerSubject, SplitAsk::TopGroups)).is_err());
    }

    #[test]
    fn only_a_combined_split_is_answered_for_the_tenant() {
        for split in [SplitAsk::None, SplitAsk::EveryGroup] {
            assert!(
                shape_of(Ask {
                    subjects: SubjectsAsk::Tenant,
                    ..ask(Grain::Total, Fold::PerSubject, split)
                })
                .is_err(),
                "should refuse: {split:?}"
            );
        }

        assert_eq!(
            shape_of(Ask {
                subjects: SubjectsAsk::Tenant,
                ..ask(Grain::Total, Fold::Combined, SplitAsk::EveryGroup)
            })
            .ok(),
            Some(QueryShape::CombinedSplit)
        );
    }
}
