pub(crate) mod condition;
pub(crate) mod delivery;
pub(crate) mod evaluation;
pub(crate) mod rule;
pub(crate) mod rules;
pub(crate) mod scalar;
pub(crate) mod schedule;

#[cfg(test)]
mod tests;

pub(crate) use rule::{Destinations, Limits, RuleDraft, RuleError};
pub(crate) use scalar::{Number, Outcome, UnknownReason};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Transition {
    Notify,
    Silent,
}

pub(crate) fn transition(previous: Option<bool>, outcome: &Outcome) -> Transition {
    match outcome {
        Outcome::Valid { breached: true, .. } if previous != Some(true) => Transition::Notify,
        Outcome::Valid { .. } | Outcome::Unknown(_) => Transition::Silent,
    }
}

pub(crate) fn last_valid_breached(previous: Option<bool>, outcome: &Outcome) -> Option<bool> {
    match outcome {
        Outcome::Valid { breached, .. } => Some(*breached),
        Outcome::Unknown(_) => previous,
    }
}
