//! Where in a condition a refusal is about, built step by step as the
//! parse walks down.

use super::{ConditionError, Problem};

/// A path step by step, so each refusal names where it is.
#[derive(Debug, Clone, Copy)]
pub(super) struct At<'a> {
    pub(super) parent: Option<&'a At<'a>>,
    step: Step<'a>,
}

#[derive(Debug, Clone, Copy)]
enum Step<'a> {
    Root,
    Key(&'a str),
    Index(usize),
}

impl At<'_> {
    fn render(&self) -> String {
        let mut steps = Vec::new();
        let mut cursor = Some(self);
        while let Some(at) = cursor {
            steps.push(at.step);
            cursor = at.parent;
        }

        let mut path = String::new();
        for step in steps.iter().rev() {
            match step {
                Step::Root => {}
                Step::Key(key) if path.is_empty() => path.push_str(key),
                Step::Key(key) => {
                    path.push('.');
                    path.push_str(key);
                }
                Step::Index(index) => {
                    path.push('[');
                    path.push_str(&index.to_string());
                    path.push(']');
                }
            }
        }

        path
    }

    pub(super) fn key<'b>(&'b self, key: &'b str) -> At<'b> {
        At {
            parent: Some(self),
            step: Step::Key(key),
        }
    }

    pub(super) fn index(&self, index: usize) -> At<'_> {
        At {
            parent: Some(self),
            step: Step::Index(index),
        }
    }

    pub(super) fn refuse<T>(&self, problem: Problem) -> Result<T, ConditionError> {
        Err(ConditionError {
            path: self.render(),
            problem,
        })
    }
}

pub(super) const ROOT: At<'static> = At {
    parent: None,
    step: Step::Root,
};
