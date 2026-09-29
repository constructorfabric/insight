//! Alert rules over custom metrics: what a rule says, what one check of it
//! found, and when a finding is worth a notification.
//!
//! A rule watches one number a stored metric produces. Every check runs the
//! metric exactly as an on-demand run would, reads one column of one row,
//! compares it with a threshold, and records the outcome on the rule. A
//! notification is owed on the first breach and again only after a valid
//! check has seen the condition clear; an unknown result changes nothing.

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

/// What one accepted outcome does to the rule's notification state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Transition {
    /// The condition is met and was not the last time a valid check looked:
    /// a notification is owed.
    Notify,
    /// Nothing to send: the condition still holds, has cleared, or the check
    /// could not tell.
    Silent,
}

/// The transition one outcome causes, given whether the last valid check
/// found the condition met.
///
/// INVARIANT: only a valid non-breach clears `previous`; an unknown result
/// keeps it, so a metric that stops answering during a breach neither
/// re-notifies when it returns nor reads as recovery.
pub(crate) fn transition(previous: Option<bool>, outcome: &Outcome) -> Transition {
    match outcome {
        Outcome::Valid { breached: true, .. } if previous != Some(true) => Transition::Notify,
        Outcome::Valid { .. } | Outcome::Unknown(_) => Transition::Silent,
    }
}

/// What a valid check leaves as the last valid finding, and what an unknown
/// one leaves untouched.
pub(crate) fn last_valid_breached(previous: Option<bool>, outcome: &Outcome) -> Option<bool> {
    match outcome {
        Outcome::Valid { breached, .. } => Some(*breached),
        Outcome::Unknown(_) => previous,
    }
}
