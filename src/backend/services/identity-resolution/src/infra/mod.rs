//! Infrastructure adapters (DB pool, external clients).

pub mod db;
pub mod identity_evidence;
pub mod identity_inputs;
#[cfg(test)]
mod identity_inputs_live_tests;
pub mod identity_persons;
