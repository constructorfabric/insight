//! Why a question to the identity mapping went unanswered.
//!
//! One variant per read. The server's own message is logged where the read
//! failed and does not travel with the error: a caller can only decide between
//! "the mapping answered" and "it did not".

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum IdentityBindingError {
    #[error("the identity mapping could not be read")]
    MappingUnreadable,
    #[error("the identity mapping's epoch could not be read")]
    EpochUnreadable,
}
