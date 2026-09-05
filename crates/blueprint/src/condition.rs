use serde::{Deserialize, Serialize};
use thiserror::Error;

use milkdrift_capability::BoundedJson;

use crate::{BindingSource, FieldId};

const MAX_PATH_SEGMENTS: usize = 32;
const MAX_CONDITION_DEPTH: usize = 16;
const MAX_CONDITION_NODES: usize = 256;

/// Error returned by bounded path or condition construction.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
#[error("invalid condition at {location}: {reason}")]
pub struct ConditionError {
    location: String,
    reason: String,
}

/// One non-executable structured-value path segment.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(
    rename_all = "snake_case",
    tag = "type",
    content = "value",
    deny_unknown_fields
)]
pub enum PathSegment {
    /// Object field selected by a validated interface-field identity.
    Field(FieldId),
    /// Bounded array index.
    Index(u16),
}

/// Safe, bounded structured-value selector.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(transparent)]
pub struct PathSelector(Vec<PathSegment>);

impl PathSelector {
    /// Constructs a selector with at most 32 segments.
    pub fn new(segments: Vec<PathSegment>) -> Result<Self, ConditionError> {
        if segments.len() > MAX_PATH_SEGMENTS {
            return Err(ConditionError {
                location: "path".to_owned(),
                reason: format!("at most {MAX_PATH_SEGMENTS} segments are allowed"),
            });
        }
        Ok(Self(segments))
    }

    /// Returns selector segments.
    #[must_use]
    pub fn segments(&self) -> &[PathSegment] {
        &self.0
    }
}

milkdrift_contracts::deserialize_via!(PathSelector, Vec<PathSegment>, |segments| Self::new(
    segments
));

/// Safe comparison operators supported by branch and repeat conditions.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Comparison {
    /// Structured equality.
    Equal,
    /// Structured inequality.
    NotEqual,
    /// Numeric less-than.
    LessThan,
    /// Numeric less-than-or-equal.
    LessThanOrEqual,
    /// Numeric greater-than.
    GreaterThan,
    /// Numeric greater-than-or-equal.
    GreaterThanOrEqual,
}

/// Operand read by a safe condition.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case", tag = "type", deny_unknown_fields)]
pub enum ConditionOperand {
    /// A small bounded literal.
    Literal {
        /// Bounded structured value.
        value: BoundedJson,
    },
    /// A declared data binding source.
    Binding {
        /// Declared source to read.
        source: BindingSource,
    },
}

/// Non-executable condition abstract syntax tree.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case", tag = "type", deny_unknown_fields)]
pub enum Condition {
    /// Boolean constant.
    Constant {
        /// Constant result.
        value: bool,
    },
    /// Every nested condition must be true.
    All {
        /// Nonempty bounded child list.
        conditions: Vec<Condition>,
    },
    /// At least one nested condition must be true.
    Any {
        /// Nonempty bounded child list.
        conditions: Vec<Condition>,
    },
    /// Logical negation.
    Not {
        /// Child condition.
        condition: Box<Condition>,
    },
    /// Compare two operands with a fixed safe operator.
    Compare {
        /// Left operand.
        left: ConditionOperand,
        /// Operator.
        comparison: Comparison,
        /// Right operand.
        right: ConditionOperand,
    },
    /// Test whether a binding resolves to a value.
    Exists {
        /// Declared source whose presence is tested.
        source: BindingSource,
    },
}

impl Condition {
    pub(crate) fn validate(&self) -> Result<(), ConditionError> {
        let mut count = 0;
        self.validate_at("condition", 0, &mut count)
    }

    fn validate_at(
        &self,
        location: &str,
        depth: usize,
        count: &mut usize,
    ) -> Result<(), ConditionError> {
        *count += 1;
        if depth > MAX_CONDITION_DEPTH || *count > MAX_CONDITION_NODES {
            return Err(ConditionError {
                location: location.to_owned(),
                reason: format!(
                    "condition exceeds depth {MAX_CONDITION_DEPTH} or {MAX_CONDITION_NODES} nodes"
                ),
            });
        }
        match self {
            Self::All { conditions } | Self::Any { conditions } => {
                if conditions.is_empty() || conditions.len() > 64 {
                    return Err(ConditionError {
                        location: location.to_owned(),
                        reason: "all/any must contain 1..=64 conditions".to_owned(),
                    });
                }
                for (index, condition) in conditions.iter().enumerate() {
                    condition.validate_at(
                        &format!("{location}.conditions[{index}]"),
                        depth + 1,
                        count,
                    )?;
                }
            }
            Self::Not { condition } => {
                condition.validate_at(&format!("{location}.not"), depth + 1, count)?;
            }
            _ => {}
        }
        Ok(())
    }
}
