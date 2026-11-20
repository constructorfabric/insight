#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TableEngine {
    MergeTree,
    ReplacingMergeTree,
    Other,
}

impl TableEngine {
    pub(crate) fn parse(value: &str) -> Self {
        if value.ends_with("ReplacingMergeTree") {
            return Self::ReplacingMergeTree;
        }
        if value.ends_with("MergeTree") {
            return Self::MergeTree;
        }

        Self::Other
    }

    pub(crate) fn requires_final(self) -> bool {
        self == Self::ReplacingMergeTree
    }
}

#[cfg(test)]
mod tests {
    use super::TableEngine;

    #[test]
    fn a_replicated_replacing_table_is_read_through_final() {
        assert!(TableEngine::parse("ReplicatedReplacingMergeTree").requires_final());
    }

    #[test]
    fn a_plain_merge_tree_is_read_as_it_is() {
        assert_eq!(TableEngine::parse("MergeTree"), TableEngine::MergeTree);
    }

    #[test]
    fn a_view_is_read_as_it_is() {
        assert_eq!(TableEngine::parse("View"), TableEngine::Other);
    }
}
