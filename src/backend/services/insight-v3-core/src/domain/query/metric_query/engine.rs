//! How a relation has to be read, which decides whether a run may count a row
//! twice.

/// The engine behind a warehouse relation, as far as reading it is concerned.
///
/// A dataset's own table is a plain `MergeTree` this service made, so this is
/// only ever `Other` on a served path.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "only a dataset's own table is read now; the rest survives for the \
                  relation path its tests still cover"
    )
)]
pub(crate) enum TableEngine {
    MergeTree,
    ReplacingMergeTree,
    Other,
}

impl TableEngine {
    /// Whether a read has to ask for the merged view. Parts of a replacing
    /// relation are not duplicate-free until they merge, which nothing
    /// promises to have happened.
    pub(crate) fn requires_final(self) -> bool {
        self == Self::ReplacingMergeTree
    }
}
