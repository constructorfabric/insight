//! Measure filter tree: `all` / `any` / `not` combinators over
//! `{field, op, value}` leaves, operators from a closed enum. The shape is the
//! MBQL / JSON-Logic predicate-tree family; nothing here is an invented
//! expression language. Deserialization admits only the documented keys, and
//! [`FilterTree::validate`] bounds depth and size so a stored filter can never
//! be pathological.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

/// INVARIANT: bounds checked by [`FilterTree::validate`]; the compiler may
/// recurse without its own depth guard only because every stored tree passed
/// this validation at write time.
const MAX_DEPTH: usize = 8;
const MAX_LEAVES: usize = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum FilterError {
    #[error("filter tree exceeds the maximum depth of {MAX_DEPTH}")]
    TooDeep,
    #[error("filter tree exceeds the maximum of {MAX_LEAVES} leaves")]
    TooManyLeaves,
    #[error("`all` and `any` require at least one child filter")]
    EmptyCombinator,
    #[error("filter field must be lowercase snake_case starting with [a-z]")]
    BadFieldName,
    #[error("operator requires a scalar value")]
    ScalarValueRequired,
    #[error("operator requires a non-empty list value")]
    ListValueRequired,
    #[error("operator takes no value")]
    ValueNotAllowed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FilterOp {
    Eq,
    Neq,
    Gt,
    Gte,
    Lt,
    Lte,
    In,
    NotIn,
    IsNull,
    NotNull,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Scalar {
    Bool(bool),
    Number(serde_json::Number),
    String(String),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum FilterValue {
    Scalar(Scalar),
    List(Vec<Scalar>),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FilterLeaf {
    pub field: String,
    pub op: FilterOp,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<FilterValue>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AllNode {
    pub all: Vec<FilterTree>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AnyNode {
    pub any: Vec<FilterTree>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NotNode {
    pub not: Box<FilterTree>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum FilterTree {
    All(AllNode),
    Any(AnyNode),
    Not(NotNode),
    Leaf(FilterLeaf),
}

impl FilterTree {
    /// Structural validation: bounds, combinator arity, field-name shape, and
    /// operator/value agreement. Field existence and type compatibility are
    /// the catalog-binding stage, not this one.
    pub fn validate(&self) -> Result<(), FilterError> {
        let mut leaves = 0usize;
        self.walk(0, &mut leaves)
    }

    /// Every field the tree references, deduplicated, for catalog binding.
    pub fn referenced_fields(&self) -> BTreeSet<&str> {
        let mut fields = BTreeSet::new();
        self.collect_fields(&mut fields);
        fields
    }

    fn walk(&self, depth: usize, leaves: &mut usize) -> Result<(), FilterError> {
        if depth >= MAX_DEPTH {
            return Err(FilterError::TooDeep);
        }
        match self {
            FilterTree::All(AllNode { all: children })
            | FilterTree::Any(AnyNode { any: children }) => {
                if children.is_empty() {
                    return Err(FilterError::EmptyCombinator);
                }
                for child in children {
                    child.walk(depth + 1, leaves)?;
                }
                Ok(())
            }
            FilterTree::Not(NotNode { not }) => not.walk(depth + 1, leaves),
            FilterTree::Leaf(leaf) => {
                *leaves += 1;
                if *leaves > MAX_LEAVES {
                    return Err(FilterError::TooManyLeaves);
                }
                leaf.validate()
            }
        }
    }

    fn collect_fields<'a>(&'a self, fields: &mut BTreeSet<&'a str>) {
        match self {
            FilterTree::All(AllNode { all: children })
            | FilterTree::Any(AnyNode { any: children }) => {
                for child in children {
                    child.collect_fields(fields);
                }
            }
            FilterTree::Not(NotNode { not }) => not.collect_fields(fields),
            FilterTree::Leaf(leaf) => {
                fields.insert(leaf.field.as_str());
            }
        }
    }
}

impl FilterLeaf {
    fn validate(&self) -> Result<(), FilterError> {
        if !is_snake_case_field(&self.field) {
            return Err(FilterError::BadFieldName);
        }
        match self.op {
            FilterOp::Eq
            | FilterOp::Neq
            | FilterOp::Gt
            | FilterOp::Gte
            | FilterOp::Lt
            | FilterOp::Lte => match &self.value {
                Some(FilterValue::Scalar(_)) => Ok(()),
                _ => Err(FilterError::ScalarValueRequired),
            },
            FilterOp::In | FilterOp::NotIn => match &self.value {
                Some(FilterValue::List(items)) if !items.is_empty() => Ok(()),
                _ => Err(FilterError::ListValueRequired),
            },
            FilterOp::IsNull | FilterOp::NotNull => match &self.value {
                None => Ok(()),
                Some(_) => Err(FilterError::ValueNotAllowed),
            },
        }
    }
}

fn is_snake_case_field(field: &str) -> bool {
    let mut chars = field.chars();
    match chars.next() {
        Some(first) if first.is_ascii_lowercase() => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(yaml: &str) -> Result<FilterTree, serde_yaml::Error> {
        serde_yaml::from_str(yaml)
    }

    fn parse_valid(yaml: &str) -> FilterTree {
        let tree = parse(yaml).expect("deserializes");
        tree.validate().expect("validates");
        tree
    }

    #[test]
    fn spec_example_deserializes_and_validates() {
        let tree = parse_valid(
            r"
all:
  - { field: state, op: eq, value: merged }
  - { field: lines_changed, op: gte, value: 500 }
",
        );
        assert_eq!(
            tree.referenced_fields().into_iter().collect::<Vec<_>>(),
            vec!["lines_changed", "state"]
        );
    }

    #[test]
    fn nested_combinators_and_null_ops() {
        parse_valid(
            r"
any:
  - not:
      all:
        - { field: closed_at, op: not_null }
  - { field: state, op: in, value: [open, draft] }
",
        );
    }

    #[test]
    fn unknown_key_is_rejected_at_deserialization() {
        assert!(parse("{ field: a, op: eq, value: 1, extra: 2 }").is_err());
        assert!(parse("{ all: [], also: [] }").is_err());
    }

    #[test]
    fn unknown_operator_is_rejected() {
        assert!(parse("{ field: a, op: like, value: x }").is_err());
    }

    #[test]
    fn empty_combinator_is_rejected() {
        let tree = parse("{ all: [] }").expect("shape deserializes");
        assert_eq!(tree.validate(), Err(FilterError::EmptyCombinator));
    }

    #[test]
    fn scalar_operator_rejects_list_and_missing_values() {
        let list = parse("{ field: a, op: eq, value: [1, 2] }").unwrap();
        assert_eq!(list.validate(), Err(FilterError::ScalarValueRequired));
        let missing = parse("{ field: a, op: eq }").unwrap();
        assert_eq!(missing.validate(), Err(FilterError::ScalarValueRequired));
    }

    #[test]
    fn list_operator_rejects_scalar_and_empty_values() {
        let scalar = parse("{ field: a, op: in, value: 1 }").unwrap();
        assert_eq!(scalar.validate(), Err(FilterError::ListValueRequired));
        let empty = parse("{ field: a, op: in, value: [] }").unwrap();
        assert_eq!(empty.validate(), Err(FilterError::ListValueRequired));
    }

    #[test]
    fn null_operator_rejects_a_value() {
        let tree = parse("{ field: a, op: is_null, value: 1 }").unwrap();
        assert_eq!(tree.validate(), Err(FilterError::ValueNotAllowed));
    }

    #[test]
    fn bad_field_names_are_rejected() {
        for field in ["State", "_x", "1a", "a-b", "a.b", ""] {
            let tree = FilterTree::Leaf(FilterLeaf {
                field: field.to_owned(),
                op: FilterOp::NotNull,
                value: None,
            });
            assert_eq!(tree.validate(), Err(FilterError::BadFieldName), "{field}");
        }
    }

    #[test]
    fn depth_cap_holds() {
        let mut tree = FilterTree::Leaf(FilterLeaf {
            field: "a".to_owned(),
            op: FilterOp::NotNull,
            value: None,
        });
        for _ in 0..MAX_DEPTH {
            tree = FilterTree::Not(NotNode {
                not: Box::new(tree),
            });
        }
        assert_eq!(tree.validate(), Err(FilterError::TooDeep));
    }

    #[test]
    fn leaf_cap_holds() {
        let leaf = |i: usize| {
            FilterTree::Leaf(FilterLeaf {
                field: format!("f{i}"),
                op: FilterOp::NotNull,
                value: None,
            })
        };
        let tree = FilterTree::All(AllNode {
            all: (0..=MAX_LEAVES).map(leaf).collect(),
        });
        assert_eq!(tree.validate(), Err(FilterError::TooManyLeaves));
    }

    #[test]
    fn serialization_round_trips() {
        let tree = parse_valid("{ field: lines_changed, op: gte, value: 500 }");
        let json = serde_json::to_string(&tree).expect("serializes");
        let back: FilterTree = serde_json::from_str(&json).expect("round-trips");
        assert_eq!(back, tree);
    }
}
